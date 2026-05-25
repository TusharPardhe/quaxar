//! Pure `LedgerMaster::fetchForHistory(...)` decision core above the current
//! Rust ledger, inbound, and SQL seams.
//!
//! The reference owner wraps this logic in `Application`, `JobQueue`,
//! `InboundLedgers`, and mutable `LedgerMaster` state. This module ports the
//! deterministic branch selection and state updates only.

use crate::{InboundLedgerReason, Ledger, LedgerHeader};
use basics::sha_map_hash::SHAMapHash;
use std::sync::Arc;

pub trait HistoryHashLookup {
    fn get_ledger_hash_for_history(
        &self,
        ledger_index: u32,
        reason: InboundLedgerReason,
    ) -> Option<SHAMapHash>;
}

pub trait HistoryLedgerLookup<L> {
    fn get_ledger_by_hash(&self, hash: SHAMapHash) -> Option<L>;
}

pub trait HistoryInboundAcquire<L> {
    fn is_failure(&self, hash: SHAMapHash) -> bool;
    fn acquire(&self, hash: SHAMapHash, seq: u32, reason: InboundLedgerReason) -> Option<L>;
}

pub trait HistorySqlInfo {
    fn earliest_ledger_seq(&self) -> u32;
    fn get_hash_by_index(&self, ledger_index: u32) -> SHAMapHash;
}

pub trait HistoryLedgerLike: Clone {
    fn seq(&self) -> u32;
    fn parent_hash(&self) -> SHAMapHash;
}

impl HistoryLedgerLike for LedgerHeader {
    fn seq(&self) -> u32 {
        self.seq
    }

    fn parent_hash(&self) -> SHAMapHash {
        self.parent_hash
    }
}

impl HistoryLedgerLike for Arc<Ledger> {
    fn seq(&self) -> u32 {
        self.header().seq
    }

    fn parent_hash(&self) -> SHAMapHash {
        self.header().parent_hash
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FetchForHistoryState<L> {
    pub fetch_seq: u32,
    pub fill_in_progress: u32,
    pub hist_ledger: Option<L>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchPackRequest {
    pub missing: u32,
    pub reason: InboundLedgerReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefetchAcquire {
    pub seq: u32,
    pub hash: SHAMapHash,
    pub reason: InboundLedgerReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchForHistoryResult<L> {
    pub progress: bool,
    pub next_state: FetchForHistoryState<L>,
    pub set_full_ledger: Option<L>,
    pub schedule_try_fill: Option<L>,
    pub request_fetch_pack: Option<FetchPackRequest>,
    pub prefetch: Vec<PrefetchAcquire>,
    pub clear_ledger: Option<u32>,
    pub missing_hash: bool,
}

#[allow(clippy::too_many_arguments)]
pub fn run_fetch_for_history<L, H, GL, IA, DB>(
    missing: u32,
    reason: InboundLedgerReason,
    ledger_fetch_size: u32,
    state: &FetchForHistoryState<L>,
    hashes: &H,
    ledger_lookup: &GL,
    inbound: &IA,
    sql: &DB,
) -> FetchForHistoryResult<L>
where
    L: HistoryLedgerLike,
    H: HistoryHashLookup,
    GL: HistoryLedgerLookup<L>,
    IA: HistoryInboundAcquire<L>,
    DB: HistorySqlInfo,
{
    let mut next_state = state.clone();
    let mut set_full_ledger = None;
    let mut schedule_try_fill = None;
    let mut request_fetch_pack = None;
    let mut prefetch = Vec::new();
    let mut clear_ledger = None;
    let mut progress = false;
    let mut missing_hash = false;

    let Some(hash) = hashes.get_ledger_hash_for_history(missing, reason) else {
        clear_ledger = Some(missing + 1);
        progress = true;
        missing_hash = true;
        return FetchForHistoryResult {
            progress,
            next_state,
            set_full_ledger,
            schedule_try_fill,
            request_fetch_pack,
            prefetch,
            clear_ledger,
            missing_hash,
        };
    };
    assert!(
        hash.is_non_zero(),
        "xrpl::LedgerMaster::fetchForHistory : found ledger"
    );

    let mut ledger = ledger_lookup.get_ledger_by_hash(hash);
    if ledger.is_none() && !inbound.is_failure(hash) {
        ledger = inbound.acquire(hash, missing, reason);
        if ledger.is_none()
            && missing != next_state.fetch_seq
            && missing > sql.earliest_ledger_seq()
        {
            next_state.fetch_seq = missing;
            request_fetch_pack = Some(FetchPackRequest { missing, reason });
        }
    }

    if let Some(ledger) = ledger {
        assert!(
            ledger.seq() == missing,
            "xrpl::LedgerMaster::fetchForHistory : sequence match"
        );
        next_state.hist_ledger = Some(ledger.clone());
        set_full_ledger = Some(ledger.clone());
        if next_state.fill_in_progress == 0
            && sql.get_hash_by_index(ledger.seq().saturating_sub(1)) == ledger.parent_hash()
        {
            next_state.fill_in_progress = ledger.seq();
            schedule_try_fill = Some(ledger);
        }
        progress = true;
    } else {
        let earliest = sql.earliest_ledger_seq();
        let fetch_size = if missing >= earliest {
            ledger_fetch_size.min((missing - earliest) + 1)
        } else {
            0
        };
        for offset in 0..fetch_size {
            let seq = missing - offset;
            if let Some(hash) = hashes.get_ledger_hash_for_history(seq, reason) {
                prefetch.push(PrefetchAcquire { seq, hash, reason });
            }
        }
    }

    FetchForHistoryResult {
        progress,
        next_state,
        set_full_ledger,
        schedule_try_fill,
        request_fetch_pack,
        prefetch,
        clear_ledger,
        missing_hash,
    }
}
