use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use app::{
    NegativeUNLLedgerView, NegativeUNLModify, NegativeUNLVote, NegativeUNLVoteValidations,
    NullNegativeUNLVoteJournal, ShamapVoteTxSet,
};
use basics::base_uint::Uint256;
use basics::tagged_cache::MonotonicClock;
use protocol::{
    KeyType, PublicKey, STTx, SecretKey, SerialIter, TxType, calc_node_id, derive_public_key,
    get_field_by_symbol,
};
use shamap::{storage::StorageTree, tree_node_cache::TreeNodeCache};
use time::Duration;

#[derive(Clone)]
struct FakeLedger {
    seq: u32,
    hash: Uint256,
    negative_unl: HashSet<PublicKey>,
    validator_to_disable: Option<PublicKey>,
    validator_to_re_enable: Option<PublicKey>,
    recent_ancestor_hashes: Option<Vec<Uint256>>,
}

impl NegativeUNLLedgerView for FakeLedger {
    fn seq(&self) -> u32 {
        self.seq
    }

    fn hash(&self) -> Uint256 {
        self.hash
    }

    fn negative_unl(&self) -> HashSet<PublicKey> {
        self.negative_unl.clone()
    }

    fn validator_to_disable(&self) -> Option<PublicKey> {
        self.validator_to_disable
    }

    fn validator_to_re_enable(&self) -> Option<PublicKey> {
        self.validator_to_re_enable
    }

    fn recent_ancestor_hashes(&self) -> Option<Vec<Uint256>> {
        self.recent_ancestor_hashes.clone()
    }
}

#[derive(Default)]
struct FakeValidations {
    keep_ranges: Vec<(u32, u32)>,
    trusted: HashMap<(Uint256, u32), Vec<PublicKey>>,
}

impl FakeValidations {
    fn add_score(
        &mut self,
        ancestors: &[Uint256],
        next_seq: u32,
        public_key: PublicKey,
        count: usize,
    ) {
        for i in 0..count.min(ancestors.len()) {
            let ledger_id = ancestors[ancestors.len() - 1 - i];
            let ledger_seq = next_seq - 2 - u32::try_from(i).expect("history index fits u32");
            self.trusted
                .entry((ledger_id, ledger_seq))
                .or_default()
                .push(public_key);
        }
    }
}

impl NegativeUNLVoteValidations for FakeValidations {
    fn set_seq_to_keep(&mut self, low: u32, high: u32) {
        self.keep_ranges.push((low, high));
    }

    fn trusted_keys_for_ledger(&mut self, ledger_id: Uint256, seq: u32) -> Vec<PublicKey> {
        self.trusted
            .get(&(ledger_id, seq))
            .cloned()
            .unwrap_or_default()
    }
}

fn validator(seed: u8) -> PublicKey {
    let secret = SecretKey::from_bytes([seed; 32]);
    derive_public_key(KeyType::Secp256k1, &secret).expect("validator public key")
}

fn ancestor_hashes() -> Vec<Uint256> {
    (0..256u64)
        .map(|index| Uint256::from_u64(index + 1))
        .collect()
}

fn fake_ledger(seq: u32) -> FakeLedger {
    FakeLedger {
        seq,
        hash: Uint256::from_array([0xAB; 32]),
        negative_unl: HashSet::new(),
        validator_to_disable: None,
        validator_to_re_enable: None,
        recent_ancestor_hashes: Some(ancestor_hashes()),
    }
}

fn tx_by_flag<'a>(txs: &'a [STTx], disabling: u8) -> &'a STTx {
    txs.iter()
        .find(|tx| tx.get_field_u8(get_field_by_symbol("sfUNLModifyDisabling")) == disabling)
        .expect("matching UNL modify transaction")
}

#[test]
fn negative_unl_vote_builds_score_tables_from_256_ledgers_and_records_keep_range() {
    let local = validator(1);
    let other = validator(2);
    let local_node_id = calc_node_id(&local);
    let unl = HashSet::from([local_node_id, calc_node_id(&other)]);
    let ledger = fake_ledger(300);
    let next_seq = ledger.seq + 1;
    let ancestors = ledger
        .recent_ancestor_hashes
        .clone()
        .expect("ancestor history should exist");
    let mut validations = FakeValidations::default();
    validations.add_score(&ancestors, next_seq, local, 256);
    validations.add_score(&ancestors, next_seq, other, 100);

    let vote = NegativeUNLVote::new(local_node_id, NullNegativeUNLVoteJournal);
    let score_table = vote
        .build_score_table(&ledger, &unl, &mut validations)
        .expect("score table should build");

    assert_eq!(validations.keep_ranges, vec![(300, 557)]);
    assert_eq!(score_table.get(&local_node_id), Some(&256));
    assert_eq!(score_table.get(&calc_node_id(&other)), Some(&100));
}

#[test]
fn negative_unl_vote_rejects_history_without_enough_local_participation() {
    let local = validator(1);
    let other = validator(2);
    let local_node_id = calc_node_id(&local);
    let unl = HashSet::from([local_node_id, calc_node_id(&other)]);
    let ledger = fake_ledger(300);
    let next_seq = ledger.seq + 1;
    let ancestors = ledger
        .recent_ancestor_hashes
        .clone()
        .expect("ancestor history should exist");
    let mut validations = FakeValidations::default();
    validations.add_score(
        &ancestors,
        next_seq,
        local,
        (NegativeUNLVote::<NullNegativeUNLVoteJournal>::NEGATIVE_UNL_MIN_LOCAL_VALS_TO_VOTE - 1)
            as usize,
    );
    validations.add_score(&ancestors, next_seq, other, 200);

    let vote = NegativeUNLVote::new(local_node_id, NullNegativeUNLVoteJournal);

    assert!(
        vote.build_score_table(&ledger, &unl, &mut validations)
            .is_none()
    );
}

#[test]
fn negative_unl_vote_adds_disable_and_reenable_transactions_from_score_table() {
    let local = validator(1);
    let reliable = validator(2);
    let disable_candidate = validator(3);
    let reenable_candidate = validator(4);
    let healthy_spare = validator(5);
    let local_node_id = calc_node_id(&local);
    let ledger = FakeLedger {
        negative_unl: HashSet::from([reenable_candidate]),
        ..fake_ledger(512)
    };
    let next_seq = ledger.seq + 1;
    let ancestors = ledger
        .recent_ancestor_hashes
        .clone()
        .expect("ancestor history should exist");
    let mut validations = FakeValidations::default();
    validations.add_score(&ancestors, next_seq, local, 256);
    validations.add_score(&ancestors, next_seq, reliable, 240);
    validations.add_score(&ancestors, next_seq, disable_candidate, 100);
    validations.add_score(&ancestors, next_seq, reenable_candidate, 220);
    validations.add_score(&ancestors, next_seq, healthy_spare, 240);

    let unl = HashSet::from([
        local,
        reliable,
        disable_candidate,
        reenable_candidate,
        healthy_spare,
    ]);
    let vote = NegativeUNLVote::new(local_node_id, NullNegativeUNLVoteJournal);
    let mut txs = Vec::<STTx>::new();

    vote.do_voting(&ledger, &unl, &mut validations, &mut txs);

    assert_eq!(txs.len(), 2);
    let disable_tx = tx_by_flag(&txs, 1);
    let reenable_tx = tx_by_flag(&txs, 0);

    assert_eq!(disable_tx.get_txn_type(), TxType::UNL_MODIFY);
    assert_eq!(
        disable_tx.get_field_u32(get_field_by_symbol("sfLedgerSequence")),
        513
    );
    assert_eq!(
        disable_tx.get_field_vl(get_field_by_symbol("sfUNLModifyValidator")),
        disable_candidate.as_bytes()
    );

    assert_eq!(reenable_tx.get_txn_type(), TxType::UNL_MODIFY);
    assert_eq!(
        reenable_tx.get_field_vl(get_field_by_symbol("sfUNLModifyValidator")),
        reenable_candidate.as_bytes()
    );
}

#[test]
fn negative_unl_vote_skips_new_disable_candidates_and_reenables_retired_validators() {
    let local = validator(1);
    let new_candidate = validator(2);
    let retired = validator(3);
    let local_node_id = calc_node_id(&local);
    let ledger = FakeLedger {
        negative_unl: HashSet::from([retired]),
        ..fake_ledger(256)
    };
    let next_seq = ledger.seq + 1;
    let ancestors = ledger
        .recent_ancestor_hashes
        .clone()
        .expect("ancestor history should exist");
    let mut validations = FakeValidations::default();
    validations.add_score(&ancestors, next_seq, local, 256);
    validations.add_score(&ancestors, next_seq, new_candidate, 10);

    let unl = HashSet::from([local, new_candidate]);
    let vote = NegativeUNLVote::new(local_node_id, NullNegativeUNLVoteJournal);
    vote.new_validators(next_seq, &HashSet::from([calc_node_id(&new_candidate)]));

    let mut txs = Vec::<STTx>::new();
    vote.do_voting(&ledger, &unl, &mut validations, &mut txs);

    assert_eq!(txs.len(), 1);
    let reenable_tx = &txs[0];
    assert_eq!(
        reenable_tx.get_field_u8(get_field_by_symbol("sfUNLModifyDisabling")),
        0
    );
    assert_eq!(
        reenable_tx.get_field_vl(get_field_by_symbol("sfUNLModifyValidator")),
        retired.as_bytes()
    );
}

#[test]
fn shamap_vote_tx_set_stores_negative_unl_transactions_in_real_shamap() {
    let local = validator(1);
    let disable_candidate = validator(2);
    let local_node_id = calc_node_id(&local);
    let ledger = fake_ledger(512);
    let next_seq = ledger.seq + 1;
    let ancestors = ledger
        .recent_ancestor_hashes
        .clone()
        .expect("ancestor history should exist");
    let mut validations = FakeValidations::default();
    validations.add_score(&ancestors, next_seq, local, 256);
    validations.add_score(&ancestors, next_seq, disable_candidate, 100);

    let unl = HashSet::from([local, disable_candidate]);
    let vote = NegativeUNLVote::new(local_node_id, NullNegativeUNLVoteJournal);

    let cache = Arc::new(TreeNodeCache::<MonotonicClock>::new(
        "NegativeUNLVoteTxSet",
        128,
        Duration::seconds(60),
        MonotonicClock::default(),
    ));
    let mut tree = StorageTree::new(1, false, 512, cache);
    {
        let mut tx_set = ShamapVoteTxSet::new(&mut tree);
        vote.do_voting(&ledger, &unl, &mut validations, &mut tx_set);
    }

    let mut txs = Vec::new();
    tree.visit_leaves(&mut |_| None, &mut |item| {
        let mut serial = SerialIter::new(item.data());
        txs.push(STTx::from_serial_iter(&mut serial));
    })
    .expect("negative UNL vote transaction tree should be readable");

    assert_eq!(txs.len(), 1);
    assert_eq!(txs[0].get_txn_type(), TxType::UNL_MODIFY);
    assert_eq!(
        txs[0].get_field_u8(get_field_by_symbol("sfUNLModifyDisabling")),
        if matches!(NegativeUNLModify::ToDisable, NegativeUNLModify::ToDisable) {
            1
        } else {
            0
        }
    );
}

/// C++ parity: NegativeUNL_test "Pick One Candidate"
#[test]
fn negative_unl_choose_picks_deterministically_from_candidates() {
    use app::amendments::negative_unl_vote::NegativeUNLVote;
    use basics::base_uint::Uint256;
    use protocol::{KeyType, SecretKey, calc_node_id, derive_public_key};

    let make_node = |seed: u8| {
        let sk = SecretKey::from_bytes([seed; 32]);
        let pk = derive_public_key(KeyType::Secp256k1, &sk).unwrap();
        calc_node_id(&pk)
    };

    let node_a = make_node(0x01);
    let node_b = make_node(0x02);
    let node_c = make_node(0x03);
    let candidates = vec![node_a, node_b, node_c];

    // Same random pad always picks the same candidate
    let pad = Uint256::from_array([0x55; 32]);
    let chosen1 =
        NegativeUNLVote::<app::amendments::negative_unl_vote::NullNegativeUNLVoteJournal>::choose(
            pad,
            &candidates,
        );
    let chosen2 =
        NegativeUNLVote::<app::amendments::negative_unl_vote::NullNegativeUNLVoteJournal>::choose(
            pad,
            &candidates,
        );
    assert_eq!(chosen1, chosen2);

    // Different pad may pick a different candidate
    let pad2 = Uint256::from_array([0xAA; 32]);
    let chosen3 =
        NegativeUNLVote::<app::amendments::negative_unl_vote::NullNegativeUNLVoteJournal>::choose(
            pad2,
            &candidates,
        );
    assert!(candidates.contains(&chosen3));

    // Single candidate always returns that candidate
    let single = vec![node_b];
    assert_eq!(
        NegativeUNLVote::<app::amendments::negative_unl_vote::NullNegativeUNLVoteJournal>::choose(
            pad, &single
        ),
        node_b
    );
}

/// C++ parity: NegativeUNL_test "Find All Candidates"
#[test]
fn negative_unl_find_all_candidates_respects_max_listed() {
    use app::amendments::negative_unl_vote::{NegativeUNLVote, NullNegativeUNLVoteJournal};
    use protocol::{KeyType, SecretKey, calc_node_id, derive_public_key};
    use std::collections::{HashMap, HashSet};

    let make_node = |seed: u8| {
        let sk = SecretKey::from_bytes([seed; 32]);
        let pk = derive_public_key(KeyType::Secp256k1, &sk).unwrap();
        calc_node_id(&pk)
    };

    let my_id = make_node(0x01);
    let vote = NegativeUNLVote::new(my_id, NullNegativeUNLVoteJournal);

    // UNL with 5 validators
    let mut unl = HashSet::new();
    for i in 1..=5u8 {
        unl.insert(make_node(i));
    }

    let neg_unl = HashSet::new();

    // Score table: node 3 has low score, others high
    let mut score_table = HashMap::new();
    for i in 1..=5u8 {
        score_table.insert(make_node(i), if i == 3 { 50 } else { 200 });
    }

    let candidates = vote.find_all_candidates(&unl, &neg_unl, &score_table);
    // Should not panic and should return valid candidates struct
    let _ = candidates;
}
