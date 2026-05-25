use std::sync::Arc;

use basics::base_uint::{Uint160, Uint256};
use ledger::{Ledger, LedgerHeader, LocalTxs};
use protocol::{
    AccountID, LedgerEntryType, STAmount, STLedgerEntry, STTx, SeqProxy, TxType, account_keylet,
    get_field_by_symbol, ticket_keylet_from_seq_proxy,
};
use shamap::item::SHAMapItem;
use shamap::mutation::MutableTree;
use shamap::sync::{SHAMapType, SyncState, SyncTree};
use shamap::tree_node::SHAMapNodeType;

fn account(hex: &str) -> AccountID {
    AccountID::from_hex(hex).expect("account hex should parse")
}

fn raw_account(account: AccountID) -> Uint160 {
    Uint160::from_slice(account.data()).expect("AccountID width should match Uint160")
}

fn payment_tx(
    source: AccountID,
    destination: AccountID,
    sequence: u32,
    ticket_sequence: Option<u32>,
    last_ledger_sequence: Option<u32>,
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
            STAmount::new_native(10, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
        if let Some(ticket_sequence) = ticket_sequence {
            tx.set_field_u32(get_field_by_symbol("sfTicketSequence"), ticket_sequence);
        }
        if let Some(last_ledger_sequence) = last_ledger_sequence {
            tx.set_field_u32(
                get_field_by_symbol("sfLastLedgerSequence"),
                last_ledger_sequence,
            );
        }
    }))
}

fn account_root_entry(account: AccountID, sequence: u32) -> STLedgerEntry {
    let mut entry = STLedgerEntry::from_type_and_key(
        LedgerEntryType::AccountRoot,
        account_keylet(raw_account(account)).key,
    );
    entry.set_account_id(get_field_by_symbol("sfAccount"), account);
    entry.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
    entry.set_field_amount(
        get_field_by_symbol("sfBalance"),
        STAmount::new_native(10_000_000, false),
    );
    entry
}

fn ticket_entry(account: AccountID, ticket_seq: SeqProxy) -> STLedgerEntry {
    let mut entry = STLedgerEntry::from_type_and_key(
        LedgerEntryType::Ticket,
        ticket_keylet_from_seq_proxy(raw_account(account), ticket_seq).key,
    );
    entry.set_account_id(get_field_by_symbol("sfAccount"), account);
    entry.set_field_u32(get_field_by_symbol("sfTicketSequence"), ticket_seq.value());
    entry
}

fn state_tree(entries: impl IntoIterator<Item = STLedgerEntry>, ledger_seq: u32) -> SyncTree {
    let mut tree = MutableTree::new(1);
    let mut any = false;

    for entry in entries {
        any = true;
        tree.add_item(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(*entry.key(), entry.get_serializer().data().to_vec()),
        )
        .expect("state entry should insert");
    }

    if !any {
        return SyncTree::new_with_type(SHAMapType::State, false, ledger_seq);
    }

    SyncTree::from_root_with_type(
        tree.root(),
        SHAMapType::State,
        false,
        ledger_seq,
        SyncState::Modifying,
    )
}

fn tx_tree(entries: impl IntoIterator<Item = Uint256>, ledger_seq: u32) -> SyncTree {
    let mut tree = MutableTree::new(1);
    let mut any = false;

    for tx_id in entries {
        any = true;
        tree.add_item(
            SHAMapNodeType::TransactionNm,
            SHAMapItem::new(tx_id, vec![0xAA; 12]),
        )
        .expect("tx entry should insert");
    }

    if !any {
        return SyncTree::new_with_type(SHAMapType::Transaction, false, ledger_seq);
    }

    SyncTree::from_root_with_type(
        tree.root(),
        SHAMapType::Transaction,
        false,
        ledger_seq,
        SyncState::Modifying,
    )
}

fn ledger_view(
    ledger_seq: u32,
    state_entries: impl IntoIterator<Item = STLedgerEntry>,
    tx_ids: impl IntoIterator<Item = Uint256>,
) -> Ledger {
    Ledger::from_maps(
        LedgerHeader {
            seq: ledger_seq,
            ..LedgerHeader::default()
        },
        state_tree(state_entries, ledger_seq),
        tx_tree(tx_ids, ledger_seq),
    )
}

#[test]
fn ledger_local_txs_get_tx_set_uses_zero_salt_and_canonical_order() {
    let source = account("1111111111111111111111111111111111111111");
    let destination = account("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA");
    let sequence_tx = payment_tx(source, destination, 2, None, None);
    let ticket_tx = payment_tx(source, destination, 0, Some(1), None);

    let local = LocalTxs::new();
    local.push_back(10, Arc::clone(&ticket_tx));
    local.push_back(10, Arc::clone(&sequence_tx));

    let set = local.get_tx_set();
    let ordered: Vec<SeqProxy> = set.iter().map(|tx| tx.get_seq_proxy()).collect();

    assert_eq!(set.key(), Uint256::zero());
    assert_eq!(ordered, vec![SeqProxy::sequence(2), SeqProxy::ticket(1)]);
}

#[test]
fn ledger_local_txs_sweep_expires_after_hold_ledgers() {
    let source = account("1111111111111111111111111111111111111111");
    let tx = payment_tx(
        source,
        account("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
        5,
        None,
        None,
    );
    let local = LocalTxs::new();
    local.push_back(10, tx);

    local
        .sweep(&ledger_view(15, [], []))
        .expect("sweep should succeed");
    assert_eq!(local.size(), 1);

    local
        .sweep(&ledger_view(16, [], []))
        .expect("sweep should succeed");
    assert_eq!(local.size(), 0);
}

#[test]
fn ledger_local_txs_sweep_uses_last_ledger_sequence_plus_one() {
    let source = account("1111111111111111111111111111111111111111");
    let tx = payment_tx(
        source,
        account("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
        5,
        None,
        Some(11),
    );
    let local = LocalTxs::new();
    local.push_back(10, tx);

    local
        .sweep(&ledger_view(12, [], []))
        .expect("sweep should succeed");
    assert_eq!(local.size(), 1);

    local
        .sweep(&ledger_view(13, [], []))
        .expect("sweep should succeed");
    assert_eq!(local.size(), 0);
}

#[test]
fn ledger_local_txs_sweep_removes_validated_ledger_transactions() {
    let source = account("1111111111111111111111111111111111111111");
    let tx = payment_tx(
        source,
        account("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
        5,
        None,
        None,
    );
    let tx_id = tx.get_transaction_id();
    let local = LocalTxs::new();
    local.push_back(10, tx);

    local
        .sweep(&ledger_view(11, [], [tx_id]))
        .expect("sweep should succeed");
    assert_eq!(local.size(), 0);
}

#[test]
fn ledger_local_txs_sweep_keeps_tx_when_account_root_is_missing() {
    let source = account("1111111111111111111111111111111111111111");
    let tx = payment_tx(
        source,
        account("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
        5,
        None,
        None,
    );
    let local = LocalTxs::new();
    local.push_back(10, tx);

    local
        .sweep(&ledger_view(11, [], []))
        .expect("sweep should succeed");
    assert_eq!(local.size(), 1);
}

#[test]
fn ledger_local_txs_sweep_removes_past_sequence_transactions() {
    let source = account("1111111111111111111111111111111111111111");
    let tx = payment_tx(
        source,
        account("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
        7,
        None,
        None,
    );
    let local = LocalTxs::new();
    local.push_back(10, tx);

    let account_root = account_root_entry(source, 8);
    local
        .sweep(&ledger_view(11, [account_root], []))
        .expect("sweep should succeed");
    assert_eq!(local.size(), 0);
}

#[test]
fn ledger_local_txs_sweep_keeps_future_tickets() {
    let source = account("1111111111111111111111111111111111111111");
    let ticket_tx = payment_tx(
        source,
        account("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
        0,
        Some(9),
        None,
    );
    let local = LocalTxs::new();
    local.push_back(10, ticket_tx);

    let account_root = account_root_entry(source, 5);
    local
        .sweep(&ledger_view(11, [account_root], []))
        .expect("sweep should succeed");
    assert_eq!(local.size(), 1);
}

#[test]
fn ledger_local_txs_sweep_removes_missing_created_tickets() {
    let source = account("1111111111111111111111111111111111111111");
    let ticket_tx = payment_tx(
        source,
        account("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
        0,
        Some(9),
        None,
    );
    let local = LocalTxs::new();
    local.push_back(10, ticket_tx);

    let account_root = account_root_entry(source, 10);
    local
        .sweep(&ledger_view(11, [account_root], []))
        .expect("sweep should succeed");
    assert_eq!(local.size(), 0);
}

#[test]
fn ledger_local_txs_sweep_keeps_present_created_tickets() {
    let source = account("1111111111111111111111111111111111111111");
    let ticket_seq = SeqProxy::ticket(9);
    let ticket_tx = payment_tx(
        source,
        account("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
        0,
        Some(ticket_seq.value()),
        None,
    );
    let local = LocalTxs::new();
    local.push_back(10, ticket_tx);

    let account_root = account_root_entry(source, 10);
    let ticket = ticket_entry(source, ticket_seq);
    local
        .sweep(&ledger_view(11, [account_root, ticket], []))
        .expect("sweep should succeed");
    assert_eq!(local.size(), 1);
}
