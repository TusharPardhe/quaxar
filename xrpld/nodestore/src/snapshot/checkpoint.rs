//! Durable activation record for a verified bootable snapshot.
//!
//! Snapshot node objects are imported into the NodeStore first. Only after the
//! loader has verified the file, ledger header, and both SHAMap roots does the
//! CLI publish this compact record. Startup can then reconstruct a lazy
//! NodeStore-backed ledger without requiring a relational `Ledgers` row.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use basics::base_uint::Uint256;
use basics::sha_map_hash::SHAMapHash;
use protocol::{LedgerHeader, calculate_ledger_hash};

use super::{SnapshotError, SnapshotManifest, manifest::SNAPSHOT_HEADER_SIZE};

/// Name of the activation record stored beside the configured NodeStore path.
pub const SNAPSHOT_BOOTSTRAP_FILENAME: &str = "snapshot-bootstrap.v1";

const SNAPSHOT_BOOTSTRAP_MAGIC: &[u8; 8] = b"xrplb\x00\x01\x00";
const SNAPSHOT_BOOTSTRAP_VERSION: u16 = 1;
const SNAPSHOT_BOOTSTRAP_HEADER_SIZE: usize = 16 + SNAPSHOT_HEADER_SIZE;
const NETWORK_ID_PRESENT: u16 = 1;

/// A verified snapshot that has been atomically activated for the next boot.
#[derive(Debug, Clone)]
pub struct SnapshotBootstrap {
    manifest: SnapshotManifest,
    network_id: Option<u32>,
}

impl SnapshotBootstrap {
    /// The verified snapshot manifest.
    pub fn manifest(&self) -> &SnapshotManifest {
        &self.manifest
    }

    /// The network configured when the snapshot was imported, if one was set.
    pub fn network_id(&self) -> Option<u32> {
        self.network_id
    }

    /// Reconstruct the exact ledger header represented by this checkpoint.
    pub fn ledger_header(&self) -> LedgerHeader {
        LedgerHeader {
            hash: SHAMapHash::new(Uint256::from_array(self.manifest.ledger_hash)),
            seq: self.manifest.ledger_seq,
            drops: self.manifest.drops,
            parent_hash: SHAMapHash::new(Uint256::from_array(self.manifest.parent_hash)),
            tx_hash: SHAMapHash::new(Uint256::from_array(self.manifest.tx_hash)),
            account_hash: SHAMapHash::new(Uint256::from_array(self.manifest.account_hash)),
            parent_close_time: self.manifest.parent_close_time,
            close_time: self.manifest.close_time,
            close_time_resolution: self.manifest.close_time_res,
            close_flags: self.manifest.close_flags,
            ..LedgerHeader::default()
        }
    }
}

/// Return the checkpoint path for a configured NodeStore directory.
pub fn snapshot_bootstrap_path(node_store_path: impl AsRef<Path>) -> PathBuf {
    node_store_path.as_ref().join(SNAPSHOT_BOOTSTRAP_FILENAME)
}

/// Atomically activate a manifest that has already passed snapshot import
/// verification. The activation record is the commit point: absent or invalid
/// records are ignored at startup.
pub fn activate_snapshot_bootstrap(
    node_store_path: impl AsRef<Path>,
    manifest: &SnapshotManifest,
    network_id: Option<u32>,
) -> Result<(), SnapshotError> {
    validate_manifest_ledger_hash(manifest)?;
    if manifest.network_id != network_id {
        return Err(SnapshotError::InvalidBootstrapRecord {
            reason: "checkpoint network identity does not match the snapshot header".to_owned(),
        });
    }

    let directory = node_store_path.as_ref();
    fs::create_dir_all(directory).map_err(|error| {
        SnapshotError::io_path("creating snapshot checkpoint directory", directory, error)
    })?;

    let final_path = snapshot_bootstrap_path(directory);
    let temporary_path = directory.join(format!(
        ".{SNAPSHOT_BOOTSTRAP_FILENAME}.{}.tmp",
        std::process::id()
    ));
    let backup_path = directory.join(format!(
        ".{SNAPSHOT_BOOTSTRAP_FILENAME}.{}.previous",
        std::process::id()
    ));
    let _ = fs::remove_file(&temporary_path);
    let _ = fs::remove_file(&backup_path);

    // Preserve the previous activation record before replacement. A hard link
    // is atomic and avoids copying credentials or mutable state; it lets us
    // restore a known-good checkpoint if the post-rename directory sync fails.
    let had_previous_checkpoint = final_path.exists();
    if had_previous_checkpoint {
        fs::hard_link(&final_path, &backup_path).map_err(|error| {
            SnapshotError::io_path("backing up active snapshot checkpoint", &backup_path, error)
        })?;
        if let Err(error) = sync_checkpoint_directory(directory) {
            let _ = fs::remove_file(&backup_path);
            return Err(SnapshotError::io_path(
                "syncing snapshot checkpoint backup",
                directory,
                error,
            ));
        }
    }

    let mut encoded = [0_u8; SNAPSHOT_BOOTSTRAP_HEADER_SIZE];
    encoded[..8].copy_from_slice(SNAPSHOT_BOOTSTRAP_MAGIC);
    encoded[8..10].copy_from_slice(&SNAPSHOT_BOOTSTRAP_VERSION.to_be_bytes());
    let flags = if network_id.is_some() {
        NETWORK_ID_PRESENT
    } else {
        0
    };
    encoded[10..12].copy_from_slice(&flags.to_be_bytes());
    encoded[12..16].copy_from_slice(&network_id.unwrap_or_default().to_be_bytes());

    let mut header_manifest = manifest.clone();
    header_manifest.chunks.clear();
    encoded[16..].copy_from_slice(&header_manifest.serialize_header());

    let write_result = (|| -> Result<(), SnapshotError> {
        let mut file = File::create(&temporary_path).map_err(|error| {
            SnapshotError::io_path(
                "creating temporary snapshot checkpoint",
                &temporary_path,
                error,
            )
        })?;
        file.write_all(&encoded).map_err(|error| {
            SnapshotError::io_path(
                "writing temporary snapshot checkpoint",
                &temporary_path,
                error,
            )
        })?;
        file.sync_all().map_err(|error| {
            SnapshotError::io_path(
                "syncing temporary snapshot checkpoint",
                &temporary_path,
                error,
            )
        })?;
        fs::rename(&temporary_path, &final_path).map_err(|error| {
            SnapshotError::io_path("activating snapshot checkpoint", &final_path, error)
        })?;
        if let Err(sync_error) = sync_checkpoint_directory(directory) {
            let rollback_result = if had_previous_checkpoint {
                fs::rename(&backup_path, &final_path)
                    .and_then(|()| sync_checkpoint_directory(directory))
            } else {
                fs::remove_file(&final_path).and_then(|()| sync_checkpoint_directory(directory))
            };
            return match rollback_result {
                Ok(()) => Err(SnapshotError::io_path(
                    "syncing activated snapshot checkpoint",
                    directory,
                    sync_error,
                )),
                Err(rollback_error) => Err(SnapshotError::activation_state_uncertain(format!(
                    "activation directory sync failed: {sync_error}; rollback could not be confirmed: {rollback_error}"
                ))),
            };
        }
        if had_previous_checkpoint {
            // A leftover backup is safe but undesirable. Its deletion is
            // best-effort because the active record is already durable.
            let _ = fs::remove_file(&backup_path);
            let _ = sync_checkpoint_directory(directory);
        }
        Ok(())
    })();

    if write_result.is_err() {
        let _ = fs::remove_file(&temporary_path);
        if !write_result
            .as_ref()
            .expect_err("error branch must contain an error")
            .is_activation_state_uncertain()
        {
            let _ = fs::remove_file(&backup_path);
        }
    }
    write_result
}

fn sync_checkpoint_directory(directory: &Path) -> Result<(), std::io::Error> {
    File::open(directory)?.sync_all()
}

/// Load the active checkpoint if and only if a complete, valid activation
/// record exists. A missing record means that no imported snapshot may boot.
pub fn load_snapshot_bootstrap(
    node_store_path: impl AsRef<Path>,
) -> Result<Option<SnapshotBootstrap>, SnapshotError> {
    let path = snapshot_bootstrap_path(node_store_path);
    let mut file = match File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(SnapshotError::io_path(
                "opening snapshot checkpoint",
                path,
                error,
            ));
        }
    };

    let mut encoded = Vec::new();
    file.read_to_end(&mut encoded)
        .map_err(|error| SnapshotError::io_path("reading snapshot checkpoint", &path, error))?;
    if encoded.len() != SNAPSHOT_BOOTSTRAP_HEADER_SIZE {
        return Err(SnapshotError::InvalidBootstrapRecord {
            reason: format!(
                "record size is {}, expected {SNAPSHOT_BOOTSTRAP_HEADER_SIZE}",
                encoded.len()
            ),
        });
    }
    if encoded[..8] != *SNAPSHOT_BOOTSTRAP_MAGIC {
        return Err(SnapshotError::InvalidBootstrapRecord {
            reason: "incorrect checkpoint magic".to_owned(),
        });
    }

    let version = u16::from_be_bytes(encoded[8..10].try_into().expect("fixed width"));
    if version != SNAPSHOT_BOOTSTRAP_VERSION {
        return Err(SnapshotError::InvalidBootstrapRecord {
            reason: format!("unsupported checkpoint version {version}"),
        });
    }
    let flags = u16::from_be_bytes(encoded[10..12].try_into().expect("fixed width"));
    if flags & !NETWORK_ID_PRESENT != 0 {
        return Err(SnapshotError::InvalidBootstrapRecord {
            reason: "unknown checkpoint flags".to_owned(),
        });
    }
    let network = u32::from_be_bytes(encoded[12..16].try_into().expect("fixed width"));
    let record_network_id = (flags & NETWORK_ID_PRESENT != 0).then_some(network);
    let manifest = SnapshotManifest::deserialize_header(&encoded[16..])?;
    validate_manifest_ledger_hash(&manifest)?;
    if manifest.network_id != record_network_id {
        return Err(SnapshotError::InvalidBootstrapRecord {
            reason: "checkpoint network identity does not match the snapshot header".to_owned(),
        });
    }

    Ok(Some(SnapshotBootstrap {
        manifest,
        network_id: record_network_id,
    }))
}

fn validate_manifest_ledger_hash(manifest: &SnapshotManifest) -> Result<(), SnapshotError> {
    let header = LedgerHeader {
        seq: manifest.ledger_seq,
        drops: manifest.drops,
        parent_hash: SHAMapHash::new(Uint256::from_array(manifest.parent_hash)),
        tx_hash: SHAMapHash::new(Uint256::from_array(manifest.tx_hash)),
        account_hash: SHAMapHash::new(Uint256::from_array(manifest.account_hash)),
        parent_close_time: manifest.parent_close_time,
        close_time: manifest.close_time,
        close_time_resolution: manifest.close_time_res,
        close_flags: manifest.close_flags,
        ..LedgerHeader::default()
    };
    let computed = *calculate_ledger_hash(&header).as_uint256().data();
    if computed == manifest.ledger_hash {
        return Ok(());
    }
    Err(SnapshotError::LedgerHashMismatch {
        expected_hex: Uint256::from_array(manifest.ledger_hash).to_string(),
        computed_hex: Uint256::from_array(computed).to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::manifest::SNAPSHOT_VERSION;

    fn manifest() -> SnapshotManifest {
        let mut manifest = SnapshotManifest {
            version: SNAPSHOT_VERSION,
            ledger_seq: 42,
            ledger_hash: [0; 32],
            account_hash: [0xA1; 32],
            tx_hash: [0xB2; 32],
            parent_hash: [0xC3; 32],
            drops: 99_000_000,
            close_time: 750_000_000,
            parent_close_time: 749_999_990,
            close_time_res: 10,
            close_flags: 0,
            network_id: Some(21_338),
            chunks: Vec::new(),
        };
        let header = LedgerHeader {
            seq: manifest.ledger_seq,
            drops: manifest.drops,
            parent_hash: SHAMapHash::new(Uint256::from_array(manifest.parent_hash)),
            tx_hash: SHAMapHash::new(Uint256::from_array(manifest.tx_hash)),
            account_hash: SHAMapHash::new(Uint256::from_array(manifest.account_hash)),
            parent_close_time: manifest.parent_close_time,
            close_time: manifest.close_time,
            close_time_resolution: manifest.close_time_res,
            close_flags: manifest.close_flags,
            ..LedgerHeader::default()
        };
        manifest.ledger_hash = *calculate_ledger_hash(&header).as_uint256().data();
        manifest
    }

    #[test]
    fn activated_checkpoint_round_trips_manifest_and_network() {
        let directory = tempfile::tempdir().expect("tempdir");
        let manifest = manifest();
        activate_snapshot_bootstrap(directory.path(), &manifest, Some(21_338))
            .expect("activation succeeds");

        let checkpoint = load_snapshot_bootstrap(directory.path())
            .expect("checkpoint read")
            .expect("checkpoint exists");
        assert_eq!(checkpoint.network_id(), Some(21_338));
        assert_eq!(checkpoint.manifest().ledger_hash, manifest.ledger_hash);
        assert_eq!(checkpoint.ledger_header().seq, manifest.ledger_seq);
        assert_eq!(
            checkpoint.ledger_header().account_hash.as_uint256().data(),
            &manifest.account_hash
        );
    }

    #[test]
    fn failed_replacement_preserves_the_existing_checkpoint() {
        let directory = tempfile::tempdir().expect("tempdir");
        let original = manifest();
        activate_snapshot_bootstrap(directory.path(), &original, Some(21_338))
            .expect("initial activation succeeds");

        let mut replacement = original.clone();
        replacement.ledger_hash = [0xFF; 32];
        assert!(matches!(
            activate_snapshot_bootstrap(directory.path(), &replacement, Some(21_338)),
            Err(SnapshotError::LedgerHashMismatch { .. })
        ));

        let active = load_snapshot_bootstrap(directory.path())
            .expect("existing checkpoint remains readable")
            .expect("existing checkpoint remains active");
        assert_eq!(active.manifest().ledger_hash, original.ledger_hash);
    }

    #[test]
    fn activation_rejects_network_identity_mismatch() {
        let directory = tempfile::tempdir().expect("tempdir");
        let manifest = manifest();

        assert!(matches!(
            activate_snapshot_bootstrap(directory.path(), &manifest, Some(1)),
            Err(SnapshotError::InvalidBootstrapRecord { .. })
        ));
        assert!(
            load_snapshot_bootstrap(directory.path())
                .expect("no checkpoint was published")
                .is_none()
        );
    }

    #[test]
    fn invalid_manifest_is_never_activated() {
        let directory = tempfile::tempdir().expect("tempdir");
        let mut manifest = manifest();
        manifest.ledger_hash = [0xFF; 32];

        assert!(matches!(
            activate_snapshot_bootstrap(directory.path(), &manifest, None),
            Err(SnapshotError::LedgerHashMismatch { .. })
        ));
        assert!(
            load_snapshot_bootstrap(directory.path())
                .expect("missing checkpoint is harmless")
                .is_none()
        );
    }

    #[test]
    fn corrupt_checkpoint_is_rejected() {
        let directory = tempfile::tempdir().expect("tempdir");
        std::fs::write(snapshot_bootstrap_path(directory.path()), b"torn")
            .expect("write corrupt record");

        assert!(matches!(
            load_snapshot_bootstrap(directory.path()),
            Err(SnapshotError::InvalidBootstrapRecord { .. })
        ));
    }
}
