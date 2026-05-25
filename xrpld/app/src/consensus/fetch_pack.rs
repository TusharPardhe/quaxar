//! Interface for fetch pack storage (delta sync optimization).
//!

use basics::base_uint::Uint256;

/// Trait for retrieving fetch packs without an application or ledgermaster object.
pub trait FetchPackContainer: Send + Sync {
    /// Retrieves partial ledger data of the corresponding hash from peers.
    /// Returns `None` if the hash isn't cached.
    fn get_fetch_pack(&self, node_hash: &Uint256) -> Option<Vec<u8>>;
}
