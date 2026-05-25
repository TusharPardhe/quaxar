use std::sync::Arc;

use basics::base_uint::Uint256;
use basics::str_hex::str_hex;
use basics::string_utilities::sql_blob_literal;
use ledger::{AcceptedLedgerTx, Fees, Ledger, LedgerHeader};
use protocol::{
    AccountID, IOUAmount, Issue, JsonOptions, JsonValue, LedgerEntryType, STAmount, STArray,
    STLedgerEntry, STObject, STTx, Serializer, StBase, TxType, account_keylet,
    currency_from_string, get_field_by_symbol,
};
use shamap::{
    item::SHAMapItem,
    mutation::MutableTree,
    sync::{SHAMapType, SyncState, SyncTree},
    tree_node::SHAMapNodeType,
};

fn account(fill: u8) -> AccountID {
    AccountID::from_array([fill; 20])
}

fn hash(fill: u8) -> Uint256 {
    Uint256::from_array([fill; 32])
}

fn account_h160(account_id: AccountID) -> basics::base_uint::Uint160 {
    basics::base_uint::Uint160::from_slice(account_id.data()).expect("account width should match")
}

fn payment_tx() -> Arc<STTx> {
    Arc::new(STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), account(0x11));
        tx.set_account_id(get_field_by_symbol("sfDestination"), account(0x22));
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(1_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), 9);
    }))
}

fn offer_create_tx(account_fill: u8, taker_gets: STAmount) -> Arc<STTx> {
    Arc::new(STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), account(account_fill));
        tx.set_field_amount(get_field_by_symbol("sfTakerGets"), taker_gets);
        tx.set_field_amount(
            get_field_by_symbol("sfTakerPays"),
            STAmount::new_native(10, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(12, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), 3);
    }))
}

fn metadata() -> STObject {
    let usd = currency_from_string("USD");

    let mut payload = STObject::new(get_field_by_symbol("sfFinalFields"));
    payload.set_account_id(get_field_by_symbol("sfAccount"), account(0x33));
    payload.set_field_amount(
        get_field_by_symbol("sfTakerPays"),
        STAmount::from_iou_amount(
            get_field_by_symbol("sfTakerPays"),
            IOUAmount::from_parts(15, 0).expect("IOU amount should normalize"),
            Issue::new(usd, account(0x44)),
        ),
    );

    let mut node = STObject::new(get_field_by_symbol("sfModifiedNode"));
    node.set_field_h256(get_field_by_symbol("sfLedgerIndex"), hash(0xAA));
    node.set_field_u16(
        get_field_by_symbol("sfLedgerEntryType"),
        LedgerEntryType::Offer.code(),
    );
    node.set_field_object(get_field_by_symbol("sfFinalFields"), payload);

    let mut affected_nodes = STArray::new(get_field_by_symbol("sfAffectedNodes"));
    affected_nodes.push_back(node);

    let mut meta = STObject::new(get_field_by_symbol("sfTransactionMetaData"));
    meta.set_field_u8(get_field_by_symbol("sfTransactionResult"), 0);
    meta.set_field_u32(get_field_by_symbol("sfTransactionIndex"), 5);
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

fn account_root_entry(account_id: AccountID, balance: u64, owner_count: u32) -> (Uint256, Vec<u8>) {
    let mut entry = STLedgerEntry::from_type_and_key(
        LedgerEntryType::AccountRoot,
        account_keylet(account_h160(account_id)).key,
    );
    entry.set_account_id(get_field_by_symbol("sfAccount"), account_id);
    entry.set_field_u32(get_field_by_symbol("sfSequence"), 1);
    entry.set_field_amount(
        get_field_by_symbol("sfBalance"),
        STAmount::new_native(balance, false),
    );
    entry.set_field_u32(get_field_by_symbol("sfOwnerCount"), owner_count);
    entry.set_field_h256(get_field_by_symbol("sfPreviousTxnID"), hash(0xCC));
    entry.set_field_u32(get_field_by_symbol("sfPreviousTxnLgrSeq"), 1);
    (
        account_keylet(account_h160(account_id)).key,
        entry.get_serializer().data().to_vec(),
    )
}

fn ledger_with_tx_and_state(
    tx: Arc<STTx>,
    meta: STObject,
    state_items: &[(Uint256, Vec<u8>)],
    fees: Fees,
) -> (Ledger, STObject) {
    let seq = 88;

    let mut state_tree = MutableTree::new(seq);
    for (key, payload) in state_items {
        state_tree
            .add_item(
                SHAMapNodeType::AccountState,
                SHAMapItem::new(*key, payload.clone()),
            )
            .expect("state item should insert");
    }

    let mut tx_tree = MutableTree::new(seq);
    tx_tree
        .add_item(
            SHAMapNodeType::TransactionMd,
            SHAMapItem::new(tx.get_transaction_id(), tx_md_payload(tx.as_ref(), &meta)),
        )
        .expect("transaction-with-metadata item should insert");

    let mut ledger = Ledger::from_maps(
        LedgerHeader {
            seq,
            ..LedgerHeader::default()
        },
        SyncTree::from_root_with_type(
            state_tree.root(),
            SHAMapType::State,
            false,
            seq,
            SyncState::Immutable,
        ),
        SyncTree::from_root_with_type(
            tx_tree.root(),
            SHAMapType::Transaction,
            false,
            seq,
            SyncState::Immutable,
        ),
    );
    ledger.set_fees(fees);
    (ledger, meta)
}

fn accepted_ledger_tx_from_closed_ledger() -> (AcceptedLedgerTx, Arc<STTx>, STObject) {
    let tx = payment_tx();
    let meta = metadata();
    let (ledger, raw_meta) =
        ledger_with_tx_and_state(tx.clone(), meta.clone(), &[], Fees::default());

    let (snapshot_tx, snapshot_meta) = ledger
        .tx_snapshot()
        .expect("closed ledger tx snapshot should succeed")
        .into_iter()
        .next()
        .expect("closed ledger should expose one transaction");

    (
        AcceptedLedgerTx::new(&ledger, snapshot_tx, snapshot_meta)
            .expect("accepted ledger tx should build"),
        tx,
        raw_meta,
    )
}

#[test]
fn accepted_ledger_tx_exposes_transaction_meta_and_result_from_closed_ledger() {
    let (accepted, tx, meta) = accepted_ledger_tx_from_closed_ledger();

    assert_eq!(accepted.get_txn(), tx.as_ref());
    assert_eq!(accepted.get_transaction_id(), tx.get_transaction_id());
    assert_eq!(accepted.get_txn_type(), TxType::PAYMENT);
    assert_eq!(accepted.get_result(), protocol::Ter::TES_SUCCESS);
    assert_eq!(accepted.get_txn_seq(), 5);
    assert_eq!(accepted.get_meta().get_lgr_seq(), 88);
    assert_eq!(
        accepted.get_affected(),
        &std::collections::BTreeSet::from([account(0x33), account(0x44)])
    );
    assert_eq!(
        accepted.get_esc_meta(),
        sql_blob_literal(&meta.get_serializer().data().to_vec())
    );
}

#[test]
fn accepted_ledger_tx_json_matches_current_cpp_shape_from_closed_ledger_data() {
    let (accepted, tx, meta) = accepted_ledger_tx_from_closed_ledger();

    let JsonValue::Object(root) = accepted.get_json() else {
        panic!("accepted ledger tx json should be an object");
    };

    assert_eq!(
        root.get("result"),
        Some(&JsonValue::String(
            "The transaction was applied. Only final in a validated ledger.".into()
        ))
    );
    assert_eq!(
        root.get("raw_meta"),
        Some(&JsonValue::String(str_hex(meta.get_serializer().data())))
    );

    let Some(JsonValue::Array(affected)) = root.get("affected") else {
        panic!("affected array should be present");
    };
    assert_eq!(
        affected,
        &vec![
            JsonValue::String(protocol::to_base58(account(0x33))),
            JsonValue::String(protocol::to_base58(account(0x44))),
        ]
    );

    let Some(JsonValue::Object(transaction)) = root.get("transaction") else {
        panic!("transaction object should be present");
    };
    assert_eq!(
        transaction.get("hash"),
        Some(&JsonValue::String(tx.get_transaction_id().to_string()))
    );

    let Some(JsonValue::Object(meta_json)) = root.get("meta") else {
        panic!("meta object should be present");
    };
    assert_eq!(
        meta_json.get("TransactionResult"),
        Some(&JsonValue::String("tesSUCCESS".to_string()))
    );

    let _ = tx.json(JsonOptions::NONE);
}

#[test]
fn accepted_ledger_tx_offer_create_includes_owner_funds_for_non_self_funded_offer() {
    let tx = offer_create_tx(0x11, STAmount::new_native(50, false));
    let meta = metadata();
    let (ledger, _) = ledger_with_tx_and_state(
        tx,
        meta,
        &[account_root_entry(account(0x11), 1_000, 2)],
        Fees {
            base: 10,
            reserve: 200,
            increment: 50,
        },
    );

    let (snapshot_tx, snapshot_meta) = ledger
        .tx_snapshot()
        .expect("closed ledger tx snapshot should succeed")
        .into_iter()
        .next()
        .expect("closed ledger should expose one transaction");
    let accepted = AcceptedLedgerTx::new(&ledger, snapshot_tx, snapshot_meta)
        .expect("accepted ledger tx should build");

    let JsonValue::Object(root) = accepted.get_json() else {
        panic!("accepted ledger tx json should be an object");
    };
    let Some(JsonValue::Object(transaction)) = root.get("transaction") else {
        panic!("transaction object should be present");
    };

    assert_eq!(
        transaction.get("owner_funds"),
        Some(&JsonValue::String("700".to_string()))
    );
}

#[test]
fn accepted_ledger_tx_offer_create_skips_owner_funds_for_self_funded_offer() {
    let usd = currency_from_string("USD");
    let tx = offer_create_tx(
        0x11,
        STAmount::from_iou_amount(
            get_field_by_symbol("sfTakerGets"),
            IOUAmount::from_parts(25, 0).expect("IOU amount should normalize"),
            Issue::new(usd, account(0x11)),
        ),
    );
    let (ledger, _) = ledger_with_tx_and_state(tx, metadata(), &[], Fees::default());

    let (snapshot_tx, snapshot_meta) = ledger
        .tx_snapshot()
        .expect("closed ledger tx snapshot should succeed")
        .into_iter()
        .next()
        .expect("closed ledger should expose one transaction");
    let accepted = AcceptedLedgerTx::new(&ledger, snapshot_tx, snapshot_meta)
        .expect("accepted ledger tx should build");

    let JsonValue::Object(root) = accepted.get_json() else {
        panic!("accepted ledger tx json should be an object");
    };
    let Some(JsonValue::Object(transaction)) = root.get("transaction") else {
        panic!("transaction object should be present");
    };

    assert!(!transaction.contains_key("owner_funds"));
}
