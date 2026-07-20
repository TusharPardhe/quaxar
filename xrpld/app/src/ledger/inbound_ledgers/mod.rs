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
// NOTE: this is NOT the acquisition registry used to sync ledgers over the
// network -- that is `InboundLedgers` above, owned by
// `AppLedgerMasterRuntime.inbound_ledgers` and driven by the bootstrap
// consensus loop / catchup loop.
//
// This type is a small resumable-request cache used exclusively by the RPC
// layer (`rpc/src/state/context.rs`: `take_resumable_inbound_ledger` /
// `persist_resumable_inbound_ledger`) to park a partially-fetched
// `InboundLedgerLocal` between paginated admin `ledger_data` requests. It has
// no relationship to peer acquisition and must not be treated as dead code
// or merged with `InboundLedgers`; the historical name is kept only to avoid
// an unrelated rename across its RPC call sites.

use basics::tagged_cache::MonotonicClock;
use basics::sha_map_hash::SHAMapHash;
use ledger::InboundLedgerLocal;
use std::collections::BTreeMap;
use basics::base_uint::Uint256;

/// RPC-owned resumable-ledger-request cache keyed by ledger hash. See the
/// module-level note above for what this type actually does and does not do.
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
