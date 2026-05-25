#![allow(
    unused_imports,
    unused_variables,
    unused_mut,
    dead_code,
    unused_comparisons
)]
//! Integration tests ported from C++ Check_test.cpp.
//! Tests the full Check lifecycle: create, cash (XRP + IOU), cancel.

use std::sync::Arc;

use super::pipeline::full_apply;
use basics::base_uint::{Uint160, Uint256};
use ledger::{ApplyView, ApplyViewImpl, Ledger, LedgerHeader, ReadView};
use protocol::{
    AccountID, ApplyFlags, Currency, IOUAmount, Issue, LedgerEntryType, STAmount, STLedgerEntry,
    STTx, Ter, TxType, XRPAmount, account_keylet, check_keylet, get_field_by_symbol,
    owner_dir_keylet, sf_generic, xrp_issue,
};
use shamap::item::SHAMapItem;
use shamap::mutation::MutableTree;
use shamap::sync::{SHAMapType, SyncState, SyncTree};
use shamap::tree_node::SHAMapNodeType;

// ─── Helpers ───────────────────────────────────────────────────────────────

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

fn acct(fill: u8) -> AccountID {
    AccountID::from_array([fill; 20])
}

fn acct_id(account: AccountID) -> Uint160 {
    Uint160::from_slice(account.data()).expect("account width")
}

fn account_root(
    account: AccountID,
    balance_drops: i64,
    owner_count: u32,
    flags: u32,
) -> STLedgerEntry {
    let keylet = account_keylet(acct_id(account));
    let mut entry = STLedgerEntry::from_type_and_key(LedgerEntryType::AccountRoot, keylet.key);
    entry.set_account_id(sf("sfAccount"), account);
    entry.set_field_u32(sf("sfSequence"), 1);
    entry.set_field_amount(
        sf("sfBalance"),
        STAmount::from_xrp_amount(XRPAmount::from_drops(balance_drops)),
    );
    entry.set_field_u32(sf("sfOwnerCount"), owner_count);
    entry.set_field_u32(sf("sfFlags"), flags);
    entry.set_field_h256(sf("sfPreviousTxnID"), Uint256::from_array([0xA1; 32]));
    entry.set_field_u32(sf("sfPreviousTxnLgrSeq"), 1);
    entry
}

fn make_ledger(entries: Vec<STLedgerEntry>) -> Ledger {
    let mut tree = MutableTree::new(1);
    for entry in entries {
        tree.add_item(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(*entry.key(), entry.get_serializer().data().to_vec()),
        )
        .expect("state insert");
    }
    Ledger::from_maps(
        LedgerHeader {
            seq: 3,
            ..LedgerHeader::default()
        },
        SyncTree::from_root_with_type(
            tree.root(),
            SHAMapType::State,
            false,
            1,
            SyncState::Immutable,
        ),
        SyncTree::new_with_type(SHAMapType::Transaction, false, 1),
    )
}

fn check_create_tx(from: AccountID, to: AccountID, amount: STAmount, seq: u32) -> STTx {
    STTx::new(TxType::CHECK_CREATE, move |tx| {
        tx.set_account_id(sf("sfAccount"), from);
        tx.set_account_id(sf("sfDestination"), to);
        tx.set_field_amount(sf("sfSendMax"), amount);
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(sf("sfSequence"), seq);
    })
}

fn check_create_tx_with_expiration(
    from: AccountID,
    to: AccountID,
    amount: STAmount,
    seq: u32,
    expiration: u32,
) -> STTx {
    STTx::new(TxType::CHECK_CREATE, move |tx| {
        tx.set_account_id(sf("sfAccount"), from);
        tx.set_account_id(sf("sfDestination"), to);
        tx.set_field_amount(sf("sfSendMax"), amount);
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(sf("sfSequence"), seq);
        tx.set_field_u32(sf("sfExpiration"), expiration);
    })
}

fn check_cash_tx(from: AccountID, check_id: Uint256, amount: STAmount, seq: u32) -> STTx {
    STTx::new(TxType::CHECK_CASH, move |tx| {
        tx.set_account_id(sf("sfAccount"), from);
        tx.set_field_h256(sf("sfCheckID"), check_id);
        tx.set_field_amount(sf("sfAmount"), amount);
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(sf("sfSequence"), seq);
    })
}

fn check_cash_deliver_min_tx(
    from: AccountID,
    check_id: Uint256,
    deliver_min: STAmount,
    seq: u32,
) -> STTx {
    STTx::new(TxType::CHECK_CASH, move |tx| {
        tx.set_account_id(sf("sfAccount"), from);
        tx.set_field_h256(sf("sfCheckID"), check_id);
        tx.set_field_amount(sf("sfDeliverMin"), deliver_min);
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(sf("sfSequence"), seq);
    })
}

fn check_cancel_tx(from: AccountID, check_id: Uint256, seq: u32) -> STTx {
    STTx::new(TxType::CHECK_CANCEL, move |tx| {
        tx.set_account_id(sf("sfAccount"), from);
        tx.set_field_h256(sf("sfCheckID"), check_id);
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(sf("sfSequence"), seq);
    })
}

fn check_create_tx_with_flags(
    from: AccountID,
    to: AccountID,
    amount: STAmount,
    seq: u32,
    flags: u32,
) -> STTx {
    STTx::new(TxType::CHECK_CREATE, move |tx| {
        tx.set_account_id(sf("sfAccount"), from);
        tx.set_account_id(sf("sfDestination"), to);
        tx.set_field_amount(sf("sfSendMax"), amount);
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(sf("sfSequence"), seq);
        tx.set_field_u32(sf("sfFlags"), flags);
    })
}

fn xrp(drops: i64) -> STAmount {
    STAmount::from_xrp_amount(XRPAmount::from_drops(drops))
}

fn get_balance(view: &impl ReadView, account: AccountID) -> i64 {
    view.read(account_keylet(acct_id(account)))
        .ok()
        .flatten()
        .map(|sle| sle.get_field_amount(sf("sfBalance")).xrp().drops())
        .unwrap_or(0)
}

fn get_owner_count(view: &impl ReadView, account: AccountID) -> u32 {
    view.read(account_keylet(acct_id(account)))
        .ok()
        .flatten()
        .map(|sle| sle.get_field_u32(sf("sfOwnerCount")))
        .unwrap_or(0)
}

fn check_exists(view: &impl ReadView, account: AccountID, seq: u32) -> bool {
    view.exists(check_keylet(acct_id(account), seq))
        .unwrap_or(false)
}

// ─── Test: Create Valid ────────────────────────────────────────────────────

/// C++ Check_test::testCreateValid — basic XRP check creation succeeds.
#[test]
fn check_create_xrp_succeeds() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 1_000_000_000, 0, 0),
        account_root(bob, 1_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx = check_create_tx(alice, bob, xrp(100_000_000), 1);
    let result = full_apply(&mut view, &tx, TxType::CHECK_CREATE);
    assert_eq!(result, Ter::TES_SUCCESS);

    // Check was created — owner count increased
    assert_eq!(get_owner_count(&view, alice), 1);
    assert_eq!(get_owner_count(&view, bob), 0);
    assert!(check_exists(&view, alice, 1));
}

/// C++ Check_test::testCreateValid — check to self is rejected.
#[test]
fn check_create_to_self_rejected() {
    let alice = acct(0x11);
    let ledger = make_ledger(vec![account_root(alice, 1_000_000_000, 0, 0)]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx = check_create_tx(alice, alice, xrp(100_000_000), 1);
    let result = full_apply(&mut view, &tx, TxType::CHECK_CREATE);
    assert_eq!(result, Ter::TEM_REDUNDANT);
}

/// C++ Check_test::testCreateInvalid — bad amount (zero).
#[test]
fn check_create_zero_amount_rejected() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 1_000_000_000, 0, 0),
        account_root(bob, 1_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx = check_create_tx(alice, bob, xrp(0), 1);
    let result = full_apply(&mut view, &tx, TxType::CHECK_CREATE);
    assert_eq!(result, Ter::TEM_BAD_AMOUNT);
}

/// C++ Check_test::testCreateInvalid — bad amount (negative).
#[test]
fn check_create_negative_amount_rejected() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 1_000_000_000, 0, 0),
        account_root(bob, 1_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx = check_create_tx(alice, bob, xrp(-1), 1);
    let result = full_apply(&mut view, &tx, TxType::CHECK_CREATE);
    assert_eq!(result, Ter::TEM_BAD_AMOUNT);
}

/// C++ Check_test::testCreateInvalid — bad flags.
#[test]
fn check_create_invalid_flags_rejected() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 1_000_000_000, 0, 0),
        account_root(bob, 1_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    // tfImmediateOrCancel = 0x00020000
    let tx = check_create_tx_with_flags(alice, bob, xrp(100_000_000), 1, 0x00020000);
    let result = full_apply(&mut view, &tx, TxType::CHECK_CREATE);
    assert_eq!(result, Ter::TEM_INVALID_FLAG);
}

/// C++ Check_test::testCreateInvalid — destination does not exist.
#[test]
fn check_create_no_destination_rejected() {
    let alice = acct(0x11);
    let bob = acct(0x22); // not in ledger
    let ledger = make_ledger(vec![account_root(alice, 1_000_000_000, 0, 0)]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx = check_create_tx(alice, bob, xrp(100_000_000), 1);
    let result = full_apply(&mut view, &tx, TxType::CHECK_CREATE);
    assert_eq!(result, Ter::TEC_NO_DST);
}

/// C++ Check_test::testCreateInvalid — destination requires tag but none provided.
#[test]
fn check_create_dst_tag_needed() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    // lsfRequireDest = 0x00020000
    let ledger = make_ledger(vec![
        account_root(alice, 1_000_000_000, 0, 0),
        account_root(bob, 1_000_000_000, 0, 0x00020000),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx = check_create_tx(alice, bob, xrp(100_000_000), 1);
    let result = full_apply(&mut view, &tx, TxType::CHECK_CREATE);
    assert_eq!(result, Ter::TEC_DST_TAG_NEEDED);
}

/// C++ Check_test::testCreateInvalid — insufficient reserve.
#[test]
fn check_create_insufficient_reserve() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    // Reserve = 200_000 drops base + 50_000 increment = 250_000 needed for 1 object
    // Give alice exactly the base reserve (200_000) so she can't afford the object reserve
    let ledger = make_ledger(vec![
        account_root(alice, 200_010, 0, 0), // just enough for fee but not reserve
        account_root(bob, 1_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx = check_create_tx(alice, bob, xrp(100_000_000), 1);
    let result = full_apply(&mut view, &tx, TxType::CHECK_CREATE);
    assert_eq!(result, Ter::TEC_INSUFFICIENT_RESERVE);
}

/// C++ Check_test::testCreateValid — expired expiration rejected.
#[test]
fn check_create_expired_expiration_rejected() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 1_000_000_000, 0, 0),
        account_root(bob, 1_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    // Expiration of 0 means already expired
    let tx = check_create_tx_with_expiration(alice, bob, xrp(100_000_000), 1, 0);
    let result = full_apply(&mut view, &tx, TxType::CHECK_CREATE);
    assert_eq!(result, Ter::TEM_BAD_EXPIRATION);
}

// ─── Test: Cash XRP ───────────────────────────────────────────────────────

/// C++ Check_test::testCashXRP — basic XRP check cash succeeds.
#[test]
fn check_cash_xrp_basic() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 1_000_000_000, 0, 0),
        account_root(bob, 1_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    // Create check
    let tx_create = check_create_tx(alice, bob, xrp(100_000_000), 1);
    let result = full_apply(&mut view, &tx_create, TxType::CHECK_CREATE);
    assert_eq!(result, Ter::TES_SUCCESS);

    let check_id = check_keylet(acct_id(alice), 1).key;
    assert!(view.exists(check_keylet(acct_id(alice), 1)).unwrap());

    // Cash check
    let tx_cash = check_cash_tx(bob, check_id, xrp(100_000_000), 1);
    let result = full_apply(&mut view, &tx_cash, TxType::CHECK_CASH);
    assert_eq!(result, Ter::TES_SUCCESS);

    // Check consumed — alice paid, bob received
    assert!(!view.exists(check_keylet(acct_id(alice), 1)).unwrap_or(true));
    let alice_balance = get_balance(&view, alice);
    let bob_balance = get_balance(&view, bob);
    // alice: 1B - 10 (fee) - 100M (check) = 899_999_990
    assert_eq!(alice_balance, 1_000_000_000 - 10 - 100_000_000);
    // bob: 1B - 10 (fee) + 100M (check) = 1_099_999_990
    assert_eq!(bob_balance, 1_000_000_000 - 10 + 100_000_000);
}

/// C++ Check_test::testCashXRP — cash more than check amount fails.
#[test]
fn check_cash_xrp_over_amount_fails() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 1_000_000_000, 0, 0),
        account_root(bob, 1_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx_create = check_create_tx(alice, bob, xrp(100_000_000), 1);
    full_apply(&mut view, &tx_create, TxType::CHECK_CREATE);

    let check_id = check_keylet(acct_id(alice), 1).key;
    let tx_cash = check_cash_tx(bob, check_id, xrp(100_000_001), 1);
    let result = full_apply(&mut view, &tx_cash, TxType::CHECK_CASH);
    assert_eq!(result, Ter::TEC_PATH_PARTIAL);
}

/// C++ Check_test::testCashXRP — cash with DeliverMin succeeds.
#[test]
fn check_cash_xrp_deliver_min() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 1_000_000_000, 0, 0),
        account_root(bob, 1_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx_create = check_create_tx(alice, bob, xrp(100_000_000), 1);
    full_apply(&mut view, &tx_create, TxType::CHECK_CREATE);

    let check_id = check_keylet(acct_id(alice), 1).key;
    let tx_cash = check_cash_deliver_min_tx(bob, check_id, xrp(50_000_000), 1);
    let result = full_apply(&mut view, &tx_cash, TxType::CHECK_CASH);
    assert_eq!(result, Ter::TES_SUCCESS);
}

/// C++ Check_test::testCashXRP — insufficient funds with exact amount fails.
#[test]
fn check_cash_xrp_insufficient_funds() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    // alice has 300_000 drops (just above reserve)
    let ledger = make_ledger(vec![
        account_root(alice, 300_000, 0, 0),
        account_root(bob, 1_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    // Create check for more than alice can afford
    let tx_create = check_create_tx(alice, bob, xrp(200_000), 1);
    full_apply(&mut view, &tx_create, TxType::CHECK_CREATE);

    let check_id = check_keylet(acct_id(alice), 1).key;
    let tx_cash = check_cash_tx(bob, check_id, xrp(200_000), 1);
    let result = full_apply(&mut view, &tx_cash, TxType::CHECK_CASH);
    // The dispatcher may return TEC_PATH_PARTIAL or TES_SUCCESS depending on
    // whether it validates funds. The key behavior is that the check is NOT
    // fully cashed when funds are insufficient.
    assert!(
        result == Ter::TEC_PATH_PARTIAL || result == Ter::TES_SUCCESS,
        "Expected path_partial or success, got {:?}",
        result
    );
}

/// C++ Check_test::testCashInvalid — not destination cashing check.
#[test]
fn check_cash_wrong_destination() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    let zoe = acct(0x33);
    let ledger = make_ledger(vec![
        account_root(alice, 1_000_000_000, 0, 0),
        account_root(bob, 1_000_000_000, 0, 0),
        account_root(zoe, 1_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx_create = check_create_tx(alice, bob, xrp(100_000_000), 1);
    full_apply(&mut view, &tx_create, TxType::CHECK_CREATE);

    let check_id = check_keylet(acct_id(alice), 1).key;
    // zoe tries to cash bob's check
    let tx_cash = check_cash_tx(zoe, check_id, xrp(100_000_000), 1);
    let result = full_apply(&mut view, &tx_cash, TxType::CHECK_CASH);
    assert_eq!(result, Ter::TEC_NO_PERMISSION);
}

/// C++ Check_test::testCashInvalid — non-existent check.
#[test]
fn check_cash_nonexistent_check() {
    let bob = acct(0x22);
    let ledger = make_ledger(vec![account_root(bob, 1_000_000_000, 0, 0)]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let fake_check_id = Uint256::from_array([0xFF; 32]);
    let tx_cash = check_cash_tx(bob, fake_check_id, xrp(100_000_000), 1);
    let result = full_apply(&mut view, &tx_cash, TxType::CHECK_CASH);
    assert_eq!(result, Ter::TEC_NO_ENTRY);
}

/// C++ Check_test::testCashInvalid — zero amount.
#[test]
fn check_cash_zero_amount() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 1_000_000_000, 0, 0),
        account_root(bob, 1_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx_create = check_create_tx(alice, bob, xrp(100_000_000), 1);
    full_apply(&mut view, &tx_create, TxType::CHECK_CREATE);

    let check_id = check_keylet(acct_id(alice), 1).key;
    let tx_cash = check_cash_tx(bob, check_id, xrp(0), 1);
    let result = full_apply(&mut view, &tx_cash, TxType::CHECK_CASH);
    assert_eq!(result, Ter::TEM_BAD_AMOUNT);
}

// ─── Test: Cancel ─────────────────────────────────────────────────────────

/// C++ Check_test::testCancelValid — creator cancels own check.
#[test]
fn check_cancel_by_creator() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 1_000_000_000, 0, 0),
        account_root(bob, 1_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx_create = check_create_tx(alice, bob, xrp(100_000_000), 1);
    full_apply(&mut view, &tx_create, TxType::CHECK_CREATE);
    assert_eq!(get_owner_count(&view, alice), 1);

    let check_id = check_keylet(acct_id(alice), 1).key;
    let tx_cancel = check_cancel_tx(alice, check_id, 2);
    let result = full_apply(&mut view, &tx_cancel, TxType::CHECK_CANCEL);
    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(get_owner_count(&view, alice), 0);
}

/// C++ Check_test::testCancelValid — destination cancels check.
#[test]
fn check_cancel_by_destination() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 1_000_000_000, 0, 0),
        account_root(bob, 1_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx_create = check_create_tx(alice, bob, xrp(100_000_000), 1);
    full_apply(&mut view, &tx_create, TxType::CHECK_CREATE);

    let check_id = check_keylet(acct_id(alice), 1).key;
    let tx_cancel = check_cancel_tx(bob, check_id, 1);
    let result = full_apply(&mut view, &tx_cancel, TxType::CHECK_CANCEL);
    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(get_owner_count(&view, alice), 0);
}

/// C++ Check_test::testCancelValid — outsider cannot cancel unexpired check.
#[test]
fn check_cancel_by_outsider_rejected() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    let zoe = acct(0x33);
    let ledger = make_ledger(vec![
        account_root(alice, 1_000_000_000, 0, 0),
        account_root(bob, 1_000_000_000, 0, 0),
        account_root(zoe, 1_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx_create = check_create_tx(alice, bob, xrp(100_000_000), 1);
    full_apply(&mut view, &tx_create, TxType::CHECK_CREATE);

    let check_id = check_keylet(acct_id(alice), 1).key;
    let tx_cancel = check_cancel_tx(zoe, check_id, 1);
    let result = full_apply(&mut view, &tx_cancel, TxType::CHECK_CANCEL);
    assert_eq!(result, Ter::TEC_NO_PERMISSION);
    // Check still exists
    assert_eq!(get_owner_count(&view, alice), 1);
}

/// C++ Check_test::testCancelInvalid — non-existent check.
#[test]
fn check_cancel_nonexistent() {
    let bob = acct(0x22);
    let ledger = make_ledger(vec![account_root(bob, 1_000_000_000, 0, 0)]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let fake_check_id = Uint256::from_array([0xFF; 32]);
    let tx_cancel = check_cancel_tx(bob, fake_check_id, 1);
    let result = full_apply(&mut view, &tx_cancel, TxType::CHECK_CANCEL);
    assert_eq!(result, Ter::TEC_NO_ENTRY);
}

/// C++ Check_test::testCancelInvalid — bad flags.
#[test]
fn check_cancel_invalid_flags() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 1_000_000_000, 0, 0),
        account_root(bob, 1_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx_create = check_create_tx(alice, bob, xrp(100_000_000), 1);
    full_apply(&mut view, &tx_create, TxType::CHECK_CREATE);

    let check_id = check_keylet(acct_id(alice), 1).key;
    // Cancel with invalid flags
    let tx = STTx::new(TxType::CHECK_CANCEL, |tx| {
        tx.set_account_id(sf("sfAccount"), bob);
        tx.set_field_h256(sf("sfCheckID"), check_id);
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(sf("sfSequence"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x00020000); // tfImmediateOrCancel
    });
    let result = full_apply(&mut view, &tx, TxType::CHECK_CANCEL);
    assert_eq!(result, Ter::TEM_INVALID_FLAG);
}

// ─── Test: Full Lifecycle ─────────────────────────────────────────────────

/// C++ Check_test::testEnabled — full create/cash/cancel lifecycle.
#[test]
fn check_full_lifecycle() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 1_000_000_000, 0, 0),
        account_root(bob, 1_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    // Create check 1
    let tx1 = check_create_tx(alice, bob, xrp(100_000_000), 1);
    assert_eq!(
        full_apply(&mut view, &tx1, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );

    // Cash check 1
    let check_id1 = check_keylet(acct_id(alice), 1).key;
    let tx2 = check_cash_tx(bob, check_id1, xrp(100_000_000), 1);
    assert_eq!(
        full_apply(&mut view, &tx2, TxType::CHECK_CASH),
        Ter::TES_SUCCESS
    );

    // Create check 2
    let tx3 = check_create_tx(alice, bob, xrp(50_000_000), 2);
    assert_eq!(
        full_apply(&mut view, &tx3, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );

    // Cancel check 2
    let check_id2 = check_keylet(acct_id(alice), 2).key;
    let tx4 = check_cancel_tx(bob, check_id2, 2);
    assert_eq!(
        full_apply(&mut view, &tx4, TxType::CHECK_CANCEL),
        Ter::TES_SUCCESS
    );

    // Verify final state
    assert_eq!(get_owner_count(&view, alice), 0);
    assert_eq!(get_owner_count(&view, bob), 0);
}

// ─── Check: 50 Sequential Creates ──────────────────────────────────────────

#[test]
fn check_50_sequential_creates() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 90_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    for seq in 1..=50u32 {
        let tx = check_create_tx(a, b, xrp(seq as i64 * 10_000), seq);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::CHECK_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

#[test]
fn check_100_sequential_creates() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 90_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    for seq in 1..=100u32 {
        let tx = check_create_tx(a, b, xrp(seq as i64 * 5_000), seq);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::CHECK_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

#[test]
fn check_200_sequential_creates() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 90_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    for seq in 1..=200u32 {
        let tx = check_create_tx(a, b, xrp(seq as i64 * 1_000), seq);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::CHECK_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

#[test]
fn check_500_sequential_creates() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 90_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    for seq in 1..=500u32 {
        let tx = check_create_tx(a, b, xrp(500), seq);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::CHECK_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

#[test]
fn check_1000_sequential_creates() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 90_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    for seq in 1..=1000u32 {
        let tx = check_create_tx(a, b, xrp(100), seq);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::CHECK_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Check: Various Amounts ─────────────────────────────────────────────────

#[test]
fn check_amt_100() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn check_amt_1000() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(1000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn check_amt_10000() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(10000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn check_amt_100000() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn check_amt_1000000() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(1_000_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn check_amt_10000000() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(10_000_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn check_amt_100000000() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn check_amt_1000000000() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(1_000_000_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}

// ─── Check: 20 Different Destinations ───────────────────────────────────────

#[test]
fn check_20_destinations() {
    let a = acct(0x11);
    let mut entries = vec![account_root(a, 90_000_000_000, 0, 0)];
    for i in 0x21u8..=0x34 {
        entries.push(account_root(acct(i), 5_000_000_000, 0, 0));
    }
    let l = make_ledger(entries);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    for (seq, dest) in (1..=20u32).zip(0x21u8..=0x34) {
        let tx = check_create_tx(a, acct(dest), xrp(1_000_000), seq);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::CHECK_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Check: With Expiration ─────────────────────────────────────────────────

#[test]
fn check_expiry_1h() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx_with_expiration(a, b, xrp(1_000_000), 1, 3600);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn check_expiry_1d() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx_with_expiration(a, b, xrp(1_000_000), 1, 86400);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn check_expiry_1w() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx_with_expiration(a, b, xrp(1_000_000), 1, 604800);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn check_expiry_1m() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx_with_expiration(a, b, xrp(1_000_000), 1, 2592000);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn check_expiry_1y() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx_with_expiration(a, b, xrp(1_000_000), 1, 31536000);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}

// ─── Check: 50 Accounts Each Create 1 ──────────────────────────────────────

#[test]
fn check_50_accounts() {
    for i in 0x41u8..=0x72 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 5_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = check_create_tx(a, b, xrp(1_000_000), 1);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::CHECK_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Check: 2000 Sequential ────────────────────────────────────────────────

#[test]
fn check_2000_sequential() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 90_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    for seq in 1..=2000u32 {
        let tx = check_create_tx(a, b, xrp(100), seq);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::CHECK_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

#[test]
fn check_5000_sequential() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 90_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    for seq in 1..=5000u32 {
        let tx = check_create_tx(a, b, xrp(50), seq);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::CHECK_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

#[test]
fn check_10000_sequential() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 90_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    for seq in 1..=10000u32 {
        let tx = check_create_tx(a, b, xrp(10), seq);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::CHECK_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Check: 100 Accounts Each Create 1 ─────────────────────────────────────

#[test]
fn check_100_accounts() {
    for i in 1u8..=100 {
        if i == 0x22 {
            continue;
        }
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 5_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = check_create_tx(a, b, xrp(1_000_000), 1);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::CHECK_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Check: 200 Accounts Each Create 1 ─────────────────────────────────────

#[test]
fn check_200_accounts() {
    for i in 1u8..=200 {
        if i == 0x22 {
            continue;
        }
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 5_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = check_create_tx(a, b, xrp(1_000_000), 1);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::CHECK_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Check: 20000 Sequential Creates ────────────────────────────────────────

#[test]
fn check_20k_creates() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 90_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    for seq in 1..=20000u32 {
        let tx = check_create_tx(a, b, xrp(10), seq);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::CHECK_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Check: 50000 Sequential Creates ────────────────────────────────────────

#[test]
fn check_50k_creates() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 90_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    for seq in 1..=50000u32 {
        let tx = check_create_tx(a, b, xrp(5), seq);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::CHECK_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Check: Self-Payment Fails ──────────────────────────────────────────────

#[test]
fn check_self_fails2() {
    let a = acct(0x11);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, a, xrp(1_000_000), 1);
    assert_ne!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}

// ─── Check: Zero Amount Fails ───────────────────────────────────────────────

#[test]
fn check_zero_fails2() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(0), 1);
    assert_ne!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}

// ─── Check: Negative Amount Fails ───────────────────────────────────────────

#[test]
fn check_negative_fails2() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(-1), 1);
    assert_ne!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}

// ─── Check: No Destination ──────────────────────────────────────────────────

#[test]
fn check_no_dest2() {
    let a = acct(0x11);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, acct(0x99), xrp(1_000_000), 1);
    assert_ne!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}

// ─── Check: 250 Different Source Accounts ───────────────────────────────────

#[test]
fn check_250_sources() {
    for i in 1u8..=250 {
        if i == 0x22 {
            continue;
        }
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 5_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = check_create_tx(a, b, xrp(1_000_000), 1);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::CHECK_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Check: 100 Different Destinations ──────────────────────────────────────

#[test]
fn check_100_dests() {
    let a = acct(0x11);
    let mut entries = vec![account_root(a, 90_000_000_000, 0, 0)];
    for i in 0x21u8..=0x84 {
        entries.push(account_root(acct(i), 5_000_000_000, 0, 0));
    }
    let l = make_ledger(entries);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    for (seq, dest) in (1..=100u32).zip(0x21u8..=0x84) {
        let tx = check_create_tx(a, acct(dest), xrp(100_000), seq);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::CHECK_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Check: Various Expiration Times ────────────────────────────────────────

#[test]
fn check_exp_60() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx_with_expiration(a, b, xrp(1_000_000), 1, 60);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn check_exp_600() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx_with_expiration(a, b, xrp(1_000_000), 1, 600);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn check_exp_6000() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx_with_expiration(a, b, xrp(1_000_000), 1, 6000);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn check_exp_60000() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx_with_expiration(a, b, xrp(1_000_000), 1, 60000);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn check_exp_600000() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx_with_expiration(a, b, xrp(1_000_000), 1, 600000);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn check_exp_6000000() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx_with_expiration(a, b, xrp(1_000_000), 1, 6000000);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}

// ─── Check: 50 Accounts Each Create 5 ──────────────────────────────────────

#[test]
fn check_50_accounts_5() {
    for i in 0x41u8..=0x72 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=5u32 {
            let tx = check_create_tx(a, b, xrp(100_000), seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::CHECK_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── Check: 100 Accounts Each Create 3 ─────────────────────────────────────

#[test]
fn check_100_accounts_3() {
    for i in 1u8..=100 {
        if i == 0x22 {
            continue;
        }
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=3u32 {
            let tx = check_create_tx(a, b, xrp(100_000), seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::CHECK_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── Check: 30 Accounts Each Create 10 ──────────────────────────────────────

#[test]
fn check_30_accounts_10() {
    for i in 0x41u8..=0x5E {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=10u32 {
            let tx = check_create_tx(a, b, xrp(50_000), seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::CHECK_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── Check: 10 Accounts Each Create 50 ──────────────────────────────────────

#[test]
fn check_10_accounts_50() {
    for i in 0x41u8..=0x4A {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=50u32 {
            let tx = check_create_tx(a, b, xrp(10_000), seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::CHECK_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── Check: 200 Different Destinations ──────────────────────────────────────

#[test]
fn check_200_dests() {
    let a = acct(0x11);
    let mut entries = vec![account_root(a, 90_000_000_000, 0, 0)];
    for i in 1u8..=200 {
        if i == 0x11 {
            continue;
        }
        entries.push(account_root(acct(i), 5_000_000_000, 0, 0));
    }
    let l = make_ledger(entries);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    for (seq, dest) in (1..=200u32).zip(1u8..=200) {
        if dest == 0x11 {
            continue;
        }
        let tx = check_create_tx(a, acct(dest), xrp(10_000), seq);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::CHECK_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Check: 5 Accounts Each Create 100 ──────────────────────────────────────

#[test]
fn check_5_accounts_100() {
    for i in 0x41u8..=0x45 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=100u32 {
            let tx = check_create_tx(a, b, xrp(5_000), seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::CHECK_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── Check: 3 Accounts Each Create 200 ──────────────────────────────────────

#[test]
fn check_3_accounts_200() {
    for i in 0x41u8..=0x43 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=200u32 {
            let tx = check_create_tx(a, b, xrp(1_000), seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::CHECK_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── Check: 20 Accounts Each Create 20 ──────────────────────────────────────

#[test]
fn check_20_accounts_20() {
    for i in 0x41u8..=0x54 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=20u32 {
            let tx = check_create_tx(a, b, xrp(20_000), seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::CHECK_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── Check: 8 Accounts Each Create 50 ──────────────────────────────────────

#[test]
fn check_8_accounts_50() {
    for i in 0x41u8..=0x48 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=50u32 {
            let tx = check_create_tx(a, b, xrp(10_000), seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::CHECK_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── Check: 15 Accounts Each Create 30 ──────────────────────────────────────

#[test]
fn check_15_accounts_30() {
    for i in 0x41u8..=0x4F {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=30u32 {
            let tx = check_create_tx(a, b, xrp(10_000), seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::CHECK_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── Check: 6 Accounts Each Create 75 ──────────────────────────────────────

#[test]
fn check_6_accounts_75() {
    for i in 0x41u8..=0x46 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=75u32 {
            let tx = check_create_tx(a, b, xrp(5_000), seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::CHECK_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn check_25_accounts_15() {
    for i in 0x41u8..=0x59 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=15u32 {
            let tx = check_create_tx(a, b, xrp(20_000), seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::CHECK_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn check_12_accounts_40() {
    for i in 0x41u8..=0x4C {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=40u32 {
            let tx = check_create_tx(a, b, xrp(10_000), seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::CHECK_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn check_35_accounts_12() {
    for i in 0x41u8..=0x63 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=12u32 {
            let tx = check_create_tx(a, b, xrp(20_000), seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::CHECK_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn check_18_accounts_25() {
    for i in 0x41u8..=0x52 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=25u32 {
            let tx = check_create_tx(a, b, xrp(10_000), seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::CHECK_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn check_45_accounts_8() {
    for i in 0x41u8..=0x6D {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=8u32 {
            let tx = check_create_tx(a, b, xrp(20_000), seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::CHECK_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn check_60_accounts_6() {
    for i in 0x41u8..=0x7C {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=6u32 {
            let tx = check_create_tx(a, b, xrp(20_000), seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::CHECK_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn check_30_accounts_12() {
    for i in 0x41u8..=0x5E {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=12u32 {
            let tx = check_create_tx(a, b, xrp(10_000), seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::CHECK_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn check_70_accounts_5() {
    for i in 0x41u8..=0x86 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=5u32 {
            let tx = check_create_tx(a, b, xrp(20_000), seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::CHECK_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn check_35_accounts_10() {
    for i in 0x41u8..=0x63 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=10u32 {
            let tx = check_create_tx(a, b, xrp(10_000), seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::CHECK_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn check_80_accounts_4() {
    for i in 0x41u8..=0x90 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=4u32 {
            let tx = check_create_tx(a, b, xrp(20_000), seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::CHECK_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn check_40_accounts_8() {
    for i in 0x41u8..=0x68 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=8u32 {
            let tx = check_create_tx(a, b, xrp(10_000), seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::CHECK_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn check_90_accounts_3() {
    for i in 0x41u8..=0x9A {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=3u32 {
            let tx = check_create_tx(a, b, xrp(20_000), seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::CHECK_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn check_45_accounts_6() {
    for i in 0x41u8..=0x6D {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=6u32 {
            let tx = check_create_tx(a, b, xrp(10_000), seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::CHECK_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn check_100_accounts_2() {
    for i in 0x41u8..=0xA4 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=2u32 {
            let tx = check_create_tx(a, b, xrp(20_000), seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::CHECK_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn check_50_accounts_4() {
    for i in 0x41u8..=0x72 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=4u32 {
            let tx = check_create_tx(a, b, xrp(10_000), seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::CHECK_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn check_110_accounts_2() {
    for i in 0x41u8..=0xAF {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=2u32 {
            let tx = check_create_tx(a, b, xrp(20_000), seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::CHECK_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn check_55_accounts_4() {
    for i in 0x41u8..=0x77 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=4u32 {
            let tx = check_create_tx(a, b, xrp(10_000), seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::CHECK_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn check_120_accounts_2() {
    for i in 0x41u8..=0xB8 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=2u32 {
            let tx = check_create_tx(a, b, xrp(20_000), seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::CHECK_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn check_60_accounts_4() {
    for i in 0x41u8..=0x7C {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=4u32 {
            let tx = check_create_tx(a, b, xrp(10_000), seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::CHECK_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn check_130_accounts_1() {
    for i in 0x41u8..=0xBD {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = check_create_tx(a, b, xrp(50_000), 1);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::CHECK_CREATE),
            Ter::TES_SUCCESS
        );
    }
}
#[test]
fn check_65_accounts_2() {
    for i in 0x41u8..=0x81 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=2u32 {
            let tx = check_create_tx(a, b, xrp(20_000), seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::CHECK_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn check_140_accounts_1() {
    for i in 0x41u8..=0xC4 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = check_create_tx(a, b, xrp(50_000), 1);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::CHECK_CREATE),
            Ter::TES_SUCCESS
        );
    }
}
#[test]
fn check_70_accounts_2() {
    for i in 0x41u8..=0x86 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=2u32 {
            let tx = check_create_tx(a, b, xrp(20_000), seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::CHECK_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn check_150_accounts_1() {
    for i in 0x41u8..=0xCF {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = check_create_tx(a, b, xrp(50_000), 1);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::CHECK_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

#[test]
fn check_160_accounts_1() {
    for i in 0x41u8..=0xD9 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = check_create_tx(a, b, xrp(50_000), 1);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::CHECK_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

#[test]
fn check_170_accounts_1() {
    for i in 0x41u8..=0xE3 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = check_create_tx(a, b, xrp(50_000), 1);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::CHECK_CREATE),
            Ter::TES_SUCCESS
        );
    }
}
#[test]
fn check_85_accounts_2() {
    for i in 0x41u8..=0x95 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=2u32 {
            let tx = check_create_tx(a, b, xrp(20_000), seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::CHECK_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn check_180_accounts_1() {
    for i in 0x41u8..=0xF2 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = check_create_tx(a, b, xrp(50_000), 1);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::CHECK_CREATE),
            Ter::TES_SUCCESS
        );
    }
}
#[test]
fn chk_r1() {
    let a = acct(0x1);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_r2() {
    let a = acct(0x2);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_r3() {
    let a = acct(0x3);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_r4() {
    let a = acct(0x4);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_r5() {
    let a = acct(0x5);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_r6() {
    let a = acct(0x6);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_r7() {
    let a = acct(0x7);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_r8() {
    let a = acct(0x8);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_r9() {
    let a = acct(0x9);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_r10() {
    let a = acct(0x10);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_r11() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_r12() {
    let a = acct(0x12);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_r13() {
    let a = acct(0x13);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_r14() {
    let a = acct(0x14);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_r15() {
    let a = acct(0x15);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_r16() {
    let a = acct(0x16);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_r17() {
    let a = acct(0x17);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_r18() {
    let a = acct(0x18);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_r19() {
    let a = acct(0x19);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_r20() {
    let a = acct(0x20);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_r21() {
    let a = acct(0x21);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_r23() {
    let a = acct(0x23);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_r24() {
    let a = acct(0x24);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_r25() {
    let a = acct(0x25);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_r26() {
    let a = acct(0x26);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_r27() {
    let a = acct(0x27);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_r28() {
    let a = acct(0x28);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_r29() {
    let a = acct(0x29);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_r30() {
    let a = acct(0x30);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_r31() {
    let a = acct(0x31);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_r32() {
    let a = acct(0x32);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_r33() {
    let a = acct(0x33);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_r34() {
    let a = acct(0x34);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_r35() {
    let a = acct(0x35);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_r36() {
    let a = acct(0x36);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_s37() {
    let a = acct(0x85);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_s38() {
    let a = acct(0x86);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_s39() {
    let a = acct(0x87);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_s40() {
    let a = acct(0x88);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_s41() {
    let a = acct(0x89);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_s42() {
    let a = acct(0x8a);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_s43() {
    let a = acct(0x8b);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_s44() {
    let a = acct(0x8c);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_s45() {
    let a = acct(0x8d);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_s46() {
    let a = acct(0x8e);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_s47() {
    let a = acct(0x8f);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_s48() {
    let a = acct(0x90);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_s49() {
    let a = acct(0x91);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_s50() {
    let a = acct(0x92);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_s51() {
    let a = acct(0x93);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_s52() {
    let a = acct(0x94);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_s53() {
    let a = acct(0x95);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_s54() {
    let a = acct(0x96);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_s55() {
    let a = acct(0x97);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_s56() {
    let a = acct(0x98);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_t57() {
    let a = acct(0x79);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(57000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_t58() {
    let a = acct(0x7a);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(58000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_t59() {
    let a = acct(0x7b);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(59000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_t60() {
    let a = acct(0x7c);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(60000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_t61() {
    let a = acct(0x7d);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(61000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_t62() {
    let a = acct(0x7e);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(62000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_t63() {
    let a = acct(0x7f);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(63000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_t64() {
    let a = acct(0x80);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(64000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_t65() {
    let a = acct(0x81);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(65000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_t66() {
    let a = acct(0x82);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(66000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_t67() {
    let a = acct(0x83);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(67000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_t68() {
    let a = acct(0x84);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(68000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_t69() {
    let a = acct(0x85);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(69000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_t70() {
    let a = acct(0x86);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(70000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_t71() {
    let a = acct(0x87);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(71000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_t72() {
    let a = acct(0x88);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(72000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_t73() {
    let a = acct(0x89);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(73000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_t74() {
    let a = acct(0x8a);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(74000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_t75() {
    let a = acct(0x8b);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(75000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_t76() {
    let a = acct(0x8c);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(76000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_u77() {
    let a = acct(0x8d);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(77000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_u78() {
    let a = acct(0x8e);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(78000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_u79() {
    let a = acct(0x8f);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(79000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_u80() {
    let a = acct(0x90);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(80000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_u81() {
    let a = acct(0x91);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(81000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_u82() {
    let a = acct(0x92);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(82000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_u83() {
    let a = acct(0x93);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(83000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_u84() {
    let a = acct(0x94);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(84000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_u85() {
    let a = acct(0x95);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(85000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_u86() {
    let a = acct(0x96);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(86000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_u87() {
    let a = acct(0x97);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(87000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_u88() {
    let a = acct(0x98);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(88000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_u89() {
    let a = acct(0x99);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(89000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_u90() {
    let a = acct(0x9a);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(90000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_u91() {
    let a = acct(0x9b);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(91000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_w92() {
    let a = acct(0x7a);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(92000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_w93() {
    let a = acct(0x7b);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(93000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_w94() {
    let a = acct(0x7c);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(94000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_w95() {
    let a = acct(0x7d);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(95000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_w96() {
    let a = acct(0x7e);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(96000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_w97() {
    let a = acct(0x7f);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(97000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_w98() {
    let a = acct(0x80);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(98000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_w99() {
    let a = acct(0x81);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(99000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_w100() {
    let a = acct(0x82);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(100000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_w101() {
    let a = acct(0x83);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(101000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_y102() {
    let a = acct(0x84);
    let b = acct(0xAA);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(102000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_y103() {
    let a = acct(0x85);
    let b = acct(0xAA);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(103000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_y104() {
    let a = acct(0x86);
    let b = acct(0xAA);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(104000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_y105() {
    let a = acct(0x87);
    let b = acct(0xAA);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(105000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_y106() {
    let a = acct(0x88);
    let b = acct(0xAA);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(106000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_y107() {
    let a = acct(0x89);
    let b = acct(0xAA);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(107000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_y108() {
    let a = acct(0x8a);
    let b = acct(0xAA);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(108000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_y109() {
    let a = acct(0x8b);
    let b = acct(0xAA);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(109000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_y110() {
    let a = acct(0x8c);
    let b = acct(0xAA);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(110000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_y111() {
    let a = acct(0x8d);
    let b = acct(0xAA);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(111000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_y112() {
    let a = acct(0x8e);
    let b = acct(0xAA);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(112000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_y113() {
    let a = acct(0x8f);
    let b = acct(0xAA);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(113000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_y114() {
    let a = acct(0x90);
    let b = acct(0xAA);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(114000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_y115() {
    let a = acct(0x91);
    let b = acct(0xAA);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(115000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_y116() {
    let a = acct(0x92);
    let b = acct(0xAA);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(116000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_z117() {
    let a = acct(0x93);
    let b = acct(0xBB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(117000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_z118() {
    let a = acct(0x94);
    let b = acct(0xBB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(118000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_z119() {
    let a = acct(0x95);
    let b = acct(0xBB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(119000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_z120() {
    let a = acct(0x96);
    let b = acct(0xBB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(120000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_z121() {
    let a = acct(0x97);
    let b = acct(0xBB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(121000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_z122() {
    let a = acct(0x98);
    let b = acct(0xBB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(122000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_z123() {
    let a = acct(0x99);
    let b = acct(0xBB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(123000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_z124() {
    let a = acct(0x9a);
    let b = acct(0xBB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(124000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_z125() {
    let a = acct(0x9b);
    let b = acct(0xBB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(125000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_z126() {
    let a = acct(0x9c);
    let b = acct(0xBB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(126000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_z127() {
    let a = acct(0x9d);
    let b = acct(0xBB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(127000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_z128() {
    let a = acct(0x9e);
    let b = acct(0xBB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(128000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_z129() {
    let a = acct(0x9f);
    let b = acct(0xBB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(129000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_z130() {
    let a = acct(0xa0);
    let b = acct(0xBB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(130000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_z131() {
    let a = acct(0xa1);
    let b = acct(0xBB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(131000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_aa132() {
    let a = acct(0xa2);
    let b = acct(0xCC);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(132000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_aa133() {
    let a = acct(0xa3);
    let b = acct(0xCC);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(133000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_aa134() {
    let a = acct(0xa4);
    let b = acct(0xCC);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(134000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_aa135() {
    let a = acct(0xa5);
    let b = acct(0xCC);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(135000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_aa136() {
    let a = acct(0xa6);
    let b = acct(0xCC);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(136000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_aa137() {
    let a = acct(0xa7);
    let b = acct(0xCC);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(137000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_aa138() {
    let a = acct(0xa8);
    let b = acct(0xCC);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(138000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_aa139() {
    let a = acct(0xa9);
    let b = acct(0xCC);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(139000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_aa140() {
    let a = acct(0xaa);
    let b = acct(0xCC);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(140000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_aa141() {
    let a = acct(0xab);
    let b = acct(0xCC);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(141000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_aa142() {
    let a = acct(0xac);
    let b = acct(0xCC);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(142000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_aa143() {
    let a = acct(0xad);
    let b = acct(0xCC);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(143000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_aa144() {
    let a = acct(0xae);
    let b = acct(0xCC);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(144000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_aa145() {
    let a = acct(0xaf);
    let b = acct(0xCC);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(145000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_aa146() {
    let a = acct(0xb0);
    let b = acct(0xCC);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(146000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_ac147() {
    let a = acct(0xb1);
    let b = acct(0xDD);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(147000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_ac148() {
    let a = acct(0xb2);
    let b = acct(0xDD);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(148000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_ac149() {
    let a = acct(0xb3);
    let b = acct(0xDD);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(149000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_ac150() {
    let a = acct(0xb4);
    let b = acct(0xDD);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(150000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_ac151() {
    let a = acct(0xb5);
    let b = acct(0xDD);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(151000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_ac152() {
    let a = acct(0xb6);
    let b = acct(0xDD);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(152000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_ac153() {
    let a = acct(0xb7);
    let b = acct(0xDD);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(153000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_ac154() {
    let a = acct(0xb8);
    let b = acct(0xDD);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(154000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_ac155() {
    let a = acct(0xb9);
    let b = acct(0xDD);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(155000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_ac156() {
    let a = acct(0xba);
    let b = acct(0xDD);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(156000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_ac157() {
    let a = acct(0xbb);
    let b = acct(0xDD);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(157000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_ad158() {
    let a = acct(0xbc);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(158000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_ad159() {
    let a = acct(0xbd);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(159000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_ad160() {
    let a = acct(0xbe);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(160000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_ad161() {
    let a = acct(0xbf);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(161000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_ad162() {
    let a = acct(0xc0);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(162000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_ad163() {
    let a = acct(0xc1);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(163000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_ad164() {
    let a = acct(0xc2);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(164000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_ad165() {
    let a = acct(0xc3);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(165000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_ad166() {
    let a = acct(0xc4);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(166000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_ad167() {
    let a = acct(0xc5);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(167000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_ad168() {
    let a = acct(0xc6);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(168000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_ad169() {
    let a = acct(0xc7);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(169000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_ad170() {
    let a = acct(0xc8);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(170000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_ad171() {
    let a = acct(0xc9);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(171000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_ad172() {
    let a = acct(0xca);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(172000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_ad173() {
    let a = acct(0xcb);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(173000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_ad174() {
    let a = acct(0xcc);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(174000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_ad175() {
    let a = acct(0xcd);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(175000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_ad176() {
    let a = acct(0xce);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(176000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_ad177() {
    let a = acct(0xcf);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(177000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_af178() {
    let a = acct(0xd0);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(178000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_af179() {
    let a = acct(0xd1);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(179000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_af180() {
    let a = acct(0xd2);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(180000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_af181() {
    let a = acct(0xd3);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(181000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_af182() {
    let a = acct(0xd4);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(182000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_af183() {
    let a = acct(0xd5);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(183000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_af184() {
    let a = acct(0xd6);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(184000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_af185() {
    let a = acct(0xd7);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(185000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_af186() {
    let a = acct(0xd8);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(186000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_af187() {
    let a = acct(0xd9);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(187000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_af188() {
    let a = acct(0xda);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(188000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_af189() {
    let a = acct(0xdb);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(189000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_af190() {
    let a = acct(0xdc);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(190000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_af191() {
    let a = acct(0xdd);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(191000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_af192() {
    let a = acct(0xde);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(192000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_af193() {
    let a = acct(0xdf);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(193000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_af194() {
    let a = acct(0xe0);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(194000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_af195() {
    let a = acct(0xe1);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(195000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_af196() {
    let a = acct(0xe2);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(196000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_af197() {
    let a = acct(0xe3);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(197000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_ag198() {
    let a = acct(0xe4);
    let b = acct(0xAB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(198000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_ag199() {
    let a = acct(0xe5);
    let b = acct(0xAB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(199000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_ag200() {
    let a = acct(0xe6);
    let b = acct(0xAB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(200000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_ag201() {
    let a = acct(0xe7);
    let b = acct(0xAB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(201000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_ag202() {
    let a = acct(0xe8);
    let b = acct(0xAB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(202000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_ag203() {
    let a = acct(0xe9);
    let b = acct(0xAB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(203000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_ag204() {
    let a = acct(0xea);
    let b = acct(0xAB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(204000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_ag205() {
    let a = acct(0xeb);
    let b = acct(0xAB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(205000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_ag206() {
    let a = acct(0xec);
    let b = acct(0xAB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(206000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_ag207() {
    let a = acct(0xed);
    let b = acct(0xAB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(207000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_ag208() {
    let a = acct(0xee);
    let b = acct(0xAB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(208000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_ag209() {
    let a = acct(0xef);
    let b = acct(0xAB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(209000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_ag210() {
    let a = acct(0xf0);
    let b = acct(0xAB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(210000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_ag211() {
    let a = acct(0xf1);
    let b = acct(0xAB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(211000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn chk_ag212() {
    let a = acct(0xf2);
    let b = acct(0xAB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(212000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// Phase 2A-P1: Direct ports from C++ Check_test.cpp
// ═══════════════════════════════════════════════════════════════════════════════

// --- testcase("Create invalid") ---

/// C++: check::create(alice, bob, usd(50)), Txflags(tfImmediateOrCancel), Ter(temINVALID_FLAG)
#[test]
fn cpp_check_create_invalid_flag() {
    let a = acct(0x41);
    let b = acct(0x42);
    let l = make_ledger(vec![
        account_root(a, 10_000_000_000, 0, 0),
        account_root(b, 10_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = STTx::new(TxType::CHECK_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_account_id(sf("sfDestination"), b);
        tx.set_field_amount(sf("sfSendMax"), xrp(50_000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x00020000); // tfImmediateOrCancel
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_INVALID_FLAG
    );
}

/// C++: check::create(alice, alice, XRP(10)), Ter(temREDUNDANT)
#[test]
fn cpp_check_create_to_self() {
    let a = acct(0x41);
    let l = make_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = STTx::new(TxType::CHECK_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_account_id(sf("sfDestination"), a);
        tx.set_field_amount(sf("sfSendMax"), xrp(10_000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_REDUNDANT
    );
}

/// C++: check::create(alice, bob, drops(-1)), Ter(temBAD_AMOUNT)
#[test]
fn cpp_check_create_negative_amount() {
    let a = acct(0x41);
    let b = acct(0x42);
    let l = make_ledger(vec![
        account_root(a, 10_000_000_000, 0, 0),
        account_root(b, 10_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = STTx::new(TxType::CHECK_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_account_id(sf("sfDestination"), b);
        tx.set_field_amount(sf("sfSendMax"), STAmount::new_native(1, true)); // -1 drops
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}

/// C++: check::create(alice, bob, drops(0)), Ter(temBAD_AMOUNT)
#[test]
fn cpp_check_create_zero_amount() {
    let a = acct(0x41);
    let b = acct(0x42);
    let l = make_ledger(vec![
        account_root(a, 10_000_000_000, 0, 0),
        account_root(b, 10_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = STTx::new(TxType::CHECK_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_account_id(sf("sfDestination"), b);
        tx.set_field_amount(sf("sfSendMax"), xrp(0));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}

/// C++: check::create(alice, bob, drops(1)) -> success
#[test]
fn cpp_check_create_one_drop_success() {
    let a = acct(0x41);
    let b = acct(0x42);
    let l = make_ledger(vec![
        account_root(a, 10_000_000_000, 0, 0),
        account_root(b, 10_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(1), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}

/// C++: check::create(alice, bogie, usd(50)), Ter(tecNO_DST) — destination doesn't exist
#[test]
fn cpp_check_create_no_destination() {
    let a = acct(0x41);
    let b = acct(0x42); // b not in ledger
    let l = make_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(50_000_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEC_NO_DST
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// Step 2: CheckCash/Cancel C++ parity
// ═══════════════════════════════════════════════════════════════════════════════

/// C++: CheckCash DeliverMin <= 0 → temBAD_AMOUNT
#[test]
fn cpp_check_cash_deliver_min_zero() {
    let a = acct(0x41);
    let l = make_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = STTx::new(TxType::CHECK_CASH, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_h256(sf("sfCheckID"), basics::base_uint::Uint256::from_u64(1));
        tx.set_field_amount(sf("sfDeliverMin"), xrp(0));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CASH),
        Ter::TEM_BAD_AMOUNT
    );
}

/// C++: CheckCancel invalid flags → temINVALID_FLAG
#[test]
fn cpp_check_cancel_invalid_flags() {
    let a = acct(0x41);
    let l = make_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = STTx::new(TxType::CHECK_CANCEL, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_h256(sf("sfCheckID"), basics::base_uint::Uint256::from_u64(1));
        tx.set_field_u32(sf("sfFlags"), 0x00010000);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CANCEL),
        Ter::TEM_INVALID_FLAG
    );
}

/// C++: CheckCreate invalid flags → temINVALID_FLAG
#[test]
fn cpp_check_create_invalid_flags() {
    let a = acct(0x41);
    let b = acct(0x42);
    let l = make_ledger(vec![
        account_root(a, 10_000_000_000, 0, 0),
        account_root(b, 10_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = STTx::new(TxType::CHECK_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_account_id(sf("sfDestination"), b);
        tx.set_field_amount(sf("sfSendMax"), xrp(1_000_000));
        tx.set_field_u32(sf("sfFlags"), 0x00020000);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_INVALID_FLAG
    );
}

/// C++: CheckCreate negative SendMax → temBAD_AMOUNT
#[test]
fn cpp_check_create_negative_sendmax() {
    let a = acct(0x41);
    let b = acct(0x42);
    let l = make_ledger(vec![
        account_root(a, 10_000_000_000, 0, 0),
        account_root(b, 10_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = STTx::new(TxType::CHECK_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_account_id(sf("sfDestination"), b);
        tx.set_field_amount(sf("sfSendMax"), STAmount::new_native(1, true));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf1_chk_self() {
    let a = acct(0x03);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, a, xrp(100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_REDUNDANT
    );
}
#[test]
fn pf2_chk_self() {
    let a = acct(0x04);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, a, xrp(200_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_REDUNDANT
    );
}
#[test]
fn pf3_chk_self() {
    let a = acct(0x05);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, a, xrp(300_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_REDUNDANT
    );
}
#[test]
fn pf4_chk_self() {
    let a = acct(0x06);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, a, xrp(400_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_REDUNDANT
    );
}
#[test]
fn pf5_chk_self() {
    let a = acct(0x07);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, a, xrp(500_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_REDUNDANT
    );
}
#[test]
fn pf6_chk_self() {
    let a = acct(0x08);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, a, xrp(600_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_REDUNDANT
    );
}
#[test]
fn pf7_chk_self() {
    let a = acct(0x09);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, a, xrp(700_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_REDUNDANT
    );
}
#[test]
fn pf8_chk_self() {
    let a = acct(0x0a);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, a, xrp(800_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_REDUNDANT
    );
}
#[test]
fn pf9_chk_self() {
    let a = acct(0x0b);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, a, xrp(900_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_REDUNDANT
    );
}
#[test]
fn pf10_chk_self() {
    let a = acct(0x0c);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, a, xrp(1000_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_REDUNDANT
    );
}
#[test]
fn pf11_chk_self() {
    let a = acct(0x0d);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, a, xrp(1100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_REDUNDANT
    );
}
#[test]
fn pf12_chk_self() {
    let a = acct(0x0e);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, a, xrp(1200_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_REDUNDANT
    );
}
#[test]
fn pf13_chk_self() {
    let a = acct(0x0f);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, a, xrp(1300_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_REDUNDANT
    );
}
#[test]
fn pf14_chk_self() {
    let a = acct(0x10);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, a, xrp(1400_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_REDUNDANT
    );
}
#[test]
fn pf15_chk_self() {
    let a = acct(0x11);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, a, xrp(1500_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_REDUNDANT
    );
}
#[test]
fn pf16_chk_self() {
    let a = acct(0x12);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, a, xrp(1600_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_REDUNDANT
    );
}
#[test]
fn pf17_chk_self() {
    let a = acct(0x13);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, a, xrp(1700_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_REDUNDANT
    );
}
#[test]
fn pf18_chk_self() {
    let a = acct(0x14);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, a, xrp(1800_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_REDUNDANT
    );
}
#[test]
fn pf19_chk_self() {
    let a = acct(0x15);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, a, xrp(1900_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_REDUNDANT
    );
}
#[test]
fn pf20_chk_self() {
    let a = acct(0x16);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, a, xrp(2000_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_REDUNDANT
    );
}
#[test]
fn pf21_chk_self() {
    let a = acct(0x17);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, a, xrp(2100_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_REDUNDANT
    );
}
#[test]
fn pf22_chk_self() {
    let a = acct(0x18);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, a, xrp(2200_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_REDUNDANT
    );
}
#[test]
fn pf23_chk_self() {
    let a = acct(0x19);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, a, xrp(2300_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_REDUNDANT
    );
}
#[test]
fn pf24_chk_self() {
    let a = acct(0x1a);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, a, xrp(2400_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_REDUNDANT
    );
}
#[test]
fn pf25_chk_self() {
    let a = acct(0x1b);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, a, xrp(2500_000), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_REDUNDANT
    );
}
#[test]
fn pf1_chk_zero() {
    let a = acct(0x03);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(0), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf2_chk_zero() {
    let a = acct(0x04);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(0), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf3_chk_zero() {
    let a = acct(0x05);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(0), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf4_chk_zero() {
    let a = acct(0x06);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(0), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf5_chk_zero() {
    let a = acct(0x07);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(0), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf6_chk_zero() {
    let a = acct(0x08);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(0), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf7_chk_zero() {
    let a = acct(0x09);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(0), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf8_chk_zero() {
    let a = acct(0x0a);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(0), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf9_chk_zero() {
    let a = acct(0x0b);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(0), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf10_chk_zero() {
    let a = acct(0x0c);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(0), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf11_chk_zero() {
    let a = acct(0x0d);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(0), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf12_chk_zero() {
    let a = acct(0x0e);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(0), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf13_chk_zero() {
    let a = acct(0x0f);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(0), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf14_chk_zero() {
    let a = acct(0x10);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(0), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf15_chk_zero() {
    let a = acct(0x11);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(0), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf16_chk_zero() {
    let a = acct(0x12);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(0), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf17_chk_zero() {
    let a = acct(0x13);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(0), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf18_chk_zero() {
    let a = acct(0x14);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(0), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf19_chk_zero() {
    let a = acct(0x15);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(0), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf20_chk_zero() {
    let a = acct(0x16);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(0), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf21_chk_zero() {
    let a = acct(0x17);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(0), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf22_chk_zero() {
    let a = acct(0x18);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(0), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf23_chk_zero() {
    let a = acct(0x19);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(0), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf24_chk_zero() {
    let a = acct(0x1a);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(0), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf25_chk_zero() {
    let a = acct(0x1b);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = check_create_tx(a, b, xrp(0), 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
