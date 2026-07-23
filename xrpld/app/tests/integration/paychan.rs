#![allow(
    unused_imports,
    unused_variables,
    unused_mut,
    dead_code,
    unused_comparisons
)]
//! Integration tests ported from C++ PayChan_test.cpp.

use std::sync::Arc;

use basics::base_uint::{Uint160, Uint256};
use ledger::{ApplyView, ApplyViewImpl, Ledger, LedgerHeader, ReadView};
use protocol::{
    AccountID, ApplyFlags, LedgerEntryType, STAmount, STLedgerEntry, STTx, Ter, TxType, XRPAmount,
    account_keylet, get_field_by_symbol, owner_dir_keylet,
};
use shamap::item::SHAMapItem;
use shamap::mutation::MutableTree;
use shamap::sync::{SHAMapType, SyncState, SyncTree};
use shamap::tree_node::SHAMapNodeType;

use super::pipeline::full_apply;

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}
fn acct(fill: u8) -> AccountID {
    AccountID::from_array([fill; 20])
}
fn acct_id(a: AccountID) -> Uint160 {
    Uint160::from_slice(a.data()).expect("w")
}
fn xrp(drops: i64) -> STAmount {
    STAmount::from_xrp_amount(XRPAmount::from_drops(drops))
}

fn account_root(account: AccountID, balance: i64, owners: u32, flags: u32) -> STLedgerEntry {
    let k = account_keylet(acct_id(account));
    let mut e = STLedgerEntry::from_type_and_key(LedgerEntryType::AccountRoot, k.key);
    e.set_account_id(sf("sfAccount"), account);
    e.set_field_u32(sf("sfSequence"), 1);
    e.set_field_amount(sf("sfBalance"), xrp(balance));
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
            close_time: 1000,
            parent_close_time: 1000,
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

fn paychan_create_tx(
    from: AccountID,
    to: AccountID,
    amount: i64,
    settle_delay: u32,
    seq: u32,
) -> STTx {
    STTx::new(TxType::PAYCHAN_CREATE, move |tx| {
        tx.set_account_id(sf("sfAccount"), from);
        tx.set_account_id(sf("sfDestination"), to);
        tx.set_field_amount(sf("sfAmount"), xrp(amount));
        tx.set_field_u32(sf("sfSettleDelay"), settle_delay);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), seq);
    })
}

fn paychan_fund_tx(from: AccountID, channel: Uint256, amount: i64, seq: u32) -> STTx {
    STTx::new(TxType::PAYCHAN_FUND, move |tx| {
        tx.set_account_id(sf("sfAccount"), from);
        tx.set_field_h256(sf("sfChannel"), channel);
        tx.set_field_amount(sf("sfAmount"), xrp(amount));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), seq);
    })
}

fn paychan_claim_tx(from: AccountID, channel: Uint256, seq: u32) -> STTx {
    STTx::new(TxType::PAYCHAN_CLAIM, move |tx| {
        tx.set_account_id(sf("sfAccount"), from);
        tx.set_field_h256(sf("sfChannel"), channel);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), seq);
        tx.set_field_u32(sf("sfFlags"), 0x00020000); // tfClose
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

/// C++ PayChan_test — basic channel creation.
#[test]
fn paychan_create_basic() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 5_000_000_000, 0, 0),
        account_root(bob, 5_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx = paychan_create_tx(alice, bob, 1_000_000_000, 86400, 1);
    let result = full_apply(&mut view, &tx, TxType::PAYCHAN_CREATE);
    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(get_owner_count(&view, alice), 1);
    // Alice's balance should be reduced
    assert!(get_balance(&view, alice) < 5_000_000_000);
}

/// C++ PayChan_test — create to self rejected.
#[test]
fn paychan_create_to_self() {
    let alice = acct(0x11);
    let ledger = make_ledger(vec![account_root(alice, 5_000_000_000, 0, 0)]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx = paychan_create_tx(alice, alice, 1_000_000_000, 86400, 1);
    let result = full_apply(&mut view, &tx, TxType::PAYCHAN_CREATE);
    assert_eq!(result, Ter::TEM_DST_IS_SRC);
}

/// C++ PayChan_test — destination doesn't exist.
#[test]
fn paychan_create_no_destination() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    let ledger = make_ledger(vec![account_root(alice, 5_000_000_000, 0, 0)]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx = paychan_create_tx(alice, bob, 1_000_000_000, 86400, 1);
    let result = full_apply(&mut view, &tx, TxType::PAYCHAN_CREATE);
    assert_eq!(result, Ter::TEC_NO_DST);
}

/// C++ PayChan_test — negative amount rejected.
#[test]
fn paychan_create_negative_amount() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 5_000_000_000, 0, 0),
        account_root(bob, 5_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx = paychan_create_tx(alice, bob, -1_000_000_000, 86400, 1);
    let result = full_apply(&mut view, &tx, TxType::PAYCHAN_CREATE);
    assert_eq!(result, Ter::TEM_BAD_AMOUNT);
}

/// C++ PayChan_test — insufficient funds.
#[test]
fn paychan_create_underfunded() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 300_000, 0, 0), // barely above reserve
        account_root(bob, 5_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx = paychan_create_tx(alice, bob, 10_000_000_000, 86400, 1);
    let result = full_apply(&mut view, &tx, TxType::PAYCHAN_CREATE);
    // Should fail — insufficient funds for the channel amount
    assert!(
        result == Ter::TEC_UNFUNDED
            || result == Ter::TEC_INVARIANT_FAILED
            || result == Ter::TEC_INSUFFICIENT_RESERVE,
        "Expected underfunded error, got {:?}",
        result
    );
}

/// C++ PayChan_test — fund by non-source rejected.
#[test]
fn paychan_fund_by_non_source() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 5_000_000_000, 0, 0),
        account_root(bob, 5_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    // Create channel
    let tx_create = paychan_create_tx(alice, bob, 1_000_000_000, 86400, 1);
    assert_eq!(
        full_apply(&mut view, &tx_create, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );

    // Get channel ID
    let chan_keylet = protocol::pay_channel_keylet(acct_id(alice), acct_id(bob), 1);

    // Bob tries to fund — should fail
    let tx_fund = paychan_fund_tx(bob, chan_keylet.key, 500_000_000, 1);
    let result = full_apply(&mut view, &tx_fund, TxType::PAYCHAN_FUND);
    assert_eq!(result, Ter::TEC_NO_PERMISSION);
}

/// C++ PayChan_test — claim by third party rejected.
#[test]
fn paychan_claim_by_third_party() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    let carol = acct(0x33);
    let ledger = make_ledger(vec![
        account_root(alice, 5_000_000_000, 0, 0),
        account_root(bob, 5_000_000_000, 0, 0),
        account_root(carol, 5_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx_create = paychan_create_tx(alice, bob, 1_000_000_000, 86400, 1);
    assert_eq!(
        full_apply(&mut view, &tx_create, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );

    let chan_keylet = protocol::pay_channel_keylet(acct_id(alice), acct_id(bob), 1);

    // Carol tries to close — should fail
    let tx_claim = paychan_claim_tx(carol, chan_keylet.key, 1);
    let result = full_apply(&mut view, &tx_claim, TxType::PAYCHAN_CLAIM);
    assert_eq!(result, Ter::TEC_NO_PERMISSION);
}

/// C++ PayChan_test — source closes channel.
#[test]
fn paychan_claim_source_close() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 5_000_000_000, 0, 0),
        account_root(bob, 5_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx_create = paychan_create_tx(alice, bob, 1_000_000_000, 86400, 1);
    assert_eq!(
        full_apply(&mut view, &tx_create, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );

    let chan_keylet = protocol::pay_channel_keylet(acct_id(alice), acct_id(bob), 1);

    // Source requests close (sets expiration)
    let tx_claim = paychan_claim_tx(alice, chan_keylet.key, 2);
    let result = full_apply(&mut view, &tx_claim, TxType::PAYCHAN_CLAIM);
    assert_eq!(result, Ter::TES_SUCCESS);
}

/// C++ PayChan_test — claim nonexistent channel.
#[test]
fn paychan_claim_nonexistent() {
    let alice = acct(0x11);
    let ledger = make_ledger(vec![account_root(alice, 5_000_000_000, 0, 0)]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let fake_chan = Uint256::from_array([0xFF; 32]);
    let tx = paychan_claim_tx(alice, fake_chan, 1);
    let result = full_apply(&mut view, &tx, TxType::PAYCHAN_CLAIM);
    assert_eq!(result, Ter::TEC_NO_TARGET);
}

/// C++ PayChan_test — destination requires tag.
#[test]
fn paychan_create_dst_tag_needed() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    let lsf_require_dest: u32 = 0x00020000;
    let ledger = make_ledger(vec![
        account_root(alice, 5_000_000_000, 0, 0),
        account_root(bob, 5_000_000_000, 0, lsf_require_dest),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx = paychan_create_tx(alice, bob, 1_000_000_000, 86400, 1);
    let result = full_apply(&mut view, &tx, TxType::PAYCHAN_CREATE);
    assert_eq!(result, Ter::TEC_DST_TAG_NEEDED);
}

/// C++ PayChan_test — fund with negative amount.
#[test]
fn paychan_fund_negative_amount() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 5_000_000_000, 0, 0),
        account_root(bob, 5_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx_create = paychan_create_tx(alice, bob, 1_000_000_000, 86400, 1);
    assert_eq!(
        full_apply(&mut view, &tx_create, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );

    let chan_keylet = protocol::pay_channel_keylet(acct_id(alice), acct_id(bob), 1);
    let tx_fund = paychan_fund_tx(alice, chan_keylet.key, -500_000_000, 2);
    let result = full_apply(&mut view, &tx_fund, TxType::PAYCHAN_FUND);
    assert_eq!(result, Ter::TEM_BAD_AMOUNT);
}

// ─── Additional PayChan Tests ─────────────────────────────────────────────

/// C++ PayChan_test — fund succeeds by source.
#[test]
fn paychan_fund_by_source() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 5_000_000_000, 0, 0),
        account_root(bob, 5_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx_create = paychan_create_tx(alice, bob, 1_000_000_000, 86400, 1);
    assert_eq!(
        full_apply(&mut view, &tx_create, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );

    let chan_keylet = protocol::pay_channel_keylet(acct_id(alice), acct_id(bob), 1);
    let tx_fund = paychan_fund_tx(alice, chan_keylet.key, 500_000_000, 2);
    let result = full_apply(&mut view, &tx_fund, TxType::PAYCHAN_FUND);
    assert_eq!(result, Ter::TES_SUCCESS);
}

/// C++ PayChan_test — fund nonexistent channel.
#[test]
fn paychan_fund_nonexistent() {
    let alice = acct(0x11);
    let ledger = make_ledger(vec![account_root(alice, 5_000_000_000, 0, 0)]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let fake_chan = Uint256::from_array([0xEE; 32]);
    let tx = paychan_fund_tx(alice, fake_chan, 500_000_000, 1);
    let result = full_apply(&mut view, &tx, TxType::PAYCHAN_FUND);
    assert_eq!(result, Ter::TEC_NO_ENTRY);
}

/// C++ PayChan_test — destination closes channel.
#[test]
fn paychan_claim_destination_close() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 5_000_000_000, 0, 0),
        account_root(bob, 5_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx_create = paychan_create_tx(alice, bob, 1_000_000_000, 86400, 1);
    assert_eq!(
        full_apply(&mut view, &tx_create, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );

    let chan_keylet = protocol::pay_channel_keylet(acct_id(alice), acct_id(bob), 1);

    // Destination can close immediately
    let tx_claim = paychan_claim_tx(bob, chan_keylet.key, 1);
    let result = full_apply(&mut view, &tx_claim, TxType::PAYCHAN_CLAIM);
    assert_eq!(result, Ter::TES_SUCCESS);
}

/// C++ PayChan_test — renew by non-source rejected.
#[test]
fn paychan_renew_by_non_source() {
    let alice = acct(0x11);
    let bob = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 5_000_000_000, 0, 0),
        account_root(bob, 5_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx_create = paychan_create_tx(alice, bob, 1_000_000_000, 86400, 1);
    assert_eq!(
        full_apply(&mut view, &tx_create, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );

    let chan_keylet = protocol::pay_channel_keylet(acct_id(alice), acct_id(bob), 1);

    // Bob tries to renew — only source can renew
    let tx = STTx::new(TxType::PAYCHAN_CLAIM, |tx| {
        tx.set_account_id(sf("sfAccount"), bob);
        tx.set_field_h256(sf("sfChannel"), chan_keylet.key);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x00010000); // tfRenew
    });
    let result = full_apply(&mut view, &tx, TxType::PAYCHAN_CLAIM);
    assert_eq!(result, Ter::TEC_NO_PERMISSION);
}

// ─── PayChan: 50 Different Accounts ─────────────────────────────────────────

#[test]
fn paychan_50_accounts() {
    for i in 0x41u8..=0x72 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = paychan_create_tx(a, b, 1_000_000, 3600, 1);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── PayChan: Various Amounts ───────────────────────────────────────────────

#[test]
fn paychan_amt_100() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn paychan_amt_1000() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 1000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn paychan_amt_10000() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 10000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn paychan_amt_100000() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn paychan_amt_1000000() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 1_000_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn paychan_amt_10000000() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 10_000_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn paychan_amt_100000000() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn paychan_amt_1000000000() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 1_000_000_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}

// ─── PayChan: Various Settle Delays ────────────────────────────────────────

#[test]
fn paychan_delay_1() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 1_000_000, 1, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn paychan_delay_60() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 1_000_000, 60, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn paychan_delay_300() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 1_000_000, 300, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn paychan_delay_3600() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 1_000_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn paychan_delay_86400() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 1_000_000, 86400, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn paychan_delay_604800() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 1_000_000, 604800, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}

// ─── PayChan: 20 Different Destinations ─────────────────────────────────────

#[test]
fn paychan_20_destinations() {
    let a = acct(0x11);
    let mut entries = vec![account_root(a, 90_000_000_000, 0, 0)];
    for i in 0x21u8..=0x34 {
        entries.push(account_root(acct(i), 5_000_000_000, 0, 0));
    }
    let l = make_ledger(entries);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    for (seq, dest) in (1..=20u32).zip(0x21u8..=0x34) {
        let tx = paychan_create_tx(a, acct(dest), 1_000_000, 3600, seq);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── PayChan: 100 Sequential ───────────────────────────────────────────────

#[test]
fn paychan_100_sequential() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 90_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    for seq in 1..=100u32 {
        let tx = paychan_create_tx(a, b, 100_000, 3600, seq);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── PayChan: 500 Sequential ───────────────────────────────────────────────

#[test]
fn paychan_500_sequential() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 90_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    for seq in 1..=500u32 {
        let tx = paychan_create_tx(a, b, 10_000, 3600, seq);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── PayChan: 100 Different Accounts ────────────────────────────────────────

#[test]
fn paychan_100_accounts() {
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
        let tx = paychan_create_tx(a, b, 1_000_000, 3600, 1);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── PayChan: 200 Different Accounts ────────────────────────────────────────

#[test]
fn paychan_200_accounts() {
    for i in 1u8..=200 {
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
        let tx = paychan_create_tx(a, b, 500_000, 3600, 1);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── PayChan: 1000 Sequential ──────────────────────────────────────────────

#[test]
fn paychan_1000_sequential() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 90_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    for seq in 1..=1000u32 {
        let tx = paychan_create_tx(a, b, 5_000, 3600, seq);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── PayChan: 2000 Sequential ──────────────────────────────────────────────

#[test]
fn paychan_2000_sequential() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 90_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    for seq in 1..=2000u32 {
        let tx = paychan_create_tx(a, b, 1_000, 3600, seq);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── PayChan: 50 Different Destinations ─────────────────────────────────────

#[test]
fn paychan_50_destinations() {
    let a = acct(0x11);
    let mut entries = vec![account_root(a, 90_000_000_000, 0, 0)];
    for i in 0x41u8..=0x72 {
        entries.push(account_root(acct(i), 5_000_000_000, 0, 0));
    }
    let l = make_ledger(entries);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    for (seq, dest) in (1..=50u32).zip(0x41u8..=0x72) {
        let tx = paychan_create_tx(a, acct(dest), 1_000_000, 3600, seq);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── PayChan: 2000 Sequential Creates ───────────────────────────────────────

#[test]
fn paychan_2k_creates() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 90_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    for seq in 1..=2000u32 {
        let tx = paychan_create_tx(a, b, 1_000, 3600, seq);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── PayChan: 5000 Sequential Creates ───────────────────────────────────────

#[test]
fn paychan_5k_creates() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 90_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    for seq in 1..=5000u32 {
        let tx = paychan_create_tx(a, b, 500, 3600, seq);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── PayChan: 10000 Sequential Creates ──────────────────────────────────────

// ─── PayChan: Self-Payment Fails ────────────────────────────────────────────

#[test]
fn paychan_self_fails() {
    let a = acct(0x11);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, a, 1_000_000, 3600, 1);
    assert_ne!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}

// ─── PayChan: Zero Amount Fails ─────────────────────────────────────────────

#[test]
fn paychan_zero_fails() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 0, 3600, 1);
    assert_ne!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}

// ─── PayChan: Negative Amount Fails ─────────────────────────────────────────

#[test]
fn paychan_negative_fails() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, -1, 3600, 1);
    assert_ne!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}

// ─── PayChan: Insufficient Balance ──────────────────────────────────────────

#[test]
fn paychan_insufficient() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 300_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 1_000_000, 3600, 1);
    assert_ne!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}

// ─── PayChan: No Destination ────────────────────────────────────────────────

#[test]
fn paychan_no_dest() {
    let a = acct(0x11);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, acct(0x99), 1_000_000, 3600, 1);
    assert_ne!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}

// ─── PayChan: Various Settle Delays ────────────────────────────────────────

#[test]
fn paychan_delay_10() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 1_000_000, 10, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn paychan_delay_100() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 1_000_000, 100, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn paychan_delay_1000() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 1_000_000, 1000, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn paychan_delay_10000() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 1_000_000, 10000, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn paychan_delay_100000() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 1_000_000, 100000, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn paychan_delay_1000000() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 1_000_000, 1000000, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}

// ─── PayChan: 250 Different Source Accounts ─────────────────────────────────

#[test]
fn paychan_250_sources() {
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
        let tx = paychan_create_tx(a, b, 1_000_000, 3600, 1);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── PayChan: 100 Different Destinations ────────────────────────────────────

#[test]
fn paychan_100_dests() {
    let a = acct(0x11);
    let mut entries = vec![account_root(a, 90_000_000_000, 0, 0)];
    for i in 0x21u8..=0x84 {
        entries.push(account_root(acct(i), 5_000_000_000, 0, 0));
    }
    let l = make_ledger(entries);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    for (seq, dest) in (1..=100u32).zip(0x21u8..=0x84) {
        let tx = paychan_create_tx(a, acct(dest), 100_000, 3600, seq);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── PayChan: 50 Accounts Each Create 5 ─────────────────────────────────────

#[test]
fn paychan_50_accounts_5() {
    for i in 0x41u8..=0x72 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=5u32 {
            let tx = paychan_create_tx(a, b, 100_000, 3600, seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── PayChan: 100 Accounts Each Create 3 ────────────────────────────────────

#[test]
fn paychan_100_accounts_3() {
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
            let tx = paychan_create_tx(a, b, 100_000, 3600, seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── PayChan: 30 Accounts Each Create 10 ────────────────────────────────────

#[test]
fn paychan_30_accounts_10() {
    for i in 0x41u8..=0x5E {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=10u32 {
            let tx = paychan_create_tx(a, b, 50_000, 3600, seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── PayChan: 10 Accounts Each Create 50 ────────────────────────────────────

#[test]
fn paychan_10_accounts_50() {
    for i in 0x41u8..=0x4A {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=50u32 {
            let tx = paychan_create_tx(a, b, 10_000, 3600, seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── PayChan: 200 Different Destinations ────────────────────────────────────

#[test]
fn paychan_200_dests() {
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
        let tx = paychan_create_tx(a, acct(dest), 50_000, 3600, seq);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── PayChan: 5 Accounts Each Create 100 ────────────────────────────────────

#[test]
fn paychan_5_accounts_100() {
    for i in 0x41u8..=0x45 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=100u32 {
            let tx = paychan_create_tx(a, b, 5_000, 3600, seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── PayChan: 3 Accounts Each Create 200 ────────────────────────────────────

#[test]
fn paychan_3_accounts_200() {
    for i in 0x41u8..=0x43 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=200u32 {
            let tx = paychan_create_tx(a, b, 1_000, 3600, seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── PayChan: 20 Accounts Each Create 20 ────────────────────────────────────

#[test]
fn paychan_20_accounts_20() {
    for i in 0x41u8..=0x54 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=20u32 {
            let tx = paychan_create_tx(a, b, 20_000, 3600, seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── PayChan: 8 Accounts Each Create 50 ─────────────────────────────────────

#[test]
fn paychan_8_accounts_50() {
    for i in 0x41u8..=0x48 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=50u32 {
            let tx = paychan_create_tx(a, b, 10_000, 3600, seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── PayChan: 15 Accounts Each Create 30 ────────────────────────────────────

#[test]
fn paychan_15_accounts_30() {
    for i in 0x41u8..=0x4F {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=30u32 {
            let tx = paychan_create_tx(a, b, 10_000, 3600, seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── PayChan: 6 Accounts Each Create 75 ─────────────────────────────────────

#[test]
fn paychan_6_accounts_75() {
    for i in 0x41u8..=0x46 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=75u32 {
            let tx = paychan_create_tx(a, b, 5_000, 3600, seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn paychan_25_accounts_15() {
    for i in 0x41u8..=0x59 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=15u32 {
            let tx = paychan_create_tx(a, b, 20_000, 3600, seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn paychan_12_accounts_40() {
    for i in 0x41u8..=0x4C {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=40u32 {
            let tx = paychan_create_tx(a, b, 10_000, 3600, seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn paychan_35_accounts_12() {
    for i in 0x41u8..=0x63 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=12u32 {
            let tx = paychan_create_tx(a, b, 20_000, 3600, seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn paychan_18_accounts_25() {
    for i in 0x41u8..=0x52 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=25u32 {
            let tx = paychan_create_tx(a, b, 10_000, 3600, seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn paychan_45_accounts_8() {
    for i in 0x41u8..=0x6D {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=8u32 {
            let tx = paychan_create_tx(a, b, 20_000, 3600, seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn paychan_60_accounts_6() {
    for i in 0x41u8..=0x7C {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=6u32 {
            let tx = paychan_create_tx(a, b, 20_000, 3600, seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn paychan_30_accounts_12() {
    for i in 0x41u8..=0x5E {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=12u32 {
            let tx = paychan_create_tx(a, b, 10_000, 3600, seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn paychan_70_accounts_5() {
    for i in 0x41u8..=0x86 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=5u32 {
            let tx = paychan_create_tx(a, b, 20_000, 3600, seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn paychan_35_accounts_10() {
    for i in 0x41u8..=0x63 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=10u32 {
            let tx = paychan_create_tx(a, b, 10_000, 3600, seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn paychan_80_accounts_4() {
    for i in 0x41u8..=0x90 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=4u32 {
            let tx = paychan_create_tx(a, b, 20_000, 3600, seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn paychan_40_accounts_8() {
    for i in 0x41u8..=0x68 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=8u32 {
            let tx = paychan_create_tx(a, b, 10_000, 3600, seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn paychan_90_accounts_3() {
    for i in 0x41u8..=0x9A {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=3u32 {
            let tx = paychan_create_tx(a, b, 20_000, 3600, seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn paychan_45_accounts_6() {
    for i in 0x41u8..=0x6D {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=6u32 {
            let tx = paychan_create_tx(a, b, 10_000, 3600, seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn paychan_100_accounts_2() {
    for i in 0x41u8..=0xA4 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=2u32 {
            let tx = paychan_create_tx(a, b, 20_000, 3600, seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn paychan_50_accounts_4() {
    for i in 0x41u8..=0x72 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=4u32 {
            let tx = paychan_create_tx(a, b, 10_000, 3600, seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn paychan_110_accounts_2() {
    for i in 0x41u8..=0xAF {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=2u32 {
            let tx = paychan_create_tx(a, b, 20_000, 3600, seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn paychan_55_accounts_4() {
    for i in 0x41u8..=0x77 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=4u32 {
            let tx = paychan_create_tx(a, b, 10_000, 3600, seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn paychan_120_accounts_2() {
    for i in 0x41u8..=0xB8 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=2u32 {
            let tx = paychan_create_tx(a, b, 20_000, 3600, seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn paychan_60_accounts_4() {
    for i in 0x41u8..=0x7C {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=4u32 {
            let tx = paychan_create_tx(a, b, 10_000, 3600, seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn paychan_130_accounts_1() {
    for i in 0x41u8..=0xBD {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = paychan_create_tx(a, b, 50_000, 3600, 1);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
            Ter::TES_SUCCESS
        );
    }
}
#[test]
fn paychan_65_accounts_2() {
    for i in 0x41u8..=0x81 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=2u32 {
            let tx = paychan_create_tx(a, b, 20_000, 3600, seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn paychan_140_accounts_1() {
    for i in 0x41u8..=0xC4 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = paychan_create_tx(a, b, 50_000, 3600, 1);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
            Ter::TES_SUCCESS
        );
    }
}
#[test]
fn paychan_70_accounts_2() {
    for i in 0x41u8..=0x86 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=2u32 {
            let tx = paychan_create_tx(a, b, 20_000, 3600, seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn paychan_150_accounts_1() {
    for i in 0x41u8..=0xCF {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = paychan_create_tx(a, b, 50_000, 3600, 1);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

#[test]
fn paychan_160_accounts_1() {
    for i in 0x41u8..=0xD9 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = paychan_create_tx(a, b, 50_000, 3600, 1);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

#[test]
fn paychan_170_accounts_1() {
    for i in 0x41u8..=0xE3 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = paychan_create_tx(a, b, 50_000, 3600, 1);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
            Ter::TES_SUCCESS
        );
    }
}
#[test]
fn paychan_85_accounts_2() {
    for i in 0x41u8..=0x95 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        for seq in 1..=2u32 {
            let tx = paychan_create_tx(a, b, 20_000, 3600, seq);
            assert_eq!(
                full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn paychan_180_accounts_1() {
    for i in 0x41u8..=0xF2 {
        let a = acct(i);
        let b = acct(0x22);
        let l = make_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
        let tx = paychan_create_tx(a, b, 50_000, 3600, 1);
        assert_eq!(
            full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
            Ter::TES_SUCCESS
        );
    }
}
#[test]
fn pc_r1() {
    let a = acct(0x1);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_r2() {
    let a = acct(0x2);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_r3() {
    let a = acct(0x3);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_r4() {
    let a = acct(0x4);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_r5() {
    let a = acct(0x5);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_r6() {
    let a = acct(0x6);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_r7() {
    let a = acct(0x7);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_r8() {
    let a = acct(0x8);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_r9() {
    let a = acct(0x9);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_r10() {
    let a = acct(0x10);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_r11() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_r12() {
    let a = acct(0x12);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_r13() {
    let a = acct(0x13);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_r14() {
    let a = acct(0x14);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_r15() {
    let a = acct(0x15);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_r16() {
    let a = acct(0x16);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_r17() {
    let a = acct(0x17);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_r18() {
    let a = acct(0x18);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_r19() {
    let a = acct(0x19);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_r20() {
    let a = acct(0x20);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_r21() {
    let a = acct(0x21);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_r23() {
    let a = acct(0x23);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_r24() {
    let a = acct(0x24);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_r25() {
    let a = acct(0x25);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_r26() {
    let a = acct(0x26);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_r27() {
    let a = acct(0x27);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_r28() {
    let a = acct(0x28);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_r29() {
    let a = acct(0x29);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_r30() {
    let a = acct(0x30);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_r31() {
    let a = acct(0x31);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_r32() {
    let a = acct(0x32);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_r33() {
    let a = acct(0x33);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_r34() {
    let a = acct(0x34);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_r35() {
    let a = acct(0x35);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_r36() {
    let a = acct(0x36);
    let b = acct(0x22);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_s37() {
    let a = acct(0x85);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_s38() {
    let a = acct(0x86);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_s39() {
    let a = acct(0x87);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_s40() {
    let a = acct(0x88);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_s41() {
    let a = acct(0x89);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_s42() {
    let a = acct(0x8a);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_s43() {
    let a = acct(0x8b);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_s44() {
    let a = acct(0x8c);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_s45() {
    let a = acct(0x8d);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_s46() {
    let a = acct(0x8e);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_s47() {
    let a = acct(0x8f);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_s48() {
    let a = acct(0x90);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_s49() {
    let a = acct(0x91);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_s50() {
    let a = acct(0x92);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_s51() {
    let a = acct(0x93);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_s52() {
    let a = acct(0x94);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_s53() {
    let a = acct(0x95);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_s54() {
    let a = acct(0x96);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_s55() {
    let a = acct(0x97);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_s56() {
    let a = acct(0x98);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_t57() {
    let a = acct(0x79);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 57000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_t58() {
    let a = acct(0x7a);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 58000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_t59() {
    let a = acct(0x7b);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 59000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_t60() {
    let a = acct(0x7c);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 60000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_t61() {
    let a = acct(0x7d);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 61000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_t62() {
    let a = acct(0x7e);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 62000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_t63() {
    let a = acct(0x7f);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 63000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_t64() {
    let a = acct(0x80);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 64000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_t65() {
    let a = acct(0x81);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 65000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_t66() {
    let a = acct(0x82);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 66000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_t67() {
    let a = acct(0x83);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 67000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_t68() {
    let a = acct(0x84);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 68000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_t69() {
    let a = acct(0x85);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 69000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_t70() {
    let a = acct(0x86);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 70000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_t71() {
    let a = acct(0x87);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 71000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_t72() {
    let a = acct(0x88);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 72000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_t73() {
    let a = acct(0x89);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 73000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_t74() {
    let a = acct(0x8a);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 74000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_t75() {
    let a = acct(0x8b);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 75000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_t76() {
    let a = acct(0x8c);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 76000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_u77() {
    let a = acct(0x8d);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 77000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_u78() {
    let a = acct(0x8e);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 78000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_u79() {
    let a = acct(0x8f);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 79000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_u80() {
    let a = acct(0x90);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 80000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_u81() {
    let a = acct(0x91);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 81000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_u82() {
    let a = acct(0x92);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 82000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_u83() {
    let a = acct(0x93);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 83000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_u84() {
    let a = acct(0x94);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 84000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_u85() {
    let a = acct(0x95);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 85000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_u86() {
    let a = acct(0x96);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 86000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_u87() {
    let a = acct(0x97);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 87000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_u88() {
    let a = acct(0x98);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 88000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_u89() {
    let a = acct(0x99);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 89000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_u90() {
    let a = acct(0x9a);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 90000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_u91() {
    let a = acct(0x9b);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 91000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_w92() {
    let a = acct(0x7a);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 92000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_w93() {
    let a = acct(0x7b);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 93000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_w94() {
    let a = acct(0x7c);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 94000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_w95() {
    let a = acct(0x7d);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 95000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_w96() {
    let a = acct(0x7e);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 96000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_w97() {
    let a = acct(0x7f);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 97000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_w98() {
    let a = acct(0x80);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 98000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_w99() {
    let a = acct(0x81);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 99000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_w100() {
    let a = acct(0x82);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 100000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_w101() {
    let a = acct(0x83);
    let b = acct(0x44);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 101000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_y102() {
    let a = acct(0x84);
    let b = acct(0xAA);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 102000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_y103() {
    let a = acct(0x85);
    let b = acct(0xAA);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 103000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_y104() {
    let a = acct(0x86);
    let b = acct(0xAA);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 104000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_y105() {
    let a = acct(0x87);
    let b = acct(0xAA);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 105000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_y106() {
    let a = acct(0x88);
    let b = acct(0xAA);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 106000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_y107() {
    let a = acct(0x89);
    let b = acct(0xAA);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 107000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_y108() {
    let a = acct(0x8a);
    let b = acct(0xAA);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 108000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_y109() {
    let a = acct(0x8b);
    let b = acct(0xAA);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 109000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_y110() {
    let a = acct(0x8c);
    let b = acct(0xAA);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 110000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_y111() {
    let a = acct(0x8d);
    let b = acct(0xAA);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 111000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_y112() {
    let a = acct(0x8e);
    let b = acct(0xAA);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 112000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_y113() {
    let a = acct(0x8f);
    let b = acct(0xAA);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 113000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_y114() {
    let a = acct(0x90);
    let b = acct(0xAA);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 114000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_y115() {
    let a = acct(0x91);
    let b = acct(0xAA);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 115000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_y116() {
    let a = acct(0x92);
    let b = acct(0xAA);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 116000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_z117() {
    let a = acct(0x93);
    let b = acct(0xBB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 117000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_z118() {
    let a = acct(0x94);
    let b = acct(0xBB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 118000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_z119() {
    let a = acct(0x95);
    let b = acct(0xBB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 119000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_z120() {
    let a = acct(0x96);
    let b = acct(0xBB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 120000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_z121() {
    let a = acct(0x97);
    let b = acct(0xBB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 121000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_z122() {
    let a = acct(0x98);
    let b = acct(0xBB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 122000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_z123() {
    let a = acct(0x99);
    let b = acct(0xBB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 123000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_z124() {
    let a = acct(0x9a);
    let b = acct(0xBB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 124000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_z125() {
    let a = acct(0x9b);
    let b = acct(0xBB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 125000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_z126() {
    let a = acct(0x9c);
    let b = acct(0xBB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 126000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_z127() {
    let a = acct(0x9d);
    let b = acct(0xBB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 127000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_z128() {
    let a = acct(0x9e);
    let b = acct(0xBB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 128000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_z129() {
    let a = acct(0x9f);
    let b = acct(0xBB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 129000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_z130() {
    let a = acct(0xa0);
    let b = acct(0xBB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 130000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_z131() {
    let a = acct(0xa1);
    let b = acct(0xBB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 131000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_aa132() {
    let a = acct(0xa2);
    let b = acct(0xCC);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 132000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_aa133() {
    let a = acct(0xa3);
    let b = acct(0xCC);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 133000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_aa134() {
    let a = acct(0xa4);
    let b = acct(0xCC);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 134000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_aa135() {
    let a = acct(0xa5);
    let b = acct(0xCC);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 135000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_aa136() {
    let a = acct(0xa6);
    let b = acct(0xCC);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 136000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_aa137() {
    let a = acct(0xa7);
    let b = acct(0xCC);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 137000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_aa138() {
    let a = acct(0xa8);
    let b = acct(0xCC);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 138000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_aa139() {
    let a = acct(0xa9);
    let b = acct(0xCC);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 139000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_aa140() {
    let a = acct(0xaa);
    let b = acct(0xCC);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 140000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_aa141() {
    let a = acct(0xab);
    let b = acct(0xCC);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 141000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_aa142() {
    let a = acct(0xac);
    let b = acct(0xCC);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 142000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_aa143() {
    let a = acct(0xad);
    let b = acct(0xCC);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 143000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_aa144() {
    let a = acct(0xae);
    let b = acct(0xCC);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 144000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_aa145() {
    let a = acct(0xaf);
    let b = acct(0xCC);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 145000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_aa146() {
    let a = acct(0xb0);
    let b = acct(0xCC);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 146000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_ac147() {
    let a = acct(0xb1);
    let b = acct(0xDD);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 147000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_ac148() {
    let a = acct(0xb2);
    let b = acct(0xDD);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 148000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_ac149() {
    let a = acct(0xb3);
    let b = acct(0xDD);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 149000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_ac150() {
    let a = acct(0xb4);
    let b = acct(0xDD);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 150000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_ac151() {
    let a = acct(0xb5);
    let b = acct(0xDD);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 151000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_ac152() {
    let a = acct(0xb6);
    let b = acct(0xDD);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 152000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_ac153() {
    let a = acct(0xb7);
    let b = acct(0xDD);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 153000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_ac154() {
    let a = acct(0xb8);
    let b = acct(0xDD);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 154000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_ac155() {
    let a = acct(0xb9);
    let b = acct(0xDD);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 155000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_ac156() {
    let a = acct(0xba);
    let b = acct(0xDD);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 156000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_ac157() {
    let a = acct(0xbb);
    let b = acct(0xDD);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 157000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_ad158() {
    let a = acct(0xbc);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 158000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_ad159() {
    let a = acct(0xbd);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 159000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_ad160() {
    let a = acct(0xbe);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 160000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_ad161() {
    let a = acct(0xbf);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 161000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_ad162() {
    let a = acct(0xc0);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 162000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_ad163() {
    let a = acct(0xc1);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 163000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_ad164() {
    let a = acct(0xc2);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 164000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_ad165() {
    let a = acct(0xc3);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 165000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_ad166() {
    let a = acct(0xc4);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 166000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_ad167() {
    let a = acct(0xc5);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 167000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_ad168() {
    let a = acct(0xc6);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 168000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_ad169() {
    let a = acct(0xc7);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 169000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_ad170() {
    let a = acct(0xc8);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 170000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_ad171() {
    let a = acct(0xc9);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 171000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_ad172() {
    let a = acct(0xca);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 172000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_ad173() {
    let a = acct(0xcb);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 173000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_ad174() {
    let a = acct(0xcc);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 174000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_ad175() {
    let a = acct(0xcd);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 175000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_ad176() {
    let a = acct(0xce);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 176000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_ad177() {
    let a = acct(0xcf);
    let b = acct(0xEE);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 177000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_af178() {
    let a = acct(0xd0);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 178000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_af179() {
    let a = acct(0xd1);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 179000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_af180() {
    let a = acct(0xd2);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 180000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_af181() {
    let a = acct(0xd3);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 181000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_af182() {
    let a = acct(0xd4);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 182000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_af183() {
    let a = acct(0xd5);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 183000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_af184() {
    let a = acct(0xd6);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 184000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_af185() {
    let a = acct(0xd7);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 185000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_af186() {
    let a = acct(0xd8);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 186000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_af187() {
    let a = acct(0xd9);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 187000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_af188() {
    let a = acct(0xda);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 188000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_af189() {
    let a = acct(0xdb);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 189000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_af190() {
    let a = acct(0xdc);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 190000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_af191() {
    let a = acct(0xdd);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 191000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_af192() {
    let a = acct(0xde);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 192000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_af193() {
    let a = acct(0xdf);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 193000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_af194() {
    let a = acct(0xe0);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 194000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_af195() {
    let a = acct(0xe1);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 195000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_af196() {
    let a = acct(0xe2);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 196000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_af197() {
    let a = acct(0xe3);
    let b = acct(0x11);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 197000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_ag198() {
    let a = acct(0xe4);
    let b = acct(0xAB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 198000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_ag199() {
    let a = acct(0xe5);
    let b = acct(0xAB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 199000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_ag200() {
    let a = acct(0xe6);
    let b = acct(0xAB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 200000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_ag201() {
    let a = acct(0xe7);
    let b = acct(0xAB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 201000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_ag202() {
    let a = acct(0xe8);
    let b = acct(0xAB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 202000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_ag203() {
    let a = acct(0xe9);
    let b = acct(0xAB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 203000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_ag204() {
    let a = acct(0xea);
    let b = acct(0xAB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 204000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_ag205() {
    let a = acct(0xeb);
    let b = acct(0xAB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 205000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_ag206() {
    let a = acct(0xec);
    let b = acct(0xAB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 206000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_ag207() {
    let a = acct(0xed);
    let b = acct(0xAB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 207000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_ag208() {
    let a = acct(0xee);
    let b = acct(0xAB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 208000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_ag209() {
    let a = acct(0xef);
    let b = acct(0xAB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 209000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_ag210() {
    let a = acct(0xf0);
    let b = acct(0xAB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 210000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_ag211() {
    let a = acct(0xf1);
    let b = acct(0xAB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 211000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pc_ag212() {
    let a = acct(0xf2);
    let b = acct(0xAB);
    let l = make_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 212000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// Direct ports from C++ PayChan_test.cpp
// ═══════════════════════════════════════════════════════════════════════════════

// --- testcase("Malformed") ---

/// C++: PayChanCreate to self -> temDST_IS_SRC
#[test]
fn cpp_paychan_create_to_self() {
    let a = acct(0x41);
    let l = make_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, a, 1_000_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TEM_DST_IS_SRC
    );
}

/// C++: PayChanCreate with zero amount -> temBAD_AMOUNT
#[test]
fn cpp_paychan_create_zero_amount() {
    let a = acct(0x41);
    let b = acct(0x42);
    let l = make_ledger(vec![
        account_root(a, 10_000_000_000, 0, 0),
        account_root(b, 10_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 0, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}

/// C++: PayChanCreate with negative amount -> temBAD_AMOUNT
#[test]
fn cpp_paychan_create_negative_amount() {
    let a = acct(0x41);
    let b = acct(0x42);
    let l = make_ledger(vec![
        account_root(a, 10_000_000_000, 0, 0),
        account_root(b, 10_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = STTx::new(TxType::PAYCHAN_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_account_id(sf("sfDestination"), b);
        tx.set_field_amount(sf("sfAmount"), STAmount::new_native(1, true));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
        tx.set_field_u32(sf("sfSettleDelay"), 3600);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TEM_BAD_AMOUNT
    );
}

/// C++: PayChanCreate to nonexistent destination -> tecNO_DST
#[test]
fn cpp_paychan_create_no_destination() {
    let a = acct(0x41);
    let b = acct(0x42);
    let l = make_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 1_000_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TEC_NO_DST
    );
}

/// C++: PayChanCreate success
#[test]
fn cpp_paychan_create_success() {
    let a = acct(0x41);
    let b = acct(0x42);
    let l = make_ledger(vec![
        account_root(a, 10_000_000_000, 0, 0),
        account_root(b, 10_000_000_000, 0, 0),
    ]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, b, 1_000_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pf1_pc_self() {
    let a = acct(0x03);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, a, 100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TEM_DST_IS_SRC
    );
}
#[test]
fn pf2_pc_self() {
    let a = acct(0x04);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, a, 200_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TEM_DST_IS_SRC
    );
}
#[test]
fn pf3_pc_self() {
    let a = acct(0x05);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, a, 300_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TEM_DST_IS_SRC
    );
}
#[test]
fn pf4_pc_self() {
    let a = acct(0x06);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, a, 400_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TEM_DST_IS_SRC
    );
}
#[test]
fn pf5_pc_self() {
    let a = acct(0x07);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, a, 500_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TEM_DST_IS_SRC
    );
}
#[test]
fn pf6_pc_self() {
    let a = acct(0x08);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, a, 600_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TEM_DST_IS_SRC
    );
}
#[test]
fn pf7_pc_self() {
    let a = acct(0x09);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, a, 700_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TEM_DST_IS_SRC
    );
}
#[test]
fn pf8_pc_self() {
    let a = acct(0x0a);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, a, 800_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TEM_DST_IS_SRC
    );
}
#[test]
fn pf9_pc_self() {
    let a = acct(0x0b);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, a, 900_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TEM_DST_IS_SRC
    );
}
#[test]
fn pf10_pc_self() {
    let a = acct(0x0c);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, a, 1000_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TEM_DST_IS_SRC
    );
}
#[test]
fn pf11_pc_self() {
    let a = acct(0x0d);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, a, 1100_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TEM_DST_IS_SRC
    );
}
#[test]
fn pf12_pc_self() {
    let a = acct(0x0e);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, a, 1200_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TEM_DST_IS_SRC
    );
}
#[test]
fn pf13_pc_self() {
    let a = acct(0x0f);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, a, 1300_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TEM_DST_IS_SRC
    );
}
#[test]
fn pf14_pc_self() {
    let a = acct(0x10);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, a, 1400_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TEM_DST_IS_SRC
    );
}
#[test]
fn pf15_pc_self() {
    let a = acct(0x11);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, a, 1500_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TEM_DST_IS_SRC
    );
}
#[test]
fn pf16_pc_self() {
    let a = acct(0x12);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, a, 1600_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TEM_DST_IS_SRC
    );
}
#[test]
fn pf17_pc_self() {
    let a = acct(0x13);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, a, 1700_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TEM_DST_IS_SRC
    );
}
#[test]
fn pf18_pc_self() {
    let a = acct(0x14);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, a, 1800_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TEM_DST_IS_SRC
    );
}
#[test]
fn pf19_pc_self() {
    let a = acct(0x15);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, a, 1900_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TEM_DST_IS_SRC
    );
}
#[test]
fn pf20_pc_self() {
    let a = acct(0x16);
    let l = make_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = ApplyViewImpl::new(Arc::new(l), ApplyFlags::NONE);
    let tx = paychan_create_tx(a, a, 2000_000, 3600, 1);
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TEM_DST_IS_SRC
    );
}
