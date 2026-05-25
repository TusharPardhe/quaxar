//! Owner-level `LedgerMaster` composition above the landed Rust ledger slices.
//!
//! This ports the ledger-owned state and gating that do not need the wider
//! application graph:
//! - validated / published / closed ledger holders,
//! - complete-ledger tracking and cache clearing,
//! - validated-range and transaction-id lookup for cached ledgers,
//! - published / validated ledger age calculations,
//! - fetch-pack cache ownership and single-dispatch gating,
//! - and the pathfinding work-dispatch counters.

use crate::{
    CanonicalTXSet, FetchPackCache, Ledger, LedgerConfig, LedgerHistory, LedgerHolder,
    LedgerJournal, LedgerMasterSweepTarget, LedgerPersistence, LocalTxs, NullLedgerJournal,
    SHAMapHash, sweep_ledger_master_like,
};
use basics::base_uint::Uint256;
use basics::hardened_hash::HardenedHashBuilder;
use basics::range_set::prev_missing;
use basics::range_set::{RangeSet, range};
use basics::tagged_cache::{CacheClock, MonotonicClock};
use protocol::STTx;
use shamap::family::{FullBelowCache, MissingNodeReporter, SHAMapFamily, SHAMapNodeFetcher};
use shamap::traversal::TraversalError;
use std::hash::BuildHasher;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use time::Duration;

pub const LEDGER_MASTER_DEFAULT_HISTORY_AGE: Duration = Duration::minutes(5);
pub const LEDGER_MASTER_DEFAULT_FETCH_PACK_AGE: Duration = Duration::seconds(45);
pub const LEDGER_MASTER_DEFAULT_PATH_FIND_JOB_LIMIT: u32 = 2;
pub const LEDGER_MASTER_MAX_PUBLISH_GAP: u32 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LedgerMasterConfig {
    pub history_cache_size: usize,
    pub history_cache_age: Duration,
    pub fetch_pack_cache_size: usize,
    pub fetch_pack_cache_age: Duration,
    pub path_find_job_limit: u32,
}

impl Default for LedgerMasterConfig {
    fn default() -> Self {
        Self {
            history_cache_size: 256,
            history_cache_age: LEDGER_MASTER_DEFAULT_HISTORY_AGE,
            fetch_pack_cache_size: 512,
            fetch_pack_cache_age: LEDGER_MASTER_DEFAULT_FETCH_PACK_AGE,
            path_find_job_limit: LEDGER_MASTER_DEFAULT_PATH_FIND_JOB_LIMIT,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedgerMasterCaughtUp {
    Yes,
    No { reason: &'static str },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedgerMasterPathWork {
    NewRequest,
    OrderBookDb,
}

#[derive(Debug, Default)]
struct PathState {
    path_ledger: Option<Arc<Ledger>>,
    path_find_threads: u32,
    new_request: bool,
}

#[derive(Debug)]
pub struct LedgerMaster<C = MonotonicClock, S = HardenedHashBuilder> {
    config: LedgerMasterConfig,
    closed_ledger: LedgerHolder,
    valid_ledger: LedgerHolder,
    published_ledger: LedgerHolder,
    ledger_history: LedgerHistory<C, S>,
    fetch_packs: FetchPackCache<C, S>,
    local_txs: LocalTxs,
    held_transactions: Mutex<CanonicalTXSet>,
    complete_ledgers: Mutex<RangeSet<u32>>,
    path_state: Mutex<PathState>,
    got_fetch_pack_in_flight: AtomicBool,
    pub_ledger_close: AtomicU32,
    valid_ledger_sign: AtomicU32,
    valid_ledger_seq: AtomicU32,
}

impl<C> LedgerMaster<C, HardenedHashBuilder>
where
    C: CacheClock + Clone,
{
    pub fn new(clock: C, config: LedgerMasterConfig) -> Self {
        Self::with_hasher(clock, HardenedHashBuilder::default(), config)
    }
}

impl<C, S> LedgerMaster<C, S>
where
    C: CacheClock + Clone,
    S: BuildHasher + Clone,
{
    pub fn with_hasher(clock: C, hasher: S, config: LedgerMasterConfig) -> Self {
        Self::with_parts(
            LedgerHistory::with_hasher(
                config.history_cache_size,
                config.history_cache_age,
                clock.clone(),
                hasher.clone(),
            ),
            FetchPackCache::with_hasher(
                config.fetch_pack_cache_size,
                config.fetch_pack_cache_age,
                clock,
                hasher,
            ),
            LocalTxs::new(),
            config,
        )
    }

    pub fn with_parts(
        ledger_history: LedgerHistory<C, S>,
        fetch_packs: FetchPackCache<C, S>,
        local_txs: LocalTxs,
        config: LedgerMasterConfig,
    ) -> Self {
        Self {
            config,
            closed_ledger: LedgerHolder::new(),
            valid_ledger: LedgerHolder::new(),
            published_ledger: LedgerHolder::new(),
            ledger_history,
            fetch_packs,
            local_txs,
            held_transactions: Mutex::new(CanonicalTXSet::new(Uint256::zero())),
            complete_ledgers: Mutex::new(RangeSet::new()),
            path_state: Mutex::new(PathState::default()),
            got_fetch_pack_in_flight: AtomicBool::new(false),
            pub_ledger_close: AtomicU32::new(0),
            valid_ledger_sign: AtomicU32::new(0),
            valid_ledger_seq: AtomicU32::new(0),
        }
    }

    pub fn config(&self) -> LedgerMasterConfig {
        self.config
    }

    pub fn ledger_history(&self) -> &LedgerHistory<C, S> {
        &self.ledger_history
    }

    pub fn fetch_pack_cache(&self) -> &FetchPackCache<C, S> {
        &self.fetch_packs
    }

    pub fn local_txs(&self) -> &LocalTxs {
        &self.local_txs
    }

    pub fn add_held_transaction(&self, transaction: Arc<STTx>) {
        self.held_transactions
            .lock()
            .expect("held-transactions mutex must not be poisoned")
            .insert(transaction);
    }

    pub fn held_transaction_count(&self) -> usize {
        self.held_transactions
            .lock()
            .expect("held-transactions mutex must not be poisoned")
            .len()
    }

    pub fn pop_acct_transaction(&self, tx: &Arc<STTx>) -> Option<Arc<STTx>> {
        self.held_transactions
            .lock()
            .expect("held-transactions mutex must not be poisoned")
            .pop_acct_transaction(tx)
    }

    pub fn take_held_transactions(&self, next_ledger_hash: Uint256) -> CanonicalTXSet {
        let mut held_transactions = self
            .held_transactions
            .lock()
            .expect("held-transactions mutex must not be poisoned");
        let mut set = CanonicalTXSet::new(next_ledger_hash);
        std::mem::swap(&mut *held_transactions, &mut set);
        set
    }

    pub fn apply_held_transactions<F>(&self, next_ledger_hash: Uint256, mut process: F) -> usize
    where
        F: FnMut(CanonicalTXSet),
    {
        let set = self.take_held_transactions(next_ledger_hash);
        let count = set.len();
        if !set.is_empty() {
            process(set);
        }
        count
    }

    pub fn set_closed_ledger(&self, ledger: Arc<Ledger>) {
        self.closed_ledger.set(Some(ledger));
    }

    pub fn closed_ledger(&self) -> Option<Arc<Ledger>> {
        self.closed_ledger.get()
    }

    pub fn validated_ledger(&self) -> Option<Arc<Ledger>> {
        self.valid_ledger.get()
    }

    pub fn published_ledger(&self) -> Option<Arc<Ledger>> {
        self.published_ledger.get()
    }

    pub fn get_ledger_by_hash(&self, hash: SHAMapHash) -> Option<Arc<Ledger>> {
        if let Some(ledger) = self.ledger_history.get_cached_ledger_by_hash(hash) {
            return Some(ledger);
        }

        let ledger = self.closed_ledger.get();
        if ledger
            .as_ref()
            .is_some_and(|ledger| ledger.header().hash == hash)
        {
            return ledger;
        }

        None
    }

    pub fn get_ledger_by_seq<J: LedgerJournal>(
        &self,
        seq: u32,
        journal: &J,
    ) -> Option<Arc<Ledger>> {
        if seq <= self.valid_ledger_seq()
            && let Some(valid) = self.valid_ledger.get()
        {
            if valid.header().seq == seq {
                return Some(valid);
            }

            if let Some(hash) = valid.hash_of_seq(seq, journal)
                && let Some(ledger) = self.ledger_history.get_cached_ledger_by_hash(hash)
            {
                return Some(ledger);
            }
        }

        if let Some(ledger) = self.ledger_history.get_cached_ledger_by_seq(seq) {
            tracing::debug!(target: "ledger", seq, "Ledger fetched from history");
            return Some(ledger);
        }

        let ledger = self.closed_ledger.get();
        if ledger
            .as_ref()
            .is_some_and(|ledger| ledger.header().seq == seq)
        {
            return ledger;
        }

        self.clear_ledger(seq);
        None
    }

    pub fn valid_ledger_seq(&self) -> u32 {
        self.valid_ledger_seq.load(Ordering::SeqCst)
    }

    pub fn have_validated(&self) -> bool {
        !self.valid_ledger.empty()
    }

    /// too far from network close time, or too far ahead of the valid ledger.
    pub fn can_be_current(&self, ledger: &Ledger, now_close_time: u32) -> bool {
        if let Some(valid_ledger) = self.validated_ledger()
            && ledger.header().seq < valid_ledger.header().seq
        {
            return false;
        }

        let parent_close_time = ledger.header().parent_close_time;
        // Use a larger limit when the validated ledger itself is old (we're catching up).
        // bypasses can_be_current entirely — so this only applies to no-quorum cases.
        let age_limit = Duration::minutes(5);
        if (self.have_validated() || ledger.header().seq > 10)
            && close_time_distance(now_close_time, parent_close_time) > age_limit
        {
            return false;
        }

        if let Some(valid_ledger) = self.validated_ledger() {
            let mut max_seq = valid_ledger.header().seq.saturating_add(10);
            if now_close_time > valid_ledger.header().parent_close_time {
                max_seq = max_seq.saturating_add(
                    now_close_time.saturating_sub(valid_ledger.header().parent_close_time) / 2,
                );
            }

            if ledger.header().seq > max_seq {
                return false;
            }
        }

        true
    }

    /// validation quorum, and the current validated sequence before promotion.
    pub fn check_accept_ledger(
        &self,
        ledger: &Ledger,
        validation_count: usize,
        needed_validations: usize,
        now_close_time: u32,
    ) -> bool {
        if !self.can_be_current(ledger, now_close_time) {
            return false;
        }
        if ledger.header().seq <= self.valid_ledger_seq() {
            return false;
        }
        if validation_count < needed_validations {
            return false;
        }
        true
    }

    /// work is needed. Returns true if advancement should be attempted.
    pub fn try_advance(&self) -> bool {
        // Can only advance with at least one validated ledger
        self.have_validated()
    }

    ///
    /// Advances the published ledger to match the validated ledger.
    /// Returns the number of ledgers published.
    pub fn do_advance(&self) -> usize {
        let valid = self.validated_ledger();
        let pub_ledger = self.published_ledger();

        let valid_seq = valid.as_ref().map(|l| l.header().seq).unwrap_or(0);
        let Some(val_ledger) = valid.as_ref() else {
            return 0;
        };

        if pub_ledger.is_none() {
            self.set_pub_ledger(Arc::clone(val_ledger));
            return 1;
        }

        let pub_seq = pub_ledger.as_ref().map(|l| l.header().seq).unwrap_or(0);
        if valid_seq <= pub_seq {
            return 0;
        }

        if valid_seq > pub_seq.saturating_add(LEDGER_MASTER_MAX_PUBLISH_GAP) {
            let gap_start = pub_seq + 1;
            let gap_end = valid_seq;
            tracing::warn!(target: "ledger", gap_start, gap_end, "Ledger gap detected");
            self.set_pub_ledger(Arc::clone(val_ledger));
            return 1;
        }

        let mut published = 0;

        // using hashOfSeq to get intermediate ledger hashes from the
        // validated ledger's skip list.
        for seq in (pub_seq + 1)..=valid_seq {
            if seq == valid_seq {
                // The validated ledger itself
                self.set_pub_ledger(Arc::clone(val_ledger));
                published += 1;
            } else {
                // Look up hash from validated ledger's skip list
                let hash = val_ledger.hash_of_seq(seq, &NullLedgerJournal);
                if let Some(hash) = hash {
                    if !hash.is_zero() {
                        // Check if we have this ledger in history
                        if let Some(ledger) = self.ledger_history.get_cached_ledger_by_hash(hash) {
                            self.set_pub_ledger(Arc::clone(&ledger));
                            published += 1;
                        } else {
                            // Don't have it — stop here, can't skip
                            break;
                        }
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            }
        }

        published
    }

    pub fn set_valid_ledger(
        &self,
        ledger: Arc<Ledger>,
        consensus_hash: Option<Uint256>,
        sign_time: Option<u32>,
    ) -> Result<(), TraversalError> {
        let validated_seq = ledger.header().seq;
        tracing::info!(target: "ledger", validated_seq, "New validated ledger");
        self.valid_ledger.set(Some(Arc::clone(&ledger)));
        self.valid_ledger_sign.store(
            sign_time.unwrap_or(ledger.header().close_time),
            Ordering::SeqCst,
        );
        self.valid_ledger_seq
            .store(ledger.header().seq, Ordering::SeqCst);
        self.local_txs.sweep(ledger.as_ref())?;
        self.ledger_history.insert(Arc::clone(&ledger), true);
        self.ledger_history.validated_ledger(ledger, consensus_hash);
        Ok(())
    }

    /// Like `set_valid_ledger` but skips `local_txs.sweep()`.
    /// Used during catchup when the ledger's state map may not be fully
    /// traversable (only acquired delta nodes are loaded).
    pub fn set_valid_ledger_no_sweep(
        &self,
        ledger: Arc<Ledger>,
        consensus_hash: Option<Uint256>,
        sign_time: Option<u32>,
    ) {
        self.valid_ledger.set(Some(Arc::clone(&ledger)));
        self.valid_ledger_sign.store(
            sign_time.unwrap_or(ledger.header().close_time),
            Ordering::SeqCst,
        );
        self.valid_ledger_seq
            .store(ledger.header().seq, Ordering::SeqCst);
        self.ledger_history.insert(Arc::clone(&ledger), true);
        self.ledger_history.validated_ledger(ledger, consensus_hash);
    }

    pub fn set_pub_ledger(&self, ledger: Arc<Ledger>) {
        let published_seq = ledger.header().seq;
        tracing::info!(target: "ledger", published_seq, "Ledger published");
        self.pub_ledger_close
            .store(ledger.header().close_time, Ordering::SeqCst);
        self.published_ledger.set(Some(ledger));
    }

    pub fn set_full_ledger(
        &self,
        persistence: &LedgerPersistence,
        mut ledger: Arc<Ledger>,
        is_synchronous: bool,
        is_current: bool,
        consensus_hash: Option<Uint256>,
        sign_time: Option<u32>,
    ) -> Result<bool, TraversalError> {
        {
            let ledger = Arc::make_mut(&mut ledger);
            ledger.set_validated();
            ledger.set_full();
        }

        if is_current {
            self.ledger_history.insert(Arc::clone(&ledger), true);
        }

        if ledger.header().seq != 0 {
            let prev_seq = ledger.header().seq - 1;
            if self.have_ledger(prev_seq) {
                let prev_ledger = self.get_ledger_by_seq(prev_seq, &NullLedgerJournal);
                if prev_ledger
                    .as_ref()
                    .is_none_or(|prev| prev.header().hash != ledger.header().parent_hash)
                {
                    self.fix_mismatch(ledger.as_ref());
                }
            }
        }

        let saved =
            persistence.pend_save_validated(Arc::clone(&ledger), is_synchronous, is_current);
        self.mark_ledger_complete(ledger.header().seq);

        if ledger.header().seq > self.valid_ledger_seq() {
            self.set_valid_ledger(Arc::clone(&ledger), consensus_hash, sign_time)?;
        }

        if self.published_ledger.empty() {
            self.set_pub_ledger(ledger);
        }

        Ok(saved)
    }

    pub fn mark_ledger_complete(&self, seq: u32) {
        self.complete_ledgers
            .lock()
            .expect("complete-ledgers mutex must not be poisoned")
            .insert(seq);
    }

    pub fn have_ledger(&self, seq: u32) -> bool {
        self.complete_ledgers
            .lock()
            .expect("complete-ledgers mutex must not be poisoned")
            .contains(seq)
    }

    pub fn clear_ledger(&self, seq: u32) {
        self.complete_ledgers
            .lock()
            .expect("complete-ledgers mutex must not be poisoned")
            .erase_interval(range(seq, seq));
    }

    pub fn clear_prior_ledgers(&self, seq: u32) {
        if seq == 0 {
            return;
        }

        self.complete_ledgers
            .lock()
            .expect("complete-ledgers mutex must not be poisoned")
            .erase_interval(range(0, seq - 1));
    }

    pub fn complete_ledgers(&self) -> RangeSet<u32> {
        self.complete_ledgers
            .lock()
            .expect("complete-ledgers mutex must not be poisoned")
            .clone()
    }

    pub fn full_validated_range(&self) -> Option<(u32, u32)> {
        let complete_ledgers = self
            .complete_ledgers
            .lock()
            .expect("complete-ledgers mutex must not be poisoned");
        let max = complete_ledgers.last()?;
        if max == 0 {
            return None;
        }

        let min = prev_missing(&complete_ledgers, max, 0).map_or(max, |missing| missing + 1);
        Some((min, max))
    }

    pub fn clear_ledger_cache_prior<P, CLOCK, FB, F, MR, NS, J>(
        &self,
        seq: u32,
        journal: &J,
        config: &LedgerConfig,
        family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
        provider: &P,
    ) -> Result<(), crate::LedgerSetupError>
    where
        P: crate::LedgerInfoProvider,
        CLOCK: CacheClock,
        FB: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
        J: LedgerJournal,
    {
        self.ledger_history
            .clear_ledger_cache_prior(seq, journal, config, family, provider)
    }

    pub fn clear_cached_ledger_entries_prior(&self, seq: u32) {
        self.ledger_history.clear_cached_ledger_entries_prior(seq);
    }

    pub fn get_published_ledger_age(&self, now_close_time: u32) -> Duration {
        ledger_age(self.pub_ledger_close.load(Ordering::SeqCst), now_close_time)
    }

    pub fn get_validated_ledger_age(&self, now_close_time: u32) -> Duration {
        ledger_age(
            self.valid_ledger_sign.load(Ordering::SeqCst),
            now_close_time,
        )
    }

    pub fn is_caught_up(&self, now_close_time: u32) -> LedgerMasterCaughtUp {
        if self.get_published_ledger_age(now_close_time) > Duration::minutes(3) {
            return LedgerMasterCaughtUp::No {
                reason: "No recently-published ledger",
            };
        }

        let valid_close = self.valid_ledger_sign.load(Ordering::SeqCst);
        let pub_close = self.pub_ledger_close.load(Ordering::SeqCst);
        if valid_close == 0 || pub_close == 0 {
            return LedgerMasterCaughtUp::No {
                reason: "No published ledger",
            };
        }

        if valid_close > pub_close.saturating_add(90) {
            return LedgerMasterCaughtUp::No {
                reason: "Published ledger lags validated ledger",
            };
        }

        LedgerMasterCaughtUp::Yes
    }

    pub fn new_path_request(&self, requests_pending: bool, is_stopping: bool) -> bool {
        let scheduled = self.new_pf_work(requests_pending, is_stopping);
        self.path_state
            .lock()
            .expect("path-state mutex must not be poisoned")
            .new_request = scheduled;
        scheduled
    }

    pub fn is_new_path_request(&self) -> bool {
        let mut state = self
            .path_state
            .lock()
            .expect("path-state mutex must not be poisoned");
        let ret = state.new_request;
        state.new_request = false;
        ret
    }

    pub fn new_order_book_db(&self, requests_pending: bool, is_stopping: bool) -> bool {
        let mut state = self
            .path_state
            .lock()
            .expect("path-state mutex must not be poisoned");
        state.path_ledger = None;
        drop(state);
        self.new_pf_work(requests_pending, is_stopping)
    }

    pub fn path_ledger(&self) -> Option<Arc<Ledger>> {
        self.path_state
            .lock()
            .expect("path-state mutex must not be poisoned")
            .path_ledger
            .clone()
    }

    pub fn set_path_ledger(&self, ledger: Option<Arc<Ledger>>) {
        self.path_state
            .lock()
            .expect("path-state mutex must not be poisoned")
            .path_ledger = ledger;
    }

    pub fn path_find_thread_count(&self) -> u32 {
        self.path_state
            .lock()
            .expect("path-state mutex must not be poisoned")
            .path_find_threads
    }

    pub fn complete_path_find_job(&self) {
        let mut state = self
            .path_state
            .lock()
            .expect("path-state mutex must not be poisoned");
        if state.path_find_threads > 0 {
            state.path_find_threads -= 1;
        }
        state.path_ledger = None;
    }

    pub fn add_fetch_pack(&self, hash: Uint256, data: Vec<u8>) {
        self.fetch_packs.add_fetch_pack(hash, data);
    }

    pub fn get_fetch_pack(&self, hash: Uint256) -> Option<Vec<u8>> {
        self.fetch_packs.get_fetch_pack(hash)
    }

    pub fn got_fetch_pack(&self, _progress: bool, _seq: u32) -> bool {
        !self.got_fetch_pack_in_flight.swap(true, Ordering::AcqRel)
    }

    pub fn txn_id_from_index(&self, ledger_seq: u32, txn_index: u32) -> Option<Uint256> {
        let (_, max) = self.full_validated_range()?;
        if ledger_seq > max {
            return None;
        }

        let ledger = self.get_ledger_by_seq(ledger_seq, &NullLedgerJournal)?;
        let txs = ledger.tx_snapshot().ok()?;

        txs.into_iter()
            .find(|(_, meta)| meta.get_index() == txn_index)
            .map(|(txn, _)| txn.get_transaction_id())
    }

    fn fix_mismatch(&self, ledger: &Ledger) {
        for seq in (1..ledger.header().seq).rev() {
            if !self.have_ledger(seq) {
                continue;
            }

            let Some(expected_hash) = ledger.hash_of_seq(seq, &NullLedgerJournal) else {
                self.clear_ledger(seq);
                continue;
            };

            match self.get_ledger_by_seq(seq, &NullLedgerJournal) {
                Some(other) if other.header().hash == expected_hash => return,
                _ => {
                    self.clear_ledger(seq);
                }
            }
        }
    }

    pub fn finish_got_fetch_pack(&self) {
        self.got_fetch_pack_in_flight
            .store(false, Ordering::Release);
    }

    pub fn get_fetch_pack_cache_size(&self) -> usize {
        self.fetch_packs.get_cache_size()
    }

    pub fn sweep(&self)
    where
        LedgerHistory<C, S>: LedgerMasterSweepTarget,
        FetchPackCache<C, S>: LedgerMasterSweepTarget,
    {
        sweep_ledger_master_like(&self.ledger_history, &self.fetch_packs);
    }

    fn new_pf_work(&self, requests_pending: bool, is_stopping: bool) -> bool {
        let mut state = self
            .path_state
            .lock()
            .expect("path-state mutex must not be poisoned");
        if !is_stopping
            && state.path_find_threads < self.config.path_find_job_limit
            && requests_pending
        {
            state.path_find_threads += 1;
        }

        state.path_find_threads > 0 && !is_stopping
    }
}

fn ledger_age(stored_close_time: u32, now_close_time: u32) -> Duration {
    if stored_close_time == 0 {
        return Duration::weeks(2);
    }

    Duration::seconds(i64::from(now_close_time.saturating_sub(stored_close_time)))
}

fn close_time_distance(first: u32, second: u32) -> Duration {
    Duration::seconds(i64::from(first.abs_diff(second)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LedgerHeader, LedgerPersistenceRuntime, NullLedgerJournal, calculate_ledger_hash};
    use basics::base_uint::Uint256;
    use basics::intrusive_pointer::{SharedIntrusive, make_shared_intrusive};
    use protocol::{
        AccountID, STAmount, STArray, STObject, STTx, Serializer, TxType, get_field_by_symbol,
    };
    use shamap::item::SHAMapItem;
    use shamap::sync::{SHAMapType, SyncState, SyncTree};
    use shamap::tree_node::{SHAMapNodeType, SHAMapTreeNode};

    struct NoopPersistenceRuntime;

    impl LedgerPersistenceRuntime for NoopPersistenceRuntime {
        fn mark_saved(&self, _hash: basics::sha_map_hash::SHAMapHash) -> bool {
            true
        }

        fn start_work(&self, _seq: u32) -> bool {
            true
        }

        fn finish_work(&self, _seq: u32) {}

        fn should_work(&self, _seq: u32, _is_synchronous: bool) -> bool {
            true
        }

        fn pending(&self, _seq: u32) -> bool {
            false
        }

        fn save_validated_ledger(&self, _ledger: Arc<Ledger>, _is_current: bool) -> bool {
            true
        }

        fn enqueue_job(
            &self,
            _job_type: crate::LedgerPersistenceJobType,
            _job_name: String,
            _job: crate::persistence::LedgerPersistenceJob,
        ) -> bool {
            true
        }
    }

    fn state_leaf(fill: u8) -> SharedIntrusive<SHAMapTreeNode> {
        make_shared_intrusive(SHAMapTreeNode::new_leaf(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(Uint256::from_array([fill; 32]), vec![fill; 12]),
            0,
        ))
    }

    fn account(fill: u8) -> AccountID {
        AccountID::from_array([fill; 20])
    }

    fn payment_tx(sequence: u32, account_fill: u8, destination_fill: u8) -> Arc<STTx> {
        Arc::new(STTx::new(TxType::PAYMENT, |tx| {
            tx.set_account_id(get_field_by_symbol("sfAccount"), account(account_fill));
            tx.set_account_id(
                get_field_by_symbol("sfDestination"),
                account(destination_fill),
            );
            tx.set_field_amount(
                get_field_by_symbol("sfAmount"),
                STAmount::new_native(1_000_000, false),
            );
            tx.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::new_native(10, false),
            );
            tx.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
        }))
    }

    fn metadata(index: u32, fill: u8) -> STObject {
        let mut final_fields = STObject::new(get_field_by_symbol("sfFinalFields"));
        final_fields.set_account_id(get_field_by_symbol("sfAccount"), account(fill));

        let mut node = STObject::new(get_field_by_symbol("sfModifiedNode"));
        node.set_field_h256(
            get_field_by_symbol("sfLedgerIndex"),
            Uint256::from_array([fill; 32]),
        );
        node.set_field_u16(get_field_by_symbol("sfLedgerEntryType"), 97);
        node.set_field_object(get_field_by_symbol("sfFinalFields"), final_fields);

        let mut affected_nodes = STArray::new(get_field_by_symbol("sfAffectedNodes"));
        affected_nodes.push_back(node);

        let mut meta = STObject::new(get_field_by_symbol("sfTransactionMetaData"));
        meta.set_field_u8(get_field_by_symbol("sfTransactionResult"), 0);
        meta.set_field_u32(get_field_by_symbol("sfTransactionIndex"), index);
        meta.set_field_array(get_field_by_symbol("sfAffectedNodes"), affected_nodes);
        meta
    }

    fn tx_md_payload(tx: &STTx, meta: &STObject) -> Vec<u8> {
        let tx_bytes = tx.get_serializer().data().to_vec();
        let meta_bytes = meta.get_serializer().data().to_vec();
        let mut serializer = Serializer::new(0);
        serializer.add_vl(&tx_bytes);
        serializer.add_vl(&meta_bytes);
        serializer.data().to_vec()
    }

    fn closed_ledger_with_txs(items: &[(Arc<STTx>, STObject)], seq: u32) -> Arc<Ledger> {
        let root = state_leaf(seq as u8);
        let mut tree = shamap::mutation::MutableTree::new(seq);
        for (tx, meta) in items {
            tree.add_item(
                SHAMapNodeType::TransactionMd,
                SHAMapItem::new(tx.get_transaction_id(), tx_md_payload(tx, meta)),
            )
            .expect("transaction-with-metadata item should insert");
        }

        let mut header = LedgerHeader {
            seq,
            account_hash: root.get_hash(),
            ..LedgerHeader::default()
        };
        header.hash = calculate_ledger_hash(&header);

        let mut ledger = Ledger::from_maps(
            header,
            SyncTree::from_root_with_type(
                root,
                SHAMapType::State,
                false,
                seq,
                SyncState::Immutable,
            ),
            SyncTree::from_root_with_type(
                tree.root(),
                SHAMapType::Transaction,
                false,
                seq,
                SyncState::Immutable,
            ),
        );
        ledger.set_immutable(true);
        Arc::new(ledger)
    }

    fn immutable_ledger(seq: u32, fill: u8) -> Arc<Ledger> {
        let root = state_leaf(fill);
        let mut header = LedgerHeader {
            seq,
            account_hash: root.get_hash(),
            parent_hash: crate::SHAMapHash::new(Uint256::from_array([fill.wrapping_add(1); 32])),
            close_time: seq + 100,
            close_time_resolution: 30,
            ..LedgerHeader::default()
        };
        header.hash = calculate_ledger_hash(&header);
        let mut ledger = Ledger::from_maps(
            header,
            SyncTree::from_root_with_type(root, SHAMapType::State, true, seq, SyncState::Modifying),
            SyncTree::new_with_type(SHAMapType::Transaction, true, seq),
        );
        ledger.set_immutable(true);
        Arc::new(ledger)
    }

    fn linked_ledger(previous: &Arc<Ledger>, close_time: u32) -> Arc<Ledger> {
        let mut ledger = Ledger::from_previous(previous, close_time);
        ledger.set_immutable(true);
        Arc::new(ledger)
    }

    #[test]
    fn master_tracks_published_and_validated_age() {
        let master = LedgerMaster::new(MonotonicClock::default(), LedgerMasterConfig::default());
        let ledger = immutable_ledger(25, 0x11);

        master.set_pub_ledger(Arc::clone(&ledger));
        master
            .set_valid_ledger(
                Arc::clone(&ledger),
                None,
                Some(ledger.header().close_time + 10),
            )
            .expect("validated ledger update should not fail");

        assert_eq!(
            master.get_published_ledger_age(ledger.header().close_time + 30),
            Duration::seconds(30)
        );
        assert_eq!(
            master.get_validated_ledger_age(ledger.header().close_time + 30),
            Duration::seconds(20)
        );
    }

    #[test]
    fn master_path_request_and_fetch_pack_dispatch_are_single_flight() {
        let master = LedgerMaster::new(MonotonicClock::default(), LedgerMasterConfig::default());

        assert!(master.new_path_request(true, false));
        assert!(master.is_new_path_request());
        assert!(!master.is_new_path_request());
        assert_eq!(master.path_find_thread_count(), 1);
        master.complete_path_find_job();
        assert_eq!(master.path_find_thread_count(), 0);

        assert!(master.got_fetch_pack(false, 10));
        assert!(!master.got_fetch_pack(false, 10));
        master.finish_got_fetch_pack();
        assert!(master.got_fetch_pack(false, 10));
    }

    #[test]
    fn master_set_full_ledger_marks_complete_and_updates_holders() {
        let master = LedgerMaster::new(MonotonicClock::default(), LedgerMasterConfig::default());
        let persistence = LedgerPersistence::new(Arc::new(NoopPersistenceRuntime));
        let ledger = immutable_ledger(30, 0x22);

        let saved = master
            .set_full_ledger(&persistence, Arc::clone(&ledger), true, true, None, None)
            .expect("full ledger orchestration should not fail");

        assert!(saved);
        assert!(master.have_ledger(30));
        assert_eq!(
            master
                .validated_ledger()
                .expect("validated ledger")
                .header()
                .hash,
            ledger.header().hash
        );
        assert_eq!(
            master
                .published_ledger()
                .expect("published ledger")
                .header()
                .hash,
            ledger.header().hash
        );
    }

    #[test]
    fn master_check_accept_ledger_preserves_cpp_can_be_current_gates() {
        let master = LedgerMaster::new(MonotonicClock::default(), LedgerMasterConfig::default());
        let valid = immutable_ledger(100, 0x10);
        master
            .set_valid_ledger(Arc::clone(&valid), None, None)
            .expect("valid ledger should update");

        let acceptable = linked_ledger(&valid, valid.header().close_time + 10);
        assert!(master.check_accept_ledger(
            acceptable.as_ref(),
            3,
            2,
            valid.header().parent_close_time + 20
        ));
        assert!(!master.check_accept_ledger(
            acceptable.as_ref(),
            1,
            2,
            valid.header().parent_close_time + 20
        ));

        let stale = immutable_ledger(99, 0x20);
        assert!(!master.check_accept_ledger(
            stale.as_ref(),
            3,
            2,
            valid.header().parent_close_time + 20
        ));

        let mut far_future = Ledger::from_previous(valid.as_ref(), valid.header().close_time + 10);
        far_future.set_ledger_info(LedgerHeader {
            seq: 10_000,
            ..far_future.header()
        });
        let far_future = Arc::new(far_future);
        assert!(!master.check_accept_ledger(
            far_future.as_ref(),
            3,
            2,
            valid.header().parent_close_time + 20
        ));

        assert!(!master.check_accept_ledger(
            acceptable.as_ref(),
            3,
            2,
            acceptable.header().parent_close_time + 301
        ));
    }

    #[test]
    fn master_cached_lookups_match_cpp_hash_and_sequence_fallbacks() {
        let master = LedgerMaster::new(MonotonicClock::default(), LedgerMasterConfig::default());
        let parent = immutable_ledger(31, 0x33);
        let child = linked_ledger(&parent, 111);

        master.set_closed_ledger(Arc::clone(&parent));
        assert!(Arc::ptr_eq(
            &master
                .get_ledger_by_hash(parent.header().hash)
                .expect("closed-ledger hash lookup"),
            &parent
        ));

        master.ledger_history().insert(Arc::clone(&parent), true);
        master
            .set_valid_ledger(Arc::clone(&child), None, None)
            .expect("valid ledger should update");

        assert!(Arc::ptr_eq(
            &master
                .get_ledger_by_seq(child.header().seq, &NullLedgerJournal)
                .expect("validated seq lookup"),
            &child
        ));
        assert!(Arc::ptr_eq(
            &master
                .get_ledger_by_seq(parent.header().seq, &NullLedgerJournal)
                .expect("validated parent lookup"),
            &parent
        ));
    }

    #[test]
    fn master_txn_id_from_index_lookup_shape() {
        let master = LedgerMaster::new(MonotonicClock::default(), LedgerMasterConfig::default());
        let expected_tx = payment_tx(2, 0x21, 0x31);
        let ledger = closed_ledger_with_txs(
            &[
                (payment_tx(3, 0x31, 0x41), metadata(9, 0x91)),
                (payment_tx(1, 0x11, 0x21), metadata(2, 0x92)),
                (Arc::clone(&expected_tx), metadata(5, 0x93)),
            ],
            88,
        );
        let persistence = LedgerPersistence::new(Arc::new(NoopPersistenceRuntime));

        master
            .set_full_ledger(&persistence, Arc::clone(&ledger), true, true, None, None)
            .expect("full ledger orchestration should not fail");

        assert_eq!(
            master.txn_id_from_index(ledger.header().seq, 5),
            Some(expected_tx.get_transaction_id())
        );
        assert_eq!(master.txn_id_from_index(ledger.header().seq, 8), None);
        assert_eq!(master.txn_id_from_index(ledger.header().seq + 1, 5), None);
    }
}
