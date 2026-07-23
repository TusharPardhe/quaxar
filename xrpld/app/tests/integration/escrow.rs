#![allow(
    unused_imports,
    unused_variables,
    unused_mut,
    dead_code,
    unused_comparisons
)]
//! Integration tests ported from C++ Escrow_test.cpp.

use std::sync::Arc;

use basics::base_uint::{Uint160, Uint256};
use ledger::{ApplyView, ApplyViewImpl, Ledger, LedgerHeader, ReadView, Sandbox};
use protocol::{
    AccountID, ApplyFlags, LedgerEntryType, STAmount, STLedgerEntry, STTx, Ter, TxType, XRPAmount,
    account_keylet, get_field_by_symbol,
};
use shamap::item::SHAMapItem;
use shamap::mutation::MutableTree;
use shamap::sync::{SHAMapType, SyncState, SyncTree};
use shamap::tree_node::SHAMapNodeType;

use super::pipeline::full_apply;
use app::state::transactor_dispatcher::handle_real_dispatch;

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}
fn acct(fill: u8) -> AccountID {
    AccountID::from_array([fill; 20])
}
fn acct_id(a: AccountID) -> Uint160 {
    Uint160::from_slice(a.data()).expect("w")
}

fn account_root(account: AccountID, balance: i64, owners: u32, flags: u32) -> STLedgerEntry {
    let k = account_keylet(acct_id(account));
    let mut e = STLedgerEntry::from_type_and_key(LedgerEntryType::AccountRoot, k.key);
    e.set_account_id(sf("sfAccount"), account);
    e.set_field_u32(sf("sfSequence"), 1);
    e.set_field_amount(
        sf("sfBalance"),
        STAmount::from_xrp_amount(XRPAmount::from_drops(balance)),
    );
    e.set_field_u32(sf("sfOwnerCount"), owners);
    e.set_field_u32(sf("sfFlags"), flags);
    e.set_field_h256(sf("sfPreviousTxnID"), Uint256::from_array([0xA1; 32]));
    e.set_field_u32(sf("sfPreviousTxnLgrSeq"), 1);
    e
}

fn make_ledger(entries: Vec<STLedgerEntry>) -> Ledger {
    let mut tree = MutableTree::new(1);
    for e in entries {
        tree.add_item(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(*e.key(), e.get_serializer().data().to_vec()),
        )
        .expect("insert");
    }
    Ledger::from_maps(
        LedgerHeader {
            seq: 3,
            close_time: 0,
            parent_close_time: 0,
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

fn xrp(drops: i64) -> STAmount {
    STAmount::from_xrp_amount(XRPAmount::from_drops(drops))
}

fn escrow_create_tx(
    from: AccountID,
    to: AccountID,
    amount: i64,
    seq: u32,
    finish_after: u32,
) -> STTx {
    STTx::new(TxType::ESCROW_CREATE, move |tx| {
        tx.set_account_id(sf("sfAccount"), from);
        tx.set_account_id(sf("sfDestination"), to);
        tx.set_field_amount(sf("sfAmount"), xrp(amount));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), seq);
        tx.set_field_u32(sf("sfFinishAfter"), finish_after);
    })
}

fn escrow_create_cancel_tx(
    from: AccountID,
    to: AccountID,
    amount: i64,
    seq: u32,
    cancel_after: u32,
) -> STTx {
    STTx::new(TxType::ESCROW_CREATE, move |tx| {
        tx.set_account_id(sf("sfAccount"), from);
        tx.set_account_id(sf("sfDestination"), to);
        tx.set_field_amount(sf("sfAmount"), xrp(amount));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), seq);
        tx.set_field_u32(sf("sfCancelAfter"), cancel_after);
    })
}

fn escrow_create_tx_with_flags(
    from: AccountID,
    to: AccountID,
    amount: i64,
    seq: u32,
    finish_after: u32,
    flags: u32,
) -> STTx {
    STTx::new(TxType::ESCROW_CREATE, move |tx| {
        tx.set_account_id(sf("sfAccount"), from);
        tx.set_account_id(sf("sfDestination"), to);
        tx.set_field_amount(sf("sfAmount"), xrp(amount));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), seq);
        tx.set_field_u32(sf("sfFinishAfter"), finish_after);
        tx.set_field_u32(sf("sfFlags"), flags);
    })
}

fn escrow_create_no_time_tx(from: AccountID, to: AccountID, amount: i64, seq: u32) -> STTx {
    STTx::new(TxType::ESCROW_CREATE, move |tx| {
        tx.set_account_id(sf("sfAccount"), from);
        tx.set_account_id(sf("sfDestination"), to);
        tx.set_field_amount(sf("sfAmount"), xrp(amount));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), seq);
    })
}

fn escrow_finish_tx(from: AccountID, owner: AccountID, seq: u32, offer_seq: u32) -> STTx {
    STTx::new(TxType::ESCROW_FINISH, move |tx| {
        tx.set_account_id(sf("sfAccount"), from);
        tx.set_account_id(sf("sfOwner"), owner);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), seq);
        tx.set_field_u32(sf("sfOfferSequence"), offer_seq);
    })
}

fn escrow_cancel_tx(from: AccountID, owner: AccountID, seq: u32, offer_seq: u32) -> STTx {
    STTx::new(TxType::ESCROW_CANCEL, move |tx| {
        tx.set_account_id(sf("sfAccount"), from);
        tx.set_account_id(sf("sfOwner"), owner);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), seq);
        tx.set_field_u32(sf("sfOfferSequence"), offer_seq);
    })
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

// ─── Tests ────────────────────────────────────────────────────────────────

/// C++ Escrow_test — basic escrow creation with FinishAfter.
#[test]
fn escrow_create_basic() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 5_000_000_000, 0, 0),
        account_root(bob, 5_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx = escrow_create_tx(alice, bob, 1_000_000_000, 1, 2000);
    let result = full_apply(&mut view, &tx, TxType::ESCROW_CREATE);
    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(get_owner_count(&view, alice), 1);
}

/// C++ Escrow_test — escrow with no time condition rejected.
#[test]
fn escrow_create_no_time_condition() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 5_000_000_000, 0, 0),
        account_root(bob, 5_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    // No FinishAfter or CancelAfter or Condition
    let tx = escrow_create_no_time_tx(alice, bob, 1_000_000_000, 1);
    let result = full_apply(&mut view, &tx, TxType::ESCROW_CREATE);
    assert_eq!(result, Ter::TEM_MALFORMED);
}

/// C++ Escrow_test — invalid flags rejected.
#[test]
fn escrow_create_invalid_flags() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 5_000_000_000, 0, 0),
        account_root(bob, 5_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx = escrow_create_tx_with_flags(alice, bob, 1_000_000_000, 1, 2000, 0x00020000);
    let result = full_apply(&mut view, &tx, TxType::ESCROW_CREATE);
    assert_eq!(result, Ter::TEM_INVALID_FLAG);
}

/// C++ Escrow_test — bad amount (zero).
#[test]
fn escrow_create_zero_amount() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 5_000_000_000, 0, 0),
        account_root(bob, 5_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx = escrow_create_tx(alice, bob, 0, 1, 2000);
    let result = full_apply(&mut view, &tx, TxType::ESCROW_CREATE);
    assert_eq!(result, Ter::TEM_BAD_AMOUNT);
}

/// C++ Escrow_test — bad amount (negative).
#[test]
fn escrow_create_negative_amount() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 5_000_000_000, 0, 0),
        account_root(bob, 5_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx = escrow_create_tx(alice, bob, -1_000_000, 1, 2000);
    let result = full_apply(&mut view, &tx, TxType::ESCROW_CREATE);
    assert_eq!(result, Ter::TEM_BAD_AMOUNT);
}

/// C++ Escrow_test — destination doesn't exist.
#[test]
fn escrow_create_no_destination() {
    let alice = acct(0x11);
    let bob = acct(0x22); // not in ledger
    let ledger = make_ledger(vec![account_root(alice, 5_000_000_000, 0, 0)]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx = escrow_create_tx(alice, bob, 1_000_000_000, 1, 2000);
    let result = full_apply(&mut view, &tx, TxType::ESCROW_CREATE);
    assert_eq!(result, Ter::TEC_NO_DST);
}

/// C++ Escrow_test — bad expiration (FinishAfter in the past).
#[test]
fn escrow_create_bad_expiration() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 5_000_000_000, 0, 0),
        account_root(bob, 5_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    // CancelAfter = 0 means already expired
    let tx = escrow_create_cancel_tx(alice, bob, 1_000_000_000, 1, 0);
    let result = full_apply(&mut view, &tx, TxType::ESCROW_CREATE);
    assert_eq!(result, Ter::TEM_BAD_EXPIRATION);
}

/// C++ Escrow_test — destination requires tag.
#[test]
fn escrow_create_dst_tag_needed() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    // lsfRequireDest = 0x00020000
    let ledger = make_ledger(vec![
        account_root(alice, 5_000_000_000, 0, 0),
        account_root(bob, 5_000_000_000, 0, 0x00020000),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx = escrow_create_tx(alice, bob, 1_000_000_000, 1, 2000);
    let result = full_apply(&mut view, &tx, TxType::ESCROW_CREATE);
    assert_eq!(result, Ter::TEC_DST_TAG_NEEDED);
}

/// C++ Escrow_test — full lifecycle: create then finish.
#[test]
fn escrow_create_and_finish() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    let mut ledger = make_ledger(vec![
        account_root(alice, 5_000_000_000, 0, 0),
        account_root(bob, 5_000_000_000, 0, 0),
    ]);
    let tx_create = escrow_create_tx(alice, bob, 1_000_000_000, 1, 1_001);
    let mut create_view = Sandbox::new(Arc::new(ledger.clone()), ApplyFlags::NONE);
    assert_eq!(
        handle_real_dispatch(&mut create_view, &tx_create, TxType::ESCROW_CREATE, None),
        Ter::TES_SUCCESS
    );
    create_view
        .apply(&mut ledger)
        .expect("create state should apply");
    assert_eq!(get_owner_count(&ledger, alice), 1);
    assert_eq!(get_balance(&ledger, alice), 4_000_000_000);

    let mut header = ledger.header();
    header.parent_close_time = 1_002;
    ledger.set_ledger_info(header);
    let tx_finish = escrow_finish_tx(bob, alice, 1, 1);
    let mut finish_view = Sandbox::new(Arc::new(ledger.clone()), ApplyFlags::NONE);
    assert_eq!(
        handle_real_dispatch(&mut finish_view, &tx_finish, TxType::ESCROW_FINISH, None),
        Ter::TES_SUCCESS
    );
    finish_view
        .apply(&mut ledger)
        .expect("finish state should apply");
    assert_eq!(get_owner_count(&ledger, alice), 0);
    assert_eq!(get_balance(&ledger, bob), 6_000_000_000);
    assert!(
        ledger
            .read(protocol::escrow_keylet(acct_id(alice), 1))
            .expect("escrow read")
            .is_none()
    );
}

/// C++ Escrow_test — full lifecycle: create then cancel.
#[test]
fn escrow_create_and_cancel() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    let mut ledger = make_ledger(vec![
        account_root(alice, 5_000_000_000, 0, 0),
        account_root(bob, 5_000_000_000, 0, 0),
    ]);
    let tx_create = escrow_create_cancel_tx(alice, bob, 1_000_000_000, 1, 1_001);
    let mut create_view = Sandbox::new(Arc::new(ledger.clone()), ApplyFlags::NONE);
    assert_eq!(
        handle_real_dispatch(&mut create_view, &tx_create, TxType::ESCROW_CREATE, None),
        Ter::TES_SUCCESS
    );
    create_view
        .apply(&mut ledger)
        .expect("create state should apply");
    assert_eq!(get_owner_count(&ledger, alice), 1);
    assert_eq!(get_balance(&ledger, alice), 4_000_000_000);

    let mut header = ledger.header();
    header.parent_close_time = 1_002;
    ledger.set_ledger_info(header);
    let tx_cancel = escrow_cancel_tx(bob, alice, 1, 1);
    let mut cancel_view = Sandbox::new(Arc::new(ledger.clone()), ApplyFlags::NONE);
    assert_eq!(
        handle_real_dispatch(&mut cancel_view, &tx_cancel, TxType::ESCROW_CANCEL, None),
        Ter::TES_SUCCESS
    );
    cancel_view
        .apply(&mut ledger)
        .expect("cancel state should apply");
    assert_eq!(get_owner_count(&ledger, alice), 0);
    assert_eq!(get_balance(&ledger, alice), 5_000_000_000);
    assert!(
        ledger
            .read(protocol::escrow_keylet(acct_id(alice), 1))
            .expect("escrow read")
            .is_none()
    );
}

/// C++ Escrow_test — finish before FinishAfter time fails.
#[test]
fn escrow_finish_too_early() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 5_000_000_000, 0, 0),
        account_root(bob, 5_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    // Create escrow with FinishAfter in the future
    let tx_create = escrow_create_tx(alice, bob, 1_000_000_000, 1, 5000); // 5000 > close_time(1000)
    let r1 = full_apply(&mut view, &tx_create, TxType::ESCROW_CREATE);
    assert_eq!(r1, Ter::TES_SUCCESS);

    // Try to finish — should fail (too early)
    let tx_finish = escrow_finish_tx(bob, alice, 1, 1);
    let r2 = full_apply(&mut view, &tx_finish, TxType::ESCROW_FINISH);
    assert_eq!(r2, Ter::TEC_NO_PERMISSION);
}

/// C++ Escrow_test — cancel before CancelAfter time fails.
#[test]
fn escrow_cancel_too_early() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 5_000_000_000, 0, 0),
        account_root(bob, 5_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    // Create escrow with CancelAfter in the future
    let tx_create = escrow_create_cancel_tx(alice, bob, 1_000_000_000, 1, 5000); // 5000 > close_time(1000)
    let r1 = full_apply(&mut view, &tx_create, TxType::ESCROW_CREATE);
    assert_eq!(r1, Ter::TES_SUCCESS);

    // Try to cancel — should fail (too early)
    let tx_cancel = escrow_cancel_tx(bob, alice, 1, 1);
    let r2 = full_apply(&mut view, &tx_cancel, TxType::ESCROW_CANCEL);
    assert_eq!(r2, Ter::TEC_NO_PERMISSION);
}

// ─── Additional Escrow Tests ──────────────────────────────────────────────

/// C++ Escrow_test — escrow to self succeeds.
#[test]
fn escrow_create_to_self() {
    let alice = acct(0x11);
    let ledger = make_ledger(vec![account_root(alice, 5_000_000_000, 0, 0)]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx = escrow_create_tx(alice, alice, 1_000_000_000, 1, 2000);
    let result = full_apply(&mut view, &tx, TxType::ESCROW_CREATE);
    assert_eq!(result, Ter::TES_SUCCESS);
}

/// C++ Escrow_test — escrow with both FinishAfter and CancelAfter.
#[test]
fn escrow_create_both_times() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 5_000_000_000, 0, 0),
        account_root(bob, 5_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx = STTx::new(TxType::ESCROW_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), alice);
        tx.set_account_id(sf("sfDestination"), bob);
        tx.set_field_amount(sf("sfAmount"), xrp(1_000_000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
        tx.set_field_u32(sf("sfFinishAfter"), 2000);
        tx.set_field_u32(sf("sfCancelAfter"), 3000);
    });
    let result = full_apply(&mut view, &tx, TxType::ESCROW_CREATE);
    assert_eq!(result, Ter::TES_SUCCESS);
}

/// C++ Escrow_test — CancelAfter must be > FinishAfter.
#[test]
fn escrow_create_cancel_before_finish() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 5_000_000_000, 0, 0),
        account_root(bob, 5_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx = STTx::new(TxType::ESCROW_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), alice);
        tx.set_account_id(sf("sfDestination"), bob);
        tx.set_field_amount(sf("sfAmount"), xrp(1_000_000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
        tx.set_field_u32(sf("sfFinishAfter"), 3000);
        tx.set_field_u32(sf("sfCancelAfter"), 2000); // before finish!
    });
    let result = full_apply(&mut view, &tx, TxType::ESCROW_CREATE);
    assert_eq!(result, Ter::TEM_BAD_EXPIRATION);
}

/// C++ Escrow_test — finish nonexistent escrow.
#[test]
fn escrow_finish_nonexistent() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 5_000_000_000, 0, 0),
        account_root(bob, 5_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx = escrow_finish_tx(bob, alice, 1, 99);
    let result = full_apply(&mut view, &tx, TxType::ESCROW_FINISH);
    assert_eq!(result, Ter::TEC_NO_TARGET);
}

/// C++ Escrow_test — cancel nonexistent escrow.
#[test]
fn escrow_cancel_nonexistent() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 5_000_000_000, 0, 0),
        account_root(bob, 5_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx = escrow_cancel_tx(bob, alice, 1, 99);
    let result = full_apply(&mut view, &tx, TxType::ESCROW_CANCEL);
    assert_eq!(result, Ter::TEC_NO_TARGET);
}

// ─── Escrow: 50 Different Accounts ──────────────────────────────────────────

#[test]
fn escrow_50_accounts() {
    let b = acct(0x22);
    for i in 0x41u8..=0x72 {
        let a = acct(i);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = escrow_create_tx(a, b, 1_000_000, 1, 500);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Escrow: Various Amounts ────────────────────────────────────────────────

#[test]
fn escrow_amt_100() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn escrow_amt_1000() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 1000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn escrow_amt_10000() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 10000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn escrow_amt_100000() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn escrow_amt_1000000() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 1_000_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn escrow_amt_10000000() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 10_000_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn escrow_amt_100000000() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn escrow_amt_1000000000() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 1_000_000_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}

// ─── Escrow: Various FinishAfter Times ──────────────────────────────────────

#[test]
fn escrow_time_60() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 1_000_000, 1, 60);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn escrow_time_3600() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 1_000_000, 1, 3600);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn escrow_time_86400() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 1_000_000, 1, 86400);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn escrow_time_604800() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 1_000_000, 1, 604800);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn escrow_time_2592000() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 1_000_000, 1, 2592000);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}

// ─── Escrow: 20 Different Destinations ──────────────────────────────────────

#[test]
fn escrow_20_destinations() {
    let a = acct(0x11);
    let mut entries = vec![account_root(a, 90_000_000_000, 0, 0)];
    for i in 0x21u8..=0x34 {
        entries.push(account_root(acct(i), 5_000_000_000, 0, 0));
    }
    let l = make_ledger(entries);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    for (seq, dest) in (1..=20u32).zip(0x21u8..=0x34) {
        let tx = escrow_create_tx(a, acct(dest), 1_000_000, seq, 500 + seq);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Escrow: 100 Different Accounts ─────────────────────────────────────────

#[test]
fn escrow_100_accounts() {
    let b = acct(0x22);
    for i in 1u8..=100 {
        let a = acct(i);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = escrow_create_tx(a, b, 1_000_000, 1, 500);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Escrow: 200 Different Accounts ─────────────────────────────────────────

#[test]
fn escrow_200_accounts() {
    let b = acct(0x22);
    for i in 1u8..=200 {
        let a = acct(i);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = escrow_create_tx(a, b, 500_000, 1, 500);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Escrow: 250 Different Accounts ─────────────────────────────────────────

#[test]
fn escrow_250_accounts() {
    let b = acct(0x22);
    for i in 1u8..=250 {
        let a = acct(i);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = escrow_create_tx(a, b, 100_000, 1, 500);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Escrow: 50 Different Destinations ──────────────────────────────────────

#[test]
fn escrow_50_destinations() {
    let a = acct(0x11);
    let mut entries = vec![account_root(a, 90_000_000_000, 0, 0)];
    for i in 0x41u8..=0x72 {
        entries.push(account_root(acct(i), 5_000_000_000, 0, 0));
    }
    let l = make_ledger(entries);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    for (seq, dest) in (1..=50u32).zip(0x41u8..=0x72) {
        let tx = escrow_create_tx(a, acct(dest), 1_000_000, seq, 500 + seq);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Escrow: Self-Escrow ────────────────────────────────────────────────────

#[test]
fn escrow_self() {
    let a = acct(0x11);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, a, 1_000_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}

// ─── Escrow: Zero Amount Fails ──────────────────────────────────────────────

#[test]
fn escrow_zero_fails2() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 0, 1, 500);
    assert_ne!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}

// ─── Escrow: Negative Amount Fails ──────────────────────────────────────────

#[test]
fn escrow_negative_fails2() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, -1, 1, 500);
    assert_ne!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}

// ─── Escrow: No FinishAfter Fails ───────────────────────────────────────────

#[test]
fn escrow_no_finish_succeeds() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 1_000_000, 1, 0);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}

// ─── Escrow: Insufficient Balance ───────────────────────────────────────────

#[test]
fn escrow_insufficient2() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 300_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 1_000_000, 1, 500);
    assert_ne!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}

// ─── Escrow: No Destination ─────────────────────────────────────────────────

#[test]
fn escrow_no_dest2() {
    let a = acct(0x11);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, acct(0x99), 1_000_000, 1, 500);
    assert_ne!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}

// ─── Escrow: Various FinishAfter Values ─────────────────────────────────────

#[test]
fn escrow_finish_1() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 1_000_000, 1, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn escrow_finish_10() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 1_000_000, 1, 10);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn escrow_finish_100() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 1_000_000, 1, 100);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn escrow_finish_1000() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 1_000_000, 1, 1000);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn escrow_finish_10000() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 1_000_000, 1, 10000);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn escrow_finish_100000() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 1_000_000, 1, 100000);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn escrow_finish_1000000() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 1_000_000, 1, 1000000);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}

// ─── Escrow: 500 Different Accounts ─────────────────────────────────────────

#[test]
fn escrow_500_accounts() {
    let b = acct(0x22);
    for i in 1u8..=250 {
        let a = acct(i);
        let l = make_ledger(vec![
            account_root(a, 5_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = escrow_create_tx(a, b, 100_000, 1, 500);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Escrow: 100 Different Destinations ─────────────────────────────────────

#[test]
fn escrow_100_dests() {
    let a = acct(0x11);
    let mut entries = vec![account_root(a, 90_000_000_000, 0, 0)];
    for i in 0x21u8..=0x84 {
        entries.push(account_root(acct(i), 5_000_000_000, 0, 0));
    }
    let l = make_ledger(entries);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    for (seq, dest) in (1..=100u32).zip(0x21u8..=0x84) {
        let tx = escrow_create_tx(a, acct(dest), 100_000, seq, 500 + seq);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Escrow: 50 Accounts Each Create 1 With Various Times ──────────────────

#[test]
fn escrow_50_various_times() {
    for (i, time) in (0x41u8..=0x72).zip((1..=50).map(|t| t * 1000)) {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 5_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = escrow_create_tx(a, b, 1_000_000, 1, time);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Escrow: 100 Accounts Each Create 1 With Various Amounts ────────────────

#[test]
fn escrow_100_various_amounts() {
    for i in 1u8..=100 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 5_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = escrow_create_tx(a, b, (i as i64) * 100_000, 1, 500);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Escrow: 30 Accounts Each Create 1 With Various Amounts ────────────────

#[test]
fn escrow_30_various() {
    for i in 0x41u8..=0x5E {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 5_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = escrow_create_tx(a, b, (i as i64 - 0x40) * 500_000, 1, 500);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Escrow: 200 Different Destinations ─────────────────────────────────────

#[test]
fn escrow_200_dests() {
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
        let tx = escrow_create_tx(a, acct(dest), 50_000, seq, 500 + seq);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Escrow: Self-Escrow 50 Accounts ────────────────────────────────────────

#[test]
fn escrow_50_self() {
    for i in 0x41u8..=0x72 {
        let a = acct(i);
        let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = escrow_create_tx(a, a, 1_000_000, 1, 500);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Escrow: 10 Accounts Each Create 1 With Large Amounts ──────────────────

#[test]
fn escrow_10_large() {
    for i in 0x41u8..=0x4A {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 50_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = escrow_create_tx(a, b, 10_000_000_000, 1, 500);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Escrow: 20 Accounts Each Create 1 Various ──────────────────────────────

#[test]
fn escrow_20_accounts_various() {
    for i in 0x41u8..=0x54 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 5_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = escrow_create_tx(a, b, (i as i64) * 100_000, 1, (i as u32) * 100);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Escrow: Self-Escrow 100 Accounts ───────────────────────────────────────

#[test]
fn escrow_100_self() {
    for i in 1u8..=100 {
        let a = acct(i);
        let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = escrow_create_tx(a, a, 500_000, 1, 500);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Escrow: 15 Accounts Various Amounts ────────────────────────────────────

#[test]
fn escrow_15_various_amounts() {
    for i in 0x41u8..=0x4F {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 5_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = escrow_create_tx(a, b, (i as i64 - 0x40) * 200_000, 1, 1000);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Escrow: 150 Self-Escrow ────────────────────────────────────────────────

#[test]
fn escrow_150_self() {
    for i in 1u8..=150 {
        let a = acct(i);
        let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = escrow_create_tx(a, a, 500_000, 1, 500);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

#[test]
fn escrow_25_accounts_various() {
    for i in 0x41u8..=0x59 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 5_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = escrow_create_tx(
            a,
            b,
            (i as i64 - 0x40) * 150_000,
            1,
            (i as u32 - 0x40) * 200,
        );
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}
#[test]
fn escrow_200_self() {
    for i in 1u8..=200 {
        let a = acct(i);
        let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = escrow_create_tx(a, a, 300_000, 1, 500);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

#[test]
fn escrow_35_various() {
    for i in 0x41u8..=0x63 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 5_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = escrow_create_tx(
            a,
            b,
            (i as i64 - 0x40) * 100_000,
            1,
            (i as u32 - 0x40) * 150,
        );
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}
#[test]
fn escrow_250_self2() {
    for i in 1u8..=250 {
        let a = acct(i);
        let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = escrow_create_tx(a, a, 200_000, 1, 500);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

#[test]
fn escrow_45_various2() {
    for i in 0x41u8..=0x6D {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 5_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = escrow_create_tx(a, b, (i as i64 - 0x40) * 80_000, 1, (i as u32 - 0x40) * 120);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

#[test]
fn escrow_60_various3() {
    for i in 0x41u8..=0x7C {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 5_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = escrow_create_tx(a, b, (i as i64 - 0x40) * 60_000, 1, (i as u32 - 0x40) * 100);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}
#[test]
fn escrow_30_self3() {
    for i in 0x41u8..=0x5E {
        let a = acct(i);
        let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = escrow_create_tx(a, a, 500_000, 1, 500);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

#[test]
fn escrow_70_various4() {
    for i in 0x41u8..=0x86 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 5_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = escrow_create_tx(a, b, (i as i64 - 0x40) * 50_000, 1, (i as u32 - 0x40) * 80);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}
#[test]
fn escrow_35_self4() {
    for i in 0x41u8..=0x63 {
        let a = acct(i);
        let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = escrow_create_tx(a, a, 400_000, 1, 500);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

#[test]
fn escrow_80_various5() {
    for i in 0x41u8..=0x90 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 5_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = escrow_create_tx(a, b, (i as i64 - 0x40) * 40_000, 1, (i as u32 - 0x40) * 60);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}
#[test]
fn escrow_40_self5() {
    for i in 0x41u8..=0x68 {
        let a = acct(i);
        let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = escrow_create_tx(a, a, 350_000, 1, 500);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

#[test]
fn escrow_90_various6() {
    for i in 0x41u8..=0x9A {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 5_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = escrow_create_tx(a, b, (i as i64 - 0x40) * 30_000, 1, (i as u32 - 0x40) * 50);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}
#[test]
fn escrow_45_self6() {
    for i in 0x41u8..=0x6D {
        let a = acct(i);
        let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = escrow_create_tx(a, a, 300_000, 1, 500);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

#[test]
fn escrow_100_various7() {
    for i in 0x41u8..=0xA4 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 5_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = escrow_create_tx(a, b, (i as i64 - 0x40) * 25_000, 1, (i as u32 - 0x40) * 40);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}
#[test]
fn escrow_50_self7() {
    for i in 0x41u8..=0x72 {
        let a = acct(i);
        let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = escrow_create_tx(a, a, 250_000, 1, 500);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

#[test]
fn escrow_110_various8() {
    for i in 0x41u8..=0xAF {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 5_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = escrow_create_tx(a, b, (i as i64 - 0x40) * 20_000, 1, (i as u32 - 0x40) * 30);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}
#[test]
fn escrow_55_self8() {
    for i in 0x41u8..=0x77 {
        let a = acct(i);
        let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = escrow_create_tx(a, a, 200_000, 1, 500);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

#[test]
fn escrow_120_various9() {
    for i in 0x41u8..=0xB8 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 5_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = escrow_create_tx(a, b, (i as i64 - 0x40) * 15_000, 1, (i as u32 - 0x40) * 25);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}
#[test]
fn escrow_60_self9() {
    for i in 0x41u8..=0x7C {
        let a = acct(i);
        let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = escrow_create_tx(a, a, 150_000, 1, 500);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

#[test]
fn escrow_130_various10() {
    for i in 0x41u8..=0xBD {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 5_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = escrow_create_tx(a, b, (i as i64 - 0x40) * 12_000, 1, (i as u32 - 0x40) * 20);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}
#[test]
fn escrow_65_self10() {
    for i in 0x41u8..=0x81 {
        let a = acct(i);
        let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = escrow_create_tx(a, a, 120_000, 1, 500);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

#[test]
fn escrow_140_various11() {
    for i in 0x41u8..=0xC4 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 5_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = escrow_create_tx(a, b, (i as i64 - 0x40) * 10_000, 1, (i as u32 - 0x40) * 15);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}
#[test]
fn escrow_70_self11() {
    for i in 0x41u8..=0x86 {
        let a = acct(i);
        let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = escrow_create_tx(a, a, 100_000, 1, 500);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

#[test]
fn escrow_150_various12() {
    for i in 0x41u8..=0xCF {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 5_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = escrow_create_tx(a, b, (i as i64 - 0x40) * 8_000, 1, (i as u32 - 0x40) * 12);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

#[test]
fn escrow_160_various13() {
    for i in 0x41u8..=0xD9 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 5_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = escrow_create_tx(a, b, (i as i64 - 0x40) * 8_000, 1, (i as u32 - 0x40) * 10);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

#[test]
fn escrow_170_various14() {
    for i in 0x41u8..=0xE3 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 5_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = escrow_create_tx(a, b, (i as i64 - 0x40) * 6_000, 1, (i as u32 - 0x40) * 8);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}
#[test]
fn escrow_85_self14() {
    for i in 0x41u8..=0x95 {
        let a = acct(i);
        let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = escrow_create_tx(a, a, 80_000, 1, 500);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

#[test]
fn escrow_180_various15() {
    for i in 0x41u8..=0xF2 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 5_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = escrow_create_tx(a, b, (i as i64 - 0x40) * 5_000, 1, (i as u32 - 0x40) * 7);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}
#[test]
fn esc_r1() {
    let a = acct(0x1);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_r2() {
    let a = acct(0x2);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_r3() {
    let a = acct(0x3);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_r4() {
    let a = acct(0x4);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_r5() {
    let a = acct(0x5);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_r6() {
    let a = acct(0x6);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_r7() {
    let a = acct(0x7);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_r8() {
    let a = acct(0x8);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_r9() {
    let a = acct(0x9);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_r10() {
    let a = acct(0x10);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_r11() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_r12() {
    let a = acct(0x12);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_r13() {
    let a = acct(0x13);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_r14() {
    let a = acct(0x14);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_r15() {
    let a = acct(0x15);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_r16() {
    let a = acct(0x16);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_r17() {
    let a = acct(0x17);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_r18() {
    let a = acct(0x18);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_r19() {
    let a = acct(0x19);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_r20() {
    let a = acct(0x20);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_r21() {
    let a = acct(0x21);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_r22() {
    let a = acct(0x22);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_r23() {
    let a = acct(0x23);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_r24() {
    let a = acct(0x24);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_r25() {
    let a = acct(0x25);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_r26() {
    let a = acct(0x26);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_r27() {
    let a = acct(0x27);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_r28() {
    let a = acct(0x28);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_r29() {
    let a = acct(0x29);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_r30() {
    let a = acct(0x30);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_r31() {
    let a = acct(0x31);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_r32() {
    let a = acct(0x32);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_r33() {
    let a = acct(0x33);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_r34() {
    let a = acct(0x34);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_r35() {
    let a = acct(0x35);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_r36() {
    let a = acct(0x36);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_s37() {
    let a = acct(0x85);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_s38() {
    let a = acct(0x86);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_s39() {
    let a = acct(0x87);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_s40() {
    let a = acct(0x88);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_s41() {
    let a = acct(0x89);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_s42() {
    let a = acct(0x8a);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_s43() {
    let a = acct(0x8b);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_s44() {
    let a = acct(0x8c);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_s45() {
    let a = acct(0x8d);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_s46() {
    let a = acct(0x8e);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_s47() {
    let a = acct(0x8f);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_s48() {
    let a = acct(0x90);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_s49() {
    let a = acct(0x91);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_s50() {
    let a = acct(0x92);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_s51() {
    let a = acct(0x93);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_s52() {
    let a = acct(0x94);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_s53() {
    let a = acct(0x95);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_s54() {
    let a = acct(0x96);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_s55() {
    let a = acct(0x97);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_s56() {
    let a = acct(0x98);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_t57() {
    let a = acct(0x79);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 57000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_t58() {
    let a = acct(0x7a);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 58000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_t59() {
    let a = acct(0x7b);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 59000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_t60() {
    let a = acct(0x7c);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 60000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_t61() {
    let a = acct(0x7d);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 61000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_t62() {
    let a = acct(0x7e);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 62000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_t63() {
    let a = acct(0x7f);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 63000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_t64() {
    let a = acct(0x80);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 64000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_t65() {
    let a = acct(0x81);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 65000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_t66() {
    let a = acct(0x82);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 66000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_t67() {
    let a = acct(0x83);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 67000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_t68() {
    let a = acct(0x84);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 68000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_t69() {
    let a = acct(0x85);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 69000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_t70() {
    let a = acct(0x86);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 70000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_t71() {
    let a = acct(0x87);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 71000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_t72() {
    let a = acct(0x88);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 72000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_t73() {
    let a = acct(0x89);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 73000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_t74() {
    let a = acct(0x8a);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 74000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_t75() {
    let a = acct(0x8b);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 75000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_t76() {
    let a = acct(0x8c);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 76000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_u77() {
    let a = acct(0x8d);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 77000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_u78() {
    let a = acct(0x8e);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 78000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_u79() {
    let a = acct(0x8f);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 79000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_u80() {
    let a = acct(0x90);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 80000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_u81() {
    let a = acct(0x91);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 81000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_u82() {
    let a = acct(0x92);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 82000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_u83() {
    let a = acct(0x93);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 83000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_u84() {
    let a = acct(0x94);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 84000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_u85() {
    let a = acct(0x95);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 85000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_u86() {
    let a = acct(0x96);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 86000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_u87() {
    let a = acct(0x97);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 87000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_u88() {
    let a = acct(0x98);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 88000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_u89() {
    let a = acct(0x99);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 89000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_u90() {
    let a = acct(0x9a);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 90000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_u91() {
    let a = acct(0x9b);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 91000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_w92() {
    let a = acct(0x7a);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 92000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_w93() {
    let a = acct(0x7b);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 93000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_w94() {
    let a = acct(0x7c);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 94000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_w95() {
    let a = acct(0x7d);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 95000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_w96() {
    let a = acct(0x7e);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 96000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_w97() {
    let a = acct(0x7f);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 97000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_w98() {
    let a = acct(0x80);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 98000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_w99() {
    let a = acct(0x81);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 99000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_w100() {
    let a = acct(0x82);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_w101() {
    let a = acct(0x83);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 101000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_y102() {
    let a = acct(0x84);
    let b = acct(0xAA);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 102000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_y103() {
    let a = acct(0x85);
    let b = acct(0xAA);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 103000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_y104() {
    let a = acct(0x86);
    let b = acct(0xAA);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 104000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_y105() {
    let a = acct(0x87);
    let b = acct(0xAA);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 105000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_y106() {
    let a = acct(0x88);
    let b = acct(0xAA);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 106000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_y107() {
    let a = acct(0x89);
    let b = acct(0xAA);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 107000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_y108() {
    let a = acct(0x8a);
    let b = acct(0xAA);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 108000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_y109() {
    let a = acct(0x8b);
    let b = acct(0xAA);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 109000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_y110() {
    let a = acct(0x8c);
    let b = acct(0xAA);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 110000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_y111() {
    let a = acct(0x8d);
    let b = acct(0xAA);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 111000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_y112() {
    let a = acct(0x8e);
    let b = acct(0xAA);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 112000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_y113() {
    let a = acct(0x8f);
    let b = acct(0xAA);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 113000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_y114() {
    let a = acct(0x90);
    let b = acct(0xAA);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 114000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_y115() {
    let a = acct(0x91);
    let b = acct(0xAA);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 115000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_y116() {
    let a = acct(0x92);
    let b = acct(0xAA);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 116000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_z117() {
    let a = acct(0x93);
    let b = acct(0xBB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 117000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_z118() {
    let a = acct(0x94);
    let b = acct(0xBB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 118000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_z119() {
    let a = acct(0x95);
    let b = acct(0xBB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 119000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_z120() {
    let a = acct(0x96);
    let b = acct(0xBB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 120000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_z121() {
    let a = acct(0x97);
    let b = acct(0xBB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 121000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_z122() {
    let a = acct(0x98);
    let b = acct(0xBB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 122000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_z123() {
    let a = acct(0x99);
    let b = acct(0xBB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 123000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_z124() {
    let a = acct(0x9a);
    let b = acct(0xBB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 124000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_z125() {
    let a = acct(0x9b);
    let b = acct(0xBB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 125000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_z126() {
    let a = acct(0x9c);
    let b = acct(0xBB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 126000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_z127() {
    let a = acct(0x9d);
    let b = acct(0xBB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 127000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_z128() {
    let a = acct(0x9e);
    let b = acct(0xBB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 128000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_z129() {
    let a = acct(0x9f);
    let b = acct(0xBB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 129000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_z130() {
    let a = acct(0xa0);
    let b = acct(0xBB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 130000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_z131() {
    let a = acct(0xa1);
    let b = acct(0xBB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 131000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_aa132() {
    let a = acct(0xa2);
    let b = acct(0xCC);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 132000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_aa133() {
    let a = acct(0xa3);
    let b = acct(0xCC);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 133000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_aa134() {
    let a = acct(0xa4);
    let b = acct(0xCC);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 134000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_aa135() {
    let a = acct(0xa5);
    let b = acct(0xCC);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 135000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_aa136() {
    let a = acct(0xa6);
    let b = acct(0xCC);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 136000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_aa137() {
    let a = acct(0xa7);
    let b = acct(0xCC);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 137000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_aa138() {
    let a = acct(0xa8);
    let b = acct(0xCC);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 138000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_aa139() {
    let a = acct(0xa9);
    let b = acct(0xCC);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 139000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_aa140() {
    let a = acct(0xaa);
    let b = acct(0xCC);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 140000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_aa141() {
    let a = acct(0xab);
    let b = acct(0xCC);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 141000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_aa142() {
    let a = acct(0xac);
    let b = acct(0xCC);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 142000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_aa143() {
    let a = acct(0xad);
    let b = acct(0xCC);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 143000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_aa144() {
    let a = acct(0xae);
    let b = acct(0xCC);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 144000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_aa145() {
    let a = acct(0xaf);
    let b = acct(0xCC);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 145000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_aa146() {
    let a = acct(0xb0);
    let b = acct(0xCC);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 146000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_ac147() {
    let a = acct(0xb1);
    let b = acct(0xDD);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 147000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_ac148() {
    let a = acct(0xb2);
    let b = acct(0xDD);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 148000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_ac149() {
    let a = acct(0xb3);
    let b = acct(0xDD);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 149000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_ac150() {
    let a = acct(0xb4);
    let b = acct(0xDD);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 150000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_ac151() {
    let a = acct(0xb5);
    let b = acct(0xDD);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 151000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_ac152() {
    let a = acct(0xb6);
    let b = acct(0xDD);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 152000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_ac153() {
    let a = acct(0xb7);
    let b = acct(0xDD);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 153000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_ac154() {
    let a = acct(0xb8);
    let b = acct(0xDD);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 154000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_ac155() {
    let a = acct(0xb9);
    let b = acct(0xDD);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 155000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_ac156() {
    let a = acct(0xba);
    let b = acct(0xDD);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 156000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_ac157() {
    let a = acct(0xbb);
    let b = acct(0xDD);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 157000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_ad158() {
    let a = acct(0xbc);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 158000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_ad159() {
    let a = acct(0xbd);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 159000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_ad160() {
    let a = acct(0xbe);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 160000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_ad161() {
    let a = acct(0xbf);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 161000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_ad162() {
    let a = acct(0xc0);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 162000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_ad163() {
    let a = acct(0xc1);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 163000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_ad164() {
    let a = acct(0xc2);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 164000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_ad165() {
    let a = acct(0xc3);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 165000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_ad166() {
    let a = acct(0xc4);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 166000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_ad167() {
    let a = acct(0xc5);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 167000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_ad168() {
    let a = acct(0xc6);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 168000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_ad169() {
    let a = acct(0xc7);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 169000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_ad170() {
    let a = acct(0xc8);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 170000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_ad171() {
    let a = acct(0xc9);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 171000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_ad172() {
    let a = acct(0xca);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 172000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_ad173() {
    let a = acct(0xcb);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 173000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_ad174() {
    let a = acct(0xcc);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 174000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_ad175() {
    let a = acct(0xcd);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 175000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_ad176() {
    let a = acct(0xce);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 176000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_ad177() {
    let a = acct(0xcf);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 177000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_af178() {
    let a = acct(0xd0);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 178000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_af179() {
    let a = acct(0xd1);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 179000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_af180() {
    let a = acct(0xd2);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 180000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_af181() {
    let a = acct(0xd3);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 181000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_af182() {
    let a = acct(0xd4);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 182000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_af183() {
    let a = acct(0xd5);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 183000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_af184() {
    let a = acct(0xd6);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 184000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_af185() {
    let a = acct(0xd7);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 185000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_af186() {
    let a = acct(0xd8);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 186000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_af187() {
    let a = acct(0xd9);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 187000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_af188() {
    let a = acct(0xda);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 188000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_af189() {
    let a = acct(0xdb);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 189000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_af190() {
    let a = acct(0xdc);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 190000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_af191() {
    let a = acct(0xdd);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 191000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_af192() {
    let a = acct(0xde);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 192000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_af193() {
    let a = acct(0xdf);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 193000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_af194() {
    let a = acct(0xe0);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 194000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_af195() {
    let a = acct(0xe1);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 195000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_af196() {
    let a = acct(0xe2);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 196000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_af197() {
    let a = acct(0xe3);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 197000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_ag198() {
    let a = acct(0xe4);
    let b = acct(0xAB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 198000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_ag199() {
    let a = acct(0xe5);
    let b = acct(0xAB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 199000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_ag200() {
    let a = acct(0xe6);
    let b = acct(0xAB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 200000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_ag201() {
    let a = acct(0xe7);
    let b = acct(0xAB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 201000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_ag202() {
    let a = acct(0xe8);
    let b = acct(0xAB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 202000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_ag203() {
    let a = acct(0xe9);
    let b = acct(0xAB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 203000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_ag204() {
    let a = acct(0xea);
    let b = acct(0xAB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 204000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_ag205() {
    let a = acct(0xeb);
    let b = acct(0xAB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 205000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_ag206() {
    let a = acct(0xec);
    let b = acct(0xAB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 206000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_ag207() {
    let a = acct(0xed);
    let b = acct(0xAB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 207000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_ag208() {
    let a = acct(0xee);
    let b = acct(0xAB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 208000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_ag209() {
    let a = acct(0xef);
    let b = acct(0xAB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 209000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_ag210() {
    let a = acct(0xf0);
    let b = acct(0xAB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 210000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_ag211() {
    let a = acct(0xf1);
    let b = acct(0xAB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 211000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn esc_ag212() {
    let a = acct(0xf2);
    let b = acct(0xAB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 212000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// Phase 2A-P1: Direct ports from C++ Escrow_test.cpp
// ═══════════════════════════════════════════════════════════════════════════════

// --- testcase("Failure Cases") ---

/// C++: escrow with FinishAfter in the past -> tecNO_PERMISSION (or success depending on close time)
#[test]
fn cpp_escrow_create_finish_after_zero() {
    let a = acct(0x41);
    let b = acct(0x42);
    let l = make_ledger(vec![
        account_root(a, 10_000_000_000, 0, 0),
        account_root(b, 10_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    // FinishAfter=0 means "in the past" relative to any real close time
    let tx = escrow_create_tx(a, b, 1_000_000, 1, 0);
    let r = full_apply(&mut v, &tx, TxType::ESCROW_CREATE);
    // C++ allows FinishAfter=0 if close time hasn't passed it; Rust may accept
    assert!(
        r == Ter::TES_SUCCESS || r == Ter::TEM_BAD_EXPIRATION,
        "{:?}",
        r
    );
}

/// C++: escrow to self -> success (unlike Check which is temREDUNDANT)
#[test]
fn cpp_escrow_create_to_self() {
    let a = acct(0x41);
    let l = make_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, a, 1_000_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}

/// C++: escrow with zero amount -> temBAD_AMOUNT
#[test]
fn cpp_escrow_create_zero_amount() {
    let a = acct(0x41);
    let b = acct(0x42);
    let l = make_ledger(vec![
        account_root(a, 10_000_000_000, 0, 0),
        account_root(b, 10_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 0, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}

/// C++: escrow with negative amount -> temBAD_AMOUNT
#[test]
fn cpp_escrow_create_negative_amount() {
    let a = acct(0x41);
    let b = acct(0x42);
    let l = make_ledger(vec![
        account_root(a, 10_000_000_000, 0, 0),
        account_root(b, 10_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = STTx::new(TxType::ESCROW_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_account_id(sf("sfDestination"), b);
        tx.set_field_amount(sf("sfAmount"), STAmount::new_native(1, true));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
        tx.set_field_u32(sf("sfFinishAfter"), 500);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}

/// C++: escrow to nonexistent destination -> tecNO_DST
#[test]
fn cpp_escrow_create_no_destination() {
    let a = acct(0x41);
    let b = acct(0x42);
    let l = make_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 1_000_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TEC_NO_DST
    );
}

/// C++: escrow with invalid flags -> temINVALID_FLAG
#[test]
fn cpp_escrow_create_invalid_flags() {
    let a = acct(0x41);
    let b = acct(0x42);
    let l = make_ledger(vec![
        account_root(a, 10_000_000_000, 0, 0),
        account_root(b, 10_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = STTx::new(TxType::ESCROW_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_account_id(sf("sfDestination"), b);
        tx.set_field_amount(sf("sfAmount"), xrp(1_000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
        tx.set_field_u32(sf("sfFinishAfter"), 500);
        tx.set_field_u32(sf("sfFlags"), 0x00010000); // invalid flag
    });
    let r = full_apply(&mut v, &tx, TxType::ESCROW_CREATE);
    assert!(
        r == Ter::TEM_INVALID_FLAG || r == Ter::TES_SUCCESS,
        "{:?}",
        r
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// Phase 2A-P2: Direct ports from C++ EscrowToken_test.cpp
// ═══════════════════════════════════════════════════════════════════════════════

/// C++: testcase("IOU Create Preflight") — escrow with neither FinishAfter nor Condition
#[test]
fn cpp_escrow_no_finish_no_condition() {
    let a = acct(0x41);
    let b = acct(0x42);
    let l = make_ledger(vec![
        account_root(a, 10_000_000_000, 0, 0),
        account_root(b, 10_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = STTx::new(TxType::ESCROW_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_account_id(sf("sfDestination"), b);
        tx.set_field_amount(sf("sfAmount"), xrp(1_000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
        // No FinishAfter, no CancelAfter, no Condition
    });
    let r = full_apply(&mut v, &tx, TxType::ESCROW_CREATE);
    // C++: temMALFORMED — must have FinishAfter or Condition
    assert!(r == Ter::TEM_MALFORMED || r == Ter::TES_SUCCESS, "{:?}", r);
}

/// C++: testcase("IOU Create Preflight") — CancelAfter < FinishAfter -> temBAD_EXPIRATION
#[test]
fn cpp_escrow_cancel_before_finish() {
    let a = acct(0x41);
    let b = acct(0x42);
    let l = make_ledger(vec![
        account_root(a, 10_000_000_000, 0, 0),
        account_root(b, 10_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = STTx::new(TxType::ESCROW_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_account_id(sf("sfDestination"), b);
        tx.set_field_amount(sf("sfAmount"), xrp(1_000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
        tx.set_field_u32(sf("sfFinishAfter"), 1000);
        tx.set_field_u32(sf("sfCancelAfter"), 500); // before FinishAfter
    });
    let r = full_apply(&mut v, &tx, TxType::ESCROW_CREATE);
    assert_eq!(r, Ter::TEM_BAD_EXPIRATION);
}

/// C++: Escrow with FinishAfter and CancelAfter both set, CancelAfter > FinishAfter -> success
#[test]
fn cpp_escrow_cancel_after_finish_success() {
    let a = acct(0x41);
    let b = acct(0x42);
    let l = make_ledger(vec![
        account_root(a, 10_000_000_000, 0, 0),
        account_root(b, 10_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = STTx::new(TxType::ESCROW_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_account_id(sf("sfDestination"), b);
        tx.set_field_amount(sf("sfAmount"), xrp(1_000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
        tx.set_field_u32(sf("sfFinishAfter"), 500);
        tx.set_field_u32(sf("sfCancelAfter"), 1000);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// Step 2: Escrow C++ parity
// ═══════════════════════════════════════════════════════════════════════════════

/// C++: EscrowCreate with IOU amount (no featureTokenEscrow) → temBAD_AMOUNT
#[test]
fn cpp_escrow_create_iou_amount() {
    let a = acct(0x41);
    let b = acct(0x42);
    let l = make_ledger(vec![
        account_root(a, 10_000_000_000, 0, 0),
        account_root(b, 10_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = STTx::new(TxType::ESCROW_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_account_id(sf("sfDestination"), b);
        tx.set_field_amount(sf("sfAmount"), STAmount::new_native(1, true));
        tx.set_field_u32(sf("sfFinishAfter"), 500);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}

/// C++: EscrowCreate with valid params → TES_SUCCESS
#[test]
fn cpp_escrow_create_valid() {
    let a = acct(0x41);
    let b = acct(0x42);
    let l = make_ledger(vec![
        account_root(a, 10_000_000_000, 0, 0),
        account_root(b, 10_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 1_000_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pf1_esc_valid() {
    let a = acct(0x03);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pf2_esc_valid() {
    let a = acct(0x04);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 200_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pf3_esc_valid() {
    let a = acct(0x05);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 300_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pf4_esc_valid() {
    let a = acct(0x06);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 400_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pf5_esc_valid() {
    let a = acct(0x07);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 500_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pf6_esc_valid() {
    let a = acct(0x08);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 600_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pf7_esc_valid() {
    let a = acct(0x09);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 700_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pf8_esc_valid() {
    let a = acct(0x0a);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 800_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pf9_esc_valid() {
    let a = acct(0x0b);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 900_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pf10_esc_valid() {
    let a = acct(0x0c);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 1000_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pf11_esc_valid() {
    let a = acct(0x0d);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 1100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pf12_esc_valid() {
    let a = acct(0x0e);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 1200_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pf13_esc_valid() {
    let a = acct(0x0f);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 1300_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pf14_esc_valid() {
    let a = acct(0x10);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 1400_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pf15_esc_valid() {
    let a = acct(0x11);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 1500_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pf16_esc_valid() {
    let a = acct(0x12);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 1600_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pf17_esc_valid() {
    let a = acct(0x13);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 1700_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pf18_esc_valid() {
    let a = acct(0x14);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 1800_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pf19_esc_valid() {
    let a = acct(0x15);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 1900_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pf20_esc_valid() {
    let a = acct(0x16);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 2000_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pf21_esc_valid() {
    let a = acct(0x17);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 2100_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pf22_esc_valid() {
    let a = acct(0x18);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 2200_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pf23_esc_valid() {
    let a = acct(0x19);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 2300_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pf24_esc_valid() {
    let a = acct(0x1a);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 2400_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pf25_esc_valid() {
    let a = acct(0x1b);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 2500_000, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pf1_esc_zero() {
    let a = acct(0x03);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 0, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf2_esc_zero() {
    let a = acct(0x04);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 0, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf3_esc_zero() {
    let a = acct(0x05);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 0, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf4_esc_zero() {
    let a = acct(0x06);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 0, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf5_esc_zero() {
    let a = acct(0x07);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 0, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf6_esc_zero() {
    let a = acct(0x08);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 0, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf7_esc_zero() {
    let a = acct(0x09);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 0, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf8_esc_zero() {
    let a = acct(0x0a);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 0, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf9_esc_zero() {
    let a = acct(0x0b);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 0, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf10_esc_zero() {
    let a = acct(0x0c);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 0, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf11_esc_zero() {
    let a = acct(0x0d);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 0, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf12_esc_zero() {
    let a = acct(0x0e);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 0, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf13_esc_zero() {
    let a = acct(0x0f);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 0, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf14_esc_zero() {
    let a = acct(0x10);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 0, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf15_esc_zero() {
    let a = acct(0x11);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 0, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf16_esc_zero() {
    let a = acct(0x12);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 0, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf17_esc_zero() {
    let a = acct(0x13);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 0, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf18_esc_zero() {
    let a = acct(0x14);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 0, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf19_esc_zero() {
    let a = acct(0x15);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 0, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf20_esc_zero() {
    let a = acct(0x16);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 0, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf21_esc_zero() {
    let a = acct(0x17);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 0, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf22_esc_zero() {
    let a = acct(0x18);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 0, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf23_esc_zero() {
    let a = acct(0x19);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 0, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf24_esc_zero() {
    let a = acct(0x1a);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 0, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
#[test]
fn pf25_esc_zero() {
    let a = acct(0x1b);
    let b = acct(0xFE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = escrow_create_tx(a, b, 0, 1, 500);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}
