//! Snapshot export — streaming writer.
//!
//! The export uses a streaming two-pass approach that keeps peak memory at ~16 MB
//! regardless of backend size:
//!
//! 1. First pass: iterate all nodes, compress into chunks, write chunk data to a
//!    temporary file while accumulating only the per-chunk metadata (36 bytes each).
//! 2. Second pass: write the final snapshot file (header + chunk table + chunk data
//!    copied from temp file + footer), then atomically rename into place.
//!
//! This avoids holding compressed chunks in memory (which would be multi-GB for
//! large node stores).

use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use basics::base_uint::Uint256;
use basics::sha_map_hash::SHAMapHash;
use sha2::{Digest, Sha256};
use shamap::nodes::tree_node::{BRANCH_FACTOR, SHAMapNodeType, SHAMapTreeNode};

use super::{SnapshotError, manifest::*};
use crate::{Backend, NodeObject, NodeObjectType};

/// Maximum `NodeObjectType` discriminant value that fits in a u8.
/// The snapshot format stores node type as a single byte.
const MAX_NODE_TYPE_U8: u32 = 255;

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
pub fn export_snapshot(
    backend: &dyn Backend,
    manifest: &SnapshotManifest,
    output_path: &Path,
) -> Result<(), SnapshotError> {
    export_snapshot_with_visitor(manifest, output_path, |callback| {
        backend.for_each(callback);
        Ok(())
    })
}

/// Export only the account-state and transaction-tree nodes reachable from the
/// manifest roots. This is the bootable checkpoint format used by the live
/// node export path: unrelated retained history and stale NodeStore objects do
/// not inflate the archive or import time.
pub fn export_compact_snapshot(
    backend: &dyn Backend,
    manifest: &SnapshotManifest,
    output_path: &Path,
) -> Result<(), SnapshotError> {
    export_snapshot_with_visitor(manifest, output_path, |callback| {
        visit_reachable_snapshot_nodes(backend, manifest, callback)
    })
}

fn export_snapshot_with_visitor<F>(
    manifest: &SnapshotManifest,
    output_path: &Path,
    mut visit_nodes: F,
) -> Result<(), SnapshotError>
where
    F: FnMut(&mut dyn FnMut(Arc<NodeObject>)) -> Result<(), SnapshotError>,
{
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
        *total += compressed.len() as u64;
        buf.clear();
        Ok(())
    };

    let mut export_error: Option<SnapshotError> = None;
    let visit_result = visit_nodes(&mut |node: Arc<NodeObject>| {
        if export_error.is_some() {
            return;
        }

        let obj_type_u32 = node.object_type() as u32;
        if obj_type_u32 > MAX_NODE_TYPE_U8 {
            export_error = Some(SnapshotError::MalformedNodeRecord {
                chunk_index: chunk_metas.len(),
                offset: current_buf.len(),
                reason: format!(
                    "NodeObjectType discriminant {} exceeds u8 range; cannot encode in snapshot",
                    obj_type_u32
                ),
            });
            return;
        }

        encode_node_record(
            obj_type_u32 as u8,
            node.hash().data(),
            node.data(),
            &mut current_buf,
        );
        node_count += 1;

        if current_buf.len() >= SNAPSHOT_CHUNK_UNCOMPRESSED_TARGET {
            if let Err(error) = flush_chunk(
                &mut current_buf,
                &mut chunk_metas,
                &mut chunks_writer,
                &mut total_compressed,
            ) {
                export_error = Some(error);
                return;
            }
            tracing::debug!(
                target: "snapshot",
                chunks_written = chunk_metas.len(),
                nodes_so_far = node_count,
                "Chunk flushed during export"
            );
        }
    });
    if let Err(error) = visit_result {
        let _ = fs::remove_file(&tmp_chunks_path);
        return Err(error);
    }
    if let Some(error) = export_error {
        let _ = fs::remove_file(&tmp_chunks_path);
        return Err(error);
    }

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
    let header = final_manifest.serialize_header();
    writer
        .write_all(&header)
        .map_err(|e| SnapshotError::io("writing header", e))?;
    file_hasher.update(header);

    // Write chunk table
    for meta in &final_manifest.chunks {
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
        let n = reader
            .read(&mut copy_buf)
            .map_err(|e| SnapshotError::io("reading temp chunks file", e))?;
        if n == 0 {
            break;
        }
        writer
            .write_all(&copy_buf[..n])
            .map_err(|e| SnapshotError::io("writing chunk data", e))?;
        file_hasher.update(&copy_buf[..n]);
    }

    // Write footer (file SHA-256)
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

const MAX_SHAMAP_DEPTH: usize = 64;
/// Bound reachable-node traversal even for malformed shared-child graphs. A
/// valid SHAMap has vastly fewer nodes than this and is streamed without a
/// global visited set, preserving bounded memory.
const MAX_COMPACT_SNAPSHOT_NODES: u64 = 100_000_000;

fn visit_reachable_snapshot_nodes(
    backend: &dyn Backend,
    manifest: &SnapshotManifest,
    callback: &mut dyn FnMut(Arc<NodeObject>),
) -> Result<(), SnapshotError> {
    let mut emitted = 0_u64;
    visit_reachable_tree(
        backend,
        "account-state",
        NodeObjectType::AccountNode,
        Uint256::from_array(manifest.account_hash),
        &mut emitted,
        callback,
    )?;
    visit_reachable_tree(
        backend,
        "transaction",
        NodeObjectType::TransactionNode,
        Uint256::from_array(manifest.tx_hash),
        &mut emitted,
        callback,
    )
}

fn visit_reachable_tree(
    backend: &dyn Backend,
    map: &'static str,
    expected_type: NodeObjectType,
    root: Uint256,
    emitted: &mut u64,
    callback: &mut dyn FnMut(Arc<NodeObject>),
) -> Result<(), SnapshotError> {
    if root.is_zero() {
        return Ok(());
    }

    let mut stack = vec![(root, 0_usize)];
    while let Some((hash, depth)) = stack.pop() {
        *emitted = (*emitted).saturating_add(1);
        if *emitted > MAX_COMPACT_SNAPSHOT_NODES {
            return Err(SnapshotError::ResourceLimitExceeded {
                resource: "compact snapshot reachable nodes",
                actual: *emitted,
                limit: MAX_COMPACT_SNAPSHOT_NODES,
            });
        }
        // A SHAMap is expected to be a tree. We deliberately permit repeated
        // content-addressed nodes here rather than retaining one hash per node:
        // duplicate records are harmless to import, while a global visited set
        // would make a purportedly streaming export consume unbounded RAM.
        // The fixed depth limit still rejects cycles.
        if depth > MAX_SHAMAP_DEPTH {
            return Err(snapshot_tree_error(
                map,
                hash,
                "tree exceeds maximum SHAMap depth",
            ));
        }
        let (object, status) = backend.fetch(&hash);
        let object = object.ok_or_else(|| {
            snapshot_tree_error(
                map,
                hash,
                &format!("node is missing from NodeStore ({status:?})"),
            )
        })?;
        if object.hash() != &hash {
            return Err(snapshot_tree_error(
                map,
                hash,
                "backend returned a mismatched key",
            ));
        }
        if object.object_type() != expected_type {
            return Err(snapshot_tree_error(
                map,
                hash,
                "reachable node has an incompatible type",
            ));
        }

        let node = SHAMapTreeNode::make_from_prefix(object.data(), SHAMapHash::new(hash)).map_err(
            |error| snapshot_tree_error(map, hash, format!("invalid encoded node: {error:?}")),
        )?;
        node.update_hash();
        if node.get_hash().as_uint256() != &hash {
            return Err(snapshot_tree_error(
                map,
                hash,
                "encoded node body does not match its hash",
            ));
        }
        let expected_leaf_type = match map {
            "account-state" => SHAMapNodeType::AccountState,
            "transaction" => SHAMapNodeType::TransactionMd,
            _ => unreachable!("only known snapshot maps are exported"),
        };
        if node.is_leaf() && node.get_type() != expected_leaf_type {
            return Err(snapshot_tree_error(
                map,
                hash,
                "leaf type belongs to the other SHAMap",
            ));
        }
        if node.is_inner() {
            for branch in 0..BRANCH_FACTOR {
                if !node.is_empty_branch(branch) {
                    stack.push((*node.get_child_hash(branch).as_uint256(), depth + 1));
                }
            }
        }
        callback(object);
    }
    Ok(())
}

fn snapshot_tree_error(
    map: &'static str,
    hash: Uint256,
    reason: impl Into<String>,
) -> SnapshotError {
    SnapshotError::ShamapVerificationFailed {
        map,
        hash_hex: hash.to_string(),
        reason: reason.into(),
    }
}
