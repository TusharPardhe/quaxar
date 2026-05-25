//! Consensus transaction types: RCLCxTx and RCLTxSet.
//!

use basics::base_uint::Uint256;
use std::collections::BTreeMap;
use std::sync::Arc;

/// A single transaction in consensus — thin wrapper over key + payload.
#[derive(Clone, Debug)]
pub struct RCLCxTx {
    key: Uint256,
    pub payload: Arc<Vec<u8>>,
}

impl RCLCxTx {
    pub fn new(key: Uint256, payload: Vec<u8>) -> Self {
        Self {
            key,
            payload: Arc::new(payload),
        }
    }

    pub fn id(&self) -> &Uint256 {
        &self.key
    }
}

/// An immutable set of transactions in consensus.
#[derive(Clone, Debug)]
pub struct RCLTxSet {
    map: BTreeMap<Uint256, RCLCxTx>,
    hash: Uint256,
}

impl RCLTxSet {
    pub fn new(hash: Uint256, map: BTreeMap<Uint256, RCLCxTx>) -> Self {
        Self { map, hash }
    }

    pub fn exists(&self, id: &Uint256) -> bool {
        self.map.contains_key(id)
    }

    pub fn find(&self, id: &Uint256) -> Option<&RCLCxTx> {
        self.map.get(id)
    }

    pub fn id(&self) -> &Uint256 {
        &self.hash
    }

    /// Find transactions not in common between this and another set.
    /// Returns map of tx_id -> true if in self, false if in other.
    pub fn compare(&self, other: &RCLTxSet) -> BTreeMap<Uint256, bool> {
        let mut result = BTreeMap::new();
        for id in self.map.keys() {
            if !other.map.contains_key(id) {
                result.insert(*id, true);
            }
        }
        for id in other.map.keys() {
            if !self.map.contains_key(id) {
                result.insert(*id, false);
            }
        }
        result
    }

    /// Create a mutable snapshot.
    pub fn snapshot_mutable(&self) -> MutableTxSet {
        MutableTxSet {
            map: self.map.clone(),
        }
    }
}

/// A mutable view of a transaction set.
#[derive(Clone, Debug)]
pub struct MutableTxSet {
    map: BTreeMap<Uint256, RCLCxTx>,
}

impl MutableTxSet {
    pub fn insert(&mut self, tx: RCLCxTx) -> bool {
        let id = *tx.id();
        self.map.insert(id, tx).is_none()
    }

    pub fn erase(&mut self, id: &Uint256) -> bool {
        self.map.remove(id).is_some()
    }
}
