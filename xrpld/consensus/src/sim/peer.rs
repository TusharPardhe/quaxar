//! Simulated consensus peer — implements the consensus adapter interface.
//!
//!
//! The Peer is the core of the simulation: it manages ledger state, transaction
//! sets, proposals, validations, and network messaging. It drives the consensus
//! algorithm by implementing the required callbacks.

use super::types::*;
use std::collections::{BTreeSet, HashMap};
use std::time::Duration;

/// Validation issued by a peer.
#[derive(Debug, Clone)]
pub struct Validation {
    pub ledger_id: LedgerID,
    pub ledger_seq: LedgerSeq,
    pub sign_time: SimTime,
    pub seen_time: SimTime,
    pub node_id: PeerID,
    pub full: bool,
    pub trusted: bool,
}

/// Processing delays for simulated peer operations.
#[derive(Debug, Clone)]
pub struct ProcessingDelays {
    pub ledger_accept: Duration,
    pub recv_validation: Duration,
}

impl Default for ProcessingDelays {
    fn default() -> Self {
        Self {
            ledger_accept: Duration::ZERO,
            recv_validation: Duration::ZERO,
        }
    }
}

/// A simulated peer in the consensus network.
///
/// Manages its own ledger state, open transactions, peer positions,
/// and drives consensus rounds.
pub struct Peer {
    pub id: PeerID,
    pub last_closed_ledger: Ledger,
    pub fully_validated_ledger: Ledger,
    pub open_txs: TxSetType,
    pub ledgers: HashMap<LedgerID, Ledger>,
    pub tx_sets: HashMap<u64, TxSet>,
    pub peer_positions: HashMap<LedgerID, Vec<Proposal>>,
    pub validations: Vec<Validation>,
    pub completed_ledgers: u32,
    pub target_ledgers: u32,
    pub run_as_validator: bool,
    pub delays: ProcessingDelays,
    pub quorum: usize,
    pub tx_injections: HashMap<LedgerSeq, Tx>,
    pub prev_proposers: usize,
    pub prev_round_time: Duration,
}

impl Peer {
    pub fn new(id: PeerID) -> Self {
        let genesis = Ledger::genesis();
        let mut ledgers = HashMap::new();
        ledgers.insert(genesis.id, genesis.clone());

        Self {
            id,
            last_closed_ledger: genesis.clone(),
            fully_validated_ledger: genesis,
            open_txs: BTreeSet::new(),
            ledgers,
            tx_sets: HashMap::new(),
            peer_positions: HashMap::new(),
            validations: Vec::new(),
            completed_ledgers: 0,
            target_ledgers: u32::MAX,
            run_as_validator: true,
            delays: ProcessingDelays::default(),
            quorum: 0,
            tx_injections: HashMap::new(),
            prev_proposers: 0,
            prev_round_time: Duration::ZERO,
        }
    }

    /// Submit a transaction to this peer's open set.
    pub fn submit(&mut self, tx: Tx) {
        self.open_txs.insert(tx);
    }

    /// Check if peer has open transactions.
    pub fn has_open_transactions(&self) -> bool {
        !self.open_txs.is_empty()
    }

    /// Acquire a ledger by ID (local lookup only in simplified sim).
    pub fn acquire_ledger(&self, ledger_id: LedgerID) -> Option<&Ledger> {
        self.ledgers.get(&ledger_id)
    }

    /// Acquire a tx set by ID.
    pub fn acquire_tx_set(&self, set_id: u64) -> Option<&TxSet> {
        self.tx_sets.get(&set_id)
    }

    /// Count trusted validations for a ledger.
    pub fn num_trusted_for_ledger(&self, ledger_id: LedgerID) -> usize {
        self.validations
            .iter()
            .filter(|v| v.ledger_id == ledger_id && v.trusted)
            .count()
    }

    /// Add a trusted validation.
    pub fn add_trusted_validation(&mut self, v: Validation) {
        let ledger_id = v.ledger_id;
        self.validations.push(v);
        self.check_fully_validated(ledger_id);
    }

    /// Check if a ledger can be deemed fully validated.
    pub fn check_fully_validated(&mut self, ledger_id: LedgerID) {
        let Some(ledger) = self.ledgers.get(&ledger_id).cloned() else {
            return;
        };
        if ledger.seq <= self.fully_validated_ledger.seq {
            return;
        }
        let count = self.num_trusted_for_ledger(ledger_id);
        if count >= self.quorum && ledger.is_ancestor(&self.fully_validated_ledger) {
            self.fully_validated_ledger = ledger;
        }
    }

    /// Close a consensus round: accept transactions, create new ledger.
    pub fn close_round(&mut self, oracle: &mut LedgerOracle, accepted_txs: TxSetType) {
        // Apply tx injections
        let mut final_txs = accepted_txs.clone();
        if let Some(&injected) = self.tx_injections.get(&self.last_closed_ledger.seq) {
            final_txs.insert(injected);
        }

        let new_ledger = oracle.accept(
            &self.last_closed_ledger,
            final_txs.clone(),
            self.last_closed_ledger.close_time_resolution,
            self.last_closed_ledger.close_time + 1,
        );

        self.ledgers.insert(new_ledger.id, new_ledger.clone());
        self.last_closed_ledger = new_ledger;

        // Remove accepted txs from open set
        self.open_txs.retain(|tx| !final_txs.contains(tx));

        self.completed_ledgers += 1;

        // Issue validation if running as validator
        if self.run_as_validator {
            let v = Validation {
                ledger_id: self.last_closed_ledger.id,
                ledger_seq: self.last_closed_ledger.seq,
                sign_time: Duration::ZERO,
                seen_time: Duration::ZERO,
                node_id: self.id,
                full: true,
                trusted: true,
            };
            self.validations.push(v);
        }
    }

    /// Get the preferred ledger based on validations.
    pub fn get_preferred_ledger(&self) -> LedgerID {
        // Simple: find the ledger with most validations at or above our seq
        let mut counts: HashMap<LedgerID, usize> = HashMap::new();
        for v in &self.validations {
            if v.trusted && v.ledger_seq >= self.fully_validated_ledger.seq {
                *counts.entry(v.ledger_id).or_default() += 1;
            }
        }
        counts
            .into_iter()
            .max_by_key(|&(_, count)| count)
            .map(|(id, _)| id)
            .unwrap_or(self.last_closed_ledger.id)
    }

    /// Handle receiving a proposal from another peer.
    pub fn handle_proposal(&mut self, p: Proposal) -> bool {
        let dest = self.peer_positions.entry(p.prev_ledger).or_default();
        if dest
            .iter()
            .any(|existing| existing.node_id == p.node_id && existing.prop_num == p.prop_num)
        {
            return false;
        }
        dest.push(p);
        true
    }

    /// Handle receiving a transaction.
    pub fn handle_tx(&mut self, tx: Tx) -> bool {
        if self.last_closed_ledger.txs.contains(&tx) {
            return false;
        }
        self.open_txs.insert(tx)
    }

    /// Handle receiving a tx set.
    pub fn handle_tx_set(&mut self, txs: TxSet) -> bool {
        let id = txs.id();
        if self.tx_sets.contains_key(&id) {
            return false;
        }
        self.tx_sets.insert(id, txs);
        true
    }

    /// Handle receiving a validation.
    pub fn handle_validation(&mut self, v: Validation) -> bool {
        if !v.trusted {
            return false;
        }
        self.add_trusted_validation(v);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_submit_and_close_round() {
        let mut oracle = LedgerOracle::new();
        let mut peer = Peer::new(1);

        peer.submit(Tx::new(10));
        peer.submit(Tx::new(20));
        assert!(peer.has_open_transactions());

        let accepted = peer.open_txs.clone();
        peer.close_round(&mut oracle, accepted);

        assert!(!peer.has_open_transactions());
        assert_eq!(peer.last_closed_ledger.seq, 1);
        assert!(peer.last_closed_ledger.txs.contains(&Tx::new(10)));
        assert!(peer.last_closed_ledger.txs.contains(&Tx::new(20)));
        assert_eq!(peer.completed_ledgers, 1);
    }

    #[test]
    fn peer_tx_injection_adds_non_consensus_tx() {
        let mut oracle = LedgerOracle::new();
        let mut peer = Peer::new(1);

        // Inject tx 42 at seq 0 (current LCL seq)
        peer.tx_injections.insert(0, Tx::new(42));
        peer.submit(Tx::new(10));

        let accepted = peer.open_txs.clone();
        peer.close_round(&mut oracle, accepted);

        assert!(peer.last_closed_ledger.txs.contains(&Tx::new(42)));
        assert!(peer.last_closed_ledger.txs.contains(&Tx::new(10)));
    }

    #[test]
    fn peer_handle_proposal_deduplicates() {
        let mut peer = Peer::new(1);
        let p = Proposal::new(2, 0, 123, 0, Duration::ZERO);

        assert!(peer.handle_proposal(p.clone()));
        assert!(!peer.handle_proposal(p)); // duplicate
    }

    #[test]
    fn peer_handle_tx_ignores_already_closed() {
        let mut oracle = LedgerOracle::new();
        let mut peer = Peer::new(1);

        peer.submit(Tx::new(5));
        peer.close_round(&mut oracle, BTreeSet::from([Tx::new(5)]));

        // Tx 5 is now in last closed ledger — should be rejected
        assert!(!peer.handle_tx(Tx::new(5)));
        // New tx should be accepted
        assert!(peer.handle_tx(Tx::new(6)));
    }

    #[test]
    fn peer_validation_tracks_fully_validated() {
        let mut oracle = LedgerOracle::new();
        let mut peer = Peer::new(1);
        peer.quorum = 2;

        // Close a round
        peer.submit(Tx::new(1));
        peer.close_round(&mut oracle, BTreeSet::from([Tx::new(1)]));
        let ledger_id = peer.last_closed_ledger.id;

        // Add validations from other peers
        peer.add_trusted_validation(Validation {
            ledger_id,
            ledger_seq: 1,
            sign_time: Duration::ZERO,
            seen_time: Duration::ZERO,
            node_id: 2,
            full: true,
            trusted: true,
        });

        // With quorum=2, we need 2 validations. Peer already validated itself.
        assert_eq!(peer.num_trusted_for_ledger(ledger_id), 2); // self + peer 2
        assert_eq!(peer.fully_validated_ledger.id, ledger_id);
    }
}
