//! A transaction discovered to be in dispute during consensus.
//!
//! Ported from rippled's `DisputedTx.h`. Created when a transaction is
//! found to be disputed (some peers include it, others don't); the object
//! persists only as long as the dispute exists. Undisputed transactions
//! have no corresponding `DisputedTx`.

use std::collections::BTreeMap;

use crate::algorithm::params::{AvalancheState, ConsensusParms, get_needed_weight};

/// A transaction discovered to be in dispute during consensus.
///
/// Type parameters:
/// - `Tx`: the transaction type.
/// - `TxId`: the transaction's unique identifier type.
/// - `NodeId`: the peer identifier type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisputedTx<Tx, TxId, NodeId> {
    /// Number of yes votes. `yays_`.
    yays: i32,
    /// Number of no votes. `nays_`.
    nays: i32,
    /// Our vote (true is yes). `ourVote_`.
    our_vote: bool,
    /// The transaction under dispute. `tx_`.
    tx: Tx,
    /// Map from peer to their vote. `votes_` (a `flat_map` in the
    /// reference; a `BTreeMap` gives the same deterministic iteration
    /// order property without needing a reservation hint).
    votes: BTreeMap<NodeId, bool>,
    /// Rounds we've gone without changing our vote. `currentVoteCounter_`.
    current_vote_counter: usize,
    /// Current avalanche acceptance-percentage phase. `avalancheState_`.
    avalanche_state: AvalancheState,
    /// How long we've been in the current avalanche phase.
    /// `avalancheCounter_`.
    avalanche_counter: usize,
    /// The transaction's id, cached so `id()` doesn't need `Tx: Clone`
    /// or a callback into the tx type. Not present in the reference
    /// (which calls `tx_.id()` directly) — kept here because callers on
    /// the Rust side pass the id in explicitly at construction, avoiding
    /// a separate `Tx::id()` trait requirement on this pure data type.
    tx_id: TxId,
}

impl<Tx, TxId, NodeId> DisputedTx<Tx, TxId, NodeId>
where
    Tx: Clone,
    TxId: Clone + Eq + Ord + ToString,
    NodeId: Clone + Eq + Ord + ToString,
{
    /// Construct a new dispute over `tx`, seeded with our own vote.
    /// `numPeers` (the reference's vote-map reservation hint) has no
    /// direct Rust equivalent and is omitted.
    pub fn new(tx: Tx, tx_id: TxId, our_vote: bool) -> Self {
        Self {
            yays: 0,
            nays: 0,
            our_vote,
            tx,
            votes: BTreeMap::new(),
            current_vote_counter: 0,
            avalanche_state: AvalancheState::Init,
            avalanche_counter: 0,
            tx_id,
        }
    }

    /// The unique id of the disputed transaction. `id()`.
    pub fn id(&self) -> &TxId {
        &self.tx_id
    }

    /// Our vote on whether the transaction should be included.
    /// `getOurVote()`.
    pub fn get_our_vote(&self) -> bool {
        self.our_vote
    }

    pub fn yays(&self) -> i32 { self.yays }
    pub fn nays(&self) -> i32 { self.nays }

    /// The disputed transaction. `tx()`.
    pub fn tx(&self) -> &Tx {
        &self.tx
    }

    /// Change our vote directly. `setOurVote()`.
    pub fn set_our_vote(&mut self, vote: bool) {
        self.our_vote = vote;
    }

    /// Change a peer's vote. Returns whether the peer's vote changed (a
    /// brand new vote counts as a change). `setVote()`.
    pub fn set_vote(&mut self, peer: NodeId, votes_yes: bool) -> bool {
        match self.votes.get_mut(&peer) {
            None => {
                self.votes.insert(peer, votes_yes);
                if votes_yes {
                    self.yays += 1;
                } else {
                    self.nays += 1;
                }
                true
            }
            Some(existing) if *existing != votes_yes => {
                if votes_yes {
                    self.nays -= 1;
                    self.yays += 1;
                } else {
                    self.yays -= 1;
                    self.nays += 1;
                }
                *existing = votes_yes;
                true
            }
            Some(_) => false,
        }
    }

    /// Remove a peer's vote. `unVote()`.
    pub fn un_vote(&mut self, peer: &NodeId) {
        if let Some(vote) = self.votes.remove(peer) {
            if vote {
                self.yays -= 1;
            } else {
                self.nays -= 1;
            }
        }
    }

    /// Whether we and our peers are "stalled" — unlikely to change our
    /// vote. `stalled()`.
    ///
    /// `peers_unchanged` mirrors the reference's `int peersUnchanged`
    /// (the number of rounds since any peer's proposal changed).
    pub fn stalled(&self, parms: &ConsensusParms, proposing: bool, peers_unchanged: usize) -> bool {
        let current_cutoff = parms
            .avalanche_cutoffs
            .get(&self.avalanche_state)
            .expect("current avalanche state must exist in the cutoff table");
        let next_cutoff = parms
            .avalanche_cutoffs
            .get(&current_cutoff.next)
            .expect("next avalanche state must exist in the cutoff table");

        // Not yet at the final avalanche state (or not there long enough) --
        // there's still room for change.
        if next_cutoff.consensus_time > current_cutoff.consensus_time
            || self.avalanche_counter < parms.av_min_rounds
        {
            return false;
        }

        // Haven't held this vote for the minimum number of rounds yet.
        if proposing && self.current_vote_counter < parms.av_min_rounds {
            return false;
        }

        // If we or peers have changed a vote recently, things could still
        // change. Only declare stalled if NEITHER side has changed in a
        // while (prevents a malicious peer from flip-flopping to block
        // consensus indefinitely).
        if peers_unchanged < parms.av_stalled_rounds
            && (proposing && self.current_vote_counter < parms.av_stalled_rounds)
        {
            return false;
        }

        // Percentage of nodes voting "yes" (including ourselves if
        // proposing and voting yes).
        let support = (self.yays + i32::from(proposing && self.our_vote)) * 100;
        let total = self.nays + self.yays + i32::from(proposing);
        if total == 0 {
            return false;
        }
        let weight = support / total;
        let min_pct = i32::try_from(parms.min_consensus_pct)
            .expect("min_consensus_pct must fit in i32");
        weight > min_pct || weight < (100 - min_pct)
    }

    /// Update our vote given progression of consensus. Returns whether our
    /// vote changed. `updateVote()`.
    ///
    /// `percent_time` is the percentage progress through the current round
    /// (e.g. 50, 90); `proposing` is whether we're proposing this round.
    pub fn update_vote(&mut self, percent_time: i32, proposing: bool, parms: &ConsensusParms) -> bool {
        if self.our_vote && self.nays == 0 {
            return false;
        }
        if !self.our_vote && self.yays == 0 {
            return false;
        }

        self.avalanche_counter += 1;
        let (required_pct, new_state) = get_needed_weight(
            parms,
            self.avalanche_state,
            percent_time,
            self.avalanche_counter,
            parms.av_min_rounds,
        );
        if let Some(new_state) = new_state {
            self.avalanche_state = new_state;
            self.avalanche_counter = 0;
        }

        let new_position = if proposing {
            // Give ourselves full weight.
            let weight =
                (self.yays * 100 + i32::from(self.our_vote) * 100) / (self.nays + self.yays + 1);
            weight
                > i32::try_from(required_pct).expect("required percent must fit in i32")
        } else {
            // Don't let us outweigh a proposing node -- just recognize
            // consensus.
            self.yays > self.nays
        };

        if new_position == self.our_vote {
            self.current_vote_counter += 1;
            return false;
        }

        self.current_vote_counter = 0;
        self.our_vote = new_position;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dispute(our_vote: bool) -> DisputedTx<u32, u32, u32> {
        DisputedTx::new(1, 100, our_vote)
    }

    #[test]
    fn set_vote_tracks_new_and_changed_votes() {
        let mut d = dispute(true);
        assert!(d.set_vote(1, true));
        assert_eq!(d.yays, 1);
        assert_eq!(d.nays, 0);

        // Same vote again: no change.
        assert!(!d.set_vote(1, true));
        assert_eq!(d.yays, 1);

        // Peer flips vote: counts as a change.
        assert!(d.set_vote(1, false));
        assert_eq!(d.yays, 0);
        assert_eq!(d.nays, 1);

        // A second, new peer voting no.
        assert!(d.set_vote(2, false));
        assert_eq!(d.nays, 2);
    }

    #[test]
    fn un_vote_removes_and_decrements() {
        let mut d = dispute(true);
        d.set_vote(1, true);
        d.set_vote(2, false);
        assert_eq!((d.yays, d.nays), (1, 1));

        d.un_vote(&1);
        assert_eq!((d.yays, d.nays), (0, 1));

        // Removing a peer that never voted is a no-op.
        d.un_vote(&99);
        assert_eq!((d.yays, d.nays), (0, 1));
    }

    #[test]
    fn update_vote_no_change_when_no_opposing_votes() {
        // our_vote=true and nays_==0 -> reference returns false immediately.
        let mut d = dispute(true);
        assert!(!d.update_vote(50, true, &ConsensusParms::default()));

        // our_vote=false and yays_==0 -> also returns false immediately.
        let mut d2 = dispute(false);
        assert!(!d2.update_vote(50, true, &ConsensusParms::default()));
    }

    #[test]
    fn update_vote_flips_when_weight_exceeds_required_threshold() {
        let parms = ConsensusParms::default();
        let mut d = dispute(false);
        // Need at least one nay to pass the initial guard for our_vote=false.
        d.set_vote(1, false);
        // Overwhelming yes votes so weight comfortably exceeds Init's 50%.
        for peer in 2..10 {
            d.set_vote(peer, true);
        }
        let changed = d.update_vote(0, true, &parms);
        assert!(changed);
        assert!(d.get_our_vote());
    }

    #[test]
    fn update_vote_non_proposing_just_follows_majority() {
        let parms = ConsensusParms::default();
        let mut d = dispute(false);
        d.set_vote(1, false); // satisfy initial guard
        d.set_vote(2, true);
        d.set_vote(3, true);
        // Non-proposing: newPosition = yays_ > nays_ = (2 > 1) = true.
        let changed = d.update_vote(0, false, &parms);
        assert!(changed);
        assert!(d.get_our_vote());
    }

    #[test]
    fn stalled_false_before_reaching_final_avalanche_state() {
        let parms = ConsensusParms::default();
        let d = dispute(true);
        // avalanche_state starts at Init, whose `next` (Mid) has a later
        // cutoff time than Init's own -- reference always returns false
        // here regardless of vote counts.
        assert!(!d.stalled(&parms, true, 100));
    }

    #[test]
    fn stalled_true_when_final_state_reached_and_lopsided() {
        let parms = ConsensusParms::default();
        let mut d = dispute(true);
        // Keep a persistent minority nay vote so update_vote's early-return
        // guard (`our_vote && nays_ == 0`) never trips, letting repeated
        // calls actually advance avalanche_counter/avalanche_state.
        d.set_vote(1, false);
        for peer in 2..20 {
            d.set_vote(peer, true);
        }
        for _ in 0..10 {
            d.update_vote(250, true, &parms);
        }
        assert_eq!(d.avalanche_state, AvalancheState::Stuck);
        assert!(d.avalanche_counter >= parms.av_min_rounds);
        assert!(d.stalled(&parms, true, parms.av_stalled_rounds));
    }
}
