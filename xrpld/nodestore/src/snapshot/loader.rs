use std::fs::File;
use std::path::Path;
use std::time::Instant;

use sha2::{Digest, Sha256};

use super::{SnapshotError, manifest::*};
use crate::{Backend, NodeObjectType, format::types::NodeRecordView};
use basics::base_uint::Uint256;

struct BulkImportGuard<'a> {
    backend: &'a dyn Backend,
    finished: bool,
}

impl<'a> BulkImportGuard<'a> {
    fn new(backend: &'a dyn Backend) -> Self {
        Self {
            backend,
            finished: false,
        }
    }

    fn finish(mut self) -> Result<(), String> {
        self.finished = true;
        self.backend.bulk_import_finish()
    }
}

impl<'a> Drop for BulkImportGuard<'a> {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.backend.bulk_import_abort();
        }
    }
}

/// Load a snapshot file from `input_path` into `backend`.
///
/// Returns the deserialized manifest so callers can verify the account root hash.
pub fn load_snapshot(
    backend: &dyn Backend,
    input_path: &Path,
) -> Result<SnapshotManifest, SnapshotError> {
    let start_time = Instant::now();
    let import_guard = BulkImportGuard::new(backend);

    let file = File::open(input_path).map_err(|e| SnapshotError::io("opening file", e))?;
    let mmap = unsafe {
        let mmap = memmap2::MmapOptions::new()
            .map(&file)
            .map_err(|e| SnapshotError::io("mmap failed", e))?;
        // Advise the kernel that we will read sequentially and it can aggressively evict pages
        let _ = mmap.advise(memmap2::Advice::Sequential);
        mmap
    };

    let mut file_hasher = Sha256::new();
    let mut mmap_offset = 0;

    // Read header
    if mmap.len() < SNAPSHOT_HEADER_SIZE {
        return Err(SnapshotError::io(
            "file too small for header",
            std::io::Error::new(std::io::ErrorKind::UnexpectedEof, ""),
        ));
    }

    let header_buf = &mmap[mmap_offset..mmap_offset + SNAPSHOT_HEADER_SIZE];
    file_hasher.update(header_buf);
    mmap_offset += SNAPSHOT_HEADER_SIZE;

    let mut manifest = SnapshotManifest::deserialize_header(header_buf)?;

    // Read chunk count from header to know how many chunk table entries to read
    let chunk_count = u32::from_be_bytes(
        header_buf[SNAPSHOT_HEADER_SIZE - 10..SNAPSHOT_HEADER_SIZE - 6]
            .try_into()
            .unwrap(),
    ) as usize;

    tracing::info!(
        target: "snapshot",
        ledger_seq = manifest.ledger_seq,
        version = manifest.version,
        chunk_count,
        "Snapshot header parsed"
    );

    // Read chunk table
    let table_size = chunk_count * CHUNK_META_SIZE;
    if mmap.len() < mmap_offset + table_size {
        return Err(SnapshotError::io(
            "file too small for chunk table",
            std::io::Error::new(std::io::ErrorKind::UnexpectedEof, ""),
        ));
    }

    for _ in 0..chunk_count {
        let entry_buf = &mmap[mmap_offset..mmap_offset + CHUNK_META_SIZE];
        file_hasher.update(entry_buf);
        manifest
            .chunks
            .push(SnapshotManifest::deserialize_chunk_meta(entry_buf));
        mmap_offset += CHUNK_META_SIZE;
    }

    // Read and process each chunk
    let mut total_nodes: u64 = 0;
    let estimated_nodes = chunk_count as u64 * 30_000;
    backend
        .bulk_import_start(estimated_nodes)
        .map_err(|e| SnapshotError::BackendWriteFailed {
            reason: format!("bulk_import_start: {e}"),
        })?;

    for (i, meta) in manifest.chunks.iter().enumerate() {
        let compressed_len = meta.compressed_len as usize;
        if mmap.len() < mmap_offset + compressed_len {
            return Err(SnapshotError::io(
                "file too small for chunk data",
                std::io::Error::new(std::io::ErrorKind::UnexpectedEof, ""),
            ));
        }

        let compressed = &mmap[mmap_offset..mmap_offset + compressed_len];
        file_hasher.update(compressed);
        mmap_offset += compressed_len;

        // Verify chunk hash
        let computed_hash: [u8; 32] = Sha256::digest(&compressed).into();
        if computed_hash != meta.sha256 {
            return Err(SnapshotError::ChunkHashMismatch {
                chunk_index: i,
                expected: meta.sha256,
                computed: computed_hash,
            });
        }

        // Decompress
        let decompressed =
            lz4_flex::block::decompress_size_prepended(&compressed).map_err(|e| {
                SnapshotError::DecompressionFailed {
                    chunk_index: i,
                    reason: e.to_string(),
                }
            })?;

        // Decode node records and build zero-copy views
        let mut views = Vec::new();
        let mut offset = 0;
        while offset < decompressed.len() {
            let (node_type_byte, hash, data_range, consumed) =
                decode_node_record(&decompressed, offset, i)?;

            let obj_type =
                NodeObjectType::try_from(node_type_byte).unwrap_or(NodeObjectType::Unknown);
            let uint_hash = Uint256::from_array(hash);

            views.push(NodeRecordView {
                object_type: obj_type,
                hash: uint_hash,
                data: &decompressed[data_range],
            });
            offset += consumed;
        }

        backend.store_views(&views);
        total_nodes += views.len() as u64;

        if (i + 1) % 10 == 0 || i + 1 == manifest.chunks.len() {
            tracing::info!(
                target: "snapshot",
                chunk = i + 1,
                total_chunks = manifest.chunks.len(),
                nodes_loaded = total_nodes,
                elapsed_ms = start_time.elapsed().as_millis() as u64,
                "Loading snapshot chunks"
            );
        }
    }

    let mmap_len = mmap.len();
    if mmap_len < mmap_offset + SNAPSHOT_FOOTER_SIZE {
        drop(mmap);
        return Err(SnapshotError::io(
            "file too small for footer",
            std::io::Error::new(std::io::ErrorKind::UnexpectedEof, ""),
        ));
    }

    let mut expected_footer = [0u8; 32];
    expected_footer.copy_from_slice(&mmap[mmap_offset..mmap_offset + SNAPSHOT_FOOTER_SIZE]);
    mmap_offset += SNAPSHOT_FOOTER_SIZE;

    let final_computed_hash: [u8; 32] = file_hasher.finalize().into();
    if final_computed_hash != expected_footer {
        return Err(SnapshotError::FileHashMismatch {
            expected: expected_footer,
            computed: final_computed_hash,
        });
    }

    // Drop the massive mmap reference before running backend.bulk_import_finish()
    // This frees RAM (potentially gigabytes of pagecache) which helps prevent OOM
    // when bulk_import_finish triggers a massive bucket_cache flush or NuDB checkpoint.
    drop(mmap);

    import_guard
        .finish()
        .map_err(|e| SnapshotError::BackendWriteFailed {
            reason: format!("bulk_import_finish: {e}"),
        })?;

    // We reached EOF gracefully
    if mmap_offset < mmap_len {
        tracing::warn!(target: "snapshot", "Ignored {} trailing bytes", mmap_len - mmap_offset);
    }

    tracing::info!(
        target: "snapshot",
        ledger_seq = manifest.ledger_seq,
        total_nodes,
        chunks = manifest.chunks.len(),
        elapsed_ms = start_time.elapsed().as_millis() as u64,
        "Snapshot load complete, integrity verified"
    );

    // Future enhancement: verify SHAMap root hash matches manifest.account_hash
    // for additional integrity assurance beyond chunk-level SHA-256 verification.

    Ok(manifest)
}
