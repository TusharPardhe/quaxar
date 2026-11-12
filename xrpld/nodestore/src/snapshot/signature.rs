//! Detached signatures for snapshot files.
//!
//! A snapshot already proves it is internally consistent: every chunk and the
//! whole file are SHA-256 checked, the loader rebuilds both SHAMap roots and
//! checks them, and the header's ledger hash is recomputed. None of that says
//! WHO made it: anyone can build a consistent snapshot of a fake ledger. A
//! signature from a key the operator trusts closes that gap.
//!
//! The signature is a sidecar `<snapshot>.sig` JSON file, so the snapshot
//! format is unchanged and unsigned snapshots still load:
//!
//! ```json
//! { "format": 1, "ledger_seq": 108283244, "ledger_hash": "…",
//!   "file_sha256": "…", "public_key": "n…", "signature": "…" }
//! ```
//!
//! IMPORTANT: the signed message is
//! `"quaxar-snapshot-v1:" + ledger_seq + ":" + ledger_hash + ":" + file_sha256`
//! (hex upper-case), so a signature cannot be moved to another file or ledger.
//! The key is an Ed25519 key in the node-public `n…` encoding.

use std::path::{Path, PathBuf};

use protocol::{KeyType, PublicKey, SecretKey, TokenType};
use protocol::serde_json::{self, Value, json};

use super::manifest::{SNAPSHOT_FOOTER_SIZE, SNAPSHOT_HEADER_SIZE};

/// Current `.sig` format.
pub const SIGNATURE_FORMAT: u64 = 1;

/// What a signature attests about a snapshot file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotIdentity {
    pub ledger_seq: u32,
    pub ledger_hash: [u8; 32],
    /// The footer's whole-file SHA-256 (the loader re-checks it).
    pub file_sha256: [u8; 32],
}

/// A parsed `.sig` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotSignature {
    pub identity: SnapshotIdentity,
    pub public_key: PublicKey,
    pub signature: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignatureError {
    Io(String),
    Malformed(String),
    /// The `.sig` names another file or ledger than the snapshot.
    IdentityMismatch(String),
    BadSignature,
    /// Valid, but by a key not in the trusted list.
    UntrustedKey(String),
    Missing(PathBuf),
}

impl std::fmt::Display for SignatureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "snapshot signature I/O error: {e}"),
            Self::Malformed(e) => write!(f, "malformed snapshot signature: {e}"),
            Self::IdentityMismatch(e) => {
                write!(f, "snapshot signature is for a different snapshot: {e}")
            }
            Self::BadSignature => write!(f, "snapshot signature does not verify"),
            Self::UntrustedKey(key) => write!(f, "snapshot signed by untrusted key {key}"),
            Self::Missing(path) => write!(f, "snapshot signature {} not found", path.display()),
        }
    }
}

impl std::error::Error for SignatureError {}

/// `<snapshot>.sig`.
pub fn signature_path(snapshot: &Path) -> PathBuf {
    let mut name = snapshot.as_os_str().to_owned();
    name.push(".sig");
    PathBuf::from(name)
}

/// Read the ledger identity and file hash straight from the header and footer,
/// without loading the snapshot.
pub fn read_identity(snapshot: &Path) -> Result<SnapshotIdentity, SignatureError> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(snapshot).map_err(|e| SignatureError::Io(e.to_string()))?;
    let len = file
        .metadata()
        .map_err(|e| SignatureError::Io(e.to_string()))?
        .len();
    if len < (SNAPSHOT_HEADER_SIZE + SNAPSHOT_FOOTER_SIZE) as u64 {
        return Err(SignatureError::Malformed("file is shorter than header + footer".into()));
    }
    let mut header = [0u8; 8 + 2 + 4 + 32];
    file.read_exact(&mut header)
        .map_err(|e| SignatureError::Io(e.to_string()))?;
    if &header[..8] != super::manifest::SNAPSHOT_MAGIC {
        return Err(SignatureError::Malformed("not a quaxar snapshot (bad magic)".into()));
    }
    let ledger_seq = u32::from_be_bytes(header[10..14].try_into().expect("4 bytes"));
    let ledger_hash: [u8; 32] = header[14..46].try_into().expect("32 bytes");
    file.seek(SeekFrom::End(-(SNAPSHOT_FOOTER_SIZE as i64)))
        .map_err(|e| SignatureError::Io(e.to_string()))?;
    let mut file_sha256 = [0u8; 32];
    file.read_exact(&mut file_sha256)
        .map_err(|e| SignatureError::Io(e.to_string()))?;
    Ok(SnapshotIdentity {
        ledger_seq,
        ledger_hash,
        file_sha256,
    })
}

fn signed_message(identity: &SnapshotIdentity) -> Vec<u8> {
    format!(
        "quaxar-snapshot-v1:{}:{}:{}",
        identity.ledger_seq,
        hex::encode_upper(identity.ledger_hash),
        hex::encode_upper(identity.file_sha256)
    )
    .into_bytes()
}

/// Sign `identity` with an Ed25519 secret key.
pub fn sign_identity(identity: &SnapshotIdentity, secret: &SecretKey) -> SnapshotSignature {
    let public_key = protocol::derive_public_key(KeyType::Ed25519, secret)
        .expect("Ed25519 public key derivation");
    let signature = protocol::sign::sign(&public_key, secret, &signed_message(identity))
        .expect("Ed25519 signing");
    SnapshotSignature {
        identity: identity.clone(),
        public_key,
        signature,
    }
}

pub fn to_json(sig: &SnapshotSignature) -> Value {
    json!({
        "format": SIGNATURE_FORMAT,
        "ledger_seq": sig.identity.ledger_seq,
        "ledger_hash": hex::encode_upper(sig.identity.ledger_hash),
        "file_sha256": hex::encode_upper(sig.identity.file_sha256),
        "public_key": sig.public_key.to_node_public_base58(),
        "signature": hex::encode_upper(&sig.signature),
    })
}

fn hex32(value: &Value, field: &str) -> Result<[u8; 32], SignatureError> {
    let text = value[field]
        .as_str()
        .ok_or_else(|| SignatureError::Malformed(format!("{field} missing")))?;
    hex::decode(text)
        .ok()
        .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
        .ok_or_else(|| SignatureError::Malformed(format!("{field} is not 32 hex bytes")))
}

/// Parse a node-public `n…` key; only Ed25519 keys are accepted.
pub fn parse_public_key(text: &str) -> Result<PublicKey, SignatureError> {
    let key = protocol::parse_base58_with_type::<PublicKey>(TokenType::NodePublic, text)
        .ok_or_else(|| SignatureError::Malformed(format!("bad public key {text}")))?;
    if key.key_type() != Some(KeyType::Ed25519) {
        return Err(SignatureError::Malformed(format!("{text} is not an Ed25519 key")));
    }
    Ok(key)
}

pub fn from_json(value: &Value) -> Result<SnapshotSignature, SignatureError> {
    if value["format"].as_u64() != Some(SIGNATURE_FORMAT) {
        return Err(SignatureError::Malformed("unsupported format".into()));
    }
    let ledger_seq = value["ledger_seq"]
        .as_u64()
        .and_then(|seq| u32::try_from(seq).ok())
        .ok_or_else(|| SignatureError::Malformed("ledger_seq missing".into()))?;
    let public_key = parse_public_key(
        value["public_key"]
            .as_str()
            .ok_or_else(|| SignatureError::Malformed("public_key missing".into()))?,
    )?;
    let signature = value["signature"]
        .as_str()
        .and_then(|text| hex::decode(text).ok())
        .ok_or_else(|| SignatureError::Malformed("signature is not hex".into()))?;
    Ok(SnapshotSignature {
        identity: SnapshotIdentity {
            ledger_seq,
            ledger_hash: hex32(value, "ledger_hash")?,
            file_sha256: hex32(value, "file_sha256")?,
        },
        public_key,
        signature,
    })
}

/// Check `sig` is a valid signature of `actual` by one of `trusted`.
pub fn verify(
    sig: &SnapshotSignature,
    actual: &SnapshotIdentity,
    trusted: &[PublicKey],
) -> Result<(), SignatureError> {
    if sig.identity != *actual {
        return Err(SignatureError::IdentityMismatch(format!(
            "signature names ledger {} file {}, snapshot is ledger {} file {}",
            sig.identity.ledger_seq,
            hex::encode_upper(sig.identity.file_sha256),
            actual.ledger_seq,
            hex::encode_upper(actual.file_sha256)
        )));
    }
    if !protocol::sign::verify(&sig.public_key, &signed_message(actual), &sig.signature) {
        return Err(SignatureError::BadSignature);
    }
    if !trusted.contains(&sig.public_key) {
        return Err(SignatureError::UntrustedKey(sig.public_key.to_node_public_base58()));
    }
    Ok(())
}

/// Sign a snapshot file and write `<snapshot>.sig`. Returns the `.sig` path.
pub fn sign_file(snapshot: &Path, secret: &SecretKey) -> Result<PathBuf, SignatureError> {
    let identity = read_identity(snapshot)?;
    let sig = sign_identity(&identity, secret);
    let path = signature_path(snapshot);
    let text = serde_json::to_string_pretty(&to_json(&sig)).expect("json");
    std::fs::write(&path, text + "\n").map_err(|e| SignatureError::Io(e.to_string()))?;
    Ok(path)
}

/// Verify `<snapshot>.sig` against the file and the trusted keys. Returns the
/// signer's key and the attested identity.
pub fn verify_file(
    snapshot: &Path,
    trusted: &[PublicKey],
) -> Result<SnapshotSignature, SignatureError> {
    let path = signature_path(snapshot);
    if !path.exists() {
        return Err(SignatureError::Missing(path));
    }
    let text = std::fs::read_to_string(&path).map_err(|e| SignatureError::Io(e.to_string()))?;
    let value: Value =
        serde_json::from_str(&text).map_err(|e| SignatureError::Malformed(e.to_string()))?;
    let sig = from_json(&value)?;
    verify(&sig, &read_identity(snapshot)?, trusted)?;
    Ok(sig)
}
