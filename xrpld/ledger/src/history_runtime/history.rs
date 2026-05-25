//! Narrow `LedgerHistory` cache/load core above the landed immutable-ledger
//! load helpers.
//!
//! This ports the current finishable the reference implementation surface:
//! - `insert(...)`,
//! - `getCacheHitRate()`,
//! - `getLedgerHash(...)`,
//! - `getLedgerBySeq(...)`,
//! - `getLedgerByHash(...)`,
//! - `builtLedger(...)`,
//! - `validatedLedger(...)`,
//! - the mismatch bookkeeping behind `handleMismatch(...)`,
//! - `fixIndex(...)`,
//! - `clearLedgerCachePrior(...)`,
//! - and `sweep()`.

use crate::{
    Ledger, LedgerConfig, LedgerHeader, LedgerHistoryFillPlan, LedgerHistorySyncState,
    LedgerInfoProvider, LedgerJournal, LedgerObjectPresence, LedgerPresence, LedgerSetupError,
    Stopper, apply_fill_plan, run_try_fill_backwalk,
};
use basics::base_uint::Uint256;
use basics::hardened_hash::HardenedHashBuilder;
use basics::sha_map_hash::SHAMapHash;
use basics::tagged_cache::{CacheClock, MonotonicClock, TaggedCache};
use protocol::JsonValue;
use shamap::family::{FullBelowCache, MissingNodeReporter, SHAMapFamily, SHAMapNodeFetcher};
use std::collections::BTreeMap;
use std::hash::BuildHasher;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use time::Duration;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConsensusValidatedEntry {
    pub built: Option<SHAMapHash>,
    pub validated: Option<SHAMapHash>,
    pub built_consensus_hash: Option<Uint256>,
    pub validated_consensus_hash: Option<Uint256>,
    pub consensus: Option<JsonValue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerHistoryMismatch {
    pub seq: u32,
    pub built: SHAMapHash,
    pub validated: SHAMapHash,
    pub built_consensus_hash: Option<Uint256>,
    pub validated_consensus_hash: Option<Uint256>,
    pub consensus: Option<JsonValue>,
}

#[derive(Debug)]
pub struct LedgerHistory<C = MonotonicClock, S = HardenedHashBuilder> {
    ledgers_by_hash: TaggedCache<SHAMapHash, Ledger, C, S>,
    ledgers_by_index: Mutex<BTreeMap<u32, SHAMapHash>>,
    consensus_validated: Mutex<BTreeMap<u32, ConsensusValidatedEntry>>,
    mismatch_count: AtomicU64,
    mismatches: Mutex<Vec<LedgerHistoryMismatch>>,
}

impl<C> LedgerHistory<C, HardenedHashBuilder>
where
    C: CacheClock,
{
    pub fn new(size: usize, age: Duration, clock: C) -> Self {
        Self::with_hasher(size, age, clock, HardenedHashBuilder::default())
    }
}

impl<C, S> LedgerHistory<C, S>
where
    C: CacheClock,
    S: BuildHasher + Clone,
{
    pub fn with_hasher(size: usize, age: Duration, clock: C, hasher: S) -> Self {
        Self {
            ledgers_by_hash: TaggedCache::with_hasher("LedgerCache", size, age, clock, hasher),
            ledgers_by_index: Mutex::new(BTreeMap::new()),
            consensus_validated: Mutex::new(BTreeMap::new()),
            mismatch_count: AtomicU64::new(0),
            mismatches: Mutex::new(Vec::new()),
        }
    }

    pub fn insert(&self, ledger: Arc<Ledger>, validated: bool) -> bool {
        if !ledger.is_immutable() {
            return false;
        }
        if ledger.state_map().root().get_hash().is_zero() {
            return false;
        }

        let already_had = self
            .ledgers_by_hash
            .canonicalize_replace_cache(&ledger.header().hash, &ledger);
        if validated {
            self.ledgers_by_index
                .lock()
                .expect("ledger-history index mutex must not be poisoned")
                .insert(ledger.header().seq, ledger.header().hash);
        }
        already_had
    }

    pub fn get_cache_hit_rate(&self) -> f32 {
        self.ledgers_by_hash.get_hit_rate()
    }

    pub fn built_ledger(&self, ledger: Arc<Ledger>, consensus_hash: Uint256, consensus: JsonValue) {
        let seq = ledger.header().seq;
        let hash = ledger.header().hash;
        assert!(
            hash.is_non_zero(),
            "xrpl::LedgerHistory::builtLedger : nonzero hash"
        );

        let mut by_seq = self
            .consensus_validated
            .lock()
            .expect("consensus-validations mutex must not be poisoned");
        let entry = by_seq.entry(seq).or_default();

        if let Some(validated) = entry.validated
            && entry.built.is_none()
            && validated != hash
        {
            self.record_mismatch(LedgerHistoryMismatch {
                seq,
                built: hash,
                validated,
                built_consensus_hash: Some(consensus_hash),
                validated_consensus_hash: entry.validated_consensus_hash,
                consensus: Some(consensus.clone()),
            });
        }

        entry.built = Some(hash);
        entry.built_consensus_hash = Some(consensus_hash);
        entry.consensus = Some(consensus);
    }

    pub fn validated_ledger(&self, ledger: Arc<Ledger>, consensus_hash: Option<Uint256>) {
        let seq = ledger.header().seq;
        let hash = ledger.header().hash;
        assert!(
            hash.is_non_zero(),
            "xrpl::LedgerHistory::validatedLedger : nonzero hash"
        );

        let mut by_seq = self
            .consensus_validated
            .lock()
            .expect("consensus-validations mutex must not be poisoned");
        let entry = by_seq.entry(seq).or_default();

        if let Some(built) = entry.built
            && entry.validated.is_none()
            && built != hash
        {
            self.record_mismatch(LedgerHistoryMismatch {
                seq,
                built,
                validated: hash,
                built_consensus_hash: entry.built_consensus_hash,
                validated_consensus_hash: consensus_hash,
                consensus: entry.consensus.clone(),
            });
        }

        entry.validated = Some(hash);
        entry.validated_consensus_hash = consensus_hash;
    }

    pub fn consensus_entry(&self, seq: u32) -> Option<ConsensusValidatedEntry> {
        self.consensus_validated
            .lock()
            .expect("consensus-validations mutex must not be poisoned")
            .get(&seq)
            .cloned()
    }

    pub fn mismatch_count(&self) -> u64 {
        self.mismatch_count.load(Ordering::SeqCst)
    }

    pub fn mismatches(&self) -> Vec<LedgerHistoryMismatch> {
        self.mismatches
            .lock()
            .expect("mismatches mutex must not be poisoned")
            .clone()
    }

    pub fn get_ledger_hash(&self, ledger_index: u32) -> SHAMapHash {
        self.ledgers_by_index
            .lock()
            .expect("ledger-history index mutex must not be poisoned")
            .get(&ledger_index)
            .copied()
            .unwrap_or_default()
    }

    pub fn get_cached_ledger_by_hash(&self, ledger_hash: SHAMapHash) -> Option<Arc<Ledger>> {
        self.ledgers_by_hash.fetch(&ledger_hash)
    }

    pub fn get_cached_ledger_by_seq(&self, ledger_index: u32) -> Option<Arc<Ledger>> {
        let hash = self
            .ledgers_by_index
            .lock()
            .expect("ledger-history index mutex must not be poisoned")
            .get(&ledger_index)
            .copied()?;
        self.get_cached_ledger_by_hash(hash)
    }

    pub fn get_ledger_by_hash<P, CLOCK, FB, F, MR, NS, J>(
        &self,
        ledger_hash: SHAMapHash,
        journal: &J,
        config: &LedgerConfig,
        family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
        provider: &P,
    ) -> Result<Option<Arc<Ledger>>, LedgerSetupError>
    where
        P: LedgerInfoProvider,
        CLOCK: CacheClock,
        FB: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
        J: LedgerJournal,
    {
        if let Some(ledger) = self.ledgers_by_hash.fetch(&ledger_hash) {
            assert!(
                ledger.is_immutable(),
                "xrpl::LedgerHistory::getLedgerByHash : immutable fetched ledger"
            );
            assert!(
                ledger.header().hash == ledger_hash,
                "xrpl::LedgerHistory::getLedgerByHash : fetched ledger hash match"
            );
            return Ok(Some(ledger));
        }

        let Some(ledger) = Ledger::load_by_hash_with_provider_and_config_or_none(
            ledger_hash,
            true,
            journal,
            config,
            family,
            provider,
        )?
        else {
            return Ok(None);
        };

        let mut ledger = Arc::new(ledger);
        assert!(
            ledger.is_immutable(),
            "xrpl::LedgerHistory::getLedgerByHash : immutable loaded ledger"
        );
        assert!(
            ledger.header().hash == ledger_hash,
            "xrpl::LedgerHistory::getLedgerByHash : loaded ledger hash match"
        );
        self.ledgers_by_hash
            .canonicalize_replace_client(&ledger.header().hash, &mut ledger);
        assert!(
            ledger.header().hash == ledger_hash,
            "xrpl::LedgerHistory::getLedgerByHash : result hash match"
        );
        Ok(Some(ledger))
    }

    pub fn get_ledger_by_seq<P, CLOCK, FB, F, MR, NS, J>(
        &self,
        ledger_index: u32,
        journal: &J,
        config: &LedgerConfig,
        family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
        provider: &P,
    ) -> Result<Option<Arc<Ledger>>, LedgerSetupError>
    where
        P: LedgerInfoProvider,
        CLOCK: CacheClock,
        FB: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
        J: LedgerJournal,
    {
        if let Some(hash) = self
            .ledgers_by_index
            .lock()
            .expect("ledger-history index mutex must not be poisoned")
            .get(&ledger_index)
            .copied()
        {
            return self.get_ledger_by_hash(hash, journal, config, family, provider);
        }

        let Some(ledger) = Ledger::load_by_index_with_provider_and_config_or_none(
            ledger_index,
            true,
            journal,
            config,
            family,
            provider,
        )?
        else {
            return Ok(None);
        };

        assert!(
            ledger.header().seq == ledger_index,
            "xrpl::LedgerHistory::getLedgerBySeq : result sequence match"
        );

        let mut ledger = Arc::new(ledger);
        assert!(
            ledger.is_immutable(),
            "xrpl::LedgerHistory::getLedgerBySeq : immutable result ledger"
        );
        self.ledgers_by_hash
            .canonicalize_replace_client(&ledger.header().hash, &mut ledger);
        self.ledgers_by_index
            .lock()
            .expect("ledger-history index mutex must not be poisoned")
            .insert(ledger.header().seq, ledger.header().hash);

        Ok((ledger.header().seq == ledger_index).then_some(ledger))
    }

    pub fn sweep(&self) {
        self.ledgers_by_hash.sweep();
    }

    pub fn fix_index(&self, ledger_index: u32, ledger_hash: SHAMapHash) -> bool {
        let mut by_index = self
            .ledgers_by_index
            .lock()
            .expect("ledger-history index mutex must not be poisoned");
        if let Some(existing) = by_index.get_mut(&ledger_index)
            && *existing != ledger_hash
        {
            *existing = ledger_hash;
            return false;
        }
        true
    }

    pub fn clear_ledger_cache_prior<P, CLOCK, FB, F, MR, NS, J>(
        &self,
        ledger_index: u32,
        journal: &J,
        config: &LedgerConfig,
        family: &SHAMapFamily<CLOCK, S, FB, F, MR, NS>,
        provider: &P,
    ) -> Result<(), LedgerSetupError>
    where
        P: LedgerInfoProvider,
        CLOCK: CacheClock,
        FB: FullBelowCache,
        F: SHAMapNodeFetcher,
        MR: MissingNodeReporter,
        J: LedgerJournal,
    {
        for hash in self.ledgers_by_hash.get_keys() {
            let ledger = self.get_ledger_by_hash(hash, journal, config, family, provider)?;
            if ledger
                .as_ref()
                .is_none_or(|ledger| ledger.header().seq < ledger_index)
            {
                self.ledgers_by_hash.del(&hash, false);
            }
        }
        Ok(())
    }

    pub fn clear_cached_ledger_entries_prior(&self, ledger_index: u32) {
        for hash in self.ledgers_by_hash.get_keys() {
            let remove = self
                .ledgers_by_hash
                .fetch(&hash)
                .is_none_or(|ledger| ledger.header().seq < ledger_index);
            if remove {
                self.ledgers_by_hash.del(&hash, false);
            }
        }
    }

    fn record_mismatch(&self, mismatch: LedgerHistoryMismatch) {
        self.mismatch_count.fetch_add(1, Ordering::SeqCst);
        self.mismatches
            .lock()
            .expect("mismatches mutex must not be poisoned")
            .push(mismatch);
    }
}

pub fn fix_gaps<L, P, DB, NS, ST>(
    state: &mut LedgerHistorySyncState<L>,
    ledger: &LedgerHeader,
    presence: &P,
    hash_pairs: &DB,
    node_store: &NS,
    stopper: &ST,
) -> LedgerHistoryFillPlan
where
    P: LedgerPresence,
    DB: crate::LedgerHashPairProvider,
    NS: LedgerObjectPresence,
    ST: Stopper,
{
    let plan = run_try_fill_backwalk(ledger, presence, hash_pairs, node_store, stopper);
    apply_fill_plan(state, ledger.seq, &plan);
    plan
}
