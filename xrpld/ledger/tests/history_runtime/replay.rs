use std::sync::Arc;

use basics::base_uint::Uint256;
use ledger::{
    Fees, InboundLedgerReason, Ledger, LedgerConfig, LedgerHeader, LedgerReplay,
    LedgerReplayTaskParameter, LedgerReplayer,
};
use overlay::SimplePeerSetBuilder;
use protocol::{
    AccountID, FeatureSet, STAmount, STArray, STObject, STTx, Serializer, TxType,
    get_field_by_symbol, skip_keylet,
};
use shamap::{
    item::SHAMapItem,
    mutation::MutableTree,
    sync::{SHAMapType, SyncState, SyncTree},
    tree_node::SHAMapNodeType,
};

fn config() -> LedgerConfig {
    LedgerConfig::new(
        Fees {
            base: 10,
            reserve: 20,
            increment: 30,
        },
        FeatureSet::new([]),
    )
}

fn account(fill: u8) -> AccountID {
    AccountID::from_array([fill; 20])
}

fn payment_tx(sequence: u32, account_fill: u8, destination_fill: u8) -> Arc<STTx> {
    Arc::new(STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), account(account_fill));
        tx.set_account_id(
            get_field_by_symbol("sfDestination"),
            account(destination_fill),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(1_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
    }))
}

fn metadata(index: u32, fill: u8) -> STObject {
    let mut final_fields = STObject::new(get_field_by_symbol("sfFinalFields"));
    final_fields.set_account_id(get_field_by_symbol("sfAccount"), account(fill));

    let mut node = STObject::new(get_field_by_symbol("sfModifiedNode"));
    node.set_field_h256(
        get_field_by_symbol("sfLedgerIndex"),
        Uint256::from_array([fill; 32]),
    );
    node.set_field_u16(get_field_by_symbol("sfLedgerEntryType"), 97);
    node.set_field_object(get_field_by_symbol("sfFinalFields"), final_fields);

    let mut affected_nodes = STArray::new(get_field_by_symbol("sfAffectedNodes"));
    affected_nodes.push_back(node);

    let mut meta = STObject::new(get_field_by_symbol("sfTransactionMetaData"));
    meta.set_field_u8(get_field_by_symbol("sfTransactionResult"), 0);
    meta.set_field_u32(get_field_by_symbol("sfTransactionIndex"), index);
    meta.set_field_array(get_field_by_symbol("sfAffectedNodes"), affected_nodes);
    meta
}

fn tx_md_payload(tx: &STTx, meta: &STObject) -> Vec<u8> {
    let tx_bytes = tx.get_serializer().data().to_vec();
    let meta_bytes = meta.get_serializer().data().to_vec();
    let mut serializer = Serializer::new(0);
    serializer.add_vl(&tx_bytes);
    serializer.add_vl(&meta_bytes);
    serializer.data().to_vec()
}

fn closed_ledger_with_txs(items: &[(Arc<STTx>, STObject)], seq: u32) -> Arc<Ledger> {
    let mut tree = MutableTree::new(seq);
    for (tx, meta) in items {
        tree.add_item(
            SHAMapNodeType::TransactionMd,
            SHAMapItem::new(tx.get_transaction_id(), tx_md_payload(tx, meta)),
        )
        .expect("transaction-with-metadata item should insert");
    }

    Arc::new(Ledger::from_maps(
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
    ))
}

#[test]
fn ledger_replay_orders_transactions_by_metadata_index() {
    let parent =
        Arc::new(Ledger::create_genesis(false, &config(), []).expect("genesis should build"));
    let replay = closed_ledger_with_txs(
        &[
            (payment_tx(3, 0x31, 0x41), metadata(9, 0x91)),
            (payment_tx(1, 0x11, 0x21), metadata(2, 0x92)),
            (payment_tx(2, 0x21, 0x31), metadata(5, 0x93)),
        ],
        88,
    );

    let replay_data = LedgerReplay::from_replay_ledger(Arc::clone(&parent), Arc::clone(&replay))
        .expect("replay ordering should build");
    let ordered = replay_data
        .ordered_txs()
        .keys()
        .copied()
        .collect::<Vec<_>>();

    assert_eq!(ordered, vec![2, 5, 9]);
    assert!(Arc::ptr_eq(replay_data.parent(), &parent));
    assert!(Arc::ptr_eq(replay_data.replay(), &replay));
}

#[test]
fn replay_task_parameter_update_and_merge_match_cpp() {
    let finish = Uint256::from_array([0xF1; 32]);
    let mid = Uint256::from_array([0xA2; 32]);
    let start = Uint256::from_array([0xB3; 32]);

    let mut full = LedgerReplayTaskParameter::new(InboundLedgerReason::Generic, finish, 2);
    assert!(full.update(finish, 10, &[mid]));
    assert_eq!(full.start_hash, mid);
    assert_eq!(full.start_seq, 9);

    let smaller = LedgerReplayTaskParameter::new(InboundLedgerReason::Generic, finish, 1);
    assert!(smaller.can_merge_into(&full));

    let mut longer = LedgerReplayTaskParameter::new(InboundLedgerReason::Generic, finish, 3);
    assert!(longer.update(finish, 10, &[start, mid]));
    let nested = LedgerReplayTaskParameter::new(InboundLedgerReason::Generic, mid, 1);
    assert!(nested.can_merge_into(&longer));
}

#[test]
fn replayer_reuses_skip_lists_and_creates_delta_slots() {
    let cfg = config();
    let genesis = Ledger::create_genesis(false, &cfg, []).expect("genesis should build");
    let mut finish = Ledger::from_previous(&genesis, 10);
    finish.update_skip_list().expect("skip list should update");
    let finish_hash = *finish.header().hash.as_uint256();

    let (skip_item, _) = finish
        .state_map()
        .peek_item_with_hash(skip_keylet().key, &mut |_| None)
        .expect("skip list lookup should succeed")
        .expect("skip list entry should exist");

    let mut replayer = LedgerReplayer::new(Arc::new(SimplePeerSetBuilder::new(Vec::new())));
    let task = replayer
        .replay(InboundLedgerReason::Generic, finish_hash, 2)
        .expect("new replay task should be accepted");

    replayer.got_skip_list(finish.header(), &skip_item);

    assert_eq!(replayer.tasks_len(), 1);
    assert_eq!(replayer.skip_lists_len(), 1);
    assert_eq!(replayer.deltas_len(), 1);
    assert!(task.lock().expect("task lock").parameter().full);
    assert!(
        replayer
            .replay(InboundLedgerReason::Generic, finish_hash, 1)
            .is_none()
    );
}

#[test]
fn replayer_stop_propagates_to_active_tasks_shutdown() {
    let cfg = config();
    let genesis = Ledger::create_genesis(false, &cfg, []).expect("genesis should build");
    let mut finish = Ledger::from_previous(&genesis, 10);
    finish.update_skip_list().expect("skip list should update");
    let finish_hash = *finish.header().hash.as_uint256();

    let mut replayer = LedgerReplayer::new(Arc::new(SimplePeerSetBuilder::new(Vec::new())));
    let task = replayer
        .replay(InboundLedgerReason::Generic, finish_hash, 2)
        .expect("new replay task should be accepted");

    replayer.stop();

    assert!(replayer.is_stopped());
    assert_eq!(replayer.tasks_len(), 0);
    assert_eq!(replayer.skip_lists_len(), 0);
    assert_eq!(replayer.deltas_len(), 0);
    assert!(task.lock().expect("task lock").is_stopped());
    assert!(
        replayer
            .replay(InboundLedgerReason::Generic, finish_hash, 1)
            .is_none()
    );
}
