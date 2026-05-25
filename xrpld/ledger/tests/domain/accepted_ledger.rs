use std::sync::Arc;

use ledger::{AcceptedLedger, Ledger, LedgerHeader};
use protocol::{
    AccountID, JsonOptions, JsonValue, STAmount, STArray, STObject, STTx, Serializer, StBase,
    TxType, get_field_by_symbol,
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
        basics::base_uint::Uint256::from_array([fill; 32]),
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
fn accepted_ledger_sorts_transactions_by_meta_index_from_real_closed_ledger() {
    let ledger = closed_ledger_with_txs(
        &[
            (payment_tx(3, 0x31, 0x41), metadata(9, 0x91)),
            (payment_tx(1, 0x11, 0x21), metadata(2, 0x92)),
            (payment_tx(2, 0x21, 0x31), metadata(5, 0x93)),
        ],
        88,
    );

    let accepted = AcceptedLedger::new(Arc::clone(&ledger))
        .expect("accepted ledger should build from a closed ledger");
    let ordered: Vec<_> = accepted.iter().map(|tx| tx.get_txn_seq()).collect();

    assert!(Arc::ptr_eq(accepted.get_ledger(), &ledger));
    assert_eq!(accepted.size(), 3);
    assert_eq!(ordered, vec![2, 5, 9]);

    let transaction_accounts: Vec<_> = accepted
        .iter()
        .map(|tx| tx.get_txn().json(JsonOptions::NONE))
        .map(|json| match json {
            JsonValue::Object(object) => object
                .get("Account")
                .and_then(|value| match value {
                    JsonValue::String(value) => Some(value.clone()),
                    _ => None,
                })
                .expect("transaction account should serialize as a string"),
            _ => panic!("transaction json should be an object"),
        })
        .collect();

    assert_eq!(
        transaction_accounts,
        vec![
            protocol::to_base58(account(0x11)),
            protocol::to_base58(account(0x21)),
            protocol::to_base58(account(0x31)),
        ]
    );
}

#[test]
fn accepted_ledger_iterates_begin_end_over_same_sorted_slice() {
    let ledger = closed_ledger_with_txs(
        &[
            (payment_tx(8, 0x51, 0x61), metadata(3, 0xA1)),
            (payment_tx(7, 0x41, 0x51), metadata(1, 0xA2)),
        ],
        90,
    );
    let accepted = AcceptedLedger::new(ledger).expect("accepted ledger should build");

    let via_iter: Vec<_> = accepted.iter().map(|tx| tx.get_transaction_id()).collect();
    let via_into_iter: Vec<_> = (&accepted)
        .into_iter()
        .map(|tx| tx.get_transaction_id())
        .collect();
    let via_slice: Vec<_> = accepted
        .as_slice()
        .iter()
        .map(|tx| tx.get_transaction_id())
        .collect();

    assert_eq!(accepted.size(), 2);
    assert!(!accepted.is_empty());
    assert_eq!(via_iter, via_into_iter);
    assert_eq!(via_iter, via_slice);
}

#[test]
fn accepted_ledger_keeps_empty_closed_ledgers_empty() {
    let accepted = AcceptedLedger::new(Arc::new(Ledger::new(
        LedgerHeader {
            seq: 91,
            ..LedgerHeader::default()
        },
        false,
    )))
    .expect("empty closed ledger should build");

    assert_eq!(accepted.size(), 0);
    assert!(accepted.is_empty());
    assert!(accepted.iter().next().is_none());
}
