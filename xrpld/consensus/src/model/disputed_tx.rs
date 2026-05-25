use crate::params::{AvalancheState, ConsensusParms, get_needed_weight};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use tracing;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisputedTx<Tx, TxId, NodeId> {
    yays: i32,
    nays: i32,
    our_vote: bool,
    tx: Tx,
    votes: BTreeMap<NodeId, bool>,
    current_vote_counter: usize,
    avalanche_state: AvalancheState,
    avalanche_counter: usize,
    tx_id: TxId,
}

impl<Tx, TxId, NodeId> DisputedTx<Tx, TxId, NodeId>
where
    Tx: Clone,
    TxId: Clone + ToString,
    NodeId: Clone + Ord + ToString,
{
    pub fn new(tx: Tx, tx_id: TxId, our_vote: bool) -> Self {
        tracing::debug!(target: "consensus", tx_hash = %tx_id.to_string(), our_vote, "New disputed transaction");
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

    pub fn id(&self) -> &TxId {
        &self.tx_id
    }

    pub fn tx(&self) -> &Tx {
        &self.tx
    }

    pub fn get_our_vote(&self) -> bool {
        self.our_vote
    }

    pub fn set_vote(&mut self, peer: NodeId, votes_yes: bool) -> bool {
        match self.votes.get_mut(&peer) {
            None => {
                tracing::debug!(target: "consensus", peer = %peer.to_string(), tx_hash = %self.tx_id.to_string(), votes_yes, "Peer vote recorded");
                self.votes.insert(peer, votes_yes);
                if votes_yes {
                    self.yays += 1;
                } else {
                    self.nays += 1;
                }
                true
            }
            Some(existing) if *existing != votes_yes => {
                tracing::debug!(target: "consensus", peer = %peer.to_string(), tx_hash = %self.tx_id.to_string(), votes_yes, "Peer vote changed");
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

    pub fn unvote(&mut self, peer: &NodeId) {
        if let Some(vote) = self.votes.remove(peer) {
            tracing::debug!(target: "consensus", peer = %peer.to_string(), tx_hash = %self.tx_id.to_string(), "Peer vote removed");
            if vote {
                self.yays -= 1;
            } else {
                self.nays -= 1;
            }
        }
    }

    pub fn update_vote(
        &mut self,
        percent_time: i32,
        proposing: bool,
        parms: &ConsensusParms,
    ) -> bool {
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
            let weight = (self.yays * 100 + if self.our_vote { 100 } else { 0 })
                / (self.nays + self.yays + 1);
            weight > i32::try_from(required_pct).expect("required percent must fit in i32")
        } else {
            self.yays > self.nays
        };

        if new_position == self.our_vote {
            self.current_vote_counter += 1;
            return false;
        }

        self.current_vote_counter = 0;
        self.our_vote = new_position;
        let dispute_result = if new_position { "include" } else { "exclude" };
        tracing::debug!(target: "consensus", tx_hash = %self.tx_id.to_string(), dispute_result, "Transaction dispute resolved");
        true
    }

    pub fn stalled(&self, parms: &ConsensusParms, proposing: bool, peers_unchanged: usize) -> bool {
        let current_cutoff = parms
            .avalanche_cutoffs
            .get(&self.avalanche_state)
            .expect("current avalanche state must exist");
        let next_cutoff = parms
            .avalanche_cutoffs
            .get(&current_cutoff.next)
            .expect("next avalanche state must exist");

        if next_cutoff.consensus_time > current_cutoff.consensus_time
            || self.avalanche_counter < parms.av_min_rounds
        {
            return false;
        }
        if proposing && self.current_vote_counter < parms.av_min_rounds {
            return false;
        }
        if peers_unchanged < parms.av_stalled_rounds
            && proposing
            && self.current_vote_counter < parms.av_stalled_rounds
        {
            return false;
        }

        let support = (self.yays + if proposing && self.our_vote { 1 } else { 0 }) * 100;
        let total = self.nays + self.yays + if proposing { 1 } else { 0 };
        if total == 0 {
            return false;
        }

        let weight = support / total;
        weight > i32::try_from(parms.min_consensus_pct).expect("consensus pct must fit in i32")
            || weight
                < 100
                    - i32::try_from(parms.min_consensus_pct).expect("consensus pct must fit in i32")
    }

    pub fn get_json(&self) -> Value
    where
        NodeId: ToString,
    {
        let votes = self
            .votes
            .iter()
            .map(|(node_id, vote)| (node_id.to_string(), json!(vote)))
            .collect::<serde_json::Map<String, Value>>();

        let mut object = serde_json::Map::new();
        object.insert("yays".to_owned(), json!(self.yays));
        object.insert("nays".to_owned(), json!(self.nays));
        object.insert("our_vote".to_owned(), json!(self.our_vote));
        if !votes.is_empty() {
            object.insert("votes".to_owned(), Value::Object(votes));
        }
        Value::Object(object)
    }
}
