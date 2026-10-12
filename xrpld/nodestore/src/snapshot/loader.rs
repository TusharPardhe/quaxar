use std::collections::HashSet;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::time::Instant;

use rayon::prelude::*;
use sha2::{Digest, Sha256};
use shamap::nodes::tree_node::SHAMapNodeType;

use super::builder::{BuiltNode, SortedLeafBuilder, prepare_leaf};
use super::{SnapshotError, manifest::*};
use crate::{Backend, Batch, NodeObject, NodeObjectType};
use basics::base_uint::Uint256;
use basics::sha_map_hash::SHAMapHash;
use protocol::{LedgerHeader, calculate_ledger_hash};

/// Number of rebuilt nodes per backend batch write.
const WRITE_BATCH_NODES: usize = 65_536;

/// Batches queued between the rebuild and the writer thread. Bounds memory and
/// applies backpressure when the store is the slower stage.
const WRITE_QUEUE_BATCHES: usize = 4;

/// Keeps a failed import fail-closed even if a backend must finalize on-disk
/// indexes before the footer and roots can be verified.
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
    /// imported `nodes` keyspace down to the snapshot (design Case 7).
    ///
    /// Left empty when `store_was_empty` is true: the store then holds exactly
    /// the rebuilt nodes, so there is nothing to reconcile and collecting the
    /// set (tens of millions of hashes) would only cost memory and time.
    pub required_nodes: HashSet<Uint256>,
    /// The backend's `nodes` keyspace was empty before the import.
    pub store_was_empty: bool,
    /// Nodes written (leaves plus rebuilt inner nodes).
    pub nodes_written: u64,
}

/// Load a snapshot file from `input_path` into `backend`.
///
/// A successful result proves that the manifest header is self-consistent and
/// that the leaves in the file rebuild exactly the advertised account-state
/// and transaction roots; every stored node is keyed by its own recomputed
/// content hash.
///
/// The load is a three-stage pipeline: leaves of each chunk are serialized and
/// hashed in parallel; the bottom-up rebuild runs sequentially (it depends on
/// key order); and a writer thread commits node batches to the store while
/// the next chunk is processed.
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
    let mut reader = BufReader::with_capacity(1 << 20, file);
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
    tracing::info!(
        target: "snapshot",
        ledger_seq = manifest.ledger_seq,
        version = manifest.version,
        chunk_count,
        "Snapshot header parsed"
    );

    // Only a store that already holds nodes needs the retained set (to sweep
    // what the snapshot does not cover). A backend that cannot tell is treated
    // as non-empty, which is the conservative choice.
    let store_was_empty = backend.nodes_is_empty().unwrap_or(false);

    backend
        .bulk_import_start((chunk_count as u64).saturating_mul(80_000))
        .map_err(|e| SnapshotError::BackendWriteFailed {
            reason: format!("bulk_import_start: {e}"),
        })?;
    let mut import_guard = BulkImportGuard::new(backend);

    let (sender, receiver) = sync_channel::<Batch>(WRITE_QUEUE_BATCHES);
    let (rebuild, written) = std::thread::scope(|scope| {
        let writer = scope.spawn(move || write_batches(backend, receiver));
        let rebuild = rebuild_from_leaves(
            &manifest,
            &mut reader,
            &mut file_hasher,
            NodeSink::new(sender, !store_was_empty),
            start,
        );
        // `rebuild` owns (and has dropped) the sender, so the writer drains and
        // exits. A writer failure takes precedence: it is the root cause of
        // any send failure the rebuild saw.
        let written = writer.join().unwrap_or_else(|_| {
            Err("snapshot writer thread panicked".to_owned())
        });
        (rebuild, written)
    });
    let nodes_written = written.map_err(|reason| SnapshotError::BackendWriteFailed { reason })?;
    let rebuilt = rebuild?;

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
    check_root("account-state", rebuilt.state_root, manifest.account_hash)?;
    check_root("transaction", rebuilt.tx_root, manifest.tx_hash)?;

    backend
        .sync_result()
        .map_err(|e| SnapshotError::BackendWriteFailed {
            reason: format!("final sync: {e}"),
        })?;
    import_guard.complete();

    tracing::info!(
        target: "snapshot",
        ledger_seq = manifest.ledger_seq,
        nodes_written,
        state_leaves = rebuilt.state_leaves,
        tx_leaves = rebuilt.tx_leaves,
        store_was_empty,
        elapsed_ms = start.elapsed().as_millis() as u64,
        "Snapshot load complete, rebuilt roots match the ledger header"
    );

    Ok(SnapshotLoadOutcome {
        manifest,
        required_nodes: rebuilt.required,
        store_was_empty,
        nodes_written,
    })
}

/// Writer stage: commit batches until the sender is dropped.
fn write_batches(backend: &dyn Backend, receiver: Receiver<Batch>) -> Result<u64, String> {
    let mut written = 0u64;
    for batch in receiver {
        backend.store_batch_result(&batch)?;
        written += batch.len() as u64;
    }
    Ok(written)
}

struct Rebuilt {
    state_root: Uint256,
    tx_root: Uint256,
    state_leaves: u64,
    tx_leaves: u64,
    required: HashSet<Uint256>,
}

/// Read, verify and decompress each chunk; hash its leaves in parallel; feed
/// them in key order to the two bottom-up builders; send every node to the
/// writer.
fn rebuild_from_leaves(
    manifest: &SnapshotManifest,
    reader: &mut impl Read,
    file_hasher: &mut Sha256,
    mut sink: NodeSink,
    start: Instant,
) -> Result<Rebuilt, SnapshotError> {
    let mut state = SortedLeafBuilder::new();
    let mut tx = SortedLeafBuilder::new();
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

        // Decode record boundaries and check map order (cheap, sequential).
        let mut records: Vec<(SHAMapNodeType, Uint256, std::ops::Range<usize>, usize)> =
            Vec::new();
        let mut offset = 0;
        while offset < decompressed.len() {
            let (tag, key, data_range, consumed) = decode_node_record(&decompressed, offset, i)?;
            let leaf_type = match tag {
                LEAF_TAG_ACCOUNT_STATE if seen_tx => {
                    return Err(malformed(i, offset, "account-state leaf after transaction leaves"));
                }
                LEAF_TAG_ACCOUNT_STATE => SHAMapNodeType::AccountState,
                LEAF_TAG_TRANSACTION => {
                    seen_tx = true;
                    SHAMapNodeType::TransactionMd
                }
                other => return Err(malformed(i, offset, &format!("unknown leaf record tag {other}"))),
            };
            records.push((leaf_type, Uint256::from_array(key), data_range, offset));
            offset += consumed;
        }

        // Serialize and hash every leaf in parallel; order is preserved.
        let leaves: Vec<Result<BuiltNode, String>> = records
            .par_iter()
            .map(|(leaf_type, key, range, _)| prepare_leaf(*leaf_type, key, &decompressed[range.clone()]))
            .collect();

        // Rebuild sequentially in key order.
        for ((leaf_type, key, _, record_offset), leaf) in records.iter().zip(leaves) {
            let leaf = leaf.map_err(|reason| malformed(i, *record_offset, &reason))?;
            let (builder, object_type) = match leaf_type {
                SHAMapNodeType::AccountState => (&mut state, NodeObjectType::AccountNode),
                _ => (&mut tx, NodeObjectType::TransactionNode),
            };
            let leaf_hash = leaf.hash;
            sink.push(leaf, object_type).map_err(|_| sink.writer_gone())?;
            let mut send_failed = false;
            let pushed = builder.push(*key, leaf_hash, &mut |node| {
                sink.push(node, object_type).map_err(|e| {
                    send_failed = true;
                    e
                })
            });
            if let Err(reason) = pushed {
                return Err(if send_failed {
                    sink.writer_gone()
                } else {
                    malformed(i, *record_offset, &reason)
                });
            }
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
        .map_err(|_| sink.writer_gone())?;
    let tx_root = tx
        .finish(&mut |node| sink.push(node, NodeObjectType::TransactionNode))
        .map_err(|_| sink.writer_gone())?;
    sink.flush().map_err(|_| sink.writer_gone())?;
    Ok(Rebuilt {
        state_root,
        tx_root,
        state_leaves,
        tx_leaves,
        required: sink.into_required(),
    })
}

fn malformed(chunk_index: usize, offset: usize, reason: &str) -> SnapshotError {
    SnapshotError::MalformedNodeRecord {
        chunk_index,
        offset,
        reason: reason.to_owned(),
    }
}

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

/// Batches rebuilt nodes for the writer thread and, if requested, records
/// every hash as part of the snapshot's retained set.
struct NodeSink {
    sender: Option<SyncSender<Batch>>,
    pending: Batch,
    required: Option<HashSet<Uint256>>,
}

impl NodeSink {
    fn new(sender: SyncSender<Batch>, collect_required: bool) -> Self {
        Self {
            sender: Some(sender),
            pending: Vec::with_capacity(WRITE_BATCH_NODES),
            required: collect_required.then(HashSet::new),
        }
    }

    fn push(&mut self, node: BuiltNode, object_type: NodeObjectType) -> Result<(), String> {
        if let Some(required) = self.required.as_mut() {
            required.insert(node.hash);
        }
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
        let batch = std::mem::replace(&mut self.pending, Vec::with_capacity(WRITE_BATCH_NODES));
        self.sender
            .as_ref()
            .expect("sink sender is live until into_required")
            .send(batch)
            .map_err(|_| "snapshot writer stopped".to_owned())
    }

    /// The writer exited early; its own error (returned from `join`) is the
    /// cause and is reported instead of this placeholder.
    fn writer_gone(&self) -> SnapshotError {
        SnapshotError::BackendWriteFailed {
            reason: "snapshot writer stopped".to_owned(),
        }
    }

    fn into_required(mut self) -> HashSet<Uint256> {
        self.sender = None; // close the channel so the writer finishes
        self.required.take().unwrap_or_default()
    }
}

impl Drop for NodeSink {
    fn drop(&mut self) {
        self.sender = None;
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
