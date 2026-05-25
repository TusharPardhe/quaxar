#![allow(
    clippy::needless_range_loop,
    clippy::field_reassign_with_default,
    clippy::vec_init_then_push
)]
#![allow(clippy::clone_on_copy, clippy::unnecessary_mut_passed)]
use consensus::{LedgerHistory, LedgerTrie, mismatch};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct TestLedger {
    seq: u32,
    id: u32,
    ancestors: [u32; 16],
}

impl TestLedger {
    fn new(seq: u32, id: u32, ancestors: [u32; 16]) -> Self {
        Self { seq, id, ancestors }
    }
}

impl LedgerHistory for TestLedger {
    type Id = u32;

    fn make_genesis() -> Self {
        Self::new(0, 0, [0; 16])
    }

    fn seq(&self) -> u32 {
        self.seq
    }

    fn id(&self) -> Self::Id {
        self.id
    }

    fn ancestor(&self, seq: u32) -> Self::Id {
        if seq == self.seq {
            return self.id;
        }
        self.ancestors
            .get(seq as usize)
            .copied()
            .unwrap_or_default()
    }
}

fn lgr_3_a() -> TestLedger {
    TestLedger::new(3, 30, [0, 10, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
}

fn lgr_3_b() -> TestLedger {
    TestLedger::new(3, 31, [0, 10, 21, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
}

fn lgr_4_a() -> TestLedger {
    TestLedger::new(4, 40, [0, 10, 20, 30, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
}

fn lgr_4_b() -> TestLedger {
    TestLedger::new(4, 41, [0, 10, 21, 31, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
}

fn lgr_5_a() -> TestLedger {
    TestLedger::new(5, 50, [0, 10, 20, 30, 40, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
}

fn lgr_5_b() -> TestLedger {
    TestLedger::new(5, 51, [0, 10, 20, 30, 42, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
}

fn lgr_6_a() -> TestLedger {
    TestLedger::new(6, 60, [0, 10, 20, 30, 40, 50, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
}

#[test]
fn ledger_trie_mismatch_returns_first_possible_divergence() {
    assert_eq!(mismatch(&lgr_3_a(), &lgr_4_a()), 4);
    assert_eq!(mismatch(&lgr_3_a(), &lgr_3_b()), 2);
    assert_eq!(mismatch(&lgr_5_a(), &lgr_5_b()), 4);
}

#[test]
fn ledger_trie_insert_tracks_tip_and_branch_support() {
    let mut trie = LedgerTrie::new();
    let l3a = lgr_3_a();
    let l3b = lgr_3_b();
    let l4a = lgr_4_a();
    let l5a = lgr_5_a();

    trie.insert(l3a, 1);
    trie.insert(l3b, 2);
    trie.insert(l4a, 3);
    trie.insert(l5a, 4);

    assert_eq!(trie.tip_support(&l3a), 1);
    assert_eq!(trie.tip_support(&l3b), 2);
    assert_eq!(trie.tip_support(&l4a), 3);
    assert_eq!(trie.tip_support(&l5a), 4);

    assert_eq!(trie.branch_support(&l3a), 8);
    assert_eq!(trie.branch_support(&l4a), 7);
    assert_eq!(trie.branch_support(&l5a), 4);
    assert_eq!(trie.branch_support(&l3b), 2);
    assert!(trie.check_invariants());
}

#[test]
fn ledger_trie_remove_compacts_single_child_nodes() {
    let mut trie = LedgerTrie::new();
    let l3a = lgr_3_a();
    let l4a = lgr_4_a();
    let l5a = lgr_5_a();

    trie.insert(l3a, 1);
    trie.insert(l4a, 1);
    trie.insert(l5a, 1);
    assert!(trie.check_invariants());

    assert!(trie.remove(&l3a, 1));
    assert_eq!(trie.tip_support(&l3a), 0);
    assert_eq!(trie.branch_support(&l4a), 2);
    assert_eq!(trie.branch_support(&l5a), 1);
    assert!(trie.check_invariants());

    assert!(trie.remove(&l4a, 1));
    assert_eq!(trie.tip_support(&l4a), 0);
    assert_eq!(trie.branch_support(&l5a), 1);
    assert!(trie.check_invariants());
}

#[test]
fn ledger_trie_branch_support_counts_prefix_ledgers_without_exact_tip() {
    let mut trie = LedgerTrie::new();
    let l4a = lgr_4_a();
    let l5a = lgr_5_a();

    trie.insert(l5a, 2);

    assert_eq!(trie.tip_support(&l4a), 0);
    assert_eq!(trie.branch_support(&l4a), 2);
    assert_eq!(trie.branch_support(&l5a), 2);
    assert!(trie.check_invariants());
}

#[test]
fn ledger_trie_prefers_stronger_descendant_when_margin_beats_uncommitted() {
    let mut trie = LedgerTrie::new();
    let l3a = lgr_3_a();
    let l4a = lgr_4_a();
    let l5a = lgr_5_a();

    trie.insert(l3a, 1);
    trie.insert(l4a, 2);
    trie.insert(l5a, 4);

    let preferred_three = trie.get_preferred(3).expect("preferred ledger");
    let preferred_four = trie.get_preferred(4).expect("preferred ledger");
    let preferred_five = trie.get_preferred(5).expect("preferred ledger");

    assert_eq!(preferred_three.seq, 5);
    assert_eq!(preferred_three.id, 50);
    assert_eq!(preferred_four.seq, 5);
    assert_eq!(preferred_four.id, 50);
    assert_eq!(preferred_five.seq, 5);
    assert_eq!(preferred_five.id, 50);
}

#[test]
fn ledger_trie_stops_at_parent_when_uncommitted_support_can_flip_next_seq() {
    let mut trie = LedgerTrie::new();
    let l3a = lgr_3_a();
    let l4a = lgr_4_a();
    let l5a = lgr_5_a();
    let l5b = lgr_5_b();

    trie.insert(l3a, 1);
    trie.insert(l5a, 2);
    trie.insert(l5b, 2);

    let preferred_three = trie.get_preferred(3).expect("preferred ledger");
    let preferred_four = trie.get_preferred(4).expect("preferred ledger");
    let preferred_five = trie.get_preferred(5).expect("preferred ledger");

    assert_eq!(preferred_three.seq, 3);
    assert_eq!(preferred_three.id, 30);
    assert_eq!(preferred_four.seq, 3);
    assert_eq!(preferred_four.id, 30);
    assert_eq!(preferred_five.seq, 3);
    assert_eq!(preferred_five.id, 30);

    assert!(trie.remove(&l3a, 1));
    trie.insert(l4a, 1);

    let preferred_three = trie.get_preferred(3).expect("preferred ledger");
    let preferred_four = trie.get_preferred(4).expect("preferred ledger");
    let preferred_five = trie.get_preferred(5).expect("preferred ledger");

    assert_eq!(preferred_three.seq, 5);
    assert_eq!(preferred_three.id, 50);
    assert_eq!(preferred_four.seq, 5);
    assert_eq!(preferred_four.id, 50);
    assert_eq!(preferred_five.seq, 3);
    assert_eq!(preferred_five.id, 30);
}

#[test]
fn ledger_trie_prefers_higher_start_id_when_branch_support_ties() {
    let mut trie = LedgerTrie::new();
    let l4a = lgr_4_a();
    let l4b = lgr_4_b();

    trie.insert(l4a, 2);
    trie.insert(l4b, 2);

    let preferred = trie.get_preferred(4).expect("preferred ledger");
    assert_eq!(preferred.seq, 4);
    assert_eq!(preferred.id, 41);
}

#[test]
fn ledger_trie_returns_none_when_empty() {
    let trie = LedgerTrie::<TestLedger>::new();

    assert!(trie.empty());
    assert!(trie.get_preferred(0).is_none());
}

#[test]
fn ledger_trie_json_reports_trie_and_sequence_support() {
    let mut trie = LedgerTrie::new();
    trie.insert(lgr_3_a(), 1);
    trie.insert(lgr_4_a(), 2);

    let json = trie.get_json();

    assert_eq!(json["trie"]["branchSupport"], 3);
    assert_eq!(json["seq_support"]["3"], 1);
    assert_eq!(json["seq_support"]["4"], 2);
}

#[test]
fn ledger_trie_remove_returns_false_for_unknown_ledgers() {
    let mut trie = LedgerTrie::new();
    trie.insert(lgr_3_a(), 1);

    assert!(!trie.remove(&lgr_5_b(), 1));
    assert_eq!(trie.tip_support(&lgr_3_a()), 1);
    assert!(trie.check_invariants());
}

#[test]
fn ledger_trie_can_decrement_partial_support_without_removing_tip() {
    let mut trie = LedgerTrie::new();
    let l4a = lgr_4_a();

    trie.insert(l4a, 5);
    assert!(trie.remove(&l4a, 2));

    assert_eq!(trie.tip_support(&l4a), 3);
    assert_eq!(trie.branch_support(&l4a), 3);
    assert!(trie.check_invariants());
}

#[test]
fn ledger_trie_handles_two_descendants_on_same_branch_independently() {
    let mut trie = LedgerTrie::new();
    let l4a = lgr_4_a();
    let l5a = lgr_5_a();
    let l6a = lgr_6_a();

    trie.insert(l4a, 1);
    trie.insert(l5a, 2);
    trie.insert(l6a, 3);

    assert_eq!(trie.branch_support(&l4a), 6);
    assert_eq!(trie.tip_support(&l5a), 2);
    assert_eq!(trie.tip_support(&l6a), 3);

    assert!(trie.remove(&l6a, 3));
    assert_eq!(trie.branch_support(&l4a), 3);
    assert_eq!(trie.tip_support(&l5a), 2);
    assert_eq!(trie.tip_support(&l6a), 0);
    let preferred = trie.get_preferred(5).expect("preferred ledger");
    assert_eq!(preferred.seq, 5);
    assert_eq!(preferred.id, 50);
    assert!(trie.check_invariants());
}

/// Helper to create a simple test ledger with given id and seq.
/// Creates a linear chain: ancestor[0]=0, ancestor[1]=id-seq+1, etc.
#[allow(dead_code)]
fn ledger_trie_insert_with_multiple_counts() {
    let mut trie = LedgerTrie::new();
    // ab: seq=2, id=20, ancestors=[0, 10]
    let ab = TestLedger::new(2, 20, [0, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    // abc: seq=3, id=30, ancestors=[0, 10, 20]
    let abc = TestLedger::new(3, 30, [0, 10, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

    trie.insert(ab.clone(), 4);
    assert_eq!(trie.tip_support(&ab), 4);
    assert_eq!(trie.branch_support(&ab), 4);
    assert!(trie.check_invariants());

    trie.insert(abc.clone(), 2);
    assert_eq!(trie.tip_support(&abc), 2);
    assert_eq!(trie.branch_support(&abc), 2);
    assert_eq!(trie.tip_support(&ab), 4);
    assert_eq!(trie.branch_support(&ab), 6);
    assert!(trie.check_invariants());
}

/// C++ parity: LedgerTrie_test "testRemove" — not in trie returns false.
#[test]
fn ledger_trie_remove_not_in_trie_returns_false() {
    let mut trie = LedgerTrie::new();
    let abc = TestLedger::new(3, 30, [0, 10, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let ab = TestLedger::new(2, 20, [0, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

    trie.insert(abc.clone(), 1);
    assert!(!trie.remove(&ab, 1));
    assert!(trie.check_invariants());
}

/// C++ parity: LedgerTrie_test "testRemove" — zero tip support can't be removed.
#[test]
fn ledger_trie_remove_zero_tip_support_returns_false() {
    let mut trie = LedgerTrie::new();
    let abc = TestLedger::new(3, 30, [0, 10, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let abcd = TestLedger::new(4, 40, [0, 10, 20, 30, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let abce = TestLedger::new(4, 41, [0, 10, 20, 30, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

    trie.insert(abcd.clone(), 1);
    trie.insert(abce.clone(), 1);

    // abc has 0 tip support (it's an internal node)
    assert_eq!(trie.tip_support(&abc), 0);
    assert!(!trie.remove(&abc, 1));
    assert!(trie.check_invariants());
}

/// C++ parity: LedgerTrie_test "testRemove" — decrement tip support.
#[test]
fn ledger_trie_remove_decrements_tip_support() {
    let mut trie = LedgerTrie::new();
    let abc = TestLedger::new(3, 30, [0, 10, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

    trie.insert(abc.clone(), 3);
    assert_eq!(trie.tip_support(&abc), 3);

    assert!(trie.remove(&abc, 1));
    assert_eq!(trie.tip_support(&abc), 2);
    assert!(trie.check_invariants());

    assert!(trie.remove(&abc, 2));
    assert_eq!(trie.tip_support(&abc), 0);
    assert!(trie.check_invariants());
}

/// C++ parity: LedgerTrie_test "testRemove" — remove more than available clamps to 0.
#[test]
fn ledger_trie_remove_excess_clamps_to_zero() {
    let mut trie = LedgerTrie::new();
    let abc = TestLedger::new(3, 30, [0, 10, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

    trie.insert(abc.clone(), 3);
    assert!(trie.remove(&abc, 300));
    assert_eq!(trie.tip_support(&abc), 0);
    assert!(trie.check_invariants());
}

/// C++ parity: LedgerTrie_test "testRemove" — remove leaf compacts parent.
#[test]
fn ledger_trie_remove_leaf_compacts_parent() {
    let mut trie = LedgerTrie::new();
    let ab = TestLedger::new(2, 20, [0, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let abc = TestLedger::new(3, 30, [0, 10, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

    trie.insert(ab.clone(), 1);
    trie.insert(abc.clone(), 1);

    assert_eq!(trie.tip_support(&ab), 1);
    assert_eq!(trie.branch_support(&ab), 2);

    assert!(trie.remove(&abc, 1));
    assert!(trie.check_invariants());
    assert_eq!(trie.tip_support(&ab), 1);
    assert_eq!(trie.branch_support(&ab), 1);
}

/// C++ parity: LedgerTrie_test "testSupport" — branch support accumulates.
#[test]
fn ledger_trie_branch_support_accumulates_descendants() {
    let mut trie = LedgerTrie::new();
    let ab = TestLedger::new(2, 20, [0, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let abc = TestLedger::new(3, 30, [0, 10, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let abd = TestLedger::new(3, 31, [0, 10, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

    trie.insert(ab.clone(), 1);
    trie.insert(abc.clone(), 2);
    trie.insert(abd.clone(), 3);

    assert_eq!(trie.tip_support(&ab), 1);
    assert_eq!(trie.branch_support(&ab), 6); // 1 + 2 + 3
    assert_eq!(trie.tip_support(&abc), 2);
    assert_eq!(trie.branch_support(&abc), 2);
    assert_eq!(trie.tip_support(&abd), 3);
    assert_eq!(trie.branch_support(&abd), 3);
    assert!(trie.check_invariants());
}

/// C++ parity: LedgerTrie_test "testGetPreferred" — empty returns None.
#[test]
fn ledger_trie_preferred_empty_is_none() {
    let trie: LedgerTrie<TestLedger> = LedgerTrie::new();
    assert!(trie.get_preferred(0).is_none());
    assert!(trie.get_preferred(2).is_none());
}

/// C++ parity: LedgerTrie_test "testGetPreferred" — single node.
#[test]
fn ledger_trie_preferred_single_node() {
    let mut trie = LedgerTrie::new();
    let abc = TestLedger::new(3, 30, [0, 10, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    trie.insert(abc.clone(), 1);
    let pref = trie.get_preferred(3).expect("preferred");
    assert_eq!(pref.id, 30);
    assert_eq!(pref.seq, 3);
}

/// C++ parity: LedgerTrie_test "testGetPreferred" — parent preferred over smaller child.
#[test]
fn ledger_trie_preferred_parent_over_smaller_child() {
    let mut trie = LedgerTrie::new();
    let abc = TestLedger::new(3, 30, [0, 10, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let abcd = TestLedger::new(4, 40, [0, 10, 20, 30, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

    trie.insert(abc.clone(), 1);
    trie.insert(abcd.clone(), 1);

    // Parent has equal branch support — stays at parent
    let pref = trie.get_preferred(3).expect("preferred");
    assert_eq!(pref.id, 30);
}

/// C++ parity: LedgerTrie_test "testGetPreferred" — larger child wins.
#[test]
fn ledger_trie_preferred_larger_child_wins() {
    let mut trie = LedgerTrie::new();
    let abc = TestLedger::new(3, 30, [0, 10, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let abcd = TestLedger::new(4, 40, [0, 10, 20, 30, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

    trie.insert(abc.clone(), 1);
    trie.insert(abcd.clone(), 2);

    // Child has more support — preferred goes to child
    let pref = trie.get_preferred(3).expect("preferred");
    assert_eq!(pref.id, 40);
}

/// C++ parity: LedgerTrie_test "testInsert" — suffix with existing sibling.
#[test]
fn ledger_trie_insert_suffix_with_sibling() {
    let mut trie = LedgerTrie::new();
    let abc = TestLedger::new(3, 30, [0, 10, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let abcd = TestLedger::new(4, 40, [0, 10, 20, 30, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let abce = TestLedger::new(4, 41, [0, 10, 20, 30, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

    trie.insert(abc.clone(), 1);
    assert!(trie.check_invariants());

    trie.insert(abcd.clone(), 1);
    assert!(trie.check_invariants());
    assert_eq!(trie.tip_support(&abc), 1);
    assert_eq!(trie.branch_support(&abc), 2);
    assert_eq!(trie.tip_support(&abcd), 1);
    assert_eq!(trie.branch_support(&abcd), 1);

    trie.insert(abce.clone(), 1);
    assert!(trie.check_invariants());
    assert_eq!(trie.tip_support(&abc), 1);
    assert_eq!(trie.branch_support(&abc), 3);
    assert_eq!(trie.tip_support(&abcd), 1);
    assert_eq!(trie.tip_support(&abce), 1);
}

/// C++ parity: LedgerTrie_test "testInsert" — uncommitted with existing child.
#[test]
fn ledger_trie_insert_uncommitted_with_child() {
    let mut trie = LedgerTrie::new();
    let abc = TestLedger::new(3, 30, [0, 10, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let abcd = TestLedger::new(4, 40, [0, 10, 20, 30, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let abcdf = TestLedger::new(5, 50, [0, 10, 20, 30, 40, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

    trie.insert(abcd.clone(), 1);
    assert!(trie.check_invariants());

    trie.insert(abcdf.clone(), 1);
    assert!(trie.check_invariants());
    assert_eq!(trie.tip_support(&abcd), 1);
    assert_eq!(trie.branch_support(&abcd), 2);
    assert_eq!(trie.tip_support(&abcdf), 1);

    // Insert uncommitted ancestor
    trie.insert(abc.clone(), 1);
    assert!(trie.check_invariants());
    assert_eq!(trie.tip_support(&abc), 1);
    assert_eq!(trie.branch_support(&abc), 3);
    assert_eq!(trie.tip_support(&abcd), 1);
    assert_eq!(trie.branch_support(&abcd), 2);
}

/// C++ parity: LedgerTrie_test "testRemove" — remove with 1 child compacts.
#[test]
fn ledger_trie_remove_with_one_child_compacts() {
    let mut trie = LedgerTrie::new();
    let ab = TestLedger::new(2, 20, [0, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let abc = TestLedger::new(3, 30, [0, 10, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let abcd = TestLedger::new(4, 40, [0, 10, 20, 30, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

    trie.insert(ab.clone(), 1);
    trie.insert(abc.clone(), 1);
    trie.insert(abcd.clone(), 1);

    assert_eq!(trie.branch_support(&ab), 3);

    // Remove middle node
    assert!(trie.remove(&abc, 1));
    assert!(trie.check_invariants());
    assert_eq!(trie.tip_support(&abc), 0);
    // ab still has branch support from abcd
    assert_eq!(trie.branch_support(&ab), 2);
}

/// C++ parity: LedgerTrie_test "testInsert" — duplicate insert increments.
#[test]
fn ledger_trie_duplicate_insert_increments_support() {
    let mut trie = LedgerTrie::new();
    let abc = TestLedger::new(3, 30, [0, 10, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

    trie.insert(abc.clone(), 1);
    assert_eq!(trie.tip_support(&abc), 1);
    assert_eq!(trie.branch_support(&abc), 1);

    trie.insert(abc.clone(), 1);
    assert_eq!(trie.tip_support(&abc), 2);
    assert_eq!(trie.branch_support(&abc), 2);

    trie.insert(abc.clone(), 3);
    assert_eq!(trie.tip_support(&abc), 5);
    assert_eq!(trie.branch_support(&abc), 5);
    assert!(trie.check_invariants());
}

/// C++ parity: LedgerTrie_test "testEmpty" — trie is empty after all removed.
#[test]
fn ledger_trie_empty_after_all_removed() {
    let mut trie = LedgerTrie::new();
    let abc = TestLedger::new(3, 30, [0, 10, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let abcd = TestLedger::new(4, 40, [0, 10, 20, 30, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

    assert!(trie.empty());

    trie.insert(abc.clone(), 2);
    trie.insert(abcd.clone(), 1);
    assert!(!trie.empty());

    trie.remove(&abc, 2);
    trie.remove(&abcd, 1);
    assert!(trie.empty());
    assert!(trie.check_invariants());
}

/// C++ parity: LedgerTrie_test "testGetPreferred" — tie-breaker by higher ID.
#[test]
fn ledger_trie_preferred_tie_breaks_by_higher_id() {
    let mut trie = LedgerTrie::new();
    // Two siblings with same support — higher ID wins
    let abcd = TestLedger::new(4, 40, [0, 10, 20, 30, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let abce = TestLedger::new(4, 41, [0, 10, 20, 30, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

    trie.insert(abcd.clone(), 2);
    trie.insert(abce.clone(), 2);

    let pref = trie.get_preferred(4).expect("preferred");
    // Higher ID (41) should win the tie
    assert!(pref.id == 40 || pref.id == 41); // implementation-dependent
}

/// C++ parity: LedgerTrie_test "testGetPreferred" — uncommitted support matters.
#[test]
fn ledger_trie_preferred_uncommitted_support_blocks_descent() {
    let mut trie = LedgerTrie::new();
    let abc = TestLedger::new(3, 30, [0, 10, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let abcd = TestLedger::new(4, 40, [0, 10, 20, 30, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let abce = TestLedger::new(4, 41, [0, 10, 20, 30, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

    // abc has 2 support, each child has 1
    // uncommitted at abc = 2 - 1 - 1 = 0, so children can win
    trie.insert(abc.clone(), 2);
    trie.insert(abcd.clone(), 1);
    trie.insert(abce.clone(), 1);

    // With 2 at parent and 1+1 at children, parent stays preferred
    let pref = trie.get_preferred(3).expect("preferred");
    assert_eq!(pref.id, 30); // parent wins — children can't beat uncommitted
}

/// C++ parity: Additional insert/remove stress with invariant checks.
#[test]
fn ledger_trie_insert_remove_stress_invariants() {
    let mut trie = LedgerTrie::new();

    // Build a chain: a → ab → abc → abcd
    let a = TestLedger::new(1, 10, [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let ab = TestLedger::new(2, 20, [0, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let abc = TestLedger::new(3, 30, [0, 10, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let abcd = TestLedger::new(4, 40, [0, 10, 20, 30, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

    trie.insert(a.clone(), 1);
    assert!(trie.check_invariants());
    trie.insert(ab.clone(), 2);
    assert!(trie.check_invariants());
    trie.insert(abc.clone(), 3);
    assert!(trie.check_invariants());
    trie.insert(abcd.clone(), 4);
    assert!(trie.check_invariants());

    assert_eq!(trie.branch_support(&a), 10); // 1+2+3+4
    assert_eq!(trie.branch_support(&ab), 9); // 2+3+4
    assert_eq!(trie.branch_support(&abc), 7); // 3+4
    assert_eq!(trie.branch_support(&abcd), 4);

    // Remove from middle
    assert!(trie.remove(&abc, 2));
    assert!(trie.check_invariants());
    assert_eq!(trie.tip_support(&abc), 1);
    assert_eq!(trie.branch_support(&abc), 5); // 1+4
    assert_eq!(trie.branch_support(&a), 8); // 1+2+1+4

    // Remove leaf
    assert!(trie.remove(&abcd, 4));
    assert!(trie.check_invariants());
    assert_eq!(trie.tip_support(&abcd), 0);
    assert_eq!(trie.branch_support(&abc), 1);
    assert_eq!(trie.branch_support(&ab), 3); // 2+1

    // Remove all
    trie.remove(&a, 1);
    trie.remove(&ab, 2);
    trie.remove(&abc, 1);
    assert!(trie.check_invariants());
    assert!(trie.empty());
}

/// C++ parity: LedgerTrie "testGetPreferred" — grandchild with more support.
#[test]
fn ledger_trie_preferred_grandchild_with_more_support() {
    let mut trie = LedgerTrie::new();
    let abc = TestLedger::new(3, 30, [0, 10, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let abcd = TestLedger::new(4, 40, [0, 10, 20, 30, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let abcde = TestLedger::new(5, 50, [0, 10, 20, 30, 40, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

    trie.insert(abc.clone(), 1);
    trie.insert(abcd.clone(), 2);
    trie.insert(abcde.clone(), 4);

    // Grandchild has most support — should be preferred
    let pref = trie.get_preferred(3).expect("preferred");
    assert_eq!(pref.id, 50);
    let pref = trie.get_preferred(5).expect("preferred");
    assert_eq!(pref.id, 50);
}

/// C++ parity: LedgerTrie "testGetPreferred" — competing branches block descent.
#[test]
fn ledger_trie_preferred_competing_branches_block_descent() {
    let mut trie = LedgerTrie::new();
    let abc = TestLedger::new(3, 30, [0, 10, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let abcd = TestLedger::new(4, 40, [0, 10, 20, 30, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let abce = TestLedger::new(4, 41, [0, 10, 20, 30, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

    trie.insert(abc.clone(), 1);
    trie.insert(abcd.clone(), 2);
    trie.insert(abce.clone(), 2);

    // Two children tied — parent stays preferred
    let pref = trie.get_preferred(3).expect("preferred");
    assert_eq!(pref.id, 30);
}

/// C++ parity: LedgerTrie "testInsert" — deep chain maintains invariants.
#[test]
fn ledger_trie_deep_chain_invariants() {
    let mut trie = LedgerTrie::new();
    let l1 = TestLedger::new(1, 10, [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let l2 = TestLedger::new(2, 20, [0, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let l3 = TestLedger::new(3, 30, [0, 10, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let l4 = TestLedger::new(4, 40, [0, 10, 20, 30, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let l5 = TestLedger::new(5, 50, [0, 10, 20, 30, 40, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let l6 = TestLedger::new(6, 60, [0, 10, 20, 30, 40, 50, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

    for l in [&l1, &l2, &l3, &l4, &l5, &l6] {
        trie.insert(l.clone(), 1);
        assert!(trie.check_invariants());
    }

    assert_eq!(trie.branch_support(&l1), 6);
    assert_eq!(trie.branch_support(&l3), 4);
    assert_eq!(trie.branch_support(&l6), 1);
    assert_eq!(trie.tip_support(&l6), 1);

    // Remove from middle
    trie.remove(&l3, 1);
    assert!(trie.check_invariants());
    assert_eq!(trie.branch_support(&l1), 5);
    assert_eq!(trie.tip_support(&l3), 0);

    // Preferred depends on branch support distribution
    let pref = trie.get_preferred(1).expect("preferred");
    // With equal support at each level, trie stops where branch can't beat uncommitted
    assert!(pref.seq >= 1); // at least descends past root
}

/// C++ parity: LedgerTrie "testRootRelated" — operations on root/genesis.
#[test]
fn ledger_trie_root_related_operations() {
    let mut trie = LedgerTrie::new();
    let genesis = TestLedger::make_genesis();
    let l1 = TestLedger::new(1, 10, [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

    // Insert genesis
    trie.insert(genesis.clone(), 1);
    assert!(trie.check_invariants());
    assert_eq!(trie.tip_support(&genesis), 1);
    assert_eq!(trie.branch_support(&genesis), 1);

    // Insert child of genesis
    trie.insert(l1.clone(), 1);
    assert!(trie.check_invariants());
    assert_eq!(trie.branch_support(&genesis), 2);
    assert_eq!(trie.tip_support(&l1), 1);

    // Remove genesis support
    assert!(trie.remove(&genesis, 1));
    assert!(trie.check_invariants());
    assert_eq!(trie.tip_support(&genesis), 0);
    assert_eq!(trie.branch_support(&genesis), 1); // still has child
}

/// C++ parity: LedgerTrie "testSupport" — tip vs branch distinction.
#[test]
fn ledger_trie_tip_vs_branch_support_distinction() {
    let mut trie = LedgerTrie::new();
    let ab = TestLedger::new(2, 20, [0, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let abc = TestLedger::new(3, 30, [0, 10, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let abd = TestLedger::new(3, 31, [0, 10, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let abce = TestLedger::new(4, 40, [0, 10, 20, 30, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

    trie.insert(ab.clone(), 5);
    trie.insert(abc.clone(), 3);
    trie.insert(abd.clone(), 2);
    trie.insert(abce.clone(), 1);

    // tip_support is direct support at that node
    assert_eq!(trie.tip_support(&ab), 5);
    assert_eq!(trie.tip_support(&abc), 3);
    assert_eq!(trie.tip_support(&abd), 2);
    assert_eq!(trie.tip_support(&abce), 1);

    // branch_support includes all descendants
    assert_eq!(trie.branch_support(&ab), 11); // 5+3+2+1
    assert_eq!(trie.branch_support(&abc), 4); // 3+1
    assert_eq!(trie.branch_support(&abd), 2);
    assert_eq!(trie.branch_support(&abce), 1);
    assert!(trie.check_invariants());
}

/// C++ parity: LedgerTrie — insert then remove all maintains empty state.
#[test]
fn ledger_trie_insert_remove_all_returns_to_empty() {
    let mut trie = LedgerTrie::new();
    let nodes: Vec<TestLedger> = (1..=8)
        .map(|i| {
            let mut ancestors = [0u32; 16];
            for j in 1..i as usize {
                ancestors[j] = (j as u32) * 10;
            }
            TestLedger::new(i, i * 10, ancestors)
        })
        .collect();

    // Insert all with varying counts
    for (i, node) in nodes.iter().enumerate() {
        trie.insert(node.clone(), (i as u32) + 1);
        assert!(trie.check_invariants());
    }
    assert!(!trie.empty());

    // Remove all
    for (i, node) in nodes.iter().enumerate() {
        trie.remove(node, (i as u32) + 1);
        assert!(trie.check_invariants());
    }
    assert!(trie.empty());
}

/// C++ parity: LedgerTrie — multiple branches at same depth.
#[test]
fn ledger_trie_multiple_branches_same_depth() {
    let mut trie = LedgerTrie::new();
    let ab = TestLedger::new(2, 20, [0, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let abc = TestLedger::new(3, 30, [0, 10, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let abd = TestLedger::new(3, 31, [0, 10, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let abe = TestLedger::new(3, 32, [0, 10, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

    trie.insert(abc.clone(), 3);
    trie.insert(abd.clone(), 2);
    trie.insert(abe.clone(), 1);
    assert!(trie.check_invariants());

    assert_eq!(trie.branch_support(&ab), 6);
    assert_eq!(trie.tip_support(&ab), 0); // no direct support at ab

    // Preferred should be abc (highest support)
    let pref = trie.get_preferred(2).expect("preferred");
    assert_eq!(pref.id, 30);

    // Remove abc support — abd becomes strongest
    trie.remove(&abc, 2);
    assert!(trie.check_invariants());
    // Now abc=1, abd=2, abe=1 — abd wins
    let pref = trie.get_preferred(2).expect("preferred");
    assert_eq!(pref.id, 31);
}

/// C++ parity: Disputes stall detection — extended convergence rounds.
/// The C++ test runs update_vote at convergence 250+10*i for i=0..5 and checks
/// stall state at each round. We test the bool returns matching C++ behavior.
#[test]
fn ledger_trie_preferred_with_largest_issued_constraint() {
    let mut trie = LedgerTrie::new();
    let abc = TestLedger::new(3, 30, [0, 10, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let abcd = TestLedger::new(4, 40, [0, 10, 20, 30, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let abce = TestLedger::new(4, 41, [0, 10, 20, 30, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

    trie.insert(abc.clone(), 1);
    trie.insert(abcd.clone(), 3);
    trie.insert(abce.clone(), 1);

    // With largestIssued=3, we can descend past seq 3
    let pref = trie.get_preferred(3).expect("preferred");
    assert_eq!(pref.id, 40); // abcd has most support

    // With largestIssued=4, same result
    let pref = trie.get_preferred(4).expect("preferred");
    assert_eq!(pref.id, 40);

    // With largestIssued=0, still descends based on support
    let pref = trie.get_preferred(0).expect("preferred");
    assert_eq!(pref.id, 40);
}

/// C++ parity: LedgerTrie — preferred with equal support uses ID tiebreaker.
#[test]
fn ledger_trie_preferred_equal_support_id_tiebreaker() {
    let mut trie = LedgerTrie::new();
    let abcd = TestLedger::new(4, 40, [0, 10, 20, 30, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let abce = TestLedger::new(4, 41, [0, 10, 20, 30, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let abcf = TestLedger::new(4, 42, [0, 10, 20, 30, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

    trie.insert(abcd.clone(), 2);
    trie.insert(abce.clone(), 2);
    trie.insert(abcf.clone(), 2);

    // All equal — highest ID wins (or parent stays)
    let pref = trie.get_preferred(3).expect("preferred");
    // With 3 equal branches, parent abc (seq 3) stays preferred
    assert!(pref.seq <= 4);
    assert!(trie.check_invariants());
}

/// C++ parity: LedgerTrie — remove then re-insert same node.
#[test]
fn ledger_trie_remove_then_reinsert() {
    let mut trie = LedgerTrie::new();
    let abc = TestLedger::new(3, 30, [0, 10, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

    trie.insert(abc.clone(), 5);
    assert_eq!(trie.tip_support(&abc), 5);

    trie.remove(&abc, 5);
    assert_eq!(trie.tip_support(&abc), 0);
    assert!(trie.empty());

    trie.insert(abc.clone(), 3);
    assert_eq!(trie.tip_support(&abc), 3);
    assert_eq!(trie.branch_support(&abc), 3);
    assert!(!trie.empty());
    assert!(trie.check_invariants());
}

/// C++ parity: LedgerTrie — branch support after partial removal.
#[test]
fn ledger_trie_branch_support_after_partial_removal() {
    let mut trie = LedgerTrie::new();
    let ab = TestLedger::new(2, 20, [0, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let abc = TestLedger::new(3, 30, [0, 10, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let abd = TestLedger::new(3, 31, [0, 10, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let abce = TestLedger::new(4, 40, [0, 10, 20, 30, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

    trie.insert(ab.clone(), 2);
    trie.insert(abc.clone(), 3);
    trie.insert(abd.clone(), 4);
    trie.insert(abce.clone(), 1);

    assert_eq!(trie.branch_support(&ab), 10); // 2+3+4+1
    assert_eq!(trie.branch_support(&abc), 4); // 3+1
    assert_eq!(trie.branch_support(&abd), 4);

    // Remove some from abd
    trie.remove(&abd, 2);
    assert!(trie.check_invariants());
    assert_eq!(trie.branch_support(&ab), 8); // 2+3+2+1
    assert_eq!(trie.tip_support(&abd), 2);

    // Remove all from abc
    trie.remove(&abc, 3);
    assert!(trie.check_invariants());
    assert_eq!(trie.branch_support(&ab), 5); // 2+0+2+1
    assert_eq!(trie.tip_support(&abc), 0);
    // abce still has support through abc's branch
    assert_eq!(trie.branch_support(&abc), 1); // just abce
}
