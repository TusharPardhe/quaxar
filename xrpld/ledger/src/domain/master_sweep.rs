//! Thin owner-level `LedgerMaster::sweep()` parity wrapper above the currently
//! landed Rust history and fetch-pack cache owners.

use crate::{FetchPackCache, LedgerHistory};
use basics::tagged_cache::CacheClock;
use std::hash::BuildHasher;

pub trait LedgerMasterSweepTarget {
    fn sweep(&self);
}

pub fn sweep_ledger_master_like<H, F>(history: &H, fetch_pack: &F)
where
    H: LedgerMasterSweepTarget,
    F: LedgerMasterSweepTarget,
{
    history.sweep();
    fetch_pack.sweep();
}

impl<C, S> LedgerMasterSweepTarget for LedgerHistory<C, S>
where
    C: CacheClock,
    S: BuildHasher + Clone,
{
    fn sweep(&self) {
        LedgerHistory::sweep(self);
    }
}

impl<C, S> LedgerMasterSweepTarget for FetchPackCache<C, S>
where
    C: CacheClock,
    S: BuildHasher + Clone,
{
    fn sweep(&self) {
        FetchPackCache::sweep(self);
    }
}
