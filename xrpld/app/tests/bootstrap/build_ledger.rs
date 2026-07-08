use std::{cell::RefCell, collections::BTreeSet, rc::Rc, sync::Arc};

use app::{
    BuildLedgerJournal, BuildLedgerView, LedgerReplay, apply_transactions, build_ledger,
    build_ledger_replay, decode_acquired_tx_set,
};
use basics::{base_uint::Uint256, intrusive_pointer::make_shared_intrusive};
use ledger::{CanonicalTXSet, Ledger, LedgerHeader};
use protocol::{
    STAmount, STArray, STObject, STTx, TxType, decode_ledger_hashes_entry, get_field_by_symbol,
    skip_keylet,
};
use shamap::{
    item::SHAMapItem,
    mutation::MutableTree,
    sync::{SHAMapType, SyncState, SyncTree},
    tree_node::{SHAMapNodeType, SHAMapTreeNode},
};
use tx::{ApplyFlags, ApplyTransactionResult};

#[derive(Default)]
struct RecordingJournal {
    debug: RefCell<Vec<String>>,
    warn: RefCell<Vec<String>>,
}

impl RecordingJournal {
    fn debug_messages(&self) -> Vec<String> {
        self.debug.borrow().clone()
    }

    fn warn_messages(&self) -> Vec<String> {
        self.warn.borrow().clone()
    }
}

impl BuildLedgerJournal for RecordingJournal {
    fn debug(&self, message: &str) {
        self.debug.borrow_mut().push(message.to_owned());
    }

    fn warn(&self, message: &str) {
        self.warn.borrow_mut().push(message.to_owned());
    }
}

#[derive(Clone)]
struct StubBuildView {
    open: bool,
    applied: Vec<Uint256>,
    events: Rc<RefCell<Vec<&'static str>>>,
}

impl StubBuildView {
    fn closed(events: Rc<RefCell<Vec<&'static str>>>) -> Self {
        Self {
            open: false,
            applied: Vec::new(),
            events,
        }
    }
}

impl BuildLedgerView for StubBuildView {
    fn open(&self) -> bool {
        self.open
    }

    fn tx_count(&self) -> usize {
        self.applied.len()
    }

    fn apply_to_ledger(self, _ledger: &mut Ledger) {
        self.events.borrow_mut().push("view.apply");
    }
}

fn account(fill: u8) -> protocol::AccountID {
    protocol::AccountID::from_hex(&format!("{fill:02x}").repeat(20))
        .expect("account hex should parse")
}

fn sample_tx(sequence: u32) -> Arc<STTx> {
    Arc::new(STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), account(0x11));
        tx.set_account_id(get_field_by_symbol("sfDestination"), account(0x22));
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(1_000_000 + u64::from(sequence), false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
    }))
}

fn sample_parent_ledger(seq: u32) -> Ledger {
    Ledger::from_maps(
        LedgerHeader {
            seq,
            ..LedgerHeader::default()
        },
        SyncTree::new_with_type(SHAMapType::State, false, seq),
        SyncTree::new_with_type(SHAMapType::Transaction, false, seq),
    )
}

fn sample_parent_with_tx(seq: u32, tx_id: Uint256) -> Ledger {
    let tx_root = make_shared_intrusive(SHAMapTreeNode::new_leaf(
        SHAMapNodeType::TransactionNm,
        SHAMapItem::new(tx_id, vec![0xAB; 20]),
        0,
    ));

    Ledger::from_maps(
        LedgerHeader {
            seq,
            ..LedgerHeader::default()
        },
        SyncTree::new_with_type(SHAMapType::State, false, seq),
        SyncTree::from_root_with_type(
            tx_root,
            SHAMapType::Transaction,
            false,
            seq,
            SyncState::Immutable,
        ),
    )
}

fn canonical_set(txs: impl IntoIterator<Item = Arc<STTx>>) -> CanonicalTXSet {
    let mut set = CanonicalTXSet::new(Uint256::from_array([0x55; 32]));
    for tx in txs {
        set.insert(tx);
    }
    set
}

fn metadata(index: u32, result: u8) -> STObject {
    let affected_nodes = STArray::new(get_field_by_symbol("sfAffectedNodes"));
    let mut meta = STObject::new(get_field_by_symbol("sfTransactionMetaData"));
    meta.set_field_u8(get_field_by_symbol("sfTransactionResult"), result);
    meta.set_field_u32(get_field_by_symbol("sfTransactionIndex"), index);
    meta.set_field_array(get_field_by_symbol("sfAffectedNodes"), affected_nodes);
    meta
}

fn tx_md_payload(tx: &STTx, index: u32) -> Vec<u8> {
    let meta = metadata(index, 0);
    let tx_bytes = tx.get_serializer().data().to_vec();
    let meta_bytes = meta.get_serializer().data().to_vec();
    let mut serializer = protocol::Serializer::new(0);
    serializer.add_vl(&tx_bytes);
    serializer.add_vl(&meta_bytes);
    serializer.data().to_vec()
}

fn ledger_with_tx_items(items: &[(Arc<STTx>, SHAMapNodeType, Vec<u8>)], seq: u32) -> Ledger {
    let mut tree = MutableTree::new(seq);
    for (tx, node_type, payload) in items {
        tree.add_item(
            *node_type,
            SHAMapItem::new(tx.get_transaction_id(), payload.clone()),
        )
        .expect("tx item should insert");
    }

    Ledger::from_maps(
        LedgerHeader {
            seq,
            ..LedgerHeader::default()
        },
        SyncTree::new_with_type(SHAMapType::State, false, seq),
        SyncTree::from_root_with_type(
            tree.root(),
            SHAMapType::Transaction,
            false,
            seq,
            SyncState::Immutable,
        ),
    )
}

#[test]
fn build_ledger_apply_transactions_skips_existing_tx_on_first_pass() {
    let duplicate = sample_tx(1);
    let built = sample_parent_with_tx(10, duplicate.get_transaction_id());
    let journal = RecordingJournal::default();
    let mut txns = canonical_set([Arc::clone(&duplicate)]);
    let mut failed = BTreeSet::new();
    let mut view = StubBuildView::closed(Rc::new(RefCell::new(Vec::new())));
    let apply_calls = RefCell::new(0usize);

    let applied = apply_transactions(
        &built,
        &mut txns,
        &mut failed,
        &mut view,
        &journal,
        &mut |_view, _tx, _retry, _flags| -> Result<ApplyTransactionResult, &'static str> {
            *apply_calls.borrow_mut() += 1;
            Ok(ApplyTransactionResult::Success)
        },
    );

    assert_eq!(applied, 0);
    assert_eq!(*apply_calls.borrow(), 0);
    assert!(txns.is_empty());
    assert!(failed.is_empty());
}

#[test]
fn build_ledger_apply_transactions_routes_success_fail_retry_and_throw() {
    let success = sample_tx(1);
    let hard_fail = sample_tx(2);
    let retry = sample_tx(3);
    let throwing = sample_tx(4);
    let built = sample_parent_ledger(20);
    let journal = RecordingJournal::default();
    let mut txns = canonical_set([
        Arc::clone(&success),
        Arc::clone(&hard_fail),
        Arc::clone(&retry),
        Arc::clone(&throwing),
    ]);
    let mut failed = BTreeSet::new();
    let mut view = StubBuildView::closed(Rc::new(RefCell::new(Vec::new())));
    let retry_calls = RefCell::new(0usize);
    let retry_flags = RefCell::new(Vec::new());

    let applied = apply_transactions(
        &built,
        &mut txns,
        &mut failed,
        &mut view,
        &journal,
        &mut |view, tx, retry_assured, flags| -> Result<ApplyTransactionResult, &'static str> {
            if tx.get_transaction_id() == success.get_transaction_id() {
                view.applied.push(tx.get_transaction_id());
                return Ok(ApplyTransactionResult::Success);
            }
            if tx.get_transaction_id() == hard_fail.get_transaction_id() {
                return Ok(ApplyTransactionResult::Fail);
            }
            if tx.get_transaction_id() == retry.get_transaction_id() {
                retry_flags.borrow_mut().push((retry_assured, flags.bits()));
                let mut seen = retry_calls.borrow_mut();
                *seen += 1;
                if retry_assured {
                    return Ok(ApplyTransactionResult::Retry);
                }
                view.applied.push(tx.get_transaction_id());
                return Ok(ApplyTransactionResult::Success);
            }

            Err("boom")
        },
    );

    assert_eq!(applied, 2);
    assert!(txns.is_empty());
    assert_eq!(
        failed,
        BTreeSet::from([
            hard_fail.get_transaction_id(),
            throwing.get_transaction_id()
        ])
    );
    assert_eq!(
        retry_flags.into_inner(),
        vec![
            (true, ApplyFlags::NONE.bits()),
            (true, ApplyFlags::NONE.bits()),
            (false, ApplyFlags::NONE.bits()),
        ]
    );
    assert_eq!(
        journal.warn_messages(),
        vec![format!(
            "Transaction {} throws: boom",
            throwing.get_transaction_id()
        )]
    );
}

#[test]
fn build_ledger_finalizes_skiplist_flush_unshare_and_accept_in() {
    let parent = Arc::new(sample_parent_ledger(30));
    let tx = sample_tx(9);
    let mut txns = canonical_set([Arc::clone(&tx)]);
    let mut failed = BTreeSet::new();
    let journal = RecordingJournal::default();
    let events = Rc::new(RefCell::new(Vec::new()));

    let built = build_ledger(
        Arc::clone(&parent),
        88,
        false,
        30,
        &mut txns,
        &mut failed,
        &journal,
        {
            let events = Rc::clone(&events);
            move |_built| StubBuildView::closed(Rc::clone(&events))
        },
        {
            let events = Rc::clone(&events);
            move |view, tx, retry_assured, flags| {
                assert!(retry_assured);
                assert_eq!(flags, ApplyFlags::NONE);
                view.applied.push(tx.get_transaction_id());
                events.borrow_mut().push("apply");
                Ok(ApplyTransactionResult::Success)
            }
        },
        {
            let events = Rc::clone(&events);
            move |_ledger| {
                events.borrow_mut().push("flush_state");
                7
            }
        },
        {
            let events = Rc::clone(&events);
            move |_ledger| {
                events.borrow_mut().push("flush_tx");
                11
            }
        },
        {
            let events = Rc::clone(&events);
            move |_ledger| {
                events.borrow_mut().push("unshare");
            }
        },
    )
    .expect("build ledger should succeed");

    assert!(failed.is_empty());
    assert!(txns.is_empty());
    assert_eq!(
        *events.borrow(),
        vec!["apply", "view.apply", "flush_state", "flush_tx", "unshare"]
    );
    assert!(built.is_immutable());
    assert_eq!(built.header().close_time, 88);
    assert_eq!(built.header().close_time_resolution, 30);
    assert_eq!(built.header().close_flags, ledger::SLCF_NO_CONSENSUS_TIME);

    let (short_list, _) = built
        .state_map()
        .peek_item_with_hash(skip_keylet().key, &mut |_| None)
        .expect("skip-list read should succeed")
        .expect("short skip-list entry should exist");
    let decoded =
        decode_ledger_hashes_entry(short_list.data()).expect("short skip-list should decode");
    assert_eq!(decoded.last_ledger_sequence, Some(parent.header().seq));
    assert_eq!(decoded.hashes, vec![*parent.header().hash.as_uint256()]);

    let debug = journal.debug_messages();
    assert!(
        debug
            .iter()
            .any(|message| message.contains("Flushed 7 accounts and 11 transaction nodes"))
    );
}

#[test]
fn build_ledger_replay_applies_ordered_txns_in_input_order_with_replay_flags() {
    let parent = Arc::new(sample_parent_ledger(40));
    let mut replay_ledger = Ledger::from_previous(&parent, 100);
    replay_ledger.set_accepted(111, 20, true);
    let replay_ledger = Arc::new(replay_ledger);

    let first = sample_tx(1);
    let second = sample_tx(2);
    let third = sample_tx(3);
    let replay = LedgerReplay::new(
        Arc::clone(&parent),
        Arc::clone(&replay_ledger),
        std::collections::BTreeMap::from([
            (2, Arc::clone(&second)),
            (1, Arc::clone(&first)),
            (3, Arc::clone(&third)),
        ]),
    );
    let journal = RecordingJournal::default();
    let seen = RefCell::new(Vec::new());

    let built = build_ledger_replay(
        &replay,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        &journal,
        |_built| StubBuildView::closed(Rc::new(RefCell::new(Vec::new()))),
        |view, tx, flags| {
            view.applied.push(tx.get_transaction_id());
            seen.borrow_mut()
                .push((tx.get_transaction_id(), flags.bits()));
        },
        |_ledger| 0,
        |_ledger| 0,
        |_ledger| {},
    )
    .expect("replay build should succeed");

    assert_eq!(
        seen.into_inner(),
        vec![
            (
                first.get_transaction_id(),
                (ApplyFlags::FAIL_HARD | ApplyFlags::RETRY).bits(),
            ),
            (
                second.get_transaction_id(),
                (ApplyFlags::FAIL_HARD | ApplyFlags::RETRY).bits(),
            ),
            (
                third.get_transaction_id(),
                (ApplyFlags::FAIL_HARD | ApplyFlags::RETRY).bits(),
            ),
        ]
    );
    assert_eq!(built.header().close_time, replay_ledger.header().close_time);
    assert_eq!(
        built.header().close_time_resolution,
        replay_ledger.header().close_time_resolution
    );
    assert_eq!(
        built.header().close_flags,
        replay_ledger.header().close_flags
    );
}

#[test]
fn acquired_tx_set_uses_canonical_order_not_tx_map_leaf_order() {
    let later = sample_tx(17264169);
    let earlier = sample_tx(17264168);
    let tx_items = vec![
        (tx_md_payload(&later, 0), later.get_transaction_id()),
        (tx_md_payload(&earlier, 1), earlier.get_transaction_id()),
    ];

    let ordered = decode_acquired_tx_set(
        &tx_items,
        Uint256::from_array([0xCF; 32]),
        shamap::tree_node::SHAMapNodeType::TransactionMd,
    );
    let ordered_sequences = ordered
        .iter()
        .map(|tx| tx.get_field_u32(get_field_by_symbol("sfSequence")))
        .collect::<Vec<_>>();

    assert_eq!(ordered_sequences, vec![17264168, 17264169]);
}

#[test]
fn ledger_replay_from_replay_ledger_sorts_by_metadata_index() {
    let parent = Arc::new(sample_parent_ledger(50));
    let first = sample_tx(1);
    let second = sample_tx(2);
    let third = sample_tx(3);
    let replay_ledger = Arc::new(ledger_with_tx_items(
        &[
            (
                Arc::clone(&third),
                SHAMapNodeType::TransactionMd,
                tx_md_payload(&third, 30),
            ),
            (
                Arc::clone(&first),
                SHAMapNodeType::TransactionMd,
                tx_md_payload(&first, 10),
            ),
            (
                Arc::clone(&second),
                SHAMapNodeType::TransactionMd,
                tx_md_payload(&second, 20),
            ),
        ],
        51,
    ));
    let replay = LedgerReplay::from_replay_ledger(Arc::clone(&parent), Arc::clone(&replay_ledger))
        .expect("replay ledger constructor should succeed");

    let ordered: Vec<_> = replay
        .ordered_txs()
        .iter()
        .map(|(index, tx)| (*index, tx.get_transaction_id()))
        .collect();

    assert_eq!(
        ordered,
        vec![
            (10, first.get_transaction_id()),
            (20, second.get_transaction_id()),
            (30, third.get_transaction_id()),
        ]
    );

    let seen = RefCell::new(Vec::new());
    build_ledger_replay(
        &replay,
        ApplyFlags::FAIL_HARD,
        &RecordingJournal::default(),
        |_built| StubBuildView::closed(Rc::new(RefCell::new(Vec::new()))),
        |view, tx, flags| {
            view.applied.push(tx.get_transaction_id());
            seen.borrow_mut()
                .push((tx.get_transaction_id(), flags.bits()));
        },
        |_ledger| 0,
        |_ledger| 0,
        |_ledger| {},
    )
    .expect("replay build should succeed");

    assert_eq!(
        seen.into_inner(),
        vec![
            (first.get_transaction_id(), ApplyFlags::FAIL_HARD.bits()),
            (second.get_transaction_id(), ApplyFlags::FAIL_HARD.bits()),
            (third.get_transaction_id(), ApplyFlags::FAIL_HARD.bits()),
        ]
    );
}

#[test]
fn ledger_replay_from_replay_ledger_keeps_first_tx_for_duplicate_index_emplace() {
    let parent = Arc::new(sample_parent_ledger(60));
    let first = sample_tx(7);
    let second = sample_tx(8);
    let replay_ledger = Arc::new(ledger_with_tx_items(
        &[
            (
                Arc::clone(&first),
                SHAMapNodeType::TransactionMd,
                tx_md_payload(&first, 10),
            ),
            (
                Arc::clone(&second),
                SHAMapNodeType::TransactionMd,
                tx_md_payload(&second, 10),
            ),
        ],
        61,
    ));

    let snapshot = replay_ledger
        .tx_snapshot()
        .expect("snapshot should succeed")
        .into_iter()
        .map(|(tx, _meta)| tx.get_transaction_id())
        .collect::<Vec<_>>();
    let replay = LedgerReplay::from_replay_ledger(parent, replay_ledger)
        .expect("replay ledger constructor should succeed");

    assert_eq!(snapshot.len(), 2);
    assert_eq!(replay.ordered_txs().len(), 1);
    assert_eq!(
        replay
            .ordered_txs()
            .get(&10)
            .expect("duplicate index should keep first tx")
            .get_transaction_id(),
        snapshot[0]
    );
}
