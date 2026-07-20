//! Unified inbound ledger acquisition service.
//!
//! This module replaces the old `inbound_ledgers` field and the legacy
//! `acquisition/inbound_ledgers.rs` with a single clean implementation
//! matching rippled's InboundLedgers architecture:
//!
//! - ONE global registry keyed by ledger hash
//! - Touch-on-access keeps entries alive
//! - 60s sweep removes idle entries
//! - 5-minute failure cooldown prevents retry storms
//! - Fixed worker pool (8 threads) processes short ticks
//! - Each acquisition wraps InboundLedgerLocal (the per-ledger state machine)

mod acquisition;
mod registry;
mod worker_pool;

pub use self::acquisition::{
    AcquisitionState, NodeStoreWriteMsg, PendingNodeStoreObject, RunDataLimiter,
    spawn_nodestore_writer, stash_stale_packet,
};
pub use self::registry::{AcquireReason, InboundLedgers};
pub use self::worker_pool::WorkerPool;

// ─── Backward-compatible stub ────────────────────────────────────────────────
//
// `InboundLedgersLocal` was removed from the `ledger` crate. This minimal stub
// keeps `AppInboundLedgers` and its callers compiling until they are rewired to
// use the new `InboundLedgers` service directly.

use basics::tagged_cache::MonotonicClock;
use basics::sha_map_hash::SHAMapHash;
use ledger::InboundLedgerLocal;
use std::collections::BTreeMap;
use basics::base_uint::Uint256;

/// Minimal backward-compatible stub for the removed `ledger::InboundLedgersLocal`.
/// Will be deleted once all callers are rewired to use `InboundLedgers`.
#[derive(Debug)]
pub struct InboundLedgersLocal<C = MonotonicClock> {
    _clock: std::marker::PhantomData<C>,
    ledgers: BTreeMap<Uint256, InboundLedgerLocal>,
    stopping: bool,
}

impl InboundLedgersLocal<MonotonicClock> {
    pub fn new() -> Self {
        Self {
            _clock: std::marker::PhantomData,
            ledgers: BTreeMap::new(),
            stopping: false,
        }
    }
}

impl Default for InboundLedgersLocal<MonotonicClock> {
    fn default() -> Self {
        Self::new()
    }
}

impl<C> InboundLedgersLocal<C> {
    pub fn stop(&mut self) {
        self.stopping = true;
    }

    pub fn is_stopping(&self) -> bool {
        self.stopping
    }

    pub fn insert(&mut self, inbound: InboundLedgerLocal) -> Option<InboundLedgerLocal> {
        self.ledgers.insert(*inbound.hash().as_uint256(), inbound)
    }

    pub fn find(&self, hash: SHAMapHash) -> Option<&InboundLedgerLocal> {
        self.ledgers.get(hash.as_uint256())
    }

    pub fn find_by_seq(&self, seq: u32) -> Option<&InboundLedgerLocal> {
        self.ledgers.values().find(|inbound| inbound.seq() == seq)
    }

    pub fn find_mut(&mut self, hash: SHAMapHash) -> Option<&mut InboundLedgerLocal> {
        self.ledgers.get_mut(hash.as_uint256())
    }

    pub fn remove(&mut self, hash: SHAMapHash) -> Option<InboundLedgerLocal> {
        self.ledgers.remove(hash.as_uint256())
    }

    pub fn remove_by_seq(&mut self, seq: u32) -> Option<InboundLedgerLocal> {
        let key = self
            .ledgers
            .iter()
            .find(|(_, v)| v.seq() == seq)
            .map(|(k, _)| *k)?;
        self.ledgers.remove(&key)
    }
}
