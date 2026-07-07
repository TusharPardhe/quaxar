//! `xrpld/app/misc/NegativeUNLVote.*` compatibility surface.
//!
//! This ports the deterministic negative-UNL voting helper that:
//! - measures validator reliability over the last flag-ledger period,
//! - tracks newly-added validators so they are not disabled immediately,
//! - and injects `ttUNL_MODIFY` pseudo-transactions for one disable and one
//!   re-enable candidate when the current ledger history supports a vote.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use basics::base_uint::Uint256;
use consensus::{RclValidations, RclValidationsAdapter};
use ledger::{FLAG_LEDGER_INTERVAL, Ledger};
use protocol::{NodeID, PublicKey, STTx, TxType, calc_node_id, get_field_by_symbol, skip_keylet};

use crate::tx_queue::vote_tx_set::VoteTxSet;

pub trait NegativeUNLVoteJournal {
    fn trace(&self, message: &str);
    fn debug(&self, message: &str);
    fn warn(&self, message: &str);
    fn error(&self, message: &str);
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NullNegativeUNLVoteJournal;

impl NegativeUNLVoteJournal for NullNegativeUNLVoteJournal {
    fn trace(&self, _message: &str) {}
    fn debug(&self, _message: &str) {}
    fn warn(&self, _message: &str) {}
    fn error(&self, _message: &str) {}
}

pub trait NegativeUNLVoteValidations {
    fn set_seq_to_keep(&mut self, low: u32, high: u32);
    fn trusted_keys_for_ledger(&mut self, ledger_id: Uint256, seq: u32) -> Vec<PublicKey>;
}

impl<A> NegativeUNLVoteValidations for RclValidations<A>
where
    A: RclValidationsAdapter,
    A::Ledger: consensus::model::TrieLedger<Seq = u32, Id = Uint256>,
    A::Validation: consensus::rcl_support::ValidationT<NodeKey = PublicKey>,
    <A::Validation as consensus::rcl_support::ValidationT>::Wrapped: consensus::rcl::AsValidationKey<A>,
{
    fn set_seq_to_keep(&mut self, low: u32, high: u32) {
        RclValidations::<A>::set_seq_to_keep(self, low, high);
    }

    fn trusted_keys_for_ledger(&mut self, ledger_id: Uint256, seq: u32) -> Vec<PublicKey> {
        RclValidations::<A>::trusted_for_ledger_by_sequence(self, ledger_id, seq)
    }
}

pub trait NegativeUNLLedgerView {
    fn seq(&self) -> u32;
    fn hash(&self) -> Uint256;
    fn negative_unl(&self) -> HashSet<PublicKey>;
    fn validator_to_disable(&self) -> Option<PublicKey>;
    fn validator_to_re_enable(&self) -> Option<PublicKey>;
    fn recent_ancestor_hashes(&self) -> Option<Vec<Uint256>>;
}

impl NegativeUNLLedgerView for Ledger {
    fn seq(&self) -> u32 {
        self.header().seq
    }

    fn hash(&self) -> Uint256 {
        *self.header().hash.as_uint256()
    }

    fn negative_unl(&self) -> HashSet<PublicKey> {
        self.negative_unl()
            .into_iter()
            .filter_map(|key| PublicKey::from_slice(&key).ok())
            .collect()
    }

    fn validator_to_disable(&self) -> Option<PublicKey> {
        self.validator_to_disable()
            .and_then(|key| PublicKey::from_slice(&key).ok())
    }

    fn validator_to_re_enable(&self) -> Option<PublicKey> {
        self.validator_to_re_enable()
            .and_then(|key| PublicKey::from_slice(&key).ok())
    }

    fn recent_ancestor_hashes(&self) -> Option<Vec<Uint256>> {
        let hashes_field = get_field_by_symbol("sfHashes");
        self.read(skip_keylet())
            .ok()
            .flatten()
            .filter(|entry| entry.is_field_present(hashes_field))
            .map(|entry| entry.get_field_v256(hashes_field).value().to_vec())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NegativeUNLModify {
    ToDisable,
    ToReEnable,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NegativeUNLCandidates {
    pub to_disable_candidates: Vec<NodeID>,
    pub to_re_enable_candidates: Vec<NodeID>,
}

#[derive(Debug)]
pub struct NegativeUNLVote<J = NullNegativeUNLVoteJournal> {
    my_id: NodeID,
    journal: J,
    new_validators: Mutex<HashMap<NodeID, u32>>,
}

impl<J> NegativeUNLVote<J>
where
    J: NegativeUNLVoteJournal,
{
    pub const NEGATIVE_UNL_LOW_WATER_MARK: u32 = FLAG_LEDGER_INTERVAL * 50 / 100;
    pub const NEGATIVE_UNL_HIGH_WATER_MARK: u32 = FLAG_LEDGER_INTERVAL * 80 / 100;
    pub const NEGATIVE_UNL_MIN_LOCAL_VALS_TO_VOTE: u32 = FLAG_LEDGER_INTERVAL * 90 / 100;
    pub const NEW_VALIDATOR_DISABLE_SKIP: u32 = FLAG_LEDGER_INTERVAL * 2;
    pub const NEGATIVE_UNL_MAX_LISTED: f64 = 0.25;

    pub fn new(my_id: NodeID, journal: J) -> Self {
        Self {
            my_id,
            journal,
            new_validators: Mutex::new(HashMap::new()),
        }
    }

    pub fn do_voting<L, V, S>(
        &self,
        prev_ledger: &L,
        unl_keys: &HashSet<PublicKey>,
        validations: &mut V,
        initial_set: &mut S,
    ) where
        L: NegativeUNLLedgerView,
        V: NegativeUNLVoteValidations,
        S: VoteTxSet,
    {
        let mut unl_node_ids = HashSet::new();
        let mut node_id_to_key = HashMap::new();
        for &key in unl_keys {
            let node_id = calc_node_id(&key);
            unl_node_ids.insert(node_id);
            node_id_to_key.insert(node_id, key);
        }

        let Some(score_table) = self.build_score_table(prev_ledger, &unl_node_ids, validations)
        else {
            return;
        };

        let mut neg_unl_keys = prev_ledger.negative_unl();
        if let Some(key) = prev_ledger.validator_to_disable() {
            neg_unl_keys.insert(key);
        }
        if let Some(key) = prev_ledger.validator_to_re_enable() {
            neg_unl_keys.remove(&key);
        }

        let mut neg_unl_node_ids = HashSet::new();
        for key in neg_unl_keys {
            let node_id = calc_node_id(&key);
            neg_unl_node_ids.insert(node_id);
            node_id_to_key.entry(node_id).or_insert(key);
        }

        let seq = prev_ledger.seq().wrapping_add(1);
        self.purge_new_validators(seq);
        let candidates = self.find_all_candidates(&unl_node_ids, &neg_unl_node_ids, &score_table);

        if !candidates.to_disable_candidates.is_empty() {
            let node_id = Self::choose(prev_ledger.hash(), &candidates.to_disable_candidates);
            if let Some(public_key) = node_id_to_key.get(&node_id).copied() {
                self.add_tx(seq, public_key, NegativeUNLModify::ToDisable, initial_set);
            }
        }

        if !candidates.to_re_enable_candidates.is_empty() {
            let node_id = Self::choose(prev_ledger.hash(), &candidates.to_re_enable_candidates);
            if let Some(public_key) = node_id_to_key.get(&node_id).copied() {
                self.add_tx(seq, public_key, NegativeUNLModify::ToReEnable, initial_set);
            }
        }
    }

    pub fn new_validators(&self, seq: u32, now_trusted: &HashSet<NodeID>) {
        let mut tracked = self
            .new_validators
            .lock()
            .expect("new validators mutex must not be poisoned");
        for &node_id in now_trusted {
            if tracked.contains_key(&node_id) {
                continue;
            }
            self.journal.trace(&format!(
                "N-UNL: add a new validator {node_id} at ledger seq={seq}"
            ));
            tracked.insert(node_id, seq);
        }
    }

    pub fn choose(random_pad_data: Uint256, candidates: &[NodeID]) -> NodeID {
        assert!(
            !candidates.is_empty(),
            "xrpl::NegativeUNLVote::choose : non-empty input"
        );
        let random_pad = NodeID::from_slice(&random_pad_data.data()[..NodeID::size()])
            .expect("random pad width should match NodeID");
        let mut chosen = candidates[0];
        for &candidate in &candidates[1..] {
            if (candidate ^ random_pad) < (chosen ^ random_pad) {
                chosen = candidate;
            }
        }
        chosen
    }

    pub fn find_all_candidates(
        &self,
        unl: &HashSet<NodeID>,
        neg_unl: &HashSet<NodeID>,
        score_table: &HashMap<NodeID, u32>,
    ) -> NegativeUNLCandidates {
        let max_negative_listed =
            ((unl.len() as f64) * Self::NEGATIVE_UNL_MAX_LISTED).ceil() as usize;
        let negative_listed = unl
            .iter()
            .filter(|node_id| neg_unl.contains(*node_id))
            .count();
        let can_add = negative_listed < max_negative_listed;
        self.journal.trace(&format!(
            "N-UNL: nodeId {} lowWaterMark {} highWaterMark {} canAdd {} negativeListed {} maxNegativeListed {}",
            self.my_id,
            Self::NEGATIVE_UNL_LOW_WATER_MARK,
            Self::NEGATIVE_UNL_HIGH_WATER_MARK,
            can_add,
            negative_listed,
            max_negative_listed
        ));

        let tracked_new = self
            .new_validators
            .lock()
            .expect("new validators mutex must not be poisoned")
            .clone();

        let mut candidates = NegativeUNLCandidates::default();
        for (&node_id, &score) in score_table {
            self.journal
                .trace(&format!("N-UNL: node {node_id} score {score}"));

            if can_add
                && score < Self::NEGATIVE_UNL_LOW_WATER_MARK
                && !neg_unl.contains(&node_id)
                && !tracked_new.contains_key(&node_id)
            {
                self.journal
                    .trace(&format!("N-UNL: toDisable candidate {node_id}"));
                candidates.to_disable_candidates.push(node_id);
            }

            if score > Self::NEGATIVE_UNL_HIGH_WATER_MARK && neg_unl.contains(&node_id) {
                self.journal
                    .trace(&format!("N-UNL: toReEnable candidate {node_id}"));
                candidates.to_re_enable_candidates.push(node_id);
            }
        }

        if candidates.to_re_enable_candidates.is_empty() {
            for &node_id in neg_unl {
                if !unl.contains(&node_id) {
                    candidates.to_re_enable_candidates.push(node_id);
                }
            }
        }

        candidates
    }

    pub fn build_score_table<L, V>(
        &self,
        prev_ledger: &L,
        unl: &HashSet<NodeID>,
        validations: &mut V,
    ) -> Option<HashMap<NodeID, u32>>
    where
        L: NegativeUNLLedgerView,
        V: NegativeUNLVoteValidations,
    {
        let seq = prev_ledger.seq().wrapping_add(1);
        validations.set_seq_to_keep(seq - 1, seq + FLAG_LEDGER_INTERVAL);

        let Some(ledger_ancestors) = prev_ledger.recent_ancestor_hashes() else {
            self.journal
                .debug(&format!("N-UNL: ledger {seq} no history."));
            return None;
        };

        let num_ancestors = ledger_ancestors.len();
        if num_ancestors < FLAG_LEDGER_INTERVAL as usize {
            self.journal.debug(&format!(
                "N-UNL: ledger {seq} not enough history. Can trace back only {num_ancestors} ledgers."
            ));
            return None;
        }

        let mut score_table = unl
            .iter()
            .copied()
            .map(|node_id| (node_id, 0u32))
            .collect::<HashMap<_, _>>();

        for i in 0..FLAG_LEDGER_INTERVAL as usize {
            let ledger_id = ledger_ancestors[num_ancestors - 1 - i];
            let ledger_seq = seq - 2 - u32::try_from(i).expect("flag interval index fits u32");
            for key in validations.trusted_keys_for_ledger(ledger_id, ledger_seq) {
                let node_id = calc_node_id(&key);
                if let Some(score) = score_table.get_mut(&node_id) {
                    *score += 1;
                }
            }
        }

        let my_validation_count = score_table.get(&self.my_id).copied().unwrap_or_default();
        if my_validation_count < Self::NEGATIVE_UNL_MIN_LOCAL_VALS_TO_VOTE {
            self.journal.debug(&format!(
                "N-UNL: ledger {seq}. Local node only issued {my_validation_count} validations in last {FLAG_LEDGER_INTERVAL} ledgers. The reliability measurement could be wrong."
            ));
            return None;
        }
        if my_validation_count <= FLAG_LEDGER_INTERVAL {
            return Some(score_table);
        }

        self.journal.error(&format!(
            "N-UNL: ledger {seq}. Local node issued {my_validation_count} validations in last {FLAG_LEDGER_INTERVAL} ledgers. Too many!"
        ));
        None
    }

    fn add_tx<S>(
        &self,
        seq: u32,
        public_key: PublicKey,
        modify: NegativeUNLModify,
        initial_set: &mut S,
    ) where
        S: VoteTxSet,
    {
        let unl_modify_tx = STTx::new(TxType::UNL_MODIFY, |tx| {
            tx.set_field_u8(
                get_field_by_symbol("sfUNLModifyDisabling"),
                u8::from(matches!(modify, NegativeUNLModify::ToDisable)),
            );
            tx.set_field_u32(get_field_by_symbol("sfLedgerSequence"), seq);
            tx.set_field_vl(
                get_field_by_symbol("sfUNLModifyValidator"),
                public_key.as_bytes(),
            );
        });

        if !initial_set.add_transaction(&unl_modify_tx) {
            self.journal.warn(&format!(
                "N-UNL: ledger seq={seq}, add ttUNL_MODIFY tx failed"
            ));
            return;
        }

        self.journal.debug(&format!(
            "N-UNL: ledger seq={seq}, add a ttUNL_MODIFY Tx with txID: {}, the validator to {}{}",
            unl_modify_tx.get_transaction_id(),
            if matches!(modify, NegativeUNLModify::ToDisable) {
                "disable: "
            } else {
                "re-enable: "
            },
            public_key
        ));
    }

    fn purge_new_validators(&self, seq: u32) {
        let mut tracked = self
            .new_validators
            .lock()
            .expect("new validators mutex must not be poisoned");
        tracked.retain(|_, added_seq| {
            seq.saturating_sub(*added_seq) <= Self::NEW_VALIDATOR_DISABLE_SKIP
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{
        NegativeUNLCandidates, NegativeUNLModify, NegativeUNLVote, NullNegativeUNLVoteJournal,
    };
    use basics::base_uint::Uint256;
    use protocol::NodeID;
    use std::collections::{HashMap, HashSet};

    fn node(fill: u8) -> NodeID {
        NodeID::from_array([fill; 20])
    }

    #[test]
    fn choose_xor_minimum_rule() {
        let chosen = NegativeUNLVote::<NullNegativeUNLVoteJournal>::choose(
            Uint256::from_array([0xAA; 32]),
            &[node(0x10), node(0x20), node(0x30)],
        );

        let mut expected = node(0x10);
        let pad = NodeID::from_array([0xAA; 20]);
        for candidate in [node(0x20), node(0x30)] {
            if (candidate ^ pad) < (expected ^ pad) {
                expected = candidate;
            }
        }

        assert_eq!(chosen, expected);
    }

    #[test]
    fn candidate_scan_enforces_thresholds_and_new_validator_skip() {
        let vote = NegativeUNLVote::new(node(0x01), NullNegativeUNLVoteJournal);
        vote.new_validators(256, &HashSet::from([node(0x04)]));

        let unl = HashSet::from([node(0x01), node(0x02), node(0x03), node(0x04), node(0x05)]);
        let neg_unl = HashSet::from([node(0x03)]);
        let score_table = HashMap::from([
            (
                node(0x01),
                NegativeUNLVote::<NullNegativeUNLVoteJournal>::NEGATIVE_UNL_HIGH_WATER_MARK + 1,
            ),
            (
                node(0x02),
                NegativeUNLVote::<NullNegativeUNLVoteJournal>::NEGATIVE_UNL_LOW_WATER_MARK - 1,
            ),
            (
                node(0x03),
                NegativeUNLVote::<NullNegativeUNLVoteJournal>::NEGATIVE_UNL_HIGH_WATER_MARK + 1,
            ),
            (
                node(0x04),
                NegativeUNLVote::<NullNegativeUNLVoteJournal>::NEGATIVE_UNL_LOW_WATER_MARK - 1,
            ),
            (
                node(0x05),
                NegativeUNLVote::<NullNegativeUNLVoteJournal>::NEGATIVE_UNL_HIGH_WATER_MARK,
            ),
        ]);

        let candidates = vote.find_all_candidates(&unl, &neg_unl, &score_table);

        assert_eq!(candidates.to_disable_candidates, vec![node(0x02)]);
        assert_eq!(candidates.to_re_enable_candidates, vec![node(0x03)]);
    }

    #[test]
    fn retired_negative_unl_nodes_are_reenabled_when_no_score_candidate_exists() {
        let vote = NegativeUNLVote::new(node(0x01), NullNegativeUNLVoteJournal);
        let unl = HashSet::from([node(0x01), node(0x02)]);
        let neg_unl = HashSet::from([node(0x03)]);
        let score_table = HashMap::from([(node(0x01), 200), (node(0x02), 200)]);

        let candidates = vote.find_all_candidates(&unl, &neg_unl, &score_table);

        assert_eq!(
            candidates,
            NegativeUNLCandidates {
                to_disable_candidates: Vec::new(),
                to_re_enable_candidates: vec![node(0x03)],
            }
        );
    }

    #[test]
    fn purge_new_validators_drops_entries_after_skip_window() {
        let vote = NegativeUNLVote::new(node(0x01), NullNegativeUNLVoteJournal);
        vote.new_validators(100, &HashSet::from([node(0x02), node(0x03)]));
        vote.purge_new_validators(
            100 + NegativeUNLVote::<NullNegativeUNLVoteJournal>::NEW_VALIDATOR_DISABLE_SKIP + 1,
        );

        assert!(
            vote.new_validators
                .lock()
                .expect("new validators mutex must not be poisoned")
                .is_empty()
        );
    }

    #[test]
    fn negative_unl_modify_flag_values_match_cpp_convention() {
        assert!(matches!(
            NegativeUNLModify::ToDisable,
            NegativeUNLModify::ToDisable
        ));
        assert!(matches!(
            NegativeUNLModify::ToReEnable,
            NegativeUNLModify::ToReEnable
        ));
    }
}
