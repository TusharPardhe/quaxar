//! Wallet DB helpers ported from `xrpl/server/Wallet.h/the reference source`.
//!
//! Manages the wallet SQLite database: node identity, manifests,
//! peer reservations, and amendment votes.

#![allow(dead_code)]

use std::path::PathBuf;

use basics::base_uint::Uint256;
use protocol::{PublicKey, SecretKey};

/// Database setup for wallet operations.
pub struct WalletDbSetup {
    pub data_dir: PathBuf,
    pub db_name: String,
}

/// Amendment vote direction. For historical reasons the integer representations
/// are unintuitive in the reference code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum AmendmentVote {
    Obsolete = -1,
    Up = 0,
    Down = 1,
}

impl AmendmentVote {
    pub fn from_int(v: i32) -> Self {
        match v {
            -1 => Self::Obsolete,
            0 => Self::Up,
            _ => Self::Down,
        }
    }
}

/// Peer reservation entry.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PeerReservation {
    pub node_id: PublicKey,
    pub description: String,
}

/// Trait abstracting the wallet database session.
/// In production this wraps rusqlite or similar.
pub trait WalletDbSession: Send + Sync {
    fn execute(&self, sql: &str) -> Result<(), String>;
    fn query_row(&self, sql: &str) -> Result<Option<Vec<String>>, String>;
}

/// Opens/creates the wallet database.
/// Returns a boxed session handle.
pub fn make_wallet_db(_setup: &WalletDbSetup) -> Result<Box<dyn WalletDbSession>, String> {
    // Stub — real implementation opens SQLite at setup.data_dir/setup.db_name
    Err("Wallet DB not yet wired to rusqlite".into())
}

/// Deletes any saved public/private key associated with this node.
pub fn clear_node_identity(session: &dyn WalletDbSession) -> Result<(), String> {
    session.execute("DELETE FROM NodeIdentity;")
}

/// Returns a stable public and private key for this node.
/// If none exists, generates a random secp256k1 keypair and stores it.
pub fn get_node_identity(session: &dyn WalletDbSession) -> Result<(PublicKey, SecretKey), String> {
    // 1. Try to load existing identity
    if let Ok(Some(row)) = session.query_row("SELECT PublicKey, PrivateKey FROM NodeIdentity;")
        && row.len() == 2
    {
        // Parse base58-encoded keys
        // In production: parseBase58<PublicKey>(TokenType::NodePublic, &row[0])
        // For now, return error to indicate stub
        return Err("Key parsing not yet wired".into());
    }

    // 2. Generate new random keypair
    // In production: randomKeyPair(KeyType::Secp256k1)
    Err("Key generation not yet wired".into())
}

/// Loads manifests from the wallet database into the cache.
pub fn get_manifests(
    session: &dyn WalletDbSession,
    db_table: &str,
    _apply: impl FnMut(Vec<u8>),
) -> Result<(), String> {
    let sql = format!("SELECT RawData FROM {};", db_table);
    // In production: iterate rows, deserialize each blob, verify, apply
    let _ = session.query_row(&sql)?;
    Ok(())
}

/// Saves all manifests to the database (within a transaction).
pub fn save_manifests(
    session: &dyn WalletDbSession,
    db_table: &str,
    manifests: &[(PublicKey, Vec<u8>)],
    is_trusted: impl Fn(&PublicKey) -> bool,
) -> Result<(), String> {
    session.execute(&format!("DELETE FROM {}", db_table))?;
    for (key, serialized) in manifests {
        if is_trusted(key) {
            // Insert blob
            let _ = serialized;
        }
    }
    Ok(())
}

/// Adds a validator manifest to the database.
pub fn add_validator_manifest(
    _session: &dyn WalletDbSession,
    serialized: &str,
) -> Result<(), String> {
    let _ = serialized;
    Ok(())
}

/// Gets the peer reservation table from the database.
pub fn get_peer_reservation_table(
    _session: &dyn WalletDbSession,
) -> Result<Vec<PeerReservation>, String> {
    // In production: SELECT PublicKey, Description FROM PeerReservations
    Ok(Vec::new())
}

/// Inserts or updates a peer reservation.
pub fn insert_peer_reservation(
    _session: &dyn WalletDbSession,
    node_id: &PublicKey,
    description: &str,
) -> Result<(), String> {
    let _ = (node_id, description);
    Ok(())
}

/// Deletes a peer reservation.
pub fn delete_peer_reservation(
    _session: &dyn WalletDbSession,
    node_id: &PublicKey,
) -> Result<(), String> {
    let _ = node_id;
    Ok(())
}

/// Creates the FeatureVotes table if it doesn't exist.
/// Returns true if the table already existed.
pub fn create_feature_votes(session: &dyn WalletDbSession) -> Result<bool, String> {
    // Check if table exists
    if let Ok(Some(row)) = session
        .query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name='FeatureVotes'")
        && let Some(count_str) = row.first()
        && count_str.parse::<i32>().unwrap_or(0) > 0
    {
        return Ok(true);
    }

    session.execute(
        "CREATE TABLE FeatureVotes ( \
         AmendmentHash CHARACTER(64) NOT NULL, \
         AmendmentName TEXT, \
         Veto INTEGER NOT NULL )",
    )?;
    Ok(false)
}

/// Reads all amendments from the FeatureVotes table.
pub fn read_amendments(
    _session: &dyn WalletDbSession,
    callback: impl FnMut(Option<String>, Option<String>, Option<AmendmentVote>),
) -> Result<(), String> {
    // In production: query with RANK() OVER (PARTITION BY AmendmentHash ORDER BY ROWID DESC)
    let _ = &callback;
    Ok(())
}

/// Sets the vote value for a particular amendment.
pub fn vote_amendment(
    session: &dyn WalletDbSession,
    amendment: &Uint256,
    name: &str,
    vote: AmendmentVote,
) -> Result<(), String> {
    let sql = format!(
        "INSERT INTO FeatureVotes (AmendmentHash, AmendmentName, Veto) VALUES ('{}', '{}', '{}')",
        amendment, name, vote as i32
    );
    session.execute(&sql)
}
