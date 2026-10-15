//! Snapshot export — streaming writer.
//!
//! The export uses a streaming two-pass approach that keeps peak memory at ~16 MB
//! regardless of backend size:
//!
//! 1. First pass: walk the ledger's two SHAMaps from their roots and write only
//!    their leaves, in ascending key order, as compressed chunks to a temporary
//!    file, accumulating only the per-chunk metadata (36 bytes each).
//! 2. Second pass: write the final snapshot file (header + chunk table + chunk data
//!    copied from temp file + footer), then atomically rename into place.
//!
//! This avoids holding compressed chunks in memory (which would be multi-GB for
//! large node stores).

use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Instant;

use rayon::prelude::*;
use sha2::{Digest, Sha256};

use super::{SnapshotError, manifest::*};
use crate::Backend;

/// Owned cooperative cancellation signal for a snapshot export task.
///
/// Cancellation is intentionally polling-only: the writer keeps filesystem
/// ownership on its worker thread, so observing this signal lets its temporary
/// file guard remove both artifacts before the task returns an error.
#[derive(Clone, Debug, Default)]
pub struct SnapshotExportCancellation {
    cancelled: Arc<AtomicBool>,
}

impl SnapshotExportCancellation {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

fn check_cancelled(cancellation: Option<&SnapshotExportCancellation>) -> Result<(), SnapshotError> {
    if cancellation.is_some_and(SnapshotExportCancellation::is_cancelled) {
        return Err(SnapshotError::ExportCancelled);
    }
    Ok(())
}

struct SnapshotTempFiles {
    chunks: PathBuf,
    snapshot: PathBuf,
}

impl SnapshotTempFiles {
    fn new(chunks: &Path, snapshot: &Path) -> Self {
        Self {
            chunks: chunks.to_path_buf(),
            snapshot: snapshot.to_path_buf(),
        }
    }
}

impl Drop for SnapshotTempFiles {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.chunks);
        let _ = fs::remove_file(&self.snapshot);
    }
}

/// Export all nodes from `backend` into a snapshot file at `output_path`.
///
/// The caller provides a pre-populated `manifest` with ledger metadata fields set.
/// The `chunks` field of `manifest` is ignored — it is rebuilt during export.
///
/// # Streaming Design
///
/// Peak memory usage is bounded to ~16 MB (one uncompressed chunk buffer + one
/// compressed output buffer). Compressed chunk data is written to a temporary file,
/// then the final snapshot is assembled in a second pass.
///
/// The final file is written to a `.tmp` sibling path, then atomically renamed
/// into place so a crash never leaves a partial file at `output_path`.
/// Hash prefixes of stored SHAMap nodes (big-endian), as in `xrpl/shamap`.
const HASH_PREFIX_INNER_NODE: u32 = 0x4D49_4E00;
const HASH_PREFIX_LEAF_NODE: u32 = 0x4D4C_4E00;
const HASH_PREFIX_TX_NODE: u32 = 0x534E_4400;
const INNER_NODE_BYTES: usize = 4 + 16 * 32;

/// A stored SHAMap node, parsed in place without allocation.
enum StoredNode<'a> {
    /// The 16 child hashes (all-zero = empty branch).
    Inner(&'a [u8]),
    /// Item key and payload.
    Leaf { key: &'a [u8; 32], data: &'a [u8] },
}

/// Parse a prefix-serialized node as quaxar stores it: an inner node is
/// `prefix || 16 child hashes`; a ledger leaf is `prefix || data || key`.
fn parse_stored_node(bytes: &[u8]) -> Result<StoredNode<'_>, String> {
    if bytes.len() < 4 {
        return Err("node shorter than its hash prefix".to_owned());
    }
    let prefix = u32::from_be_bytes(bytes[..4].try_into().expect("4 bytes"));
    match prefix {
        HASH_PREFIX_INNER_NODE if bytes.len() == INNER_NODE_BYTES => Ok(StoredNode::Inner(&bytes[4..])),
        HASH_PREFIX_INNER_NODE => Err(format!("inner node has {} bytes", bytes.len())),
        HASH_PREFIX_LEAF_NODE | HASH_PREFIX_TX_NODE if bytes.len() >= 4 + 32 => {
            let split = bytes.len() - 32;
            Ok(StoredNode::Leaf {
                key: bytes[split..].try_into().expect("32 bytes"),
                data: &bytes[4..split],
            })
        }
        other => Err(format!("unexpected node prefix {other:#010x}")),
    }
}

fn fetch_node(backend: &dyn Backend, hash: &[u8; 32]) -> Result<std::sync::Arc<crate::NodeObject>, SnapshotError> {
    let key = basics::base_uint::Uint256::from_array(*hash);
    let (object, status) = backend.fetch(&key);
    object.ok_or_else(|| SnapshotError::BackendTraversalFailed {
        reason: format!("SHAMap node {key} missing from store ({status:?})"),
    })
}

/// Visit the leaves of the subtree rooted at `root` in ascending key order.
///
/// Depth-first, children visited in branch order 0..15, which is ascending key
/// order for a radix tree. The explicit stack holds at most 15 pending siblings
/// per level, so memory is O(depth). A missing node fails the export (the
/// anchor ledger must be fully present in the store).
fn walk_leaves(
    backend: &dyn Backend,
    root: [u8; 32],
    cancellation: Option<&SnapshotExportCancellation>,
    visit: &mut dyn FnMut(&[u8; 32], &[u8]) -> Result<(), SnapshotError>,
) -> Result<(), SnapshotError> {
    if root == [0u8; 32] {
        return Ok(());
    }
    let mut stack = vec![root];
    let mut visited: u64 = 0;
    while let Some(hash) = stack.pop() {
        visited += 1;
        if visited % 16_384 == 0 {
            check_cancelled(cancellation)?;
        }
        let object = fetch_node(backend, &hash)?;
        let parsed = parse_stored_node(object.data()).map_err(|reason| {
            SnapshotError::BackendTraversalFailed {
                reason: format!("SHAMap node {}: {reason}", basics::base_uint::Uint256::from_array(hash)),
            }
        })?;
        match parsed {
            StoredNode::Inner(children) => {
                // Push in reverse so branch 0 is visited first.
                for child in children.chunks_exact(32).rev() {
                    if child.iter().any(|b| *b != 0) {
                        stack.push(child.try_into().expect("32 bytes"));
                    }
                }
            }
            StoredNode::Leaf { key, data } => visit(key, data)?,
        }
    }
    Ok(())
}

/// Split the export into independent segments, in output order: the state
/// map's subtrees two levels below the root (up to 256, disjoint and in
/// ascending key order), then the whole transaction map.
fn plan_segments(
    backend: &dyn Backend,
    manifest: &SnapshotManifest,
) -> Result<Vec<(u8, [u8; 32])>, SnapshotError> {
    let mut segments = Vec::new();
    if manifest.account_hash != [0u8; 32] {
        split_subtree(backend, manifest.account_hash, 2, &mut segments)?;
    }
    if manifest.tx_hash != [0u8; 32] {
        segments.push((LEAF_TAG_TRANSACTION, manifest.tx_hash));
    }
    Ok(segments)
}

/// Push the state-map subtrees `depth` levels below `hash`, in branch order.
fn split_subtree(
    backend: &dyn Backend,
    hash: [u8; 32],
    depth: usize,
    segments: &mut Vec<(u8, [u8; 32])>,
) -> Result<(), SnapshotError> {
    if depth == 0 {
        segments.push((LEAF_TAG_ACCOUNT_STATE, hash));
        return Ok(());
    }
    let object = fetch_node(backend, &hash)?;
    match parse_stored_node(object.data()) {
        Ok(StoredNode::Inner(children)) => {
            for child in children.chunks_exact(32) {
                if child.iter().any(|b| *b != 0) {
                    split_subtree(backend, child.try_into().expect("32 bytes"), depth - 1, segments)?;
                }
            }
            Ok(())
        }
        // A leaf above the split depth is its own segment.
        Ok(StoredNode::Leaf { .. }) => {
            segments.push((LEAF_TAG_ACCOUNT_STATE, hash));
            Ok(())
        }
        Err(reason) => Err(SnapshotError::BackendTraversalFailed {
            reason: format!(
                "SHAMap node {}: {reason}",
                basics::base_uint::Uint256::from_array(hash)
            ),
        }),
    }
}

/// One exported segment: its compressed chunks and their metadata, in order.
struct SegmentOutput {
    chunks: Vec<(Vec<u8>, ChunkMeta)>,
    leaves: u64,
}

fn compress_chunk(raw: &[u8]) -> (Vec<u8>, ChunkMeta) {
    let compressed = lz4_flex::block::compress_prepend_size(raw);
    let sha256: [u8; 32] = Sha256::digest(&compressed).into();
    let meta = ChunkMeta {
        compressed_len: compressed.len() as u32,
        sha256,
    };
    (compressed, meta)
}

/// Walk one segment and encode its leaves into compressed chunks.
fn export_segment(
    backend: &dyn Backend,
    tag: u8,
    root: [u8; 32],
    cancellation: Option<&SnapshotExportCancellation>,
) -> Result<SegmentOutput, SnapshotError> {
    let mut out = SegmentOutput {
        chunks: Vec::new(),
        leaves: 0,
    };
    let mut buf: Vec<u8> = Vec::with_capacity(SNAPSHOT_CHUNK_UNCOMPRESSED_TARGET + 4096);
    walk_leaves(backend, root, cancellation, &mut |key, data| {
        encode_node_record(tag, key, data, &mut buf);
        out.leaves += 1;
        if buf.len() >= SNAPSHOT_CHUNK_UNCOMPRESSED_TARGET {
            out.chunks.push(compress_chunk(&buf));
            buf.clear();
        }
        Ok(())
    })?;
    if !buf.is_empty() {
        out.chunks.push(compress_chunk(&buf));
    }
    Ok(out)
}

/// Export all nodes without a cancellation signal.
///
/// This retains the historical behavior used by automatic exports.
pub fn export_snapshot(
    backend: &dyn Backend,
    manifest: &SnapshotManifest,
    output_path: &Path,
) -> Result<(), SnapshotError> {
    export_snapshot_inner(backend, manifest, output_path, None)
}

/// Export all nodes while polling `cancellation` between records, chunk work,
/// and final-file copy operations.
pub fn export_snapshot_with_cancellation(
    backend: &dyn Backend,
    manifest: &SnapshotManifest,
    output_path: &Path,
    cancellation: &SnapshotExportCancellation,
) -> Result<(), SnapshotError> {
    export_snapshot_inner(backend, manifest, output_path, Some(cancellation))
}

fn export_snapshot_inner(
    backend: &dyn Backend,
    manifest: &SnapshotManifest,
    output_path: &Path,
    cancellation: Option<&SnapshotExportCancellation>,
) -> Result<(), SnapshotError> {
    let start = Instant::now();
    tracing::info!(
        target: "snapshot",
        path = %output_path.display(),
        ledger_seq = manifest.ledger_seq,
        "Starting snapshot export"
    );

    // Temporary file for chunk data (lives beside the final output)
    let tmp_chunks_path = output_path.with_extension("xrpls.chunks.tmp");
    let tmp_final_path = output_path.with_extension("xrpls.tmp");

    // Clean up any leftover temp files from a prior failed export
    let _ = fs::remove_file(&tmp_chunks_path);
    let _ = fs::remove_file(&tmp_final_path);
    let _temporary_files = SnapshotTempFiles::new(&tmp_chunks_path, &tmp_final_path);
    check_cancelled(cancellation)?;

    let chunks_file = File::create(&tmp_chunks_path)
        .map_err(|e| SnapshotError::io_path("creating temp chunks file", &tmp_chunks_path, e))?;
    let mut chunks_writer = BufWriter::new(chunks_file);

    // ─── Pass 1: stream nodes into compressed chunks on disk ─────────────────

    // v2: write only the leaves of the ledger's two SHAMaps, in ascending key
    // order (state map first, then the transaction map). Inner nodes are not
    // exported; the loader rebuilds them and verifies the rebuilt roots.
    //
    // The walk is one random store lookup per node, so it is split into
    // disjoint subtrees processed in parallel. Segments are handled in
    // windows and written in segment order, so the file is deterministic and
    // keys stay globally ascending; memory is bounded by one window of
    // compressed segment output.
    let segments = plan_segments(backend, manifest)?;
    let window = (rayon::current_num_threads() * 2).max(1);
    let mut chunk_metas: Vec<ChunkMeta> = Vec::new();
    let mut node_count: u64 = 0;
    let mut total_compressed: u64 = 0;
    for batch in segments.chunks(window) {
        check_cancelled(cancellation)?;
        let outputs: Vec<Result<SegmentOutput, SnapshotError>> = batch
            .par_iter()
            .map(|(tag, root)| export_segment(backend, *tag, *root, cancellation))
            .collect();
        for output in outputs {
            let output = output?;
            node_count += output.leaves;
            for (compressed, meta) in output.chunks {
                chunks_writer
                    .write_all(&compressed)
                    .map_err(|e| SnapshotError::io("writing chunk to temp file", e))?;
                total_compressed += compressed.len() as u64;
                chunk_metas.push(meta);
            }
        }
    }
    check_cancelled(cancellation)?;

    chunks_writer
        .flush()
        .map_err(|e| SnapshotError::io("flushing temp chunks file", e))?;
    chunks_writer
        .get_ref()
        .sync_all()
        .map_err(|e| SnapshotError::io("syncing temp chunks file", e))?;
    check_cancelled(cancellation)?;

    tracing::info!(
        target: "snapshot",
        node_count,
        chunk_count = chunk_metas.len(),
        compressed_mb = total_compressed / (1024 * 1024),
        elapsed_ms = start.elapsed().as_millis() as u64,
        "Node iteration complete, assembling final snapshot file"
    );

    // ─── Pass 2: assemble final snapshot file ────────────────────────────────

    let mut final_manifest = manifest.clone();
    final_manifest.chunks = chunk_metas;

    let final_file = File::create(&tmp_final_path)
        .map_err(|e| SnapshotError::io_path("creating temp snapshot file", &tmp_final_path, e))?;
    let mut writer = BufWriter::new(final_file);
    let mut file_hasher = Sha256::new();

    // Write header
    check_cancelled(cancellation)?;
    let header = final_manifest.serialize_header();
    writer
        .write_all(&header)
        .map_err(|e| SnapshotError::io("writing header", e))?;
    file_hasher.update(header);

    // Write chunk table
    for meta in &final_manifest.chunks {
        check_cancelled(cancellation)?;
        let entry = SnapshotManifest::serialize_chunk_meta(meta);
        writer
            .write_all(&entry)
            .map_err(|e| SnapshotError::io("writing chunk table", e))?;
        file_hasher.update(entry);
    }

    // Copy chunk data from temp file into final file
    let chunks_read_file = File::open(&tmp_chunks_path)
        .map_err(|e| SnapshotError::io_path("reopening temp chunks file", &tmp_chunks_path, e))?;
    let mut reader = BufReader::new(chunks_read_file);
    let mut copy_buf = vec![0u8; 64 * 1024]; // 64KB copy buffer
    loop {
        check_cancelled(cancellation)?;
        let n = reader
            .read(&mut copy_buf)
            .map_err(|e| SnapshotError::io("reading temp chunks file", e))?;
        if n == 0 {
            break;
        }
        check_cancelled(cancellation)?;
        writer
            .write_all(&copy_buf[..n])
            .map_err(|e| SnapshotError::io("writing chunk data", e))?;
        file_hasher.update(&copy_buf[..n]);
    }

    // Write footer (file SHA-256)
    check_cancelled(cancellation)?;
    let file_hash: [u8; 32] = file_hasher.finalize().into();
    writer
        .write_all(&file_hash)
        .map_err(|e| SnapshotError::io("writing footer", e))?;

    writer
        .flush()
        .map_err(|e| SnapshotError::io("flushing snapshot file", e))?;
    writer
        .get_ref()
        .sync_all()
        .map_err(|e| SnapshotError::io("syncing snapshot file to disk", e))?;

    // ─── Atomic rename into place ────────────────────────────────────────────

    check_cancelled(cancellation)?;
    fs::rename(&tmp_final_path, output_path)
        .map_err(|e| SnapshotError::io_path("renaming snapshot to final path", output_path, e))?;

    // Clean up temp chunks file
    let _ = fs::remove_file(&tmp_chunks_path);

    let file_size = fs::metadata(output_path).map(|m| m.len()).unwrap_or(0);
    tracing::info!(
        target: "snapshot",
        path = %output_path.display(),
        node_count,
        chunk_count = final_manifest.chunks.len(),
        file_size_mb = file_size / (1024 * 1024),
        elapsed_ms = start.elapsed().as_millis() as u64,
        "Snapshot export complete"
    );

    Ok(())
}
