use std::sync::Arc;

use ledger::{Ledger, LedgerHeader};
use protocol::{STAmount, STArray, STObject, STTx, TxType, get_field_by_symbol};
use shamap::{
    item::SHAMapItem,
    mutation::MutableTree,
    sync::{SHAMapType, SyncState, SyncTree},
    tree_node::SHAMapNodeType,
};

fn account(fill: u8) -> protocol::AccountID {
    protocol::AccountID::from_array([fill; 20])
}

fn payment_tx(sequence: u32, fill: u8) -> Arc<STTx> {
    Arc::new(STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), account(fill));
        tx.set_account_id(
            get_field_by_symbol("sfDestination"),
            account(fill.wrapping_add(1)),
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
fn ledger_tx_read_transaction_md_decodes_sttx_and_metadata() {
    let tx = payment_tx(1, 0x11);
    let ledger = ledger_with_tx_items(
        &[(
            Arc::clone(&tx),
            SHAMapNodeType::TransactionMd,
            tx_md_payload(&tx, 7),
        )],
        88,
    );

    let (decoded_tx, meta) = ledger
        .tx_read(tx.get_transaction_id())
        .expect("tx read should succeed")
        .expect("tx should exist");

    assert_eq!(decoded_tx.get_transaction_id(), tx.get_transaction_id());
    assert_eq!(meta.get_index(), 7);
    assert_eq!(meta.get_lgr_seq(), 88);
}

#[test]
fn ledger_tx_read_rejects_transaction_nm_payload_on_closed_ledger_contract() {
    let tx = payment_tx(2, 0x21);
    let ledger = ledger_with_tx_items(
        &[(
            Arc::clone(&tx),
            SHAMapNodeType::TransactionNm,
            tx.get_serializer().data().to_vec(),
        )],
        89,
    );

    let result = ledger.tx_read(tx.get_transaction_id());

    assert!(result.is_err());
}

#[test]
fn ledger_tx_snapshot_matches_txmap_iteration_order() {
    let first = payment_tx(3, 0x31);
    let second = payment_tx(4, 0x41);
    let third = payment_tx(5, 0x51);
    let ledger = ledger_with_tx_items(
        &[
            (
                Arc::clone(&second),
                SHAMapNodeType::TransactionMd,
                tx_md_payload(&second, 20),
            ),
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
        ],
        90,
    );

    let ordered_ids: Vec<_> = ledger
        .tx_snapshot()
        .expect("tx snapshot should succeed")
        .into_iter()
        .map(|(tx, _meta)| tx.get_transaction_id())
        .collect();

    let mut expected = vec![
        first.get_transaction_id(),
        second.get_transaction_id(),
        third.get_transaction_id(),
    ];
    expected.sort();

    assert_eq!(ordered_ids, expected);
}
