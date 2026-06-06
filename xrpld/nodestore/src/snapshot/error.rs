//! Error types for the snapshot sub-system.
//!
//! All public error variants carry enough context to produce a meaningful
//! diagnostic message without requiring the caller to consult additional
//! state.

use std::fmt;
use std::path::PathBuf;

/// Every possible failure in the snapshot writer, loader, or verifier.
#[derive(Debug)]
pub enum SnapshotError {
    /// I/O error with context.
    Io {
        context: &'static str,
        path: Option<PathBuf>,
        source: std::io::Error,
    },

    /// The file did not begin with the expected magic bytes.
    BadMagic {
        got: Vec<u8>,
    },

    /// The snapshot format version is not supported by this binary.
    UnsupportedVersion {
        found: u16,
        max_supported: u16,
    },

    /// A compressed chunk failed LZ4 decompression.
    DecompressionFailed {
        chunk_index: usize,
        reason: String,
    },

    /// A chunk's SHA-256 hash did not match the value recorded in the manifest.
    ChunkHashMismatch {
        chunk_index: usize,
        expected: [u8; 32],
        computed: [u8; 32],
    },

    /// The file-level footer hash did not match.
    FileHashMismatch {
        expected: [u8; 32],
        computed: [u8; 32],
    },

    /// After loading all nodes the reconstructed account-state root hash
    /// does not match the value stored in the snapshot header.
    AccountRootMismatch {
        expected_hex: String,
        computed_hex: String,
    },

    /// A node record inside a chunk was malformed.
    MalformedNodeRecord {
        chunk_index: usize,
        offset: usize,
        reason: String,
    },

    /// The backend refused a write.
    BackendWriteFailed {
        reason: String,
    },

    /// Nodestore is not available (no backend attached).
    NoNodeStore,

    /// A ledger that was requested for export was not found.
    LedgerNotFound {
        seq: Option<u32>,
        hash: Option<String>,
    },

    /// Attempted to export while the node store is not open.
    NodeStoreNotOpen,
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { context, path, source } => {
                if let Some(p) = path {
                    write!(f, "snapshot I/O error ({context}) at {}: {source}", p.display())
                } else {
                    write!(f, "snapshot I/O error ({context}): {source}")
                }
            }
            Self::BadMagic { got } => {
                write!(f, "snapshot file has wrong magic bytes: got {:02x?}", got)
            }
            Self::UnsupportedVersion { found, max_supported } => {
                write!(
                    f,
                    "snapshot version {found} is not supported (max supported: {max_supported})"
                )
            }
            Self::DecompressionFailed { chunk_index, reason } => {
                write!(f, "snapshot chunk {chunk_index} decompression failed: {reason}")
            }
            Self::ChunkHashMismatch { chunk_index, expected, computed } => {
                write!(
                    f,
                    "snapshot chunk {chunk_index} integrity check failed: expected {:02x?}, got {:02x?}",
                    &expected[..8],
                    &computed[..8]
                )
            }
            Self::FileHashMismatch { expected, computed } => {
                write!(
                    f,
                    "snapshot file integrity check failed: expected {:02x?}, got {:02x?}",
                    &expected[..8],
                    &computed[..8]
                )
            }
            Self::AccountRootMismatch { expected_hex, computed_hex } => {
                write!(
                    f,
                    "snapshot post-load account root mismatch: expected {expected_hex}, got {computed_hex}"
                )
            }
            Self::MalformedNodeRecord { chunk_index, offset, reason } => {
                write!(
                    f,
                    "snapshot chunk {chunk_index} contains malformed node record at offset {offset}: {reason}"
                )
            }
            Self::BackendWriteFailed { reason } => {
                write!(f, "snapshot loader: backend write failed: {reason}")
            }
            Self::NoNodeStore => {
                write!(f, "snapshot operation requires an attached node store")
            }
            Self::LedgerNotFound { seq, hash } => match (seq, hash) {
                (Some(s), _) => write!(f, "ledger seq={s} not found for snapshot export"),
                (_, Some(h)) => write!(f, "ledger hash={h} not found for snapshot export"),
                _ => write!(f, "requested ledger not found for snapshot export"),
            },
            Self::NodeStoreNotOpen => {
                write!(f, "snapshot export requires an open node store")
            }
        }
    }
}

impl std::error::Error for SnapshotError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl SnapshotError {
    /// Construct an I/O error with contextual labels.
    pub(crate) fn io(context: &'static str, source: std::io::Error) -> Self {
        Self::Io { context, path: None, source }
    }

    /// Construct an I/O error with contextual labels and a file path.
    pub(crate) fn io_path(
        context: &'static str,
        path: impl Into<PathBuf>,
        source: std::io::Error,
    ) -> Self {
        Self::Io {
            context,
            path: Some(path.into()),
            source,
        }
    }
}
