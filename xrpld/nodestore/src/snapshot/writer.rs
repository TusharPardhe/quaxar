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
/// Visit the leaves of the SHAMap rooted at `root` in ascending key order.
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
    use basics::base_uint::Uint256;
    use basics::sha_map_hash::SHAMapHash;
    use shamap::nodes::tree_node::{BRANCH_FACTOR, SHAMapTreeNode};

    let root = Uint256::from_array(root);
    if root.is_zero() {
        return Ok(());
    }
    let mut stack = vec![root];
    let mut visited: u64 = 0;
    while let Some(hash) = stack.pop() {
        visited += 1;
        if visited % 65_536 == 0 {
            check_cancelled(cancellation)?;
        }
        let (object, status) = backend.fetch(&hash);
        let object = object.ok_or_else(|| SnapshotError::BackendTraversalFailed {
            reason: format!("SHAMap node {hash} missing from store ({status:?})"),
        })?;
        let node = SHAMapTreeNode::make_from_prefix(object.data(), SHAMapHash::new(hash))
            .map_err(|e| SnapshotError::BackendTraversalFailed {
                reason: format!("SHAMap node {hash} does not decode: {e:?}"),
            })?;
        if node.is_inner() {
            // Push in reverse so branch 0 is visited first.
            for branch in (0..BRANCH_FACTOR).rev() {
                if !node.is_empty_branch(branch) {
                    stack.push(*node.get_child_hash(branch).as_uint256());
                }
            }
        } else {
            let item = node
                .peek_item()
                .ok_or_else(|| SnapshotError::BackendTraversalFailed {
                    reason: format!("SHAMap leaf {hash} has no item"),
                })?;
            visit(item.key().data(), item.data())?;
        }
    }
    Ok(())
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

    let mut chunk_metas: Vec<ChunkMeta> = Vec::new();
    let mut current_buf: Vec<u8> = Vec::with_capacity(SNAPSHOT_CHUNK_UNCOMPRESSED_TARGET + 4096);
    let mut node_count: u64 = 0;
    let mut total_compressed: u64 = 0;

    // Closure to flush one chunk to the temp file
    let flush_chunk = |buf: &mut Vec<u8>,
                       metas: &mut Vec<ChunkMeta>,
                       writer: &mut BufWriter<File>,
                       total: &mut u64|
     -> Result<(), SnapshotError> {
        check_cancelled(cancellation)?;
        if buf.is_empty() {
            return Ok(());
        }
        let compressed = lz4_flex::block::compress_prepend_size(buf);
        let hash: [u8; 32] = Sha256::digest(&compressed).into();
        let compressed_len = compressed.len() as u32;
        metas.push(ChunkMeta {
            compressed_len,
            sha256: hash,
        });
        writer
            .write_all(&compressed)
            .map_err(|e| SnapshotError::io("writing chunk to temp file", e))?;
        check_cancelled(cancellation)?;
        *total += compressed.len() as u64;
        buf.clear();
        Ok(())
    };

    // The callback itself cannot return a result, so capture serialization
    // errors while the backend reports its own traversal failures directly.
    let mut export_error: Option<SnapshotError> = None;

    // v2: write only the leaves of the ledger's two SHAMaps, in ascending key
    // order (state map first, then the transaction map). Inner nodes are not
    // exported; the loader rebuilds them from the sorted leaves and verifies
    // the rebuilt roots against the header. A depth-first walk that visits
    // branches 0..15 in order yields leaves in ascending key order, and keeps
    // only O(depth) nodes in memory.
    let mut emit_leaf = |map_tag: u8, key: &[u8; 32], data: &[u8]| -> Result<(), SnapshotError> {
        encode_node_record(map_tag, key, data, &mut current_buf);
        node_count += 1;
        if current_buf.len() >= SNAPSHOT_CHUNK_UNCOMPRESSED_TARGET {
            flush_chunk(
                &mut current_buf,
                &mut chunk_metas,
                &mut chunks_writer,
                &mut total_compressed,
            )?;
        }
        Ok(())
    };
    for (map_tag, root) in [
        (LEAF_TAG_ACCOUNT_STATE, manifest.account_hash),
        (LEAF_TAG_TRANSACTION, manifest.tx_hash),
    ] {
        if let Err(e) = walk_leaves(backend, root, cancellation, &mut |key, data| {
            emit_leaf(map_tag, key, data)
        }) {
            export_error = Some(e);
            break;
        }
    }
    drop(emit_leaf);

    // Check for errors captured during iteration
    if let Some(e) = export_error {
        let _ = fs::remove_file(&tmp_chunks_path);
        return Err(e);
    }
    check_cancelled(cancellation)?;

    // Flush remaining buffer
    flush_chunk(
        &mut current_buf,
        &mut chunk_metas,
        &mut chunks_writer,
        &mut total_compressed,
    )?;

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
