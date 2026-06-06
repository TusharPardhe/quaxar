use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use sha2::{Digest, Sha256};

use crate::{Backend, NodeObject};
use super::{SnapshotError, manifest::*};

/// Export all nodes from `backend` into a snapshot file at `output_path`.
///
/// The caller provides a pre-populated `manifest` with ledger metadata fields set.
/// The `chunks` field of `manifest` is ignored — it is rebuilt during export.
pub fn export_snapshot(
    backend: &dyn Backend,
    manifest: &SnapshotManifest,
    output_path: &Path,
) -> Result<(), SnapshotError> {
    let start = Instant::now();
    tracing::info!(
        target: "snapshot",
        path = %output_path.display(),
        ledger_seq = manifest.ledger_seq,
        "Starting snapshot export"
    );

    // Pass 1: collect all nodes into compressed chunks
    let mut chunks: Vec<Vec<u8>> = Vec::new();
    let mut chunk_metas: Vec<ChunkMeta> = Vec::new();
    let mut current_buf: Vec<u8> = Vec::new();
    let mut node_count: u64 = 0;

    let flush_chunk = |buf: &mut Vec<u8>, chunks: &mut Vec<Vec<u8>>, metas: &mut Vec<ChunkMeta>| {
        if buf.is_empty() {
            return;
        }
        let compressed = lz4_flex::block::compress_prepend_size(buf);
        let hash: [u8; 32] = Sha256::digest(&compressed).into();
        metas.push(ChunkMeta {
            compressed_len: compressed.len() as u32,
            sha256: hash,
        });
        chunks.push(compressed);
        buf.clear();
    };

    backend.for_each(&mut |node: Arc<NodeObject>| {
        let obj_type = node.object_type() as u32 as u8;
        encode_node_record(obj_type, node.hash().data(), node.data(), &mut current_buf);
        node_count += 1;
        if current_buf.len() >= SNAPSHOT_CHUNK_UNCOMPRESSED_TARGET {
            flush_chunk(&mut current_buf, &mut chunks, &mut chunk_metas);
            tracing::debug!(
                target: "snapshot",
                chunks_written = chunks.len(),
                nodes_so_far = node_count,
                "Chunk flushed during export"
            );
        }
    });
    // Flush remaining
    flush_chunk(&mut current_buf, &mut chunks, &mut chunk_metas);

    tracing::info!(
        target: "snapshot",
        node_count,
        chunk_count = chunks.len(),
        elapsed_ms = start.elapsed().as_millis() as u64,
        "Node iteration complete, writing snapshot file"
    );

    // Build final manifest with computed chunks
    let mut final_manifest = manifest.clone();
    final_manifest.chunks = chunk_metas;

    // Pass 2: write to file
    let file = File::create(output_path)
        .map_err(|e| SnapshotError::io_path("creating snapshot file", output_path, e))?;
    let mut writer = BufWriter::new(file);
    let mut file_hasher = Sha256::new();

    // Write header
    let header = final_manifest.serialize_header();
    writer.write_all(&header)
        .map_err(|e| SnapshotError::io("writing header", e))?;
    file_hasher.update(&header);

    // Write chunk table
    for meta in &final_manifest.chunks {
        let entry = SnapshotManifest::serialize_chunk_meta(meta);
        writer.write_all(&entry)
            .map_err(|e| SnapshotError::io("writing chunk table", e))?;
        file_hasher.update(&entry);
    }

    // Write chunk data
    for chunk_data in &chunks {
        writer.write_all(chunk_data)
            .map_err(|e| SnapshotError::io("writing chunk data", e))?;
        file_hasher.update(chunk_data);
    }

    // Write footer (file hash)
    let file_hash: [u8; 32] = file_hasher.finalize().into();
    writer.write_all(&file_hash)
        .map_err(|e| SnapshotError::io("writing footer", e))?;

    writer.flush()
        .map_err(|e| SnapshotError::io("flushing snapshot file", e))?;

    let file_size = std::fs::metadata(output_path)
        .map(|m| m.len())
        .unwrap_or(0);
    tracing::info!(
        target: "snapshot",
        path = %output_path.display(),
        node_count,
        chunk_count = chunks.len(),
        file_size_mb = file_size / (1024 * 1024),
        elapsed_ms = start.elapsed().as_millis() as u64,
        "Snapshot export complete"
    );

    Ok(())
}
