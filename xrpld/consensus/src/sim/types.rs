//! Core simulation types: SimTime, Tx, TxSet, Ledger, LedgerOracle, Proposal.
//!

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::{Hash, Hasher};
use std::time::Duration;

// ─── SimTime ─────────────────────────────────────────────────────────────────

/// Simulated time point (nanoseconds from epoch).
pub type SimTime = Duration;

/// Simulated duration.
pub type SimDuration = Duration;

// ─── Tx ──────────────────────────────────────────────────────────────────────

/// A single simulated transaction, identified by an integer ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tx {
    pub id: u32,
}

impl Tx {
    pub fn new(id: u32) -> Self {
        Self { id }
    }
}

/// The set of transactions in a ledger or proposal.
pub type TxSetType = BTreeSet<Tx>;

/// A transaction set with a computed ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxSet {
    pub txs: TxSetType,
    id: u64,
}

impl TxSet {
    pub fn new(txs: TxSetType) -> Self {
        let id = Self::calc_id(&txs);
        Self { txs, id }
    }

    pub fn empty() -> Self {
        Self::new(BTreeSet::new())
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn exists(&self, tx_id: u32) -> bool {
        self.txs.contains(&Tx::new(tx_id))
    }

    pub fn find(&self, tx_id: u32) -> Option<&Tx> {
        self.txs.get(&Tx::new(tx_id))
    }

    /// Compute differences: returns map of tx_id → bool (true = in self not other).
    pub fn compare(&self, other: &TxSet) -> BTreeMap<u32, bool> {
        let mut res = BTreeMap::new();
        for tx in self.txs.difference(&other.txs) {
            res.insert(tx.id, true);
        }
        for tx in other.txs.difference(&self.txs) {
            res.insert(tx.id, false);
        }
        res
    }

    fn calc_id(txs: &TxSetType) -> u64 {
        let mut h: u64 = 0;
        for tx in txs {
            h = h
                .wrapping_mul(131)
                .wrapping_add(tx.id as u64)
                .wrapping_add(1);
        }
        h
    }
}

/// Mutable transaction set for building proposals.
#[derive(Debug, Clone)]
pub struct MutableTxSet {
    pub txs: TxSetType,
}

impl MutableTxSet {
    pub fn from_txset(s: &TxSet) -> Self {
        Self { txs: s.txs.clone() }
    }

    pub fn insert(&mut self, t: Tx) -> bool {
        self.txs.insert(t)
    }

    pub fn erase(&mut self, tx_id: u32) -> bool {
        self.txs.remove(&Tx::new(tx_id))
    }

    pub fn into_txset(self) -> TxSet {
        TxSet::new(self.txs)
    }
}

// ─── PeerID ──────────────────────────────────────────────────────────────────

/// Peer identifier in the simulation.
pub type PeerID = u32;

// ─── Ledger ──────────────────────────────────────────────────────────────────

/// Ledger sequence number.
pub type LedgerSeq = u32;

/// Ledger ID (unique identifier assigned by the oracle).
pub type LedgerID = u32;

/// Close time for simulated ledgers (seconds since epoch).
pub type CloseTime = u32;

/// A simulated ledger — immutable value type.
#[derive(Debug, Clone)]
pub struct Ledger {
    pub id: LedgerID,
    pub seq: LedgerSeq,
    pub txs: TxSetType,
    pub close_time_resolution: Duration,
    pub close_time: CloseTime,
    pub close_time_agree: bool,
    pub parent_id: LedgerID,
    pub parent_close_time: CloseTime,
    pub ancestors: Vec<LedgerID>,
}

impl Ledger {
    /// Create the genesis ledger.
    pub fn genesis() -> Self {
        Self {
            id: 0,
            seq: 0,
            txs: BTreeSet::new(),
            close_time_resolution: Duration::from_secs(30),
            close_time: 0,
            close_time_agree: true,
            parent_id: 0,
            parent_close_time: 0,
            ancestors: Vec::new(),
        }
    }

    /// Check if `ancestor` is an ancestor of this ledger.
    pub fn is_ancestor(&self, ancestor: &Ledger) -> bool {
        if ancestor.seq >= self.seq {
            return ancestor.id == self.id;
        }
        self.ancestor_id(ancestor.seq) == Some(ancestor.id)
    }

    /// Get the ancestor ID at a given sequence.
    pub fn ancestor_id(&self, seq: LedgerSeq) -> Option<LedgerID> {
        if seq == self.seq {
            return Some(self.id);
        }
        if seq > self.seq {
            return None;
        }
        self.ancestors.get(seq as usize).copied()
    }
}

impl PartialEq for Ledger {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl Eq for Ledger {}

impl PartialOrd for Ledger {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Ledger {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.id.cmp(&other.id)
    }
}

impl Hash for Ledger {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.hash(state);
    }
}

/// Find the first sequence where two ledgers diverge.
pub fn mismatch(a: &Ledger, b: &Ledger) -> LedgerSeq {
    let min_seq = a.seq.min(b.seq);
    for seq in 0..=min_seq {
        if a.ancestor_id(seq) != b.ancestor_id(seq) {
            return seq;
        }
    }
    min_seq + 1
}

// ─── LedgerOracle ────────────────────────────────────────────────────────────

/// Maintains unique ledgers for a simulation. Assigns IDs and tracks history.
///
pub struct LedgerOracle {
    /// Map from instance hash → (LedgerID, Ledger)
    instances: HashMap<u64, Ledger>,
    next_id: LedgerID,
}

impl Default for LedgerOracle {
    fn default() -> Self {
        Self::new()
    }
}

impl LedgerOracle {
    pub fn new() -> Self {
        let genesis = Ledger::genesis();
        let mut instances = HashMap::new();
        instances.insert(0, genesis);
        Self {
            instances,
            next_id: 1,
        }
    }

    /// Look up a ledger by ID.
    pub fn lookup(&self, id: LedgerID) -> Option<&Ledger> {
        self.instances.values().find(|l| l.id == id)
    }

    /// Accept transactions and create a new ledger.
    /// If an identical ledger already exists, returns the existing one.
    pub fn accept(
        &mut self,
        curr: &Ledger,
        txs: TxSetType,
        close_time_resolution: Duration,
        consensus_close_time: CloseTime,
    ) -> Ledger {
        let mut all_txs = curr.txs.clone();
        all_txs.extend(txs);

        let mut ancestors = curr.ancestors.clone();
        ancestors.push(curr.id);

        // Check if we already have this exact ledger (deduplication)
        let _dedup_key = (curr.id, all_txs.clone(), consensus_close_time);
        for existing in self.instances.values() {
            if existing.parent_id == curr.id
                && existing.txs == all_txs
                && existing.close_time == consensus_close_time
                && existing.seq == curr.seq + 1
            {
                return existing.clone();
            }
        }

        let new_ledger = Ledger {
            id: self.next_id,
            seq: curr.seq + 1,
            txs: all_txs,
            close_time_resolution,
            close_time: consensus_close_time,
            close_time_agree: consensus_close_time != 0,
            parent_id: curr.id,
            parent_close_time: curr.close_time,
            ancestors,
        };

        self.next_id += 1;
        let id = new_ledger.id;
        self.instances.insert(id as u64, new_ledger.clone());
        new_ledger
    }

    /// Accept a single transaction with default timing.
    pub fn accept_tx(&mut self, curr: &Ledger, tx: Tx) -> Ledger {
        let close_time = curr.close_time + 1;
        self.accept(
            curr,
            BTreeSet::from([tx]),
            curr.close_time_resolution,
            close_time,
        )
    }

    /// Count distinct branches among a set of ledgers.
    pub fn branches(ledgers: &BTreeSet<Ledger>) -> usize {
        let ledger_vec: Vec<&Ledger> = ledgers.iter().collect();
        let mut branch_count = ledger_vec.len();

        for i in 0..ledger_vec.len() {
            for j in (i + 1)..ledger_vec.len() {
                if ledger_vec[i].is_ancestor(ledger_vec[j])
                    || ledger_vec[j].is_ancestor(ledger_vec[i])
                {
                    branch_count -= 1;
                    break;
                }
            }
        }
        branch_count.max(1)
    }
}

// ─── LedgerHistoryHelper ─────────────────────────────────────────────────────

/// Helper for creating controlled ledger histories in tests.
///
pub struct LedgerHistoryHelper {
    pub oracle: LedgerOracle,
    next_tx: u32,
    ledgers: HashMap<String, Ledger>,
}

impl Default for LedgerHistoryHelper {
    fn default() -> Self {
        Self::new()
    }
}

impl LedgerHistoryHelper {
    pub fn new() -> Self {
        let mut ledgers = HashMap::new();
        ledgers.insert(String::new(), Ledger::genesis());
        Self {
            oracle: LedgerOracle::new(),
            next_tx: 0,
            ledgers,
        }
    }

    /// Get or create the ledger with the given string history.
    pub fn get(&mut self, s: &str) -> Ledger {
        if let Some(l) = self.ledgers.get(s) {
            return l.clone();
        }

        let parent = self.get(&s[..s.len() - 1]);
        self.next_tx += 1;
        let new_ledger = self.oracle.accept_tx(&parent, Tx::new(self.next_tx));
        self.ledgers.insert(s.to_string(), new_ledger.clone());
        new_ledger
    }
}

// ─── Proposal ────────────────────────────────────────────────────────────────

/// A consensus proposal — a position taken by a peer.
///
#[derive(Debug, Clone)]
pub struct Proposal {
    pub node_id: PeerID,
    pub prev_ledger: LedgerID,
    pub position: u64, // TxSet::ID
    pub close_time: CloseTime,
    pub time: SimTime,
    pub prop_num: u32,
}

impl Proposal {
    pub fn new(
        node_id: PeerID,
        prev_ledger: LedgerID,
        position: u64,
        close_time: CloseTime,
        time: SimTime,
    ) -> Self {
        Self {
            node_id,
            prev_ledger,
            position,
            close_time,
            time,
            prop_num: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tx_set_id_is_deterministic() {
        let s1 = TxSet::new(BTreeSet::from([Tx::new(1), Tx::new(2), Tx::new(3)]));
        let s2 = TxSet::new(BTreeSet::from([Tx::new(1), Tx::new(2), Tx::new(3)]));
        assert_eq!(s1.id(), s2.id());

        let s3 = TxSet::new(BTreeSet::from([Tx::new(1), Tx::new(2)]));
        assert_ne!(s1.id(), s3.id());
    }

    #[test]
    fn tx_set_compare_finds_differences() {
        let s1 = TxSet::new(BTreeSet::from([Tx::new(1), Tx::new(2), Tx::new(3)]));
        let s2 = TxSet::new(BTreeSet::from([Tx::new(2), Tx::new(3), Tx::new(4)]));
        let diff = s1.compare(&s2);
        assert_eq!(diff.get(&1), Some(&true)); // in s1 not s2
        assert_eq!(diff.get(&4), Some(&false)); // in s2 not s1
        assert_eq!(diff.get(&2), None); // in both
    }

    #[test]
    fn ledger_oracle_creates_unique_ids() {
        let mut oracle = LedgerOracle::new();
        let genesis = Ledger::genesis();
        let l1 = oracle.accept_tx(&genesis, Tx::new(1));
        let l2 = oracle.accept_tx(&genesis, Tx::new(2));
        assert_ne!(l1.id, l2.id);
        assert_eq!(l1.seq, 1);
        assert_eq!(l2.seq, 1);
    }

    #[test]
    fn ledger_ancestry_works() {
        let mut oracle = LedgerOracle::new();
        let genesis = Ledger::genesis();
        let l1 = oracle.accept_tx(&genesis, Tx::new(1));
        let l2 = oracle.accept_tx(&l1, Tx::new(2));

        assert!(l2.is_ancestor(&genesis));
        assert!(l2.is_ancestor(&l1));
        assert!(!l1.is_ancestor(&l2));
    }

    #[test]
    fn ledger_history_helper_creates_branches() {
        let mut hh = LedgerHistoryHelper::new();
        let a = hh.get("a");
        let ab = hh.get("ab");
        let ac = hh.get("ac");

        assert_eq!(a.seq, 1);
        assert_eq!(ab.seq, 2);
        assert_eq!(ac.seq, 2);
        assert_ne!(ab.id, ac.id); // different branches
        assert_eq!(ab.parent_id, a.id);
        assert_eq!(ac.parent_id, a.id);
    }

    #[test]
    fn mismatch_finds_divergence_point() {
        let mut hh = LedgerHistoryHelper::new();
        let ab = hh.get("ab");
        let ac = hh.get("ac");
        assert_eq!(mismatch(&ab, &ac), 2); // diverge at seq 2
    }

    #[test]
    fn branches_counts_distinct_histories() {
        let mut hh = LedgerHistoryHelper::new();
        let ab = hh.get("ab");
        let ac = hh.get("ac");
        let abd = hh.get("abd");

        // ab and abd are on same branch (abd descends from ab)
        // ac is a different branch
        let set = BTreeSet::from([ab.clone(), ac.clone()]);
        assert_eq!(LedgerOracle::branches(&set), 2);

        let set2 = BTreeSet::from([ab, abd]);
        assert_eq!(LedgerOracle::branches(&set2), 1);
    }
}
