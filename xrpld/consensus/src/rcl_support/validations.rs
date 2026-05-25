use crate::ledger_trie::{LedgerHistory, LedgerTrie};
use crate::params::ConsensusParms;
use basics::base_uint::Uint256;
use basics::chrono::NetClockTimePoint;
use protocol::PublicKey;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use tracing;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationStatus {
    Current,
    Stale,
    BadSeq,
    Conflicting,
    Multiple,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RclValidation {
    pub ledger_id: Uint256,
    pub seq: u32,
    pub sign_time: NetClockTimePoint,
    pub seen_time: NetClockTimePoint,
    pub key: PublicKey,
    pub trusted: bool,
    pub full: bool,
    pub load_fee: Option<u32>,
    pub cookie: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RclValidatedLedger {
    pub ledger_id: Uint256,
    pub ledger_seq: u32,
    pub ancestors: Vec<Uint256>,
}

impl RclValidatedLedger {
    pub fn genesis() -> Self {
        Self {
            ledger_id: Uint256::default(),
            ledger_seq: 0,
            ancestors: Vec::new(),
        }
    }

    pub fn min_seq(&self) -> u32 {
        self.ledger_seq
            .saturating_sub(u32::try_from(self.ancestors.len()).expect("ancestor len fits u32"))
    }

    pub fn ancestor(&self, seq: u32) -> Uint256 {
        if seq == self.ledger_seq {
            return self.ledger_id;
        }
        if seq >= self.min_seq() && seq <= self.ledger_seq {
            let diff = usize::try_from(self.ledger_seq - seq).expect("diff fits usize");
            return self.ancestors[self.ancestors.len() - diff];
        }
        Uint256::default()
    }

    pub fn seq(&self) -> u32 {
        self.ledger_seq
    }

    pub fn id(&self) -> Uint256 {
        self.ledger_id
    }
}

impl LedgerHistory for RclValidatedLedger {
    type Id = Uint256;

    fn make_genesis() -> Self {
        Self::genesis()
    }

    fn seq(&self) -> u32 {
        self.ledger_seq
    }

    fn id(&self) -> Self::Id {
        self.ledger_id
    }

    fn ancestor(&self, seq: u32) -> Self::Id {
        RclValidatedLedger::ancestor(self, seq)
    }
}

pub trait RclValidationsAdapter: Clone {
    fn now(&self) -> NetClockTimePoint;
    fn acquire(&mut self, ledger_id: &Uint256) -> Option<RclValidatedLedger>;
}

#[derive(Debug, Clone, Default)]
struct SeqEnforcer {
    seq: u32,
    when: Option<NetClockTimePoint>,
}

impl SeqEnforcer {
    fn check(&mut self, now: NetClockTimePoint, seq: u32, parms: &ConsensusParms) -> bool {
        if self
            .when
            .is_some_and(|when| duration_between(when, now) > parms.validation_set_expires)
        {
            self.seq = 0;
            self.when = None;
        }
        if seq <= self.seq {
            return false;
        }
        self.seq = seq;
        self.when = Some(now);
        true
    }
}

#[derive(Debug, Clone)]
struct TimedValidationSet {
    last_access: NetClockTimePoint,
    validations: BTreeMap<PublicKey, RclValidation>,
}

impl TimedValidationSet {
    fn new(now: NetClockTimePoint) -> Self {
        Self {
            last_access: now,
            validations: BTreeMap::new(),
        }
    }

    fn touch(&mut self, now: NetClockTimePoint) {
        self.last_access = now;
    }

    fn insert(&mut self, now: NetClockTimePoint, node_id: PublicKey, validation: RclValidation) {
        self.touch(now);
        self.validations.insert(node_id, validation);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KeepRange {
    low: u32,
    high: u32,
}

#[derive(Debug, Clone)]
pub struct RclValidations<A: RclValidationsAdapter> {
    parms: ConsensusParms,
    adaptor: A,
    current: BTreeMap<PublicKey, RclValidation>,
    by_ledger: HashMap<Uint256, TimedValidationSet>,
    by_sequence: BTreeMap<u32, TimedValidationSet>,
    acquiring: BTreeMap<(u32, Uint256), BTreeSet<PublicKey>>,
    last_ledger: BTreeMap<PublicKey, RclValidatedLedger>,
    local_seq_enforcer: SeqEnforcer,
    seq_enforcers: HashMap<PublicKey, SeqEnforcer>,
    keep_range: Option<KeepRange>,
    next_keep_refresh: Option<NetClockTimePoint>,
    trie: LedgerTrie<RclValidatedLedger>,
}

impl<A: RclValidationsAdapter> RclValidations<A> {
    pub fn new(adaptor: A, parms: ConsensusParms) -> Self {
        Self {
            parms,
            adaptor,
            current: BTreeMap::new(),
            by_ledger: HashMap::new(),
            by_sequence: BTreeMap::new(),
            acquiring: BTreeMap::new(),
            last_ledger: BTreeMap::new(),
            local_seq_enforcer: SeqEnforcer::default(),
            seq_enforcers: HashMap::new(),
            keep_range: None,
            next_keep_refresh: None,
            trie: LedgerTrie::new(),
        }
    }

    pub fn adaptor(&self) -> &A {
        &self.adaptor
    }

    pub fn adaptor_mut(&mut self) -> &mut A {
        &mut self.adaptor
    }

    pub fn add(&mut self, node_id: PublicKey, validation: RclValidation) -> ValidationStatus {
        let now = self.adaptor.now();
        self.expire_validation_sets(now);
        if !self.is_current(now, &validation) {
            tracing::warn!(target: "consensus", signer = %node_id, "Stale validation (too old)");
            return ValidationStatus::Stale;
        }

        if !validation.trusted {
            tracing::debug!(target: "consensus", signer = %node_id, "Untrusted validation dropped");
        }

        let seq = validation.seq;
        tracing::debug!(target: "consensus", signer = %node_id, seq, "Validation received");

        if !self
            .seq_enforcers
            .entry(node_id)
            .or_default()
            .check(now, validation.seq, &self.parms)
        {
            if let Some(existing) = self
                .by_sequence
                .get(&validation.seq)
                .and_then(|set| set.validations.get(&node_id))
            {
                if existing.ledger_id != validation.ledger_id
                    || existing.sign_time != validation.sign_time
                {
                    return ValidationStatus::Conflicting;
                }
                if existing.cookie != validation.cookie {
                    return ValidationStatus::Multiple;
                }
            }
            return ValidationStatus::BadSeq;
        }

        let prior = if let Some(existing) = self.current.get(&node_id) {
            if validation.seq < existing.seq {
                return ValidationStatus::Stale;
            }
            if validation.seq == existing.seq {
                if validation.ledger_id == existing.ledger_id {
                    return ValidationStatus::Multiple;
                }
                return ValidationStatus::Conflicting;
            }
            Some((existing.seq, existing.ledger_id))
        } else {
            None
        };

        self.by_ledger
            .entry(validation.ledger_id)
            .or_insert_with(|| TimedValidationSet::new(now))
            .insert(now, node_id, validation.clone());
        self.by_sequence
            .entry(validation.seq)
            .or_insert_with(|| TimedValidationSet::new(now))
            .insert(now, node_id, validation.clone());
        self.current.insert(node_id, validation.clone());

        if validation.trusted {
            self.update_trusted_ledger(node_id, &validation, prior);
        }

        ValidationStatus::Current
    }

    pub fn set_seq_to_keep(&mut self, low: u32, high: u32) {
        assert!(low < high, "xrpl::Validations::setSeqToKeep : valid inputs");
        self.keep_range = Some(KeepRange { low, high });
        self.next_keep_refresh = None;
    }

    pub fn current_trusted(&mut self) -> Vec<RclValidation> {
        self.flush_stale();
        let result: Vec<RclValidation> = self
            .current
            .values()
            .filter(|validation| validation.trusted && validation.full)
            .cloned()
            .collect();
        if !result.is_empty() {
            let seq = result.first().map(|v| v.seq).unwrap_or(0);
            let trusted_count = result.len();
            tracing::info!(target: "consensus", seq, trusted_count, "Validation quorum reached");
        }
        result
    }

    pub fn current_node_ids(&mut self) -> BTreeSet<PublicKey> {
        self.flush_stale();
        self.current.keys().copied().collect()
    }

    pub fn num_trusted_for_ledger(&mut self, ledger_id: Uint256) -> usize {
        let count = self.trusted_for_ledger(ledger_id).len();
        tracing::debug!(target: "consensus", %ledger_id, trusted_count = count, "Trusted validations for ledger");
        count
    }

    pub fn trusted_for_ledger(&mut self, ledger_id: Uint256) -> Vec<PublicKey> {
        self.flush_stale();
        let now = self.adaptor.now();
        self.by_ledger
            .get_mut(&ledger_id)
            .into_iter()
            .flat_map(|set| {
                set.touch(now);
                set.validations.iter()
            })
            .filter_map(|(node_id, validation)| {
                (validation.trusted && validation.full).then_some(*node_id)
            })
            .collect()
    }

    pub fn trusted_for_ledger_by_sequence(
        &mut self,
        ledger_id: Uint256,
        seq: u32,
    ) -> Vec<PublicKey> {
        self.flush_stale();
        let now = self.adaptor.now();
        self.by_ledger
            .get_mut(&ledger_id)
            .into_iter()
            .flat_map(|set| {
                set.touch(now);
                set.validations.iter()
            })
            .filter_map(|(node_id, validation)| {
                (validation.trusted && validation.full && validation.seq == seq).then_some(*node_id)
            })
            .collect()
    }

    pub fn fees(&mut self, ledger_id: Uint256, base_fee: u32) -> Vec<u32> {
        self.flush_stale();
        let now = self.adaptor.now();
        self.by_ledger
            .get_mut(&ledger_id)
            .into_iter()
            .flat_map(|set| {
                set.touch(now);
                set.validations.values()
            })
            .filter_map(|validation| {
                (validation.trusted && validation.full)
                    .then_some(validation.load_fee.unwrap_or(base_fee))
            })
            .collect()
    }

    pub fn get_json_trie(&mut self) -> Value {
        self.flush_stale();
        self.trie.get_json()
    }

    pub fn get_preferred(&mut self, curr: RclValidatedLedger) -> Option<(u32, Uint256)> {
        self.flush_stale();
        if let Some(preferred) = self.trie.get_preferred(self.local_seq_enforcer.seq) {
            tracing::debug!(target: "consensus", preferred_seq = preferred.seq, "Preferred ledger from trie");
            if preferred.seq == curr.seq() + 1 && preferred.ancestor(curr.seq()) == curr.id() {
                return Some((curr.seq(), curr.id()));
            }
            if preferred.seq > curr.seq() {
                return Some((preferred.seq, preferred.id));
            }
            if curr.ancestor(preferred.seq) != preferred.id {
                return Some((preferred.seq, preferred.id));
            }
            return Some((curr.seq(), curr.id()));
        }

        self.acquiring
            .iter()
            .max_by(|left, right| (left.1.len(), left.0.1).cmp(&(right.1.len(), right.0.1)))
            .map(|(ledger, _)| *ledger)
    }

    pub fn get_preferred_with_min_seq(
        &mut self,
        curr: RclValidatedLedger,
        min_valid_seq: u32,
    ) -> Uint256 {
        self.get_preferred(curr.clone())
            .filter(|(seq, _)| *seq >= min_valid_seq)
            .map(|(_, id)| id)
            .unwrap_or_else(|| curr.id())
    }

    pub fn get_nodes_after(&mut self, ledger: &RclValidatedLedger, ledger_id: Uint256) -> usize {
        self.flush_stale();
        if ledger.id() == ledger_id {
            return self
                .trie
                .branch_support(ledger)
                .saturating_sub(self.trie.tip_support(ledger)) as usize;
        }

        self.last_ledger
            .values()
            .filter(|curr| curr.seq() > 0 && curr.ancestor(curr.seq() - 1) == ledger_id)
            .count()
    }

    pub fn can_validate_seq(&mut self, seq: u32) -> bool {
        let result = self
            .local_seq_enforcer
            .check(self.adaptor.now(), seq, &self.parms);
        if !result {
            tracing::debug!(target: "consensus", seq, "Cannot validate sequence (already validated)");
        }
        result
    }

    pub fn laggards(&mut self, seq: u32, trusted_keys: &mut BTreeSet<PublicKey>) -> usize {
        self.flush_stale();
        let mut laggards = 0usize;
        for validation in self.current.values() {
            if duration_between(validation.seen_time, self.adaptor.now())
                > self.parms.validation_freshness
            {
                continue;
            }
            if !trusted_keys.contains(&validation.key) {
                continue;
            }
            if validation.seq >= seq {
                trusted_keys.remove(&validation.key);
            } else {
                laggards += 1;
                trusted_keys.remove(&validation.key);
            }
        }
        if laggards > 0 {
            tracing::debug!(target: "consensus", seq, laggards, "Validation laggards detected");
        }
        laggards
    }

    pub fn trust_changed(&mut self, added: &BTreeSet<PublicKey>, removed: &BTreeSet<PublicKey>) {
        tracing::info!(target: "consensus", added = added.len(), removed = removed.len(), "Validation trust list changed");
        self.flush_stale();

        let updates = self
            .current
            .iter()
            .filter_map(|(node_id, validation)| {
                if added.contains(node_id) {
                    Some((*node_id, true, validation.clone()))
                } else if removed.contains(node_id) {
                    Some((*node_id, false, validation.clone()))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        for (node_id, trust, validation) in updates {
            if let Some(current) = self.current.get_mut(&node_id) {
                current.trusted = trust;
            }
            if trust {
                self.update_trusted_ledger(node_id, &validation, None);
            } else {
                self.remove_trie_tracking(&node_id, &validation);
            }
        }

        for set in self.by_ledger.values_mut() {
            for (node_id, validation) in &mut set.validations {
                if added.contains(node_id) {
                    validation.trusted = true;
                } else if removed.contains(node_id) {
                    validation.trusted = false;
                }
            }
        }

        for set in self.by_sequence.values_mut() {
            for (node_id, validation) in &mut set.validations {
                if added.contains(node_id) {
                    validation.trusted = true;
                } else if removed.contains(node_id) {
                    validation.trusted = false;
                }
            }
        }
    }

    pub fn check_acquired(&mut self) {
        let pending = self
            .acquiring
            .keys()
            .copied()
            .collect::<Vec<(u32, Uint256)>>();
        for (seq, ledger_id) in pending {
            if let Some(ledger) = self.adaptor.acquire(&ledger_id)
                && let Some(nodes) = self.acquiring.remove(&(seq, ledger_id))
            {
                tracing::debug!(target: "consensus", seq, validators = nodes.len(), "Acquired pending validation ledger");
                for node_id in nodes {
                    self.update_trie_ledger(node_id, ledger.clone());
                }
            }
        }
    }

    pub fn last_ledger(&self, node_id: PublicKey) -> Option<&RclValidatedLedger> {
        self.last_ledger.get(&node_id)
    }

    fn update_trusted_ledger(
        &mut self,
        node_id: PublicKey,
        validation: &RclValidation,
        prior: Option<(u32, Uint256)>,
    ) {
        if let Some(prior) = prior
            && let Some(acquiring) = self.acquiring.get_mut(&prior)
        {
            acquiring.remove(&node_id);
            if acquiring.is_empty() {
                self.acquiring.remove(&prior);
            }
        }

        self.check_acquired();

        let key = (validation.seq, validation.ledger_id);
        if let Some(acquiring) = self.acquiring.get_mut(&key) {
            acquiring.insert(node_id);
        } else if let Some(ledger) = self.adaptor.acquire(&validation.ledger_id) {
            self.update_trie_ledger(node_id, ledger);
        } else {
            self.acquiring.entry(key).or_default().insert(node_id);
        }
    }

    fn flush_stale(&mut self) {
        let now = self.adaptor.now();
        self.expire_validation_sets(now);
        let stale = self
            .current
            .iter()
            .filter_map(|(node_id, validation)| {
                (!self.is_current(now, validation)).then_some((*node_id, validation.clone()))
            })
            .collect::<Vec<_>>();

        if !stale.is_empty() {
            tracing::debug!(target: "consensus", count = stale.len(), "Flushing stale validations");
        }
        for (node_id, validation) in stale {
            self.current.remove(&node_id);
            self.remove_trie_tracking(&node_id, &validation);
        }
        self.check_acquired();
    }

    fn remove_trie_tracking(&mut self, node_id: &PublicKey, validation: &RclValidation) {
        if let Some(acquiring) = self
            .acquiring
            .get_mut(&(validation.seq, validation.ledger_id))
        {
            acquiring.remove(node_id);
            if acquiring.is_empty() {
                self.acquiring
                    .remove(&(validation.seq, validation.ledger_id));
            }
        }

        if let Some(ledger) = self.last_ledger.get(node_id)
            && ledger.id() == validation.ledger_id
        {
            let removed = self.trie.remove(ledger, 1);
            assert!(removed, "trusted validation ledger must exist in trie");
            self.last_ledger.remove(node_id);
        }
    }

    fn update_trie_ledger(&mut self, node_id: PublicKey, ledger: RclValidatedLedger) {
        tracing::debug!(target: "consensus", signer = %node_id, seq = ledger.ledger_seq, "Updating validation trie");
        if let Some(existing) = self.last_ledger.insert(node_id, ledger.clone()) {
            let removed = self.trie.remove(&existing, 1);
            assert!(
                removed,
                "prior trusted validation ledger must exist in trie"
            );
        }
        self.trie.insert(ledger, 1);
    }

    fn is_current(&self, now: NetClockTimePoint, validation: &RclValidation) -> bool {
        let wall_age = duration_between(validation.sign_time, now);
        let local_age = duration_between(validation.seen_time, now);
        let early_age = duration_between(now, validation.sign_time);

        wall_age <= self.parms.validation_valid_wall
            && local_age <= self.parms.validation_valid_local
            && early_age <= self.parms.validation_valid_early
    }

    fn expire_validation_sets(&mut self, now: NetClockTimePoint) {
        if let Some(keep_range) = self.keep_range {
            let needs_refresh = self
                .next_keep_refresh
                .is_none_or(|refresh_at| now >= refresh_at);
            if needs_refresh {
                self.touch_keep_range(now, keep_range);
                let refresh_after = self
                    .parms
                    .validation_set_expires
                    .saturating_sub(self.parms.validation_freshness);
                self.next_keep_refresh = Some(
                    now + time::Duration::seconds(
                        i64::try_from(refresh_after.as_secs()).expect("refresh duration fits i64"),
                    ),
                );
            }
        }

        let expiry = self.parms.validation_set_expires;
        self.by_ledger
            .retain(|_, set| duration_between(set.last_access, now) <= expiry);
        self.by_sequence
            .retain(|_, set| duration_between(set.last_access, now) <= expiry);
    }

    fn touch_keep_range(&mut self, now: NetClockTimePoint, keep_range: KeepRange) {
        for set in self.by_ledger.values_mut() {
            if let Some(validation) = set.validations.values().next()
                && keep_range.low <= validation.seq
                && validation.seq < keep_range.high
            {
                set.touch(now);
            }
        }

        for (seq, set) in &mut self.by_sequence {
            if keep_range.low <= *seq && *seq < keep_range.high {
                set.touch(now);
            }
        }
    }
}

fn duration_between(start: NetClockTimePoint, end: NetClockTimePoint) -> std::time::Duration {
    let diff = end - start;
    std::time::Duration::from_secs(
        u64::try_from(diff.whole_seconds().max(0)).expect("seconds must fit u64"),
    )
}
