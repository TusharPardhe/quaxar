//! Shared pseudo-transaction insertion seam for app-owned voting helpers.
//!
//! The current xrpld callers inject `ttFEE` and `ttUNL_MODIFY` transactions
//! directly into the consensus transaction set. This module keeps that write
//! surface explicit so the vote helpers can be unit-tested against a simple
//! vector while still supporting real `SHAMap` insertion for the migrated app
//! runtime.

use std::hash::BuildHasher;

use basics::tagged_cache::CacheClock;
use protocol::{STTx, serialize_blob};
use shamap::{item::SHAMapItem, storage::StorageTree, tree_node::SHAMapNodeType};

pub trait VoteTxSet {
    fn add_transaction(&mut self, tx: &STTx) -> bool;
}

impl VoteTxSet for Vec<STTx> {
    fn add_transaction(&mut self, tx: &STTx) -> bool {
        self.push(tx.clone());
        true
    }
}

pub struct ShamapVoteTxSet<'a, C, S> {
    set: &'a mut StorageTree<C, S>,
}

impl<'a, C, S> ShamapVoteTxSet<'a, C, S> {
    pub fn new(set: &'a mut StorageTree<C, S>) -> Self {
        Self { set }
    }
}

impl<C, S> VoteTxSet for ShamapVoteTxSet<'_, C, S>
where
    C: CacheClock,
    S: BuildHasher + Clone,
{
    fn add_transaction(&mut self, tx: &STTx) -> bool {
        self.set
            .add_item(
                SHAMapNodeType::TransactionNm,
                SHAMapItem::new(tx.get_transaction_id(), serialize_blob(tx)),
            )
            .unwrap_or(false)
    }
}
