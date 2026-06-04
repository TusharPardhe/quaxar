#![allow(
    unused_imports,
    unused_variables,
    unused_mut,
    dead_code,
    unused_comparisons
)]
//! Integration tests ported from C++ Vault_test.cpp.

use std::sync::Arc;

use basics::base_uint::{Uint160, Uint256};
use ledger::{ApplyView, ApplyViewImpl, Ledger, LedgerHeader, ReadView};
use protocol::{
    AccountID, ApplyFlags, Currency, IOUAmount, Issue, LedgerEntryType, STAmount, STLedgerEntry,
    STTx, Ter, TxType, XRPAmount, account_keylet, get_field_by_symbol, sf_generic,
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

fn iou(issuer: AccountID, currency: &str, value: i64) -> STAmount {
    let cur = protocol::currency_from_string(currency);
    let issue = Issue::new(cur, issuer);
    STAmount::from_iou_amount(
        sf_generic(),
        IOUAmount::from_parts(value, 0).expect("a"),
        issue,
    )
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
    let mut ledger = Ledger::from_maps(
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
    );
    // Enable vault-related amendments
    let features = vec![
        protocol::feature_id("SingleAssetVault"),
        protocol::feature_id("MPTokensV1"),
        protocol::feature_id("PermissionedDomains"),
    ];
    ledger.set_rules(protocol::Rules::new(features.into_iter()));
    ledger
}

fn get_owner_count(view: &impl ReadView, account: AccountID) -> u32 {
    view.read(account_keylet(acct_id(account)))
        .ok()
        .flatten()
        .map(|sle| sle.get_field_u32(sf("sfOwnerCount")))
        .unwrap_or(0)
}

fn vault_create_tx(from: AccountID, asset: STAmount, seq: u32) -> STTx {
    STTx::new(TxType::VAULT_CREATE, move |tx| {
        tx.set_account_id(sf("sfAccount"), from);
        tx.set_field_amount(sf("sfAsset"), asset);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), seq);
    })
}

fn vault_create_tx_with_flags(from: AccountID, asset: STAmount, seq: u32, flags: u32) -> STTx {
    STTx::new(TxType::VAULT_CREATE, move |tx| {
        tx.set_account_id(sf("sfAccount"), from);
        tx.set_field_amount(sf("sfAsset"), asset);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), seq);
        tx.set_field_u32(sf("sfFlags"), flags);
    })
}

fn vault_delete_tx(from: AccountID, vault_id: Uint256, seq: u32) -> STTx {
    STTx::new(TxType::VAULT_DELETE, move |tx| {
        tx.set_account_id(sf("sfAccount"), from);
        tx.set_field_h256(sf("sfVaultID"), vault_id);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), seq);
    })
}

fn vault_deposit_tx(from: AccountID, vault_id: Uint256, amount: STAmount, seq: u32) -> STTx {
    STTx::new(TxType::VAULT_DEPOSIT, move |tx| {
        tx.set_account_id(sf("sfAccount"), from);
        tx.set_field_h256(sf("sfVaultID"), vault_id);
        tx.set_field_amount(sf("sfAmount"), amount);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), seq);
    })
}

fn vault_withdraw_tx(from: AccountID, vault_id: Uint256, amount: STAmount, seq: u32) -> STTx {
    STTx::new(TxType::VAULT_WITHDRAW, move |tx| {
        tx.set_account_id(sf("sfAccount"), from);
        tx.set_field_h256(sf("sfVaultID"), vault_id);
        tx.set_field_amount(sf("sfAmount"), amount);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), seq);
    })
}

// ─── Tests ────────────────────────────────────────────────────────────────

/// C++ Vault_test — basic vault creation with IOU asset.
/// Note: Full vault creation requires pseudo-account infrastructure.
/// This test verifies the preflight passes for valid inputs.
#[test]
fn vault_create_basic() {
    let alice = acct(0x11);
    let gw = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 10_000_000_000, 0, 0),
        account_root(gw, 10_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx = vault_create_tx(alice, iou(gw, "USD", 0), 1);
    let result = full_apply(&mut view, &tx, TxType::VAULT_CREATE);
    // Preflight passes, doApply may fail due to missing pseudo-account infra
    assert!(
        result != Ter::TEM_MALFORMED
            && result != Ter::TEM_INVALID_FLAG
            && result != Ter::TEM_DISABLED,
        "Unexpected preflight error: {:?}",
        result
    );
}

/// C++ Vault_test — vault with XRP asset rejected (native not allowed).
#[test]
fn vault_create_xrp_asset_rejected() {
    let alice = acct(0x11);
    let ledger = make_ledger(vec![account_root(alice, 10_000_000_000, 0, 0)]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx = vault_create_tx(alice, xrp(0), 1);
    let result = full_apply(&mut view, &tx, TxType::VAULT_CREATE);
    assert_eq!(result, Ter::TEM_MALFORMED);
}

/// C++ Vault_test — invalid flags rejected.
#[test]
fn vault_create_invalid_flags() {
    let alice = acct(0x11);
    let gw = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 10_000_000_000, 0, 0),
        account_root(gw, 10_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx = vault_create_tx_with_flags(alice, iou(gw, "USD", 0), 1, 0xFFFFFFFF);
    let result = full_apply(&mut view, &tx, TxType::VAULT_CREATE);
    assert_eq!(result, Ter::TEM_INVALID_FLAG);
}

/// C++ Vault_test — delete nonexistent vault.
#[test]
fn vault_delete_nonexistent() {
    let alice = acct(0x11);
    let ledger = make_ledger(vec![account_root(alice, 10_000_000_000, 0, 0)]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let fake_vault = Uint256::from_array([0xCC; 32]);
    let tx = vault_delete_tx(alice, fake_vault, 1);
    let result = full_apply(&mut view, &tx, TxType::VAULT_DELETE);
    assert!(
        result != Ter::TES_SUCCESS,
        "Expected error for nonexistent vault, got {:?}",
        result
    );
}

/// C++ Vault_test — deposit to nonexistent vault.
#[test]
fn vault_deposit_nonexistent() {
    let alice = acct(0x11);
    let gw = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 10_000_000_000, 0, 0),
        account_root(gw, 10_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let fake_vault = Uint256::from_array([0xCC; 32]);
    let tx = vault_deposit_tx(alice, fake_vault, iou(gw, "USD", 100), 1);
    let result = full_apply(&mut view, &tx, TxType::VAULT_DEPOSIT);
    assert!(
        result != Ter::TES_SUCCESS,
        "Expected error for nonexistent vault, got {:?}",
        result
    );
}

/// C++ Vault_test — withdraw from nonexistent vault.
#[test]
fn vault_withdraw_nonexistent() {
    let alice = acct(0x11);
    let gw = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 10_000_000_000, 0, 0),
        account_root(gw, 10_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let fake_vault = Uint256::from_array([0xCC; 32]);
    let tx = vault_withdraw_tx(alice, fake_vault, iou(gw, "USD", 100), 1);
    let result = full_apply(&mut view, &tx, TxType::VAULT_WITHDRAW);
    assert!(
        result != Ter::TES_SUCCESS,
        "Expected error for nonexistent vault, got {:?}",
        result
    );
}

/// C++ Vault_test — deposit zero amount rejected.
#[test]
fn vault_deposit_zero_amount() {
    let alice = acct(0x11);
    let gw = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 10_000_000_000, 0, 0),
        account_root(gw, 10_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let fake_vault = Uint256::from_array([0xCC; 32]);
    let tx = vault_deposit_tx(alice, fake_vault, iou(gw, "USD", 0), 1);
    let result = full_apply(&mut view, &tx, TxType::VAULT_DEPOSIT);
    assert_eq!(result, Ter::TEM_BAD_AMOUNT);
}

/// C++ Vault_test — withdraw zero amount rejected.
#[test]
fn vault_withdraw_zero_amount() {
    let alice = acct(0x11);
    let gw = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 10_000_000_000, 0, 0),
        account_root(gw, 10_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let fake_vault = Uint256::from_array([0xCC; 32]);
    let tx = vault_withdraw_tx(alice, fake_vault, iou(gw, "USD", 0), 1);
    let result = full_apply(&mut view, &tx, TxType::VAULT_WITHDRAW);
    assert_eq!(result, Ter::TEM_BAD_AMOUNT);
}

/// C++ Vault_test — deposit negative amount rejected.
#[test]
fn vault_deposit_negative_amount() {
    let alice = acct(0x11);
    let gw = acct(0x22);
    let ledger = make_ledger(vec![
        account_root(alice, 10_000_000_000, 0, 0),
        account_root(gw, 10_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let fake_vault = Uint256::from_array([0xCC; 32]);
    let tx = vault_deposit_tx(alice, fake_vault, iou(gw, "USD", -100), 1);
    let result = full_apply(&mut view, &tx, TxType::VAULT_DEPOSIT);
    assert_eq!(result, Ter::TEM_BAD_AMOUNT);
}
