use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;
use std::sync::Arc;

use sha2::{Digest, Sha256};

use basics::base_uint::Uint256;
use crate::{Backend, Batch, NodeObject, NodeObjectType};
use super::{SnapshotError, manifest::*};

/// Load a snapshot file from `input_path` into `backend`.
///
/// Returns the deserialized manifest so callers can verify the account root hash.
pub fn load_snapshot(
    backend: &dyn Backend,
    input_path: &Path,
) -> Result<SnapshotManifest, SnapshotError> {
    let file = File::open(input_path)
        .map_err(|e| SnapshotError::io_path("opening snapshot file", input_path, e))?;
    let mut reader = BufReader::new(file);
    let mut file_hasher = Sha256::new();

    // Read header
    let mut header_buf = [0u8; SNAPSHOT_HEADER_SIZE];
    reader.read_exact(&mut header_buf)
        .map_err(|e| SnapshotError::io("reading header", e))?;
    file_hasher.update(&header_buf);

    let mut manifest = SnapshotManifest::deserialize_header(&header_buf)?;

    // Read chunk count from header to know how many chunk table entries to read
    let chunk_count = u32::from_be_bytes(
        header_buf[SNAPSHOT_HEADER_SIZE - 10..SNAPSHOT_HEADER_SIZE - 6]
            .try_into()
            .unwrap(),
    ) as usize;

    // Read chunk table
    for _ in 0..chunk_count {
        let mut entry_buf = [0u8; CHUNK_META_SIZE];
        reader.read_exact(&mut entry_buf)
            .map_err(|e| SnapshotError::io("reading chunk table", e))?;
        file_hasher.update(&entry_buf);
        manifest.chunks.push(SnapshotManifest::deserialize_chunk_meta(&entry_buf));
    }

    // Read and process each chunk
    for (i, meta) in manifest.chunks.iter().enumerate() {
        let mut compressed = vec![0u8; meta.compressed_len as usize];
        reader.read_exact(&mut compressed)
            .map_err(|e| SnapshotError::io("reading chunk data", e))?;
        file_hasher.update(&compressed);

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
        let decompressed = lz4_flex::block::decompress_size_prepended(&compressed)
            .map_err(|e| SnapshotError::DecompressionFailed {
                chunk_index: i,
                reason: e.to_string(),
            })?;

        // Decode node records and build batch
        let mut batch: Batch = Vec::new();
        let mut offset = 0;
        while offset < decompressed.len() {
            let (node_type_byte, hash, data_range, consumed) =
                decode_node_record(&decompressed, offset, i)?;

            let obj_type = NodeObjectType::try_from(node_type_byte).unwrap_or(NodeObjectType::Unknown);
            let data = decompressed[data_range].to_vec();
            let uint_hash = Uint256::from_array(hash);
            let node = Arc::new(NodeObject::new(obj_type, data, uint_hash));
            batch.push(node);
            offset += consumed;
        }

        backend.store_batch(&batch);
    }

    // Read and verify footer
    let mut footer = [0u8; SNAPSHOT_FOOTER_SIZE];
    reader.read_exact(&mut footer)
        .map_err(|e| SnapshotError::io("reading footer", e))?;

    let computed_file_hash: [u8; 32] = file_hasher.finalize().into();
    if computed_file_hash != footer {
        return Err(SnapshotError::FileHashMismatch {
            expected: footer,
            computed: computed_file_hash,
        });
    }

    Ok(manifest)
}
