use std::collections::HashSet;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use sha2::{Digest, Sha256};
use shamap::nodes::tree_node::SHAMapNodeType;

use super::builder::{BuiltNode, SortedLeafBuilder};
use super::{SnapshotError, manifest::*};
use crate::{Backend, Batch, NodeObject, NodeObjectType};
use basics::base_uint::Uint256;
use basics::sha_map_hash::SHAMapHash;
use protocol::{LedgerHeader, calculate_ledger_hash};

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

/// Result of a successful snapshot load.
pub struct SnapshotLoadOutcome {
    /// The verified manifest (header fields and chunk table).
    pub manifest: SnapshotManifest,
    /// Every node rebuilt from the two verified SHAMap roots: the exact
    /// retained set for this snapshot. A pruned store uses it to reconcile the
    /// imported `nodes` keyspace down to the snapshot and to stamp the anchor
    /// (design Case 7). Empty roots contribute nothing.
    pub required_nodes: HashSet<Uint256>,
}

/// Load a snapshot file from `input_path` into `backend`.
///
/// Returns the verified manifest and the set of node hashes of the two rebuilt
/// SHAMaps. A successful result proves that the manifest header is
/// self-consistent and that the leaves in the file rebuild exactly the
/// advertised account-state and transaction roots; every stored node is keyed
/// by its own recomputed content hash.
pub fn load_snapshot(
    backend: &dyn Backend,
    input_path: &Path,
) -> Result<SnapshotLoadOutcome, SnapshotError> {
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

    let estimated_nodes = (chunk_count as u64).saturating_mul(30_000);
    backend
        .bulk_import_start(estimated_nodes)
        .map_err(|e| SnapshotError::BackendWriteFailed {
            reason: format!("bulk_import_start: {e}"),
        })?;
    let mut import_guard = BulkImportGuard::new(backend);

    // v2: leaves only, sorted by key (account-state map, then transaction
    // map). Each map is rebuilt bottom-up with O(depth) memory; every rebuilt
    // node is written to the store as soon as it is final. Because each node is
    // keyed by its own recomputed content hash, matching the rebuilt roots with
    // the header's account_hash / tx_hash (checked below) verifies every leaf
    // and every inner node in this single pass, with no read-back walk.
    let mut sink = NodeSink::new(backend);
    let mut state = SortedLeafBuilder::new(SHAMapNodeType::AccountState);
    let mut tx = SortedLeafBuilder::new(SHAMapNodeType::TransactionMd);
    let mut seen_tx = false;

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
        let decompressed = decompress_chunk(i, &compressed)?;

        let mut offset = 0;
        while offset < decompressed.len() {
            let (tag, key, data_range, consumed) = decode_node_record(&decompressed, offset, i)?;
            let key = Uint256::from_array(key);
            let data = decompressed[data_range].to_vec();
            let pushed = match tag {
                LEAF_TAG_ACCOUNT_STATE if seen_tx => {
                    Err("account-state leaf after transaction leaves".to_owned())
                }
                LEAF_TAG_ACCOUNT_STATE => state.push(key, data, &mut |node| {
                    sink.push(node, NodeObjectType::AccountNode)
                }),
                LEAF_TAG_TRANSACTION => {
                    seen_tx = true;
                    tx.push(key, data, &mut |node| {
                        sink.push(node, NodeObjectType::TransactionNode)
                    })
                }
                other => Err(format!("unknown leaf record tag {other}")),
            };
            if let Err(reason) = pushed {
                return Err(sink.error_or(SnapshotError::MalformedNodeRecord {
                    chunk_index: i,
                    offset,
                    reason,
                }));
            }
            offset += consumed;
        }

        if (i + 1) % 50 == 0 || i + 1 == manifest.chunks.len() {
            tracing::info!(
                target: "snapshot",
                chunk = i + 1,
                total_chunks = manifest.chunks.len(),
                leaves = state.leaves() + tx.leaves(),
                elapsed_ms = start.elapsed().as_millis() as u64,
                "Loading snapshot chunks"
            );
        }
    }

    let state_leaves = state.leaves();
    let tx_leaves = tx.leaves();
    let state_root = state
        .finish(&mut |node| sink.push(node, NodeObjectType::AccountNode))
        .map_err(|reason| sink.finish_error("account-state", reason))?;
    let tx_root = tx
        .finish(&mut |node| sink.push(node, NodeObjectType::TransactionNode))
        .map_err(|reason| sink.finish_error("transaction", reason))?;
    sink.flush()
        .map_err(|reason| SnapshotError::BackendWriteFailed { reason })?;
    let total_nodes = sink.written;
    let required_nodes = std::mem::take(&mut sink.required);

    // Finalize any backend bulk-import indexes before the footer and roots are
    // checked. The guard restores the incomplete-import marker if a later
    // check fails, so finalization is not publication.
    backend
        .bulk_import_finish()
        .map_err(|e| SnapshotError::BackendWriteFailed {
            reason: format!("bulk_import_finish: {e}"),
        })?;

    let mut footer = [0u8; SNAPSHOT_FOOTER_SIZE];
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
    let mut trailing_byte = [0u8; 1];
    if reader
        .read(&mut trailing_byte)
        .map_err(|e| SnapshotError::io("checking for trailing data", e))?
        != 0
    {
        return Err(SnapshotError::TrailingData);
    }

    // The rebuilt roots must equal the ledger header's roots. This is the
    // verification of every node written above.
    check_root("account-state", state_root, manifest.account_hash)?;
    check_root("transaction", tx_root, manifest.tx_hash)?;

    backend
        .sync_result()
        .map_err(|e| SnapshotError::BackendWriteFailed {
            reason: format!("final sync: {e}"),
        })?;
    import_guard.complete();

    tracing::info!(
        target: "snapshot",
        ledger_seq = manifest.ledger_seq,
        total_nodes,
        chunks = manifest.chunks.len(),
        required_nodes = required_nodes.len(),
        state_leaves,
        tx_leaves,
        elapsed_ms = start.elapsed().as_millis() as u64,
        "Snapshot load complete, ledger and SHAMap roots verified"
    );

    Ok(SnapshotLoadOutcome {
        manifest,
        required_nodes,
    })
}

/// Number of rebuilt nodes buffered before one backend batch write.
const WRITE_BATCH_NODES: usize = 65_536;

/// Check the declared size of one compressed chunk and decompress it.
fn decompress_chunk(chunk_index: usize, compressed: &[u8]) -> Result<Vec<u8>, SnapshotError> {
    let (declared_uncompressed_len, _) =
        lz4_flex::block::uncompressed_size(compressed).map_err(|e| {
            SnapshotError::DecompressionFailed {
                chunk_index,
                reason: e.to_string(),
            }
        })?;
    if declared_uncompressed_len > SNAPSHOT_MAX_UNCOMPRESSED_CHUNK_BYTES {
        return Err(SnapshotError::ResourceLimitExceeded {
            resource: "uncompressed chunk bytes",
            actual: declared_uncompressed_len as u64,
            limit: SNAPSHOT_MAX_UNCOMPRESSED_CHUNK_BYTES as u64,
        });
    }
    lz4_flex::block::decompress_size_prepended(compressed).map_err(|e| {
        SnapshotError::DecompressionFailed {
            chunk_index,
            reason: e.to_string(),
        }
    })
}

/// Collects rebuilt nodes, writes them to the backend in batches, and records
/// every hash as part of the snapshot's retained set.
struct NodeSink<'a> {
    backend: &'a dyn Backend,
    pending: Batch,
    required: HashSet<Uint256>,
    written: u64,
    write_error: Option<String>,
}

impl<'a> NodeSink<'a> {
    fn new(backend: &'a dyn Backend) -> Self {
        Self {
            backend,
            pending: Vec::with_capacity(WRITE_BATCH_NODES),
            required: HashSet::new(),
            written: 0,
            write_error: None,
        }
    }

    fn push(&mut self, node: BuiltNode, object_type: NodeObjectType) -> Result<(), String> {
        self.required.insert(node.hash);
        self.pending
            .push(Arc::new(NodeObject::new(object_type, node.data, node.hash)));
        if self.pending.len() >= WRITE_BATCH_NODES {
            self.flush()?;
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<(), String> {
        if self.pending.is_empty() {
            return Ok(());
        }
        if let Err(reason) = self.backend.store_batch_result(&self.pending) {
            self.write_error = Some(reason.clone());
            return Err(reason);
        }
        self.written += self.pending.len() as u64;
        self.pending.clear();
        Ok(())
    }

    /// A backend write failure takes precedence over the input error it caused.
    fn error_or(&self, input_error: SnapshotError) -> SnapshotError {
        match &self.write_error {
            Some(reason) => SnapshotError::BackendWriteFailed {
                reason: reason.clone(),
            },
            None => input_error,
        }
    }

    fn finish_error(&self, map: &'static str, reason: String) -> SnapshotError {
        self.error_or(SnapshotError::ShamapVerificationFailed {
            map,
            hash_hex: String::new(),
            reason,
        })
    }
}

fn check_root(map: &'static str, rebuilt: Uint256, expected: [u8; 32]) -> Result<(), SnapshotError> {
    let expected = Uint256::from_array(expected);
    if rebuilt == expected {
        return Ok(());
    }
    Err(SnapshotError::ShamapVerificationFailed {
        map,
        hash_hex: expected.to_string(),
        reason: format!("rebuilt root {rebuilt} does not match the ledger header"),
    })
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
