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
            // Validated-arrived-first ordering: our built ledger (`ledger`) is
            // in hand; fetch the validated sibling from cache and diff.
            if let Some(valid_ledger) =
                self.get_cached_ledger_by_hash(SHAMapHash::new(*validated.as_uint256()))
            {
                self.fork_state_diff(seq, hash, validated, &ledger, &valid_ledger);
            }
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
            // FORK STATE DIFF: fetch our built ledger from cache (validated
            // `ledger` is in hand) and run the shared diff.
            if let Some(built_ledger) =
                self.get_cached_ledger_by_hash(SHAMapHash::new(*built.as_uint256()))
            {
                self.fork_state_diff(seq, built, hash, &built_ledger, &ledger);
            }
        }

        entry.validated = Some(hash);
        entry.validated_consensus_hash = consensus_hash;
    }

    /// Diagnostic: walk the state maps of our built ledger and the validated
    /// sibling and log the EXACT ledger entries whose value differs, decoding
    /// each built-side SLE's type (and, for Offers, remaining amounts) - the
    /// definitive fork localization. Fires from both the built-first and
    /// validated-first mismatch orderings.
    fn fork_state_diff(
        &self,
        seq: u32,
        built: SHAMapHash,
        validated: SHAMapHash,
        built_ledger: &Arc<Ledger>,
        valid_ledger: &Arc<Ledger>,
    ) {
        let (Some(bf), Some(vf)) =
            (built_ledger.node_fetcher_closure(), valid_ledger.node_fetcher_closure())
        else {
            return;
        };
        // Pass 1: collect the divergent keys by walking the state-map diff.
        let mut diff_hashes: Vec<Uint256> = Vec::new();
        let mut bf2 = move |h| bf(h);
        let mut vf2 = move |h| vf(h);
        let mut visit = |node: &basics::memory::intrusive_pointer::SharedIntrusive<shamap::nodes::tree_node::SHAMapTreeNode>| {
            if node.is_leaf()
                && let Some(item) = node.peek_item()
                && diff_hashes.len() < 32
            {
                let key = item.key();
                if !diff_hashes.contains(&key) {
                    diff_hashes.push(key);
                }
            }
            true
        };
        let _ = built_ledger.state_map().visit_differences(
            Some(valid_ledger.state_map()),
            &mut bf2,
            &mut vf2,
            &mut visit,
        );
        // Pass 2: for each divergent key, decode BOTH our built and the
        // validated SLE and emit the exact field-level delta. This removes any
        // dependency on historical ledger RPC (which retention can block).
        // Also cross-reference each key against the entries THIS ledger's own
        // transactions touched (from our built tx-map metadata): a divergent
        // key tagged THIS_LEDGER pinpoints the root transaction; INHERITED keys
        // are cascaded from an earlier fork.
        let touched = Self::this_ledger_touched_keys(built_ledger);
        let mut diff_keys: Vec<String> = Vec::new();
        for key in &diff_hashes {
            let origin = if touched.contains(key) { "THIS_LEDGER" } else { "INHERITED" };
            let keylet = protocol::Keylet::new(protocol::LedgerEntryType::Any, *key);
            let ours = built_ledger.read(keylet).ok().flatten();
            let theirs = valid_ledger.read(keylet).ok().flatten();
            let desc = match (ours, theirs) {
                (Some(o), None) => {
                    format!("[{origin}] {:?} PRESENT_IN_OURS_ABSENT_IN_VALIDATED {}", o.get_type(), Self::sle_salient(&o))
                }
                (None, Some(t)) => {
                    format!("[{origin}] {:?} ABSENT_IN_OURS_PRESENT_IN_VALIDATED {}", t.get_type(), Self::sle_salient(&t))
                }
                (Some(o), Some(t)) => {
                    format!(
                        "[{origin}] {:?} VALUE_DIFF ours{{{}}} validated{{{}}}",
                        o.get_type(),
                        Self::sle_salient(&o),
                        Self::sle_salient(&t)
                    )
                }
                (None, None) => format!("[{origin}] <absent-in-both?>"),
            };
            diff_keys.push(format!("{key}={desc}"));
        }
        let touched_divergent = diff_hashes.iter().filter(|k| touched.contains(k)).count();
        tracing::warn!(
            target: "lcl_audit",
            event = "fork_state_diff",
            seq,
            built = %built,
            validated = %validated,
            divergent_sle_count = diff_keys.len(),
            touched_divergent,
            divergent_sle_keys = ?diff_keys,
            "FORK_STATE_DIFF: ledger entries differing between our built ledger and the validated sibling"
        );
    }

    /// Collect the set of ledger-entry keys touched by THIS ledger's own
    /// transactions, by walking our built ledger's transaction map and reading
    /// each tx metadata's affected-node LedgerIndex values. Used to classify a
    /// divergent SLE as rooted in this ledger vs inherited from an earlier fork.
    fn this_ledger_touched_keys(built_ledger: &Arc<Ledger>) -> std::collections::HashSet<Uint256> {
        use protocol::get_field_by_symbol as f;
        let mut touched = std::collections::HashSet::new();
        let seq = built_ledger.header().seq;
        let Some(fetch_arc) = built_ledger.node_fetcher_closure() else {
            return touched;
        };
        let mut fetch = move |h| fetch_arc(h);
        let mut visit = |item: &shamap::nodes::item::SHAMapItem| {
            if let Ok((_tx, meta)) = crate::decode_transaction_md_item(seq, item)
            {
                for n in meta.get_nodes().iter() {
                    let k = n.get_field_h256(f("sfLedgerIndex"));
                    if k != Uint256::default() {
                        touched.insert(k);
                    }
                }
            }
        };
        let _ = built_ledger.tx_map().visit_leaves(&mut fetch, &mut visit);
        touched
    }

    /// Render the salient fields of an SLE for fork diagnosis: entry type plus
    /// the fields most likely to carry a value-level fork (amounts, balances,
    /// owner counts, sequences, flags).
    fn sle_salient(sle: &protocol::STLedgerEntry) -> String {
        use protocol::get_field_by_symbol as f;
        let ty = sle.get_type();
        let mut parts: Vec<String> = Vec::new();
        let amt = |sym: &str| -> Option<String> {
            let a = sle.get_field_amount(f(sym));
            Some(format!("{sym}[m={} e={} neg={}]", a.mantissa(), a.exponent(), a.negative() as u8))
        };
        match ty {
            protocol::LedgerEntryType::Offer => {
                if let Some(s) = amt("sfTakerGets") { parts.push(s); }
                if let Some(s) = amt("sfTakerPays") { parts.push(s); }
                parts.push(format!("Seq={}", sle.get_field_u32(f("sfSequence"))));
                parts.push(format!("Flags={}", sle.get_field_u32(f("sfFlags"))));
                parts.push(format!("BookDir={}", sle.get_field_h256(f("sfBookDirectory"))));
            }
            protocol::LedgerEntryType::AccountRoot => {
                if let Some(s) = amt("sfBalance") { parts.push(s); }
                parts.push(format!("OwnerCount={}", sle.get_field_u32(f("sfOwnerCount"))));
                parts.push(format!("Seq={}", sle.get_field_u32(f("sfSequence"))));
                parts.push(format!("Flags={}", sle.get_field_u32(f("sfFlags"))));
            }
            protocol::LedgerEntryType::RippleState => {
                if let Some(s) = amt("sfBalance") { parts.push(s); }
                parts.push(format!("Flags={}", sle.get_field_u32(f("sfFlags"))));
            }
            protocol::LedgerEntryType::DirectoryNode => {
                parts.push(format!("IndexNext={}", sle.get_field_u64(f("sfIndexNext"))));
                parts.push(format!("Flags={}", sle.get_field_u32(f("sfFlags"))));
            }
            _ => {}
        }
        parts.join(" ")
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

    /// Remove one exact ledger identity from both hash and validated-sequence
    /// indexes. A same-sequence successor must remain cached and complete.
    pub fn remove_exact(&self, ledger_hash: SHAMapHash, ledger_seq: u32) -> bool {
        if self
            .ledgers_by_hash
            .fetch(&ledger_hash)
            .is_none_or(|ledger| ledger.header().seq != ledger_seq)
        {
            return false;
        }
        self.ledgers_by_hash.del(&ledger_hash, false);
        self.ledgers_by_index
            .lock()
            .expect("ledger-history index mutex must not be poisoned")
            .retain(|index, hash| *index != ledger_seq || *hash != ledger_hash);
        true
    }

    /// Remove one exact ledger from both hash and validated-sequence indexes.
    /// This is intentionally narrower than cache sweeping: it compensates a
    /// provisional inbound publication that failed its final NodeStore fence.
    pub fn remove(&self, ledger_hash: SHAMapHash) -> bool {
        let removed = self.ledgers_by_hash.fetch(&ledger_hash).is_some();
        self.ledgers_by_hash.del(&ledger_hash, false);
        self.ledgers_by_index
            .lock()
            .expect("ledger-history index mutex must not be poisoned")
            .retain(|_, hash| *hash != ledger_hash);
        removed
    }

    pub fn sweep(&self)
    where
        S: Send,
    {
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
