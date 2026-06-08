//! Snapshot manifest and binary format constants.
//!
//! ## Wire Format
//!
//! ```text
//! [SNAPSHOT FILE]
//! ├── Header (fixed size: SNAPSHOT_HEADER_SIZE bytes)
//! │   ├── magic:            [u8; 8]   = SNAPSHOT_MAGIC
//! │   ├── version:          u16 BE
//! │   ├── ledger_seq:       u32 BE
//! │   ├── ledger_hash:      [u8; 32]
//! │   ├── account_hash:     [u8; 32]  (state_map root; verified after load)
//! │   ├── tx_hash:          [u8; 32]  (tx_map root; informational)
//! │   ├── parent_hash:      [u8; 32]
//! │   ├── drops:            u64 BE
//! │   ├── close_time:       u32 BE
//! │   ├── parent_close_time:u32 BE
//! │   ├── close_time_res:   u8
//! │   ├── close_flags:      u8
//! │   ├── chunk_count:      u32 BE
//! │   └── _reserved:        [u8; 6]   (padding to 8-byte boundary; must be zero)
//! │
//! ├── Chunk Table (chunk_count × CHUNK_META_SIZE bytes)
//! │   For each chunk:
//! │   ├── compressed_len:   u32 BE
//! │   └── sha256_hash:      [u8; 32]  (sha256 of compressed_data)
//! │
//! ├── Chunk Data (chunk_count variable-length blobs)
//! │   For each chunk (compressed bytes, in order):
//! │   └── <compressed_len bytes of lz4-compressed node records>
//! │       Each record:
//! │       ├── node_type:    u8   (NodeObjectType discriminant)
//! │       ├── hash:         [u8; 32]
//! │       ├── data_len:     varint (base-127 little-endian)
//! │       └── data:         [u8; data_len]
//! │
//! └── Footer (SNAPSHOT_FOOTER_SIZE bytes)
//!     └── file_sha256:      [u8; 32]  (sha256 of all bytes before the footer)
//! ```

/// 8-byte magic that identifies an xrpld snapshot file.
/// ASCII "xrpls\0\x01\x00" — the `\x01` encodes the format family version.
pub const SNAPSHOT_MAGIC: &[u8; 8] = b"xrpls\x00\x01\x00";

/// Highest snapshot format version this binary can read.
pub const SNAPSHOT_MAX_VERSION: u16 = 1;

/// Current format version written by this binary.
pub const SNAPSHOT_VERSION: u16 = 1;

/// Target uncompressed size of each data chunk (8 MiB).
/// The last chunk may be smaller.
pub const SNAPSHOT_CHUNK_UNCOMPRESSED_TARGET: usize = 8 * 1024 * 1024;

/// Size of the fixed-width snapshot file header in bytes.
pub const SNAPSHOT_HEADER_SIZE: usize = 8  // magic
    + 2  // version
    + 4  // ledger_seq
    + 32 // ledger_hash
    + 32 // account_hash
    + 32 // tx_hash
    + 32 // parent_hash
    + 8  // drops
    + 4  // close_time
    + 4  // parent_close_time
    + 1  // close_time_res
    + 1  // close_flags
    + 4  // chunk_count
    + 6; // _reserved (padding)

/// Size of each chunk metadata entry in the chunk table.
/// Layout: [compressed_len: u32 BE] [sha256: 32 bytes]
pub const CHUNK_META_SIZE: usize = 4 + 32;

/// Size of the file footer.
pub const SNAPSHOT_FOOTER_SIZE: usize = 32;

/// Recommended file name pattern for a snapshot.
/// Usage: `snapshot_filename(seq, hash_hex_prefix)`.
pub fn snapshot_filename(ledger_seq: u32, hash_prefix: &str) -> String {
    format!("snapshot-{ledger_seq}-{hash_prefix}.xrpls")
}

// ─── In-memory manifest structures ──────────────────────────────────────────

/// Per-chunk metadata as stored in the chunk table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkMeta {
    /// Number of compressed bytes for this chunk.
    pub compressed_len: u32,
    /// SHA-256 hash of the compressed bytes.
    pub sha256: [u8; 32],
}

/// Complete snapshot manifest — everything needed to verify and load a snapshot.
#[derive(Debug, Clone)]
pub struct SnapshotManifest {
    /// Format version (must be `SNAPSHOT_VERSION`).
    pub version: u16,
    /// Ledger sequence number this snapshot covers.
    pub ledger_seq: u32,
    /// Full ledger hash (SHA-512/2 over canonical wire fields).
    pub ledger_hash: [u8; 32],
    /// SHAMap root hash of the account-state tree.
    pub account_hash: [u8; 32],
    /// SHAMap root hash of the transaction tree.
    pub tx_hash: [u8; 32],
    /// Hash of the parent ledger.
    pub parent_hash: [u8; 32],
    /// Total XRP drops in the ledger.
    pub drops: u64,
    /// Ledger close time (Ripple epoch seconds).
    pub close_time: u32,
    /// Parent ledger close time.
    pub parent_close_time: u32,
    /// Close-time rounding resolution.
    pub close_time_res: u8,
    /// Close flags (SLCF_NO_CONSENSUS_TIME = 0x01).
    pub close_flags: u8,
    /// Per-chunk metadata, in order.
    pub chunks: Vec<ChunkMeta>,
}

impl SnapshotManifest {
    /// Total number of bytes occupied by the chunk table.
    pub fn chunk_table_size(&self) -> usize {
        self.chunks.len() * CHUNK_META_SIZE
    }

    /// Byte offset at which chunk data begins (after header + chunk table).
    pub fn chunk_data_offset(&self) -> usize {
        SNAPSHOT_HEADER_SIZE + self.chunk_table_size()
    }

    /// Serialize the header into a fixed-size byte array.
    pub fn serialize_header(&self) -> [u8; SNAPSHOT_HEADER_SIZE] {
        let mut buf = [0u8; SNAPSHOT_HEADER_SIZE];
        let mut pos = 0;

        buf[pos..pos + 8].copy_from_slice(SNAPSHOT_MAGIC);
        pos += 8;

        buf[pos..pos + 2].copy_from_slice(&self.version.to_be_bytes());
        pos += 2;

        buf[pos..pos + 4].copy_from_slice(&self.ledger_seq.to_be_bytes());
        pos += 4;

        buf[pos..pos + 32].copy_from_slice(&self.ledger_hash);
        pos += 32;

        buf[pos..pos + 32].copy_from_slice(&self.account_hash);
        pos += 32;

        buf[pos..pos + 32].copy_from_slice(&self.tx_hash);
        pos += 32;

        buf[pos..pos + 32].copy_from_slice(&self.parent_hash);
        pos += 32;

        buf[pos..pos + 8].copy_from_slice(&self.drops.to_be_bytes());
        pos += 8;

        buf[pos..pos + 4].copy_from_slice(&self.close_time.to_be_bytes());
        pos += 4;

        buf[pos..pos + 4].copy_from_slice(&self.parent_close_time.to_be_bytes());
        pos += 4;

        buf[pos] = self.close_time_res;
        pos += 1;

        buf[pos] = self.close_flags;
        pos += 1;

        let chunk_count = self.chunks.len() as u32;
        buf[pos..pos + 4].copy_from_slice(&chunk_count.to_be_bytes());
        pos += 4;

        // _reserved — 6 bytes, already zero
        let _ = pos;

        buf
    }

    /// Serialize one chunk metadata entry.
    pub fn serialize_chunk_meta(meta: &ChunkMeta) -> [u8; CHUNK_META_SIZE] {
        let mut buf = [0u8; CHUNK_META_SIZE];
        buf[..4].copy_from_slice(&meta.compressed_len.to_be_bytes());
        buf[4..36].copy_from_slice(&meta.sha256);
        buf
    }

    /// Deserialize the fixed-size header from a byte slice.
    ///
    /// Returns the manifest (with an empty `chunks` vec) on success.
    /// The caller must then deserialize the chunk table separately.
    pub fn deserialize_header(buf: &[u8]) -> Result<Self, crate::snapshot::SnapshotError> {
        use crate::snapshot::SnapshotError;

        if buf.len() < SNAPSHOT_HEADER_SIZE {
            return Err(SnapshotError::io(
                "header too short",
                std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "snapshot header is truncated",
                ),
            ));
        }

        let mut pos = 0;

        // Magic
        let magic = &buf[pos..pos + 8];
        if magic != SNAPSHOT_MAGIC {
            return Err(SnapshotError::BadMagic { got: magic.to_vec() });
        }
        pos += 8;

        // Version
        let version = u16::from_be_bytes(buf[pos..pos + 2].try_into().unwrap());
        if version > SNAPSHOT_MAX_VERSION {
            return Err(SnapshotError::UnsupportedVersion {
                found: version,
                max_supported: SNAPSHOT_MAX_VERSION,
            });
        }
        pos += 2;

        // ledger_seq
        let ledger_seq = u32::from_be_bytes(buf[pos..pos + 4].try_into().unwrap());
        pos += 4;

        // ledger_hash
        let ledger_hash: [u8; 32] = buf[pos..pos + 32].try_into().unwrap();
        pos += 32;

        // account_hash
        let account_hash: [u8; 32] = buf[pos..pos + 32].try_into().unwrap();
        pos += 32;

        // tx_hash
        let tx_hash: [u8; 32] = buf[pos..pos + 32].try_into().unwrap();
        pos += 32;

        // parent_hash
        let parent_hash: [u8; 32] = buf[pos..pos + 32].try_into().unwrap();
        pos += 32;

        // drops
        let drops = u64::from_be_bytes(buf[pos..pos + 8].try_into().unwrap());
        pos += 8;

        // close_time
        let close_time = u32::from_be_bytes(buf[pos..pos + 4].try_into().unwrap());
        pos += 4;

        // parent_close_time
        let parent_close_time = u32::from_be_bytes(buf[pos..pos + 4].try_into().unwrap());
        pos += 4;

        // close_time_res
        let close_time_res = buf[pos];
        pos += 1;

        // close_flags
        let close_flags = buf[pos];
        pos += 1;

        // chunk_count
        let chunk_count = u32::from_be_bytes(buf[pos..pos + 4].try_into().unwrap()) as usize;
        let _ = pos;

        Ok(Self {
            version,
            ledger_seq,
            ledger_hash,
            account_hash,
            tx_hash,
            parent_hash,
            drops,
            close_time,
            parent_close_time,
            close_time_res,
            close_flags,
            chunks: Vec::with_capacity(chunk_count),
        })
    }

    /// Deserialize one `ChunkMeta` entry from a byte slice.
    pub fn deserialize_chunk_meta(buf: &[u8]) -> ChunkMeta {
        debug_assert!(buf.len() >= CHUNK_META_SIZE);
        let compressed_len = u32::from_be_bytes(buf[..4].try_into().unwrap());
        let sha256: [u8; 32] = buf[4..36].try_into().unwrap();
        ChunkMeta { compressed_len, sha256 }
    }
}

// ─── Node-record wire helpers ────────────────────────────────────────────────

/// Encode a single node record into `out`.
///
/// Layout: `[type: u8][hash: 32][data_len: varint][data: N]`
///
/// Returns the number of bytes written.
pub fn encode_node_record(
    node_type: u8,
    hash: &[u8; 32],
    data: &[u8],
    out: &mut Vec<u8>,
) -> usize {
    let before = out.len();
    out.push(node_type);
    out.extend_from_slice(hash);
    // varint-encode data_len
    let mut n = data.len();
    loop {
        let byte = (n & 0x7F) as u8;
        n >>= 7;
        if n == 0 {
            out.push(byte);
            break;
        } else {
            out.push(byte | 0x80);
        }
    }
    out.extend_from_slice(data);
    out.len() - before
}

/// Decode one node record from `buf` starting at `offset`.
///
/// Returns `(node_type, hash, data_slice_range, consumed_bytes)` on success,
/// or an error if the buffer is truncated or malformed.
pub fn decode_node_record(
    buf: &[u8],
    offset: usize,
    chunk_index: usize,
) -> Result<(u8, [u8; 32], std::ops::Range<usize>, usize), crate::snapshot::SnapshotError> {
    use crate::snapshot::SnapshotError;

    let start = offset;
    let mut pos = offset;

    if pos >= buf.len() {
        return Err(SnapshotError::MalformedNodeRecord {
            chunk_index,
            offset: pos,
            reason: "buffer exhausted before node type byte".to_owned(),
        });
    }
    let node_type = buf[pos];
    pos += 1;

    if pos + 32 > buf.len() {
        return Err(SnapshotError::MalformedNodeRecord {
            chunk_index,
            offset: pos,
            reason: "buffer truncated before hash field".to_owned(),
        });
    }
    let hash: [u8; 32] = buf[pos..pos + 32].try_into().unwrap();
    pos += 32;

    // decode varint
    let mut data_len: usize = 0;
    let mut shift = 0usize;
    loop {
        if pos >= buf.len() {
            return Err(SnapshotError::MalformedNodeRecord {
                chunk_index,
                offset: pos,
                reason: "buffer truncated inside data_len varint".to_owned(),
            });
        }
        let byte = buf[pos] as usize;
        pos += 1;
        data_len |= (byte & 0x7F) << shift;
        shift += 7;
        if byte & 0x80 == 0 {
            break;
        }
        if shift >= 64 {
            return Err(SnapshotError::MalformedNodeRecord {
                chunk_index,
                offset: pos,
                reason: "data_len varint overflow".to_owned(),
            });
        }
    }

    if pos + data_len > buf.len() {
        return Err(SnapshotError::MalformedNodeRecord {
            chunk_index,
            offset: pos,
            reason: format!(
                "data_len={data_len} extends past end of buffer (buf.len()={})",
                buf.len()
            ),
        });
    }
    let data_range = pos..pos + data_len;
    pos += data_len;

    Ok((node_type, hash, data_range, pos - start))
}
