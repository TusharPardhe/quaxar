use app::{AppLedgerMasterRuntime, ApplicationRoot, Transaction};
use basics::base_uint::{Uint160, Uint256};
use basics::sha_map_hash::SHAMapHash;
use ledger::{LEDGER_DEFAULT_TIME_RESOLUTION, Ledger, LedgerHeader};
use protocol::{
    AccountID, LedgerEntryType, STAmount, STLedgerEntry, STTx, TxType, account_keylet,
    get_field_by_symbol,
};
use shamap::item::SHAMapItem;
use shamap::mutation::MutableTree;
use shamap::sync::{SHAMapType, SyncState, SyncTree};
use shamap::tree_node::SHAMapNodeType;
use std::sync::Arc;

fn account(hex: &str) -> AccountID {
    AccountID::from_hex(hex).expect("account hex should parse")
}

fn raw_account_id(account: AccountID) -> Uint160 {
    Uint160::from_slice(account.data()).expect("account width should match Uint160")
}

fn payment_tx(
    source: AccountID,
    destination: AccountID,
    sequence: u32,
    ticket_sequence: Option<u32>,
    fee_drops: u64,
) -> Arc<STTx> {
    Arc::new(STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), source);
        tx.set_account_id(get_field_by_symbol("sfDestination"), destination);
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(1_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(fee_drops, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
        if let Some(ticket_sequence) = ticket_sequence {
            tx.set_field_u32(get_field_by_symbol("sfTicketSequence"), ticket_sequence);
        }
    }))
}

fn ledger_view(seq: u32, account: AccountID, account_sequence: u32, tx_ids: &[Uint256]) -> Ledger {
    let mut state_tree = MutableTree::new(1);
    let mut account_root = STLedgerEntry::from_type_and_key(
        LedgerEntryType::AccountRoot,
        account_keylet(raw_account_id(account)).key,
    );
    account_root.set_account_id(get_field_by_symbol("sfAccount"), account);
    account_root.set_field_u32(get_field_by_symbol("sfSequence"), account_sequence);
    state_tree
        .add_item(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(
                account_keylet(raw_account_id(account)).key,
                account_root.get_serializer().data().to_vec(),
            ),
        )
        .expect("account root should insert");

    let mut tx_tree = MutableTree::new(1);
    for (index, tx_id) in tx_ids.iter().enumerate() {
        tx_tree
            .add_item(
                SHAMapNodeType::TransactionNm,
                SHAMapItem::new(*tx_id, vec![index as u8 + 1; 12]),
            )
            .expect("tx should insert");
    }

    Ledger::from_maps(
        LedgerHeader {
            seq,
            close_time: 700 + seq,
            close_time_resolution: LEDGER_DEFAULT_TIME_RESOLUTION,
            ..LedgerHeader::default()
        },
        SyncTree::from_root_with_type(
            state_tree.root(),
            SHAMapType::State,
            false,
            seq,
            SyncState::Modifying,
        ),
        SyncTree::from_root_with_type(
            tx_tree.root(),
            SHAMapType::Transaction,
            false,
            seq,
            SyncState::Modifying,
        ),
    )
}

#[test]
fn app_ledger_master_runtime_updates_local_tx_count_like_networkops_update_local_tx() {
    let runtime = AppLedgerMasterRuntime::default();
    let source = account("1111111111111111111111111111111111111111");
    let tx = payment_tx(
        source,
        account("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
        1,
        None,
        10,
    );
    let tx_id = tx.get_transaction_id();

    runtime.push_local_tx(10, Arc::clone(&tx));
    assert_eq!(runtime.get_local_tx_count(), 1);

    runtime
        .update_local_tx(&ledger_view(11, source, 1, &[tx_id]))
        .expect("local tx sweep should succeed");

    assert_eq!(runtime.get_local_tx_count(), 0);
}

#[test]
fn app_ledger_master_runtime_applies_held_transactions_through_callback() {
    let runtime = AppLedgerMasterRuntime::default();
    let source = account("2222222222222222222222222222222222222222");
    let first = payment_tx(
        source,
        account("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
        1,
        None,
        10,
    );
    let second = payment_tx(
        source,
        account("BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB"),
        2,
        None,
        11,
    );

    runtime.add_held_sttx(Arc::clone(&second));
    runtime.add_held_sttx(Arc::clone(&first));

    let mut seen = Vec::new();
    let drained = runtime.apply_held_transactions(SHAMapHash::new(Uint256::from_u64(61)), |set| {
        seen = set.iter().map(|tx| tx.get_transaction_id()).collect()
    });

    assert_eq!(drained, 2);
    assert_eq!(
        seen,
        vec![first.get_transaction_id(), second.get_transaction_id()]
    );
    assert_eq!(runtime.held_transaction_count(), 0);
}

#[test]
fn application_root_can_own_and_expose_ledger_master_runtime_behaviors() {
    let mut root = ApplicationRoot::new(1).expect("application root");
    let runtime = Arc::new(AppLedgerMasterRuntime::default());
    assert!(
        root.attach_ledger_master_runtime(Arc::clone(&runtime))
            .is_none()
    );
    assert!(root.ledger_master_runtime().is_some());

    let source = account("3333333333333333333333333333333333333333");
    let current = payment_tx(
        source,
        account("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
        5,
        None,
        10,
    );
    let next = payment_tx(
        source,
        account("BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB"),
        6,
        None,
        11,
    );
    let next_tx = Transaction::new(Arc::clone(&next));
    runtime.push_local_tx(10, Arc::clone(&current));
    assert_eq!(root.local_tx_count(), Some(1));

    root.update_local_tx(&ledger_view(11, source, 5, &[current.get_transaction_id()]))
        .expect("local tx update should succeed");
    assert_eq!(root.local_tx_count(), Some(0));

    assert!(root.add_held_transaction(&next_tx));
    assert_eq!(root.held_transaction_count(), Some(1));
    assert_eq!(
        root.pop_acct_transaction(&Transaction::new(Arc::clone(&current)))
            .expect("next sequential transaction should pop")
            .get_transaction_id(),
        next.get_transaction_id()
    );
    assert_eq!(root.held_transaction_count(), Some(0));
}
