use std::collections::HashSet;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use sha2::{Digest, Sha256};
use shamap::nodes::tree_node::{BRANCH_FACTOR, SHAMapNodeType, SHAMapTreeNode};

use super::{SnapshotError, manifest::*};
use crate::{Backend, Batch, NodeObject, NodeObjectType};
use basics::base_uint::Uint256;
use basics::sha_map_hash::SHAMapHash;
use protocol::{LedgerHeader, calculate_ledger_hash};

const MAX_SHAMAP_DEPTH: usize = 64;

/// Structured, monotonic import updates for CLI and operator integrations.
/// Events are emitted only after their named operation has completed, except
/// for phase-start events such as `FinalizingNodeStore` and `VerifyingFileHash`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotImportProgress {
    HeaderValidated {
        ledger_seq: u32,
        chunk_count: usize,
    },
    ChunkTableRead {
        chunk_count: usize,
        compressed_bytes: u64,
    },
    BulkImportStarted {
        estimated_nodes: u64,
    },
    ChunkImported {
        chunk_index: usize,
        chunk_count: usize,
        nodes_loaded: u64,
        compressed_bytes_loaded: u64,
    },
    FinalizingNodeStore,
    VerifyingFileHash,
    VerifyingShamapRoot {
        map: &'static str,
    },
    SyncingNodeStore,
    Complete {
        ledger_seq: u32,
        chunk_count: usize,
        nodes_loaded: u64,
        elapsed_ms: u64,
    },
}

/// Keeps a failed import fail-closed even if a backend must finalize on-disk
/// indexes before the footer and SHAMap graph can be verified.
struct BulkImportGuard<'a> {
    backend: &'a dyn Backend,
    completed: bool,
}

impl<'a> BulkImportGuard<'a> {
    fn new(backend: &'a dyn Backend) -> Self {
        Self {
            backend,
            completed: false,
        }
    }

    fn complete(&mut self) {
        self.completed = true;
    }
}

impl Drop for BulkImportGuard<'_> {
    fn drop(&mut self) {
        if !self.completed {
            self.backend.bulk_import_abort();
        }
    }
}

/// Read and validate the fixed snapshot manifest without importing any NodeStore
/// objects. Callers use this preflight to reject a network-mismatched or legacy
/// snapshot before it can modify the configured database.
pub fn read_snapshot_manifest(input_path: &Path) -> Result<SnapshotManifest, SnapshotError> {
    let file = File::open(input_path)
        .map_err(|error| SnapshotError::io_path("opening snapshot header", input_path, error))?;
    let mut reader = BufReader::new(file);
    let mut header_buf = [0u8; SNAPSHOT_HEADER_SIZE];
    reader
        .read_exact(&mut header_buf)
        .map_err(|error| SnapshotError::io("reading snapshot header", error))?;
    let manifest = SnapshotManifest::deserialize_header(&header_buf)?;
    verify_manifest_ledger_hash(&manifest)?;
    Ok(manifest)
}

/// Load a snapshot file without receiving operational progress updates.
///
/// This compatibility wrapper is appropriate for programmatic callers that
/// only need the verified manifest. CLI callers should use
/// [`load_snapshot_with_progress`].
pub fn load_snapshot(
    backend: &dyn Backend,
    input_path: &Path,
) -> Result<SnapshotManifest, SnapshotError> {
    load_snapshot_with_progress(backend, input_path, |_| {})
}

/// Load a snapshot file from `input_path` into `backend`, reporting each
/// completed stage and every imported chunk through `on_progress`.
///
/// Returns the verified manifest. A successful result proves that the manifest
/// header is self-consistent and that both advertised SHAMap roots are complete,
/// correctly typed, and content-addressed by their encoded bytes.
pub fn load_snapshot_with_progress(
    backend: &dyn Backend,
    input_path: &Path,
    mut on_progress: impl FnMut(SnapshotImportProgress),
) -> Result<SnapshotManifest, SnapshotError> {
    let start = Instant::now();
    tracing::info!(
        target: "snapshot",
        path = %input_path.display(),
        "Starting snapshot load"
    );

    let file = File::open(input_path)
        .map_err(|e| SnapshotError::io_path("opening snapshot file", input_path, e))?;
    let mut reader = BufReader::new(file);
    let mut file_hasher = Sha256::new();

    let mut header_buf = [0u8; SNAPSHOT_HEADER_SIZE];
    reader
        .read_exact(&mut header_buf)
        .map_err(|e| SnapshotError::io("reading header", e))?;
    file_hasher.update(header_buf);

    let mut manifest = SnapshotManifest::deserialize_header(&header_buf)?;
    verify_manifest_ledger_hash(&manifest)?;

    let chunk_count = u32::from_be_bytes(
        header_buf[SNAPSHOT_HEADER_SIZE - 10..SNAPSHOT_HEADER_SIZE - 6]
            .try_into()
            .expect("snapshot chunk-count field is fixed width"),
    ) as usize;
    if chunk_count > SNAPSHOT_MAX_CHUNKS {
        return Err(SnapshotError::ResourceLimitExceeded {
            resource: "chunk count",
            actual: chunk_count as u64,
            limit: SNAPSHOT_MAX_CHUNKS as u64,
        });
    }
    manifest
        .chunks
        .try_reserve(chunk_count)
        .map_err(|_| SnapshotError::ResourceLimitExceeded {
            resource: "chunk table allocation",
            actual: chunk_count as u64,
            limit: SNAPSHOT_MAX_CHUNKS as u64,
        })?;
    on_progress(SnapshotImportProgress::HeaderValidated {
        ledger_seq: manifest.ledger_seq,
        chunk_count,
    });

    tracing::info!(
        target: "snapshot",
        ledger_seq = manifest.ledger_seq,
        version = manifest.version,
        chunk_count,
        "Snapshot header parsed"
    );

    for _ in 0..chunk_count {
        let mut entry_buf = [0u8; CHUNK_META_SIZE];
        reader
            .read_exact(&mut entry_buf)
            .map_err(|e| SnapshotError::io("reading chunk table", e))?;
        file_hasher.update(entry_buf);
        manifest
            .chunks
            .push(SnapshotManifest::deserialize_chunk_meta(&entry_buf));
    }

    let compressed_bytes = manifest
        .chunks
        .iter()
        .map(|chunk| u64::from(chunk.compressed_len))
        .sum();
    on_progress(SnapshotImportProgress::ChunkTableRead {
        chunk_count,
        compressed_bytes,
    });

    let estimated_nodes = (chunk_count as u64).saturating_mul(30_000);
    backend
        .bulk_import_start(estimated_nodes)
        .map_err(|e| SnapshotError::BackendWriteFailed {
            reason: format!("bulk_import_start: {e}"),
        })?;
    let mut import_guard = BulkImportGuard::new(backend);
    on_progress(SnapshotImportProgress::BulkImportStarted { estimated_nodes });

    let mut total_nodes = 0u64;
    let mut compressed_bytes_loaded = 0u64;
    for (i, meta) in manifest.chunks.iter().enumerate() {
        let compressed_len = meta.compressed_len as usize;
        if compressed_len > SNAPSHOT_MAX_COMPRESSED_CHUNK_BYTES {
            return Err(SnapshotError::ResourceLimitExceeded {
                resource: "compressed chunk bytes",
                actual: compressed_len as u64,
                limit: SNAPSHOT_MAX_COMPRESSED_CHUNK_BYTES as u64,
            });
        }

        let mut compressed = vec![0u8; compressed_len];
        reader
            .read_exact(&mut compressed)
            .map_err(|e| SnapshotError::io("reading chunk data", e))?;
        file_hasher.update(&compressed);

        let computed_hash: [u8; 32] = Sha256::digest(&compressed).into();
        if computed_hash != meta.sha256 {
            return Err(SnapshotError::ChunkHashMismatch {
                chunk_index: i,
                expected: meta.sha256,
                computed: computed_hash,
            });
        }

        let (declared_uncompressed_len, _) = lz4_flex::block::uncompressed_size(&compressed)
            .map_err(|e| SnapshotError::DecompressionFailed {
                chunk_index: i,
                reason: e.to_string(),
            })?;
        if declared_uncompressed_len > SNAPSHOT_MAX_UNCOMPRESSED_CHUNK_BYTES {
            return Err(SnapshotError::ResourceLimitExceeded {
                resource: "uncompressed chunk bytes",
                actual: declared_uncompressed_len as u64,
                limit: SNAPSHOT_MAX_UNCOMPRESSED_CHUNK_BYTES as u64,
            });
        }
        let decompressed =
            lz4_flex::block::decompress_size_prepended(&compressed).map_err(|e| {
                SnapshotError::DecompressionFailed {
                    chunk_index: i,
                    reason: e.to_string(),
                }
            })?;

        let mut batch: Batch = Vec::new();
        let mut offset = 0;
        while offset < decompressed.len() {
            let (node_type_byte, hash, data_range, consumed) =
                decode_node_record(&decompressed, offset, i)?;
            let object_type =
                NodeObjectType::try_from(node_type_byte).unwrap_or(NodeObjectType::Unknown);
            batch.push(Arc::new(NodeObject::new(
                object_type,
                decompressed[data_range].to_vec(),
                Uint256::from_array(hash),
            )));
            offset += consumed;
        }

        backend.store_batch(&batch);
        total_nodes += batch.len() as u64;
        compressed_bytes_loaded =
            compressed_bytes_loaded.saturating_add(meta.compressed_len as u64);
        on_progress(SnapshotImportProgress::ChunkImported {
            chunk_index: i + 1,
            chunk_count: manifest.chunks.len(),
            nodes_loaded: total_nodes,
            compressed_bytes_loaded,
        });
        if (i + 1) % 10 == 0 || i + 1 == manifest.chunks.len() {
            tracing::info!(
                target: "snapshot",
                chunk = i + 1,
                total_chunks = manifest.chunks.len(),
                nodes_loaded = total_nodes,
                elapsed_ms = start.elapsed().as_millis() as u64,
                "Loading snapshot chunks"
            );
        }
    }

    // NuDB needs this to flush its bulk index before graph fetches work. The
    // guard restores its incomplete-import marker if any later verification
    // fails, so finalization is not publication.
    on_progress(SnapshotImportProgress::FinalizingNodeStore);
    backend
        .bulk_import_finish()
        .map_err(|e| SnapshotError::BackendWriteFailed {
            reason: format!("bulk_import_finish: {e}"),
        })?;

    let mut footer = [0u8; SNAPSHOT_FOOTER_SIZE];
    on_progress(SnapshotImportProgress::VerifyingFileHash);
    reader
        .read_exact(&mut footer)
        .map_err(|e| SnapshotError::io("reading footer", e))?;
    let computed_file_hash: [u8; 32] = file_hasher.finalize().into();
    if computed_file_hash != footer {
        return Err(SnapshotError::FileHashMismatch {
            expected: footer,
            computed: computed_file_hash,
        });
    }

    on_progress(SnapshotImportProgress::VerifyingShamapRoot {
        map: "account-state",
    });
    verify_shamap_root(backend, "account-state", manifest.account_hash)?;
    on_progress(SnapshotImportProgress::VerifyingShamapRoot { map: "transaction" });
    verify_shamap_root(backend, "transaction", manifest.tx_hash)?;
    on_progress(SnapshotImportProgress::SyncingNodeStore);
    backend
        .sync_result()
        .map_err(|reason| SnapshotError::BackendWriteFailed {
            reason: format!("final sync: {reason}"),
        })?;
    import_guard.complete();
    on_progress(SnapshotImportProgress::Complete {
        ledger_seq: manifest.ledger_seq,
        chunk_count: manifest.chunks.len(),
        nodes_loaded: total_nodes,
        elapsed_ms: start.elapsed().as_millis() as u64,
    });

    tracing::info!(
        target: "snapshot",
        ledger_seq = manifest.ledger_seq,
        total_nodes,
        chunks = manifest.chunks.len(),
        elapsed_ms = start.elapsed().as_millis() as u64,
        "Snapshot load complete, ledger and SHAMap roots verified"
    );

    Ok(manifest)
}

fn verify_manifest_ledger_hash(manifest: &SnapshotManifest) -> Result<(), SnapshotError> {
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

fn verify_shamap_root(
    backend: &dyn Backend,
    map: &'static str,
    root_bytes: [u8; 32],
) -> Result<(), SnapshotError> {
    let root = Uint256::from_array(root_bytes);
    if root.is_zero() {
        return Ok(());
    }

    let mut visiting = HashSet::new();
    let mut verified = HashSet::new();
    verify_shamap_node(backend, map, root, 0, &mut visiting, &mut verified)
}

fn verify_shamap_node(
    backend: &dyn Backend,
    map: &'static str,
    hash: Uint256,
    depth: usize,
    visiting: &mut HashSet<Uint256>,
    verified: &mut HashSet<Uint256>,
) -> Result<(), SnapshotError> {
    if verified.contains(&hash) {
        return Ok(());
    }
    if depth > MAX_SHAMAP_DEPTH {
        return Err(shamap_error(map, hash, "tree exceeds maximum SHAMap depth"));
    }
    if !visiting.insert(hash) {
        return Err(shamap_error(map, hash, "cycle in reachable SHAMap graph"));
    }

    let (object, status) = backend.fetch(&hash);
    let object = object.ok_or_else(|| {
        shamap_error(
            map,
            hash,
            &format!("node is missing from imported store ({status:?})"),
        )
    })?;
    if object.hash() != &hash {
        return Err(shamap_error(
            map,
            hash,
            "backend returned a NodeObject with a mismatched key",
        ));
    }

    let expected_object_type = match map {
        "account-state" => NodeObjectType::AccountNode,
        "transaction" => NodeObjectType::TransactionNode,
        _ => unreachable!("only known snapshot maps are verified"),
    };
    if object.object_type() != expected_object_type {
        return Err(shamap_error(
            map,
            hash,
            "reachable node has an incompatible NodeObject type",
        ));
    }

    let node = SHAMapTreeNode::make_from_prefix(object.data(), SHAMapHash::new(hash))
        .map_err(|error| shamap_error(map, hash, &format!("invalid encoded node: {error:?}")))?;
    // `make_from_prefix` deliberately accepts a known hash for normal trusted
    // fetch paths. Snapshot input is untrusted, so force recomputation before
    // trusting the decoded graph.
    node.update_hash();
    if node.get_hash().as_uint256() != &hash {
        return Err(shamap_error(
            map,
            hash,
            "encoded node body does not match its content-addressed hash",
        ));
    }

    if node.is_leaf() {
        let expected_leaf_type = match map {
            "account-state" => SHAMapNodeType::AccountState,
            "transaction" => SHAMapNodeType::TransactionMd,
            _ => unreachable!("only known snapshot maps are verified"),
        };
        if node.get_type() != expected_leaf_type {
            return Err(shamap_error(
                map,
                hash,
                "leaf type belongs to the other SHAMap",
            ));
        }
    } else {
        for branch in 0..BRANCH_FACTOR {
            if !node.is_empty_branch(branch) {
                verify_shamap_node(
                    backend,
                    map,
                    *node.get_child_hash(branch).as_uint256(),
                    depth + 1,
                    visiting,
                    verified,
                )?;
            }
        }
    }

    visiting.remove(&hash);
    verified.insert(hash);
    Ok(())
}

fn shamap_error(map: &'static str, hash: Uint256, reason: impl Into<String>) -> SnapshotError {
    SnapshotError::ShamapVerificationFailed {
        map,
        hash_hex: hash.to_string(),
        reason: reason.into(),
    }
}
