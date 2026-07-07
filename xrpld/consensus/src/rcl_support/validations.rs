//! Generic tracking of current and recent ledger validations. Ported from
//! rippled's `Validations.h`.
//!
//! This is the RCL (Ripple Consensus Ledger) adaptation layer's foundation:
//! it decides which validations are "current" (fresh enough to trust),
//! enforces the monotonically-increasing-sequence invariant per validator
//! (Byzantine detection), and feeds a [`LedgerTrie`] to compute the
//! network's preferred working ledger.
//!
//! # Deviations from the reference
//!
//! - `beast::aged_unordered_map` (a map with LRU-by-touch expiry against an
//!   injected clock) has no direct Rust equivalent in this codebase. This
//!   port uses a plain `HashMap` plus a separate `last_touched: HashMap<K,
//!   Instant>` side table, with `expire()` sweeping both by comparing
//!   against `validation_set_expires`. This is less memory-efficient than
//!   an intrusive aged container but is a straightforward, correct
//!   translation; if profiling later shows this matters, it can be
//!   swapped for a proper aged-map crate without changing the public API.
//! - The reference threads a `beast::Journal` through `expire()` purely
//!   for a debug timing log; this port uses `tracing::debug!` instead.
//! - `getJsonTrie()` is omitted (RPC-facing presentation concern, per the
//!   same rationale as `Consensus::getJson` in Phase 3).
//! - The `Adaptor::MutexType` customization point is dropped: the
//!   reference allows swapping the mutex type but always uses
//!   `std::mutex` in practice. This port uses `parking_lot::Mutex`
//!   directly, matching the JobQueue rewrite's established convention.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::{Duration, Instant};

use basics::chrono::NetClockTimePoint;
use parking_lot::Mutex;

use crate::model::ledger_trie::{LedgerTrie, SpanTip, TrieLedger};

/// Timing parameters controlling validation staleness and expiration.
/// Matches `ValidationParms`.
///
/// Deferred here from Phase 1 (`ConsensusParms`) after discovering during
/// that phase's rewrite that these fields belong to a *separate* struct in
/// the reference, not `ConsensusParms` -- see Phase 1's design-decision
/// notes.
#[derive(Debug, Clone, Copy)]
pub struct ValidationParms {
    /// Seconds a validation remains current after its ledger's close time.
    pub validation_current_wall: Duration,
    /// Seconds a validation remains current after first observed locally.
    pub validation_current_local: Duration,
    /// Seconds before a close time that a validation is still acceptable.
    pub validation_current_early: Duration,
    /// Seconds before a validation set for a given ledger hash expires.
    pub validation_set_expires: Duration,
    /// Seconds since being seen for a validation to be considered fresh.
    pub validation_freshness: Duration,
}

impl Default for ValidationParms {
    fn default() -> Self {
        Self {
            validation_current_wall: Duration::from_secs(5 * 60),
            validation_current_local: Duration::from_secs(3 * 60),
            validation_current_early: Duration::from_secs(3 * 60),
            validation_set_expires: Duration::from_secs(10 * 60),
            validation_freshness: Duration::from_secs(20),
        }
    }
}

/// Enforces that a validation must be larger than all unexpired validation
/// sequence numbers previously issued by the validator this tracks.
/// Matches `SeqEnforcer<Seq>`.
#[derive(Debug, Clone, Copy)]
pub struct SeqEnforcer<Seq> {
    seq: Seq,
    when: Option<Instant>,
}

impl<Seq: Copy + Default + PartialOrd> Default for SeqEnforcer<Seq> {
    fn default() -> Self {
        Self { seq: Seq::default(), when: None }
    }
}

impl<Seq: Copy + Default + PartialOrd> SeqEnforcer<Seq> {
    /// Try advancing the largest observed validation sequence. Returns
    /// `false` if `s` violates the invariant that a validation must be
    /// larger than all unexpired validation sequence numbers. Matches
    /// `SeqEnforcer::operator()`.
    pub fn try_advance(&mut self, now: Instant, s: Seq, p: &ValidationParms) -> bool {
        if let Some(when) = self.when
            && now > when + p.validation_set_expires
        {
            self.seq = Seq::default();
        }
        if s <= self.seq {
            return false;
        }
        self.seq = s;
        self.when = Some(now);
        true
    }

    /// The largest sequence number seen. Matches `SeqEnforcer::largest`.
    pub fn largest(&self) -> Seq {
        self.seq
    }
}

/// Whether a validation can still be considered the current validation
/// from its issuing node, based on when it was signed and first seen.
/// Matches the free function `isCurrent`.
///
/// As in the reference, this deliberately avoids any subtraction that
/// could overflow/underflow an unsigned time value: `NetClockTimePoint`
/// arithmetic here uses `time::Duration` (signed, seconds-granularity),
/// matching the reference's "promoted to signed 64-bit" comment.
pub fn is_current(p: &ValidationParms, now: NetClockTimePoint, sign_time: NetClockTimePoint, seen_time: NetClockTimePoint) -> bool {
    let early = time::Duration::seconds(p.validation_current_early.as_secs() as i64);
    let wall = time::Duration::seconds(p.validation_current_wall.as_secs() as i64);
    let local = time::Duration::seconds(p.validation_current_local.as_secs() as i64);

    let now_secs = now.as_seconds() as i64;
    let sign_secs = sign_time.as_seconds() as i64;
    let seen_secs = seen_time.as_seconds() as i64;

    (sign_secs > now_secs - early.whole_seconds())
        && (sign_secs < now_secs + wall.whole_seconds())
        && (seen_time == NetClockTimePoint::default() || seen_secs < now_secs + local.whole_seconds())
}

/// Outcome of attempting to add a validation. Matches `ValStatus`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValStatus {
    /// A new, current validation was added.
    Current,
    /// Not current, or older than the current one from this node.
    Stale,
    /// Violates the increasing-sequence requirement.
    BadSeq,
    /// Multiple validations by a validator for the same ledger.
    Multiple,
    /// Multiple validations by a validator for different ledgers.
    Conflicting,
}

impl std::fmt::Display for ValStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ValStatus::Current => "current",
            ValStatus::Stale => "stale",
            ValStatus::BadSeq => "badSeq",
            ValStatus::Multiple => "multiple",
            ValStatus::Conflicting => "conflicting",
        };
        f.write_str(s)
    }
}

/// A single validation, as required by [`Validations`]. Matches the
/// reference's implicit `Validation` concept.
pub trait ValidationT: Clone {
    type LedgerId: Eq + std::hash::Hash + Clone + Ord + ToString;
    type Seq: Copy + Default + Ord + std::hash::Hash + std::ops::Sub<u32, Output = Self::Seq>;
    type NodeId: Eq + std::hash::Hash + Clone + Ord;
    type NodeKey: Eq + std::hash::Hash + Clone + Ord;
    /// The implementation-specific type this validation wraps, returned by
    /// `currentTrusted`/`getTrustedForLedger`. Matches `unwrap()`.
    type Wrapped;

    fn ledger_id(&self) -> Self::LedgerId;
    fn seq(&self) -> Self::Seq;
    fn sign_time(&self) -> NetClockTimePoint;
    fn seen_time(&self) -> NetClockTimePoint;
    fn key(&self) -> Self::NodeKey;
    fn trusted(&self) -> bool;
    fn set_trusted(&mut self);
    fn set_untrusted(&mut self);
    fn full(&self) -> bool;
    fn node_id(&self) -> Self::NodeId;
    fn load_fee(&self) -> Option<u32>;
    /// A disambiguating token for near-simultaneous validations at the same
    /// sequence (matches the reference's `cookie()`, used only to
    /// distinguish `Multiple` from `Conflicting` in `add`).
    fn cookie(&self) -> u64;
    fn unwrap(self) -> Self::Wrapped;
}

/// A ledger usable by [`Validations`]: everything [`TrieLedger`] requires,
/// plus a direct `id()` accessor.
///
/// [`TrieLedger`] itself never needs to ask a ledger for its own id in
/// isolation -- internally it always calls `ancestor(seq)`, since a
/// ledger's own id is `ancestor(self.seq())`. `Validations` asks for a
/// ledger's own id far more often (comparing tracked ledgers, trie
/// removal, node bookkeeping), so this trait adds that as a named,
/// zero-cost convenience rather than spelling `ancestor(self.seq())`
/// everywhere it's needed.
pub trait ValidationsLedger: TrieLedger {
    fn id(&self) -> Self::Id {
        self.ancestor(self.seq())
    }
}

/// Bridges [`Validations`] to a specific application. Matches the
/// reference's `Adaptor` concept.
pub trait ValidationsAdaptor {
    type Ledger: ValidationsLedger;
    type Validation: ValidationT<LedgerId = <Self::Ledger as TrieLedger>::Id, Seq = <Self::Ledger as TrieLedger>::Seq>;

    /// The current network time, used to determine staleness.
    fn now(&self) -> NetClockTimePoint;

    /// Attempt to acquire a specific ledger (e.g. from local storage or the
    /// network), for updating the [`LedgerTrie`].
    fn acquire(&self, ledger_id: &<Self::Ledger as TrieLedger>::Id) -> Option<Self::Ledger>;
}

type NodeIdOf<A> = <<A as ValidationsAdaptor>::Validation as ValidationT>::NodeId;
type NodeKeyOf<A> = <<A as ValidationsAdaptor>::Validation as ValidationT>::NodeKey;
type LedgerIdOf<A> = <<A as ValidationsAdaptor>::Ledger as TrieLedger>::Id;
type SeqOf<A> = <<A as ValidationsAdaptor>::Ledger as TrieLedger>::Seq;

/// A validation set expiring after `validation_set_expires` of disuse,
/// unless its sequence falls in a protected "keep" range. Backs both
/// `by_ledger` and `by_sequence` (see module-level deviation note on
/// `beast::aged_unordered_map`).
struct AgedMap<K: Eq + std::hash::Hash + Clone, V> {
    entries: HashMap<K, V>,
    last_touched: HashMap<K, Instant>,
}

impl<K: Eq + std::hash::Hash + Clone, V> AgedMap<K, V> {
    fn new() -> Self {
        Self { entries: HashMap::new(), last_touched: HashMap::new() }
    }

    fn touch(&mut self, key: &K, now: Instant) {
        self.last_touched.insert(key.clone(), now);
    }

    fn get_or_insert_with(&mut self, key: K, now: Instant, default: impl FnOnce() -> V) -> &mut V {
        self.last_touched.insert(key.clone(), now);
        self.entries.entry(key).or_insert_with(default)
    }

    fn get(&self, key: &K) -> Option<&V> {
        self.entries.get(key)
    }

    /// Remove entries not touched within `expires_after`, except those for
    /// which `keep` (if given) returns `true` -- those are instead
    /// re-touched so they survive another `expires_after` window. Matches
    /// the reference's `expire()` behavior of refreshing (`touch`-ing)
    /// entries within the protected `toKeep_` range instead of letting
    /// them age out.
    fn expire(&mut self, now: Instant, expires_after: Duration, keep: Option<&dyn Fn(&K) -> bool>) {
        if let Some(keep) = keep {
            let to_refresh: Vec<K> = self.entries.keys().filter(|k| keep(k)).cloned().collect();
            for k in to_refresh {
                self.touch(&k, now);
            }
        }

        let expired: Vec<K> = self
            .last_touched
            .iter()
            .filter(|&(_, &touched)| now.saturating_duration_since(touched) > expires_after)
            .map(|(k, _)| k.clone())
            .collect();
        for k in expired {
            self.entries.remove(&k);
            self.last_touched.remove(&k);
        }
    }

    fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Maintains current and recent ledger validations. Matches the
/// reference's `Validations<Adaptor>`.
///
/// The reference documents this as thread-safe via an internal mutex that
/// guards only `Validations`' own members, not the adaptor. This port
/// preserves that: all mutable state lives behind a single
/// `parking_lot::Mutex`, and the adaptor is called without holding it
/// locked wherever the reference does the same (adaptor calls are meant to
/// be cheap, side-effect-free lookups).
pub struct Validations<A: ValidationsAdaptor> {
    parms: ValidationParms,
    adaptor: A,
    inner: Mutex<Inner<A>>,
}

struct Inner<A: ValidationsAdaptor> {
    /// Current validations from listed/trusted nodes. Matches `current_`.
    current: HashMap<NodeIdOf<A>, A::Validation>,
    /// Enforces the local node's own increasing-sequence invariant.
    local_seq_enforcer: SeqEnforcer<SeqOf<A>>,
    /// Per-node sequence enforcers (Byzantine detection). Matches
    /// `seqEnforcers_`.
    seq_enforcers: HashMap<NodeIdOf<A>, SeqEnforcer<SeqOf<A>>>,
    /// Validations indexed by ledger id. Matches `byLedger_`.
    by_ledger: AgedMap<LedgerIdOf<A>, HashMap<NodeIdOf<A>, A::Validation>>,
    /// Validations indexed by sequence. Matches `bySequence_`.
    by_sequence: AgedMap<SeqOf<A>, HashMap<NodeIdOf<A>, A::Validation>>,
    /// A `[low, high)` range of sequences to protect from expiry.
    keep: Option<(SeqOf<A>, SeqOf<A>)>,
    /// Ancestry trie over trusted validated ledgers.
    trie: LedgerTrie<A::Ledger>,
    /// Last validated ledger successfully acquired per node; if present,
    /// accounted for in `trie`. Matches `lastLedger_`.
    last_ledger: HashMap<NodeIdOf<A>, A::Ledger>,
    /// Ledgers being acquired from the network, keyed by `(seq, id)`, with
    /// the set of nodes waiting on that acquisition. Matches `acquiring_`.
    acquiring: BTreeMap<(SeqOf<A>, LedgerIdOf<A>), HashSet<NodeIdOf<A>>>,
}

impl<A: ValidationsAdaptor> Inner<A> {
    fn new() -> Self {
        Self {
            current: HashMap::new(),
            local_seq_enforcer: SeqEnforcer::default(),
            seq_enforcers: HashMap::new(),
            by_ledger: AgedMap::new(),
            by_sequence: AgedMap::new(),
            keep: None,
            trie: LedgerTrie::new(),
            last_ledger: HashMap::new(),
            acquiring: BTreeMap::new(),
        }
    }

    /// Remove support of a validated ledger from the trie. Matches
    /// `removeTrie`.
    fn remove_trie(&mut self, node_id: &NodeIdOf<A>, val: &A::Validation) {
        let key = (val.seq(), val.ledger_id());
        if let Some(set) = self.acquiring.get_mut(&key) {
            set.remove(node_id);
            if set.is_empty() {
                self.acquiring.remove(&key);
            }
        }
        if let Some(last) = self.last_ledger.get(node_id)
            && last.id() == val.ledger_id()
        {
            self.trie.remove(last.id(), last.seq(), 1);
            self.last_ledger.remove(node_id);
        }
    }

    /// Check pending ledger acquisitions for completion. Matches
    /// `checkAcquired`.
    fn check_acquired(&mut self, adaptor: &A) {
        let pending: Vec<(SeqOf<A>, LedgerIdOf<A>)> = self.acquiring.keys().cloned().collect();
        for key in pending {
            let (_, ledger_id) = &key;
            if let Some(ledger) = adaptor.acquire(ledger_id) {
                let nodes: Vec<NodeIdOf<A>> = self.acquiring.get(&key).map(|s| s.iter().cloned().collect()).unwrap_or_default();
                for node_id in nodes {
                    self.update_trie_ledger(&node_id, ledger.clone());
                }
                self.acquiring.remove(&key);
            }
        }
    }

    /// Update the trie to reflect a newly-acquired validated ledger for
    /// `node_id`. Matches the two-argument `updateTrie(lock, nodeID,
    /// ledger)` overload.
    fn update_trie_ledger(&mut self, node_id: &NodeIdOf<A>, ledger: A::Ledger) {
        if let Some(prior) = self.last_ledger.insert(node_id.clone(), ledger.clone()) {
            self.trie.remove(prior.id(), prior.seq(), 1);
        }
        self.trie.insert(&ledger, 1);
    }

    /// Process a new trusted validation, updating the trie once its ledger
    /// is acquired (or queuing the acquisition). Matches the four-argument
    /// `updateTrie(lock, nodeID, val, prior)` overload.
    fn update_trie_validation(&mut self, adaptor: &A, node_id: &NodeIdOf<A>, val: &A::Validation, prior: Option<(SeqOf<A>, LedgerIdOf<A>)>) {
        debug_assert!(val.trusted(), "update_trie_validation: input validation must be trusted");

        if let Some(prior_key) = prior
            && let Some(set) = self.acquiring.get_mut(&prior_key)
        {
            set.remove(node_id);
            if set.is_empty() {
                self.acquiring.remove(&prior_key);
            }
        }

        self.check_acquired(adaptor);

        let val_key = (val.seq(), val.ledger_id());
        if let Some(set) = self.acquiring.get_mut(&val_key) {
            set.insert(node_id.clone());
        } else if let Some(ledger) = adaptor.acquire(&val.ledger_id()) {
            self.update_trie_ledger(node_id, ledger);
        } else {
            self.acquiring.entry(val_key).or_default().insert(node_id.clone());
        }
    }

    /// Iterate current validations, evicting stale ones first. Matches
    /// `current(lock, pre, f)`.
    fn current_live(&mut self, adaptor: &A, parms: &ValidationParms) -> Vec<(NodeIdOf<A>, A::Validation)> {
        let now = adaptor.now();
        let stale: Vec<NodeIdOf<A>> = self
            .current
            .iter()
            .filter(|(_, v)| !is_current(parms, now, v.sign_time(), v.seen_time()))
            .map(|(k, _)| k.clone())
            .collect();
        for node_id in &stale {
            if let Some(v) = self.current.get(node_id).cloned() {
                self.remove_trie(node_id, &v);
            }
            self.current.remove(node_id);
        }
        self.current.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }

    /// Access the trie after flushing stale validations and checking
    /// pending acquisitions. Matches `withTrie`.
    fn with_trie<R>(&mut self, adaptor: &A, parms: &ValidationParms, f: impl FnOnce(&LedgerTrie<A::Ledger>) -> R) -> R {
        let _ = self.current_live(adaptor, parms);
        self.check_acquired(adaptor);
        f(&self.trie)
    }
}

impl<A: ValidationsAdaptor> Validations<A> {
    /// Construct a new `Validations` tracker. Matches the reference's
    /// constructor (minus the injected `steady_clock` for aged-map
    /// expiry, which this port derives from `Instant::now()` directly at
    /// each call site rather than threading a clock object through).
    pub fn new(parms: ValidationParms, adaptor: A) -> Self {
        Self { parms, adaptor, inner: Mutex::new(Inner::new()) }
    }

    pub fn adaptor(&self) -> &A {
        &self.adaptor
    }

    pub fn parms(&self) -> &ValidationParms {
        &self.parms
    }

    /// Whether the local node may issue a validation for sequence `s`.
    /// Matches `canValidateSeq`.
    pub fn can_validate_seq(&self, s: SeqOf<A>) -> bool {
        let mut inner = self.inner.lock();
        inner.local_seq_enforcer.try_advance(Instant::now(), s, &self.parms)
    }

    /// Attempt to add a new validation. Matches `add`.
    pub fn add(&self, node_id: NodeIdOf<A>, val: A::Validation) -> ValStatus {
        if !is_current(&self.parms, self.adaptor.now(), val.sign_time(), val.seen_time()) {
            return ValStatus::Stale;
        }

        let mut inner = self.inner.lock();
        let now = Instant::now();

        let seq_entry = inner.by_sequence.get_or_insert_with(val.seq(), now, HashMap::new);
        let seq_inserted = !seq_entry.contains_key(&node_id);
        if !seq_inserted {
            let existing = seq_entry.get(&node_id).expect("checked contains_key above");
            let diff_secs = (existing.sign_time().as_seconds() as i64 - val.sign_time().as_seconds() as i64).unsigned_abs();
            if Duration::from_secs(diff_secs) > self.parms.validation_current_wall && val.sign_time() > existing.sign_time() {
                seq_entry.insert(node_id.clone(), val.clone());
            }
        } else {
            seq_entry.insert(node_id.clone(), val.clone());
        }

        let enforcer = inner.seq_enforcers.entry(node_id.clone()).or_default();
        if !enforcer.try_advance(now, val.seq(), &self.parms) {
            let seq_entry = inner.by_sequence.get(&val.seq()).expect("just inserted above");
            if let Some(existing) = seq_entry.get(&node_id)
                && existing.seq() == val.seq()
            {
                if existing.ledger_id() != val.ledger_id() {
                    return ValStatus::Conflicting;
                }
                if existing.sign_time() != val.sign_time() {
                    return ValStatus::Conflicting;
                }
                if existing.cookie() != val.cookie() {
                    return ValStatus::Multiple;
                }
            }
            return ValStatus::BadSeq;
        }

        inner.by_ledger.get_or_insert_with(val.ledger_id(), now, HashMap::new).insert(node_id.clone(), val.clone());

        match inner.current.get(&node_id).cloned() {
            None => {
                if val.trusted() {
                    inner.update_trie_validation(&self.adaptor, &node_id, &val, None);
                }
                inner.current.insert(node_id, val);
            }
            Some(old_val) => {
                if val.sign_time() > old_val.sign_time() {
                    let old_key = (old_val.seq(), old_val.ledger_id());
                    inner.current.insert(node_id.clone(), val.clone());
                    if val.trusted() {
                        inner.update_trie_validation(&self.adaptor, &node_id, &val, Some(old_key));
                    }
                } else {
                    return ValStatus::Stale;
                }
            }
        }

        ValStatus::Current
    }

    /// Set the `[low, high)` range of sequences to protect from expiry.
    /// Matches `setSeqToKeep`.
    pub fn set_seq_to_keep(&self, low: SeqOf<A>, high: SeqOf<A>) {
        debug_assert!(low < high, "set_seq_to_keep: low must be less than high");
        self.inner.lock().keep = Some((low, high));
    }

    /// Remove validation sets not accessed within `validation_set_expires`
    /// and not protected by `set_seq_to_keep`. Matches `expire`.
    pub fn expire(&self) {
        let start = Instant::now();
        {
            let mut inner = self.inner.lock();
            let now = Instant::now();

            if let Some((low, high)) = inner.keep {
                // by_ledger is keyed by ledger id, not sequence, so we must
                // look at each entry's validations to find their sequence
                // (matches the reference reading
                // `validationMap.begin()->second.seq()`).
                let to_refresh: Vec<LedgerIdOf<A>> = inner
                    .by_ledger
                    .entries
                    .iter()
                    .filter_map(|(ledger_id, validations)| {
                        let seq = validations.values().next()?.seq();
                        (low <= seq && seq < high).then_some(*ledger_id)
                    })
                    .collect();
                for ledger_id in to_refresh {
                    inner.by_ledger.touch(&ledger_id, now);
                }

                let seq_keep = |seq: &SeqOf<A>| low <= *seq && *seq < high;
                inner.by_ledger.expire(now, self.parms.validation_set_expires, None);
                inner.by_sequence.expire(now, self.parms.validation_set_expires, Some(&seq_keep));
            } else {
                inner.by_ledger.expire(now, self.parms.validation_set_expires, None);
                inner.by_sequence.expire(now, self.parms.validation_set_expires, None);
            }
        }
        tracing::debug!(elapsed = ?start.elapsed(), "Validations sets sweep lock duration");
    }

    /// Update trusted status of known validations to reflect UNL changes.
    /// Matches `trustChanged`.
    pub fn trust_changed(&self, added: &HashSet<NodeIdOf<A>>, removed: &HashSet<NodeIdOf<A>>) {
        let mut inner = self.inner.lock();

        let node_ids: Vec<NodeIdOf<A>> = inner.current.keys().cloned().collect();
        for node_id in node_ids {
            if added.contains(&node_id) {
                if let Some(v) = inner.current.get_mut(&node_id) {
                    v.set_trusted();
                }
                let v = inner.current.get(&node_id).cloned().expect("just updated");
                inner.update_trie_validation(&self.adaptor, &node_id, &v, None);
            } else if removed.contains(&node_id) {
                if let Some(v) = inner.current.get_mut(&node_id) {
                    v.set_untrusted();
                }
                let v = inner.current.get(&node_id).cloned().expect("just updated");
                inner.remove_trie(&node_id, &v);
            }
        }

        let ledger_ids: Vec<LedgerIdOf<A>> = inner.by_ledger.entries.keys().cloned().collect();
        for ledger_id in ledger_ids {
            if let Some(map) = inner.by_ledger.entries.get_mut(&ledger_id) {
                for (node_id, v) in map.iter_mut() {
                    if added.contains(node_id) {
                        v.set_trusted();
                    } else if removed.contains(node_id) {
                        v.set_untrusted();
                    }
                }
            }
        }
    }

    /// The sequence and id of the preferred working ledger, or `None` if
    /// no trusted validations exist to determine it. Matches `getPreferred`
    /// (the `optional<pair<Seq, ID>>`-returning overload).
    pub fn get_preferred(&self, curr: &A::Ledger) -> Option<(SeqOf<A>, LedgerIdOf<A>)> {
        let mut inner = self.inner.lock();
        let largest = inner.local_seq_enforcer.largest();
        let preferred: Option<SpanTip<A::Ledger>> = inner.with_trie(&self.adaptor, &self.parms, |trie| trie.get_preferred(largest));

        let Some(preferred) = preferred else {
            // No trusted validations; fall back to majority over acquiring
            // ledgers, breaking ties by ledger id.
            return inner
                .acquiring
                .iter()
                .max_by(|a, b| (a.1.len(), &a.0.1).cmp(&(b.1.len(), &b.0.1)))
                .map(|(key, _)| *key);
        };

        // If we are the parent of the preferred ledger, stick with our
        // current ledger since we might be about to generate it.
        if preferred.seq == curr.seq() + 1 && preferred.ancestor(curr.seq()) == curr.id() {
            return Some((curr.seq(), curr.id()));
        }

        // A ledger ahead of us is preferred regardless of chain.
        if preferred.seq > curr.seq() {
            return Some((preferred.seq, preferred.id));
        }

        // Only switch to an earlier/same sequence if it's a different chain.
        if curr.ancestor(preferred.seq) != preferred.id {
            return Some((preferred.seq, preferred.id));
        }

        Some((curr.seq(), curr.id()))
    }

    /// The id of the preferred working ledger, only if its sequence is at
    /// least `min_valid_seq`; otherwise `curr`'s id. Matches the
    /// `(curr, minValidSeq)` overload of `getPreferred`.
    pub fn get_preferred_min_seq(&self, curr: &A::Ledger, min_valid_seq: SeqOf<A>) -> LedgerIdOf<A> {
        match self.get_preferred(curr) {
            Some((seq, id)) if seq >= min_valid_seq => id,
            _ => curr.id(),
        }
    }

    /// The preferred last-closed-ledger id for the next consensus round,
    /// falling back to the dominant peer-reported LCL if no trusted
    /// validations exist. Matches `getPreferredLCL`.
    pub fn get_preferred_lcl(&self, lcl: &A::Ledger, min_seq: SeqOf<A>, peer_counts: &BTreeMap<LedgerIdOf<A>, u32>) -> LedgerIdOf<A> {
        if let Some((seq, id)) = self.get_preferred(lcl) {
            return if seq >= min_seq { id } else { lcl.id() };
        }

        peer_counts.iter().max_by(|a, b| (a.1, a.0).cmp(&(b.1, b.0))).map(|(id, _)| *id).unwrap_or_else(|| lcl.id())
    }

    /// The number of current trusted validators working on a descendant of
    /// `ledger_id`. If `ledger.id() != ledger_id`, only counts immediate
    /// children of `ledger_id`. Matches `getNodesAfter`.
    pub fn get_nodes_after(&self, ledger: &A::Ledger, ledger_id: &LedgerIdOf<A>) -> usize {
        let mut inner = self.inner.lock();
        if ledger.id() == *ledger_id {
            return inner.with_trie(&self.adaptor, &self.parms, |trie| {
                trie.branch_support(ledger) as usize - trie.tip_support(ledger.id()) as usize
            });
        }
        inner.last_ledger.values().filter(|l| l.seq() > SeqOf::<A>::default() && l.ancestor(l.seq() - 1) == *ledger_id).count()
    }

    /// Trusted, full validations from currently-current validators.
    /// Matches `currentTrusted`.
    pub fn current_trusted(&self) -> Vec<<A::Validation as ValidationT>::Wrapped> {
        let mut inner = self.inner.lock();
        inner.current_live(&self.adaptor, &self.parms).into_iter().filter(|(_, v)| v.trusted() && v.full()).map(|(_, v)| v.unwrap()).collect()
    }

    /// Node ids associated with current validations. Matches
    /// `getCurrentNodeIDs`.
    pub fn get_current_node_ids(&self) -> HashSet<NodeIdOf<A>> {
        let mut inner = self.inner.lock();
        inner.current_live(&self.adaptor, &self.parms).into_iter().map(|(k, _)| k).collect()
    }

    /// Count of trusted full validations for `ledger_id`. Matches
    /// `numTrustedForLedger`.
    pub fn num_trusted_for_ledger(&self, ledger_id: &LedgerIdOf<A>) -> usize {
        let mut inner = self.inner.lock();
        inner.by_ledger.touch(ledger_id, Instant::now());
        inner.by_ledger.get(ledger_id).map(|m| m.values().filter(|v| v.trusted() && v.full()).count()).unwrap_or(0)
    }

    /// Trusted full validations for `ledger_id` at sequence `seq`. Matches
    /// `getTrustedForLedger`.
    pub fn get_trusted_for_ledger(&self, ledger_id: &LedgerIdOf<A>, seq: SeqOf<A>) -> Vec<<A::Validation as ValidationT>::Wrapped> {
        let mut inner = self.inner.lock();
        inner.by_ledger.touch(ledger_id, Instant::now());
        inner
            .by_ledger
            .get(ledger_id)
            .map(|m| m.values().filter(|v| v.trusted() && v.full() && v.seq() == seq).cloned().map(|v| v.unwrap()).collect())
            .unwrap_or_default()
    }

    /// Fees reported by trusted full validators for `ledger_id`. Matches
    /// `fees`.
    pub fn fees(&self, ledger_id: &LedgerIdOf<A>, base_fee: u32) -> Vec<u32> {
        let mut inner = self.inner.lock();
        inner.by_ledger.touch(ledger_id, Instant::now());
        inner
            .by_ledger
            .get(ledger_id)
            .map(|m| m.values().filter(|v| v.trusted() && v.full()).map(|v| v.load_fee().unwrap_or(base_fee)).collect())
            .unwrap_or_default()
    }

    /// Clear all current validations. Matches `flush`.
    pub fn flush(&self) {
        self.inner.lock().current.clear();
    }

    /// Count lagging trusted proposers, removing seen-online proposers
    /// from `trusted_keys` as a side effect (matching the reference's
    /// documented behavior). Matches `laggards`.
    pub fn laggards(&self, seq: SeqOf<A>, trusted_keys: &mut HashSet<NodeKeyOf<A>>) -> usize {
        let mut inner = self.inner.lock();
        let now = self.adaptor.now();
        let live = inner.current_live(&self.adaptor, &self.parms);

        let mut laggards = 0usize;
        for (_, v) in live {
            let seen_plus_freshness =
                basics::chrono::NetClockTimePoint::new(v.seen_time().as_seconds().saturating_add(self.parms.validation_freshness.as_secs() as u32));
            if now < seen_plus_freshness && trusted_keys.remove(&v.key()) && seq > v.seq() {
                laggards += 1;
            }
        }
        laggards
    }

    pub fn size_of_current_cache(&self) -> usize {
        self.inner.lock().current.len()
    }

    pub fn size_of_seq_enforcers_cache(&self) -> usize {
        self.inner.lock().seq_enforcers.len()
    }

    pub fn size_of_by_ledger_cache(&self) -> usize {
        self.inner.lock().by_ledger.len()
    }

    pub fn size_of_by_sequence_cache(&self) -> usize {
        self.inner.lock().by_sequence.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    type NodeId = u32;
    type NodeKey = u32;
    type LedgerId = u8;

    #[derive(Debug, Clone, PartialEq, Eq, Default)]
    struct MockLedger {
        history: Vec<LedgerId>,
    }

    impl MockLedger {
        fn genesis_() -> Self {
            Self { history: vec![0] }
        }
        fn child(&self, id: LedgerId) -> Self {
            let mut history = self.history.clone();
            history.push(id);
            Self { history }
        }
    }

    impl TrieLedger for MockLedger {
        type Seq = u32;
        type Id = LedgerId;

        fn genesis() -> Self {
            MockLedger::genesis_()
        }
        fn seq(&self) -> u32 {
            (self.history.len() - 1) as u32
        }
        fn ancestor(&self, s: u32) -> LedgerId {
            self.history.get(s as usize).copied().unwrap_or(0)
        }
        fn mismatch(&self, other: &Self) -> u32 {
            let max_check = self.seq().min(other.seq()) + 1;
            for s in 0..max_check {
                if self.ancestor(s) != other.ancestor(s) {
                    return s;
                }
            }
            max_check
        }
    }

    impl ValidationsLedger for MockLedger {}

    #[derive(Debug, Clone)]
    struct MockValidation {
        ledger_id: LedgerId,
        seq: u32,
        sign_time: NetClockTimePoint,
        seen_time: NetClockTimePoint,
        key: NodeKey,
        node_id: NodeId,
        trusted: bool,
        full: bool,
        cookie: u64,
    }

    impl ValidationT for MockValidation {
        type LedgerId = LedgerId;
        type Seq = u32;
        type NodeId = NodeId;
        type NodeKey = NodeKey;
        type Wrapped = MockValidation;

        fn ledger_id(&self) -> LedgerId {
            self.ledger_id
        }
        fn seq(&self) -> u32 {
            self.seq
        }
        fn sign_time(&self) -> NetClockTimePoint {
            self.sign_time
        }
        fn seen_time(&self) -> NetClockTimePoint {
            self.seen_time
        }
        fn key(&self) -> NodeKey {
            self.key
        }
        fn trusted(&self) -> bool {
            self.trusted
        }
        fn set_trusted(&mut self) {
            self.trusted = true;
        }
        fn set_untrusted(&mut self) {
            self.trusted = false;
        }
        fn full(&self) -> bool {
            self.full
        }
        fn node_id(&self) -> NodeId {
            self.node_id
        }
        fn load_fee(&self) -> Option<u32> {
            None
        }
        fn cookie(&self) -> u64 {
            self.cookie
        }
        fn unwrap(self) -> MockValidation {
            self
        }
    }

    struct MockAdaptor {
        now: RefCell<NetClockTimePoint>,
        ledgers: RefCell<HashMap<LedgerId, MockLedger>>,
    }

    impl MockAdaptor {
        fn new(now: u32) -> Self {
            Self { now: RefCell::new(NetClockTimePoint::new(now)), ledgers: RefCell::new(HashMap::new()) }
        }
        fn set_now(&self, now: u32) {
            *self.now.borrow_mut() = NetClockTimePoint::new(now);
        }
        fn register_ledger(&self, ledger: MockLedger) {
            self.ledgers.borrow_mut().insert(ledger.id(), ledger);
        }
    }

    impl ValidationsAdaptor for MockAdaptor {
        type Ledger = MockLedger;
        type Validation = MockValidation;

        fn now(&self) -> NetClockTimePoint {
            *self.now.borrow()
        }
        fn acquire(&self, ledger_id: &LedgerId) -> Option<MockLedger> {
            self.ledgers.borrow().get(ledger_id).cloned()
        }
    }

    fn val(ledger_id: LedgerId, seq: u32, now: u32, node_id: NodeId, key: NodeKey) -> MockValidation {
        MockValidation {
            ledger_id,
            seq,
            sign_time: NetClockTimePoint::new(now),
            seen_time: NetClockTimePoint::new(now),
            key,
            node_id,
            trusted: true,
            full: true,
            cookie: 0,
        }
    }

    #[test]
    fn is_current_accepts_fresh_validation() {
        let p = ValidationParms::default();
        let now = NetClockTimePoint::new(1000);
        assert!(is_current(&p, now, now, now));
    }

    #[test]
    fn is_current_rejects_validation_signed_too_far_in_past_or_future() {
        let p = ValidationParms::default();
        let now = NetClockTimePoint::new(10_000);
        let too_early = NetClockTimePoint::new(10_000 - p.validation_current_early.as_secs() as u32 - 1);
        assert!(!is_current(&p, now, too_early, now));

        let too_late = NetClockTimePoint::new(10_000 + p.validation_current_wall.as_secs() as u32 + 1);
        assert!(!is_current(&p, now, too_late, now));
    }

    #[test]
    fn seq_enforcer_rejects_non_increasing_sequences() {
        let p = ValidationParms::default();
        let mut enforcer: SeqEnforcer<u32> = SeqEnforcer::default();
        let t0 = Instant::now();

        assert!(enforcer.try_advance(t0, 5, &p));
        assert_eq!(enforcer.largest(), 5);
        assert!(!enforcer.try_advance(t0, 5, &p));
        assert!(!enforcer.try_advance(t0, 3, &p));
        assert!(enforcer.try_advance(t0, 6, &p));
    }

    #[test]
    fn seq_enforcer_resets_after_expiry() {
        let p = ValidationParms::default();
        let mut enforcer: SeqEnforcer<u32> = SeqEnforcer::default();
        let t0 = Instant::now();

        assert!(enforcer.try_advance(t0, 10, &p));
        let t1 = t0 + p.validation_set_expires + Duration::from_secs(1);
        // After expiry, even a smaller sequence should be accepted again.
        assert!(enforcer.try_advance(t1, 2, &p));
        assert_eq!(enforcer.largest(), 2);
    }

    #[test]
    fn add_accepts_first_validation_as_current() {
        let adaptor = MockAdaptor::new(1000);
        let genesis = MockLedger::genesis_();
        let child = genesis.child(1);
        adaptor.register_ledger(genesis);
        adaptor.register_ledger(child.clone());
        let validations = Validations::new(ValidationParms::default(), adaptor);

        // seq=1, not 0: SeqEnforcer starts at Seq::default()==0 and
        // requires strictly-increasing sequences, so 0 can never satisfy
        // `s <= self.seq` -- this matches the reference's SeqEnforcer
        // exactly (genesis/seq-0 ledgers are never validated in practice).
        let v = val(child.id(), 1, 1000, 1, 100);
        assert_eq!(validations.add(1, v), ValStatus::Current);
        assert_eq!(validations.size_of_current_cache(), 1);
    }

    #[test]
    fn add_rejects_stale_validation() {
        let adaptor = MockAdaptor::new(1000);
        let validations = Validations::new(ValidationParms::default(), adaptor);

        let p = ValidationParms::default();
        let far_past = 1000 - p.validation_current_early.as_secs() as u32 - 10;
        let v = val(0, 0, far_past, 1, 100);
        assert_eq!(validations.add(1, v), ValStatus::Stale);
    }

    #[test]
    fn add_rejects_bad_sequence_from_same_node() {
        let adaptor = MockAdaptor::new(1000);
        let genesis = MockLedger::genesis_();
        adaptor.register_ledger(genesis.clone());
        let child = genesis.child(1);
        adaptor.register_ledger(child.clone());
        let validations = Validations::new(ValidationParms::default(), adaptor);

        assert_eq!(validations.add(1, val(child.id(), 5, 1000, 1, 100)), ValStatus::Current);
        // A lower sequence from the same node violates the invariant.
        assert_eq!(validations.add(1, val(genesis.id(), 3, 1000, 1, 100)), ValStatus::BadSeq);
    }

    #[test]
    fn add_detects_conflicting_validation_same_sequence_different_ledger() {
        let adaptor = MockAdaptor::new(1000);
        let genesis = MockLedger::genesis_();
        adaptor.register_ledger(genesis.clone());
        let child_a = genesis.child(1);
        let child_b = genesis.child(2);
        adaptor.register_ledger(child_a.clone());
        adaptor.register_ledger(child_b.clone());
        let validations = Validations::new(ValidationParms::default(), adaptor);

        assert_eq!(validations.add(1, val(child_a.id(), 5, 1000, 1, 100)), ValStatus::Current);
        // Same node, same sequence, different ledger id -> Byzantine signal.
        let status = validations.add(1, val(child_b.id(), 5, 1000, 1, 100));
        assert_eq!(status, ValStatus::Conflicting);
    }

    #[test]
    fn add_detects_conflicting_validation_when_second_message_is_a_genuinely_distinct_replay() {
        // Regression test for the Byzantine-detection wiring itself: two
        // validations for the same (node, seq) submitted as fully separate
        // `add()` calls (not overwriting each other beforehand) must be
        // compared against each other, not against themselves. This
        // exercises the same path as
        // `add_detects_conflicting_validation_same_sequence_different_ledger`
        // but additionally asserts the intermediate tracked state to rule
        // out a "compares val against the value it just inserted" bug,
        // which would make every field trivially equal and always fall
        // through to BadSeq instead of Conflicting/Multiple.
        let adaptor = MockAdaptor::new(1000);
        let genesis = MockLedger::genesis_();
        let child_a = genesis.child(1);
        let child_b = genesis.child(2);
        adaptor.register_ledger(genesis);
        adaptor.register_ledger(child_a.clone());
        adaptor.register_ledger(child_b.clone());
        let validations = Validations::new(ValidationParms::default(), adaptor);

        let first = val(child_a.id(), 5, 1000, 1, 100);
        assert_eq!(validations.add(1, first.clone()), ValStatus::Current);

        // Same sequence, same sign time, different ledger AND different
        // cookie -- ledger mismatch must win and report Conflicting, not
        // Multiple, matching the reference's check order (ledgerID check
        // comes before the cookie check).
        let mut second = val(child_b.id(), 5, 1000, 1, 100);
        second.cookie = 999;
        assert_eq!(validations.add(1, second), ValStatus::Conflicting);

        // A genuinely fresh node/seq pair reporting Multiple: same ledger,
        // same sign time, but a different cookie.
        let third = val(child_a.id(), 6, 1000, 2, 200);
        assert_eq!(validations.add(2, third.clone()), ValStatus::Current);
        let mut fourth = third.clone();
        fourth.cookie = 12345;
        // Node 2's enforcer already advanced to 6 from `third`, so a repeat
        // at seq=6 is rejected by the enforcer and compared against the
        // tracked entry (`third`) -- same ledger/signTime, different
        // cookie -> Multiple.
        assert_eq!(validations.add(2, fourth), ValStatus::Multiple);
    }

    #[test]
    fn can_validate_seq_enforces_local_monotonic_invariant() {
        let adaptor = MockAdaptor::new(1000);
        let validations = Validations::new(ValidationParms::default(), adaptor);

        assert!(validations.can_validate_seq(1));
        assert!(validations.can_validate_seq(2));
        assert!(!validations.can_validate_seq(2));
        assert!(!validations.can_validate_seq(1));
    }

    #[test]
    fn get_preferred_returns_none_with_no_trusted_validations() {
        let adaptor = MockAdaptor::new(1000);
        let genesis = MockLedger::genesis_();
        let validations = Validations::new(ValidationParms::default(), adaptor);
        assert_eq!(validations.get_preferred(&genesis), None);
    }

    #[test]
    fn get_preferred_prefers_ledger_with_more_trusted_validation_support() {
        let adaptor = MockAdaptor::new(1000);
        let genesis = MockLedger::genesis_();
        let child_a = genesis.child(1);
        let child_b = genesis.child(2);
        adaptor.register_ledger(genesis.clone());
        adaptor.register_ledger(child_a.clone());
        adaptor.register_ledger(child_b.clone());
        let validations = Validations::new(ValidationParms::default(), adaptor);

        // Two validators support child_a, one supports child_b.
        validations.add(1, val(child_a.id(), 1, 1000, 1, 100));
        validations.add(2, val(child_a.id(), 1, 1000, 2, 200));
        validations.add(3, val(child_b.id(), 1, 1000, 3, 300));

        // `get_preferred` special-cases "we are the immediate parent of
        // the preferred ledger" by sticking with `curr` (matching the
        // reference: "we might be about to generate it"), so querying
        // with `genesis` as `curr` would trivially return genesis
        // regardless of which child has more support. To exercise the
        // actual branch-preference comparison, query branch_support
        // directly via the trie instead of get_preferred's curr-relative
        // logic.
        let preferred = validations.get_preferred(&child_a).expect("trusted validations exist");
        assert_eq!(preferred.1, child_a.id());
        assert_eq!(preferred.0, child_a.seq());
    }

    #[test]
    fn trust_changed_removes_untrusted_nodes_from_trie_support() {
        let adaptor = MockAdaptor::new(1000);
        let genesis = MockLedger::genesis_();
        let child = genesis.child(1);
        adaptor.register_ledger(genesis.clone());
        adaptor.register_ledger(child.clone());
        let validations = Validations::new(ValidationParms::default(), adaptor);

        validations.add(1, val(child.id(), 1, 1000, 1, 100));
        assert_eq!(validations.get_nodes_after(&child, &child.id()), 0);

        let mut removed = HashSet::new();
        removed.insert(1u32);
        validations.trust_changed(&HashSet::new(), &removed);

        // With the only trusted validator removed, there's no support left.
        assert_eq!(validations.get_preferred(&genesis), None);
    }

    #[test]
    fn flush_clears_current_validations() {
        let adaptor = MockAdaptor::new(1000);
        let genesis = MockLedger::genesis_();
        let child = genesis.child(1);
        adaptor.register_ledger(genesis);
        adaptor.register_ledger(child.clone());
        let validations = Validations::new(ValidationParms::default(), adaptor);

        validations.add(1, val(child.id(), 1, 1000, 1, 100));
        assert_eq!(validations.size_of_current_cache(), 1);
        validations.flush();
        assert_eq!(validations.size_of_current_cache(), 0);
    }

    #[test]
    fn num_trusted_for_ledger_counts_only_trusted_full_validations() {
        let adaptor = MockAdaptor::new(1000);
        let genesis = MockLedger::genesis_();
        let child = genesis.child(1);
        adaptor.register_ledger(genesis);
        adaptor.register_ledger(child.clone());
        let validations = Validations::new(ValidationParms::default(), adaptor);

        let mut untrusted = val(child.id(), 1, 1000, 2, 200);
        untrusted.trusted = false;
        validations.add(1, val(child.id(), 1, 1000, 1, 100));
        validations.add(2, untrusted);

        assert_eq!(validations.num_trusted_for_ledger(&child.id()), 1);
    }

    #[test]
    fn current_trusted_evicts_validations_once_they_go_stale() {
        let adaptor = MockAdaptor::new(1000);
        let genesis = MockLedger::genesis_();
        let child = genesis.child(1);
        adaptor.register_ledger(genesis);
        adaptor.register_ledger(child.clone());
        let validations = Validations::new(ValidationParms::default(), adaptor);

        validations.add(1, val(child.id(), 1, 1000, 1, 100));
        assert_eq!(validations.current_trusted().len(), 1);

        // Advance the network clock well past validation_current_wall so
        // the validation, signed at t=1000, is no longer current.
        let p = ValidationParms::default();
        let far_future = 1000 + p.validation_current_wall.as_secs() as u32 + 100;
        validations.adaptor().set_now(far_future);

        assert_eq!(validations.current_trusted().len(), 0);
        assert_eq!(validations.size_of_current_cache(), 0);
    }
}
