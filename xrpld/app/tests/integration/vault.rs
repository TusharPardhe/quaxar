#![allow(
    unused_imports,
    unused_variables,
    unused_mut,
    dead_code,
    unused_comparisons
)]
//! Integration tests ported from C++ Vault_test.cpp.

use std::sync::Arc;

use basics::{
    base_uint::{Uint160, Uint192, Uint256},
    number::NumberParts as RuntimeNumber,
};
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
        // Pulsar force-enables all amendments. Cover the final-withdrawal
        // cleanup path that is otherwise absent from this lifecycle fixture.
        protocol::feature_id("fixCleanup3_1_3"),
        protocol::feature_id("fixCleanup3_2_0"),
        protocol::feature_id("fixCleanup3_3_0"),
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
        tx.set_field_issue(
            sf("sfAsset"),
            protocol::STIssue::new_with_asset(sf("sfAsset"), asset.asset()),
        );
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), seq);
    })
}

fn vault_create_tx_with_flags(from: AccountID, asset: STAmount, seq: u32, flags: u32) -> STTx {
    STTx::new(TxType::VAULT_CREATE, move |tx| {
        tx.set_account_id(sf("sfAccount"), from);
        tx.set_field_issue(
            sf("sfAsset"),
            protocol::STIssue::new_with_asset(sf("sfAsset"), asset.asset()),
        );
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
fn vault_create_native_issue_asset_matches_rpc_setup() {
    let alice = acct(0x31);
    let ledger = make_ledger(vec![account_root(alice, 10_000_000_000, 0, 0)]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let tx = STTx::new(TxType::VAULT_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), alice);
        tx.set_field_issue(
            sf("sfAsset"),
            protocol::STIssue::new_with_asset(
                sf("sfAsset"),
                protocol::Asset::Issue(protocol::xrp_issue()),
            ),
        );
        tx.set_field_vl(sf("sfData"), b"test");
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });

    assert_eq!(
        full_apply(&mut view, &tx, TxType::VAULT_CREATE),
        Ter::TES_SUCCESS,
    );
}

#[test]
fn vault_create_deposit_then_full_withdraw_xrp_matches_txcompat_delete_setup() {
    // Mirrors Pulsar's vault_delete_basic setup: create an XRP vault, deposit
    // 10,000,000 drops, then redeem that exact amount before VaultDelete.
    let alice = acct(0x32);
    let ledger = make_ledger(vec![account_root(alice, 10_000_000_000, 0, 0)]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);
    let create = STTx::new(TxType::VAULT_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), alice);
        tx.set_field_issue(
            sf("sfAsset"),
            protocol::STIssue::new_with_asset(
                sf("sfAsset"),
                protocol::Asset::Issue(protocol::xrp_issue()),
            ),
        );
        tx.set_field_vl(sf("sfData"), b"test");
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut view, &create, TxType::VAULT_CREATE),
        Ter::TES_SUCCESS
    );

    let vault_id = protocol::vault_keylet(acct_id(alice), 1).key;
    let deposit = vault_deposit_tx(alice, vault_id, xrp(10_000_000), 2);
    assert_eq!(
        full_apply(&mut view, &deposit, TxType::VAULT_DEPOSIT),
        Ter::TES_SUCCESS
    );

    let withdraw = vault_withdraw_tx(alice, vault_id, xrp(10_000_000), 3);
    assert_eq!(
        full_apply(&mut view, &withdraw, TxType::VAULT_WITHDRAW),
        Ter::TES_SUCCESS,
        "a sole shareholder must be able to redeem the exact XRP deposit"
    );
}

#[test]
fn vault_create_xrp_asset_rejected() {
    let alice = acct(0x11);
    let ledger = make_ledger(vec![account_root(alice, 10_000_000_000, 0, 0)]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);
    let tx = STTx::new(TxType::VAULT_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), alice);
        // sfAsset must be serialized as an Issue. An Amount-form XRP asset is
        // malformed even though native Issue-form XRP is valid.
        tx.set_field_amount(sf("sfAsset"), xrp(0));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });

    assert_eq!(
        full_apply(&mut view, &tx, TxType::VAULT_CREATE),
        Ter::TEM_MALFORMED
    );
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

fn vault_withdraw_to_tx(
    from: AccountID,
    vault_id: Uint256,
    amount: STAmount,
    destination: AccountID,
    credential_ids: Option<Vec<Uint256>>,
) -> STTx {
    STTx::new(TxType::VAULT_WITHDRAW, move |tx| {
        tx.set_account_id(sf("sfAccount"), from);
        tx.set_field_h256(sf("sfVaultID"), vault_id);
        tx.set_field_amount(sf("sfAmount"), amount);
        tx.set_account_id(sf("sfDestination"), destination);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 3);
        if let Some(ids) = credential_ids.as_ref() {
            tx.set_field_v256(
                sf("sfCredentialIDs"),
                protocol::STVector256::from_values(sf("sfCredentialIDs"), ids.clone()),
            );
        }
    })
}

fn vault_withdraw_deposit_auth_fixture() -> (
    ApplyViewImpl<Ledger>,
    AccountID,
    AccountID,
    AccountID,
    AccountID,
    Uint256,
) {
    let shareholder = acct(0x81);
    let destination = acct(0x82);
    let credential_issuer = acct(0x83);
    let ledger = make_ledger(vec![
        account_root(shareholder, 10_000_000_000, 0, 0),
        account_root(destination, 10_000_000_000, 0, protocol::lsfDepositAuth),
        account_root(credential_issuer, 10_000_000_000, 0, 0),
    ]);
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);
    let create = vault_create_tx(shareholder, STAmount::from_xrp_amount(XRPAmount::new()), 1);
    assert_eq!(
        full_apply(&mut view, &create, TxType::VAULT_CREATE),
        Ter::TES_SUCCESS
    );
    let vault_id = protocol::vault_keylet(acct_id(shareholder), 1).key;
    assert_eq!(
        full_apply(
            &mut view,
            &vault_deposit_tx(shareholder, vault_id, xrp(10_000_000), 2),
            TxType::VAULT_DEPOSIT,
        ),
        Ter::TES_SUCCESS
    );
    let pseudo = view
        .read(protocol::vault_keylet_from_key(vault_id))
        .expect("vault read")
        .expect("vault exists")
        .get_account_id(sf("sfAccount"));
    (
        view,
        shareholder,
        destination,
        credential_issuer,
        pseudo,
        vault_id,
    )
}

fn credential_preauth_entry(
    destination: AccountID,
    issuer: AccountID,
    credential_type: &[u8],
) -> STLedgerEntry {
    let credential_hash = protocol::sha512_half_slices(&[issuer.data(), credential_type]);
    let mut entry = STLedgerEntry::new(protocol::deposit_preauth_credentials_keylet(
        acct_id(destination),
        &[credential_hash],
    ));
    entry.set_account_id(sf("sfAccount"), destination);
    let mut authorized = protocol::STObject::make_inner_object(sf("sfCredential"));
    authorized.set_account_id(sf("sfIssuer"), issuer);
    authorized.set_field_vl(sf("sfCredentialType"), credential_type);
    let mut credentials = protocol::STArray::new(sf("sfAuthorizeCredentials"));
    credentials.push_back(authorized);
    entry.set_field_array(sf("sfAuthorizeCredentials"), credentials);
    entry.set_field_u64(sf("sfOwnerNode"), 0);
    entry
}

fn accepted_credential_entry(
    shareholder: AccountID,
    issuer: AccountID,
    credential_type: &[u8],
) -> STLedgerEntry {
    let keylet =
        protocol::credential_keylet(acct_id(shareholder), acct_id(issuer), credential_type);
    let mut entry = STLedgerEntry::from_type_and_key(LedgerEntryType::Credential, keylet.key);
    entry.set_account_id(sf("sfSubject"), shareholder);
    entry.set_account_id(sf("sfIssuer"), issuer);
    entry.set_field_vl(sf("sfCredentialType"), credential_type);
    entry.set_field_u64(sf("sfIssuerNode"), 0);
    entry.set_field_u64(sf("sfSubjectNode"), 0);
    entry.set_field_u32(sf("sfFlags"), protocol::lsfAccepted);
    entry
}

fn account_balance(view: &impl ReadView, account: AccountID) -> i64 {
    view.read(account_keylet(acct_id(account)))
        .expect("account read")
        .expect("account exists")
        .get_field_amount(sf("sfBalance"))
        .xrp()
        .drops()
}

#[test]
fn vault_withdraw_deposit_auth_credential_preauth_succeeds_in_live_apply() {
    let (mut view, shareholder, destination, issuer, _pseudo, vault_id) =
        vault_withdraw_deposit_auth_fixture();
    let credential_type = b"vault-withdraw";
    let credential = accepted_credential_entry(shareholder, issuer, credential_type);
    let credential_id = *credential.key();
    view.insert(Arc::new(credential))
        .expect("credential insert");
    view.insert(Arc::new(credential_preauth_entry(
        destination,
        issuer,
        credential_type,
    )))
    .expect("credential preauth insert");

    assert_eq!(
        app::state::vault::apply_vault_withdraw(
            &mut view,
            &vault_withdraw_to_tx(
                shareholder,
                vault_id,
                xrp(1_000_000),
                destination,
                Some(vec![credential_id]),
            ),
        ),
        Ter::TES_SUCCESS
    );
}

#[test]
fn vault_withdraw_deposit_auth_vault_pseudo_preauth_succeeds_in_live_apply() {
    let (mut view, shareholder, destination, _issuer, pseudo, vault_id) =
        vault_withdraw_deposit_auth_fixture();
    let mut preauth = STLedgerEntry::new(protocol::deposit_preauth_keylet(
        acct_id(destination),
        acct_id(pseudo),
    ));
    preauth.set_account_id(sf("sfAccount"), destination);
    preauth.set_account_id(sf("sfAuthorize"), pseudo);
    preauth.set_field_u64(sf("sfOwnerNode"), 0);
    view.insert(Arc::new(preauth))
        .expect("vault pseudo preauth insert");

    assert_eq!(
        app::state::vault::apply_vault_withdraw(
            &mut view,
            &vault_withdraw_to_tx(shareholder, vault_id, xrp(1_000_000), destination, None),
        ),
        Ter::TES_SUCCESS
    );
}

#[test]
fn vault_withdraw_deposit_auth_shareholder_only_preauth_rejects_in_live_apply() {
    let (mut view, shareholder, destination, _issuer, _pseudo, vault_id) =
        vault_withdraw_deposit_auth_fixture();
    let mut preauth = STLedgerEntry::new(protocol::deposit_preauth_keylet(
        acct_id(destination),
        acct_id(shareholder),
    ));
    preauth.set_account_id(sf("sfAccount"), destination);
    preauth.set_account_id(sf("sfAuthorize"), shareholder);
    preauth.set_field_u64(sf("sfOwnerNode"), 0);
    view.insert(Arc::new(preauth))
        .expect("shareholder preauth insert");

    assert_eq!(
        app::state::vault::apply_vault_withdraw(
            &mut view,
            &vault_withdraw_to_tx(shareholder, vault_id, xrp(1_000_000), destination, None),
        ),
        Ter::TEC_NO_PERMISSION
    );
}

#[test]
fn vault_withdraw_deposit_auth_invalid_credential_rejects_without_mutation_in_live_apply() {
    let (mut view, shareholder, destination, _issuer, _pseudo, vault_id) =
        vault_withdraw_deposit_auth_fixture();
    let vault_keylet = protocol::vault_keylet_from_key(vault_id);
    let before_vault = view
        .read(vault_keylet)
        .expect("vault read")
        .expect("vault exists")
        .get_serializer()
        .data()
        .to_vec();
    let before_destination_balance = account_balance(&view, destination);
    let share_id = view
        .read(vault_keylet)
        .expect("vault read")
        .expect("vault exists")
        .get_field_h192(sf("sfShareMPTID"));
    let share_keylet = protocol::mptoken_keylet_from_mptid(share_id, acct_id(shareholder));
    let before_shares = view
        .read(share_keylet)
        .expect("share holding read")
        .expect("share holding exists")
        .get_serializer()
        .data()
        .to_vec();
    let invalid_credential_id = Uint256::from_array([0xD4; 32]);

    assert_eq!(
        app::state::vault::apply_vault_withdraw(
            &mut view,
            &vault_withdraw_to_tx(
                shareholder,
                vault_id,
                xrp(1_000_000),
                destination,
                Some(vec![invalid_credential_id]),
            ),
        ),
        Ter::TEF_INTERNAL
    );
    assert_eq!(
        view.read(vault_keylet)
            .expect("vault read")
            .expect("vault exists")
            .get_serializer()
            .data(),
        before_vault.as_slice(),
        "failed credential authorization must not mutate the vault"
    );
    assert_eq!(
        account_balance(&view, destination),
        before_destination_balance,
        "failed credential authorization must not pay the destination"
    );
    assert_eq!(
        view.read(share_keylet)
            .expect("share holding read")
            .expect("share holding exists")
            .get_serializer()
            .data(),
        before_shares.as_slice(),
        "failed credential authorization must not burn shareholder shares"
    );
}

// #53: fully-impaired self-withdrawals historically create an empty recipient
// holding.  fixCleanup3_4_0 suppresses only that zero-payout write.  Keep this
// at the live Vault boundary because the observable behavior is the resulting
// ledger entry, not just the conversion helper output.
fn static_share_id(account: AccountID, sequence: u32) -> Uint192 {
    let mut bytes = [0u8; 24];
    bytes[..4].copy_from_slice(&sequence.to_be_bytes());
    bytes[4..].copy_from_slice(account.data());
    Uint192::from_slice(&bytes).expect("share id width")
}

fn static_mpt_issuance(issuer: AccountID, sequence: u32, outstanding: u64) -> STLedgerEntry {
    let mut entry = STLedgerEntry::new(protocol::mpt_issuance_keylet(sequence, acct_id(issuer)));
    entry.set_account_id(sf("sfIssuer"), issuer);
    entry.set_field_u32(sf("sfSequence"), sequence);
    entry.set_field_u64(sf("sfOutstandingAmount"), outstanding);
    entry.set_field_u32(sf("sfFlags"), 0);
    entry.set_field_u64(sf("sfOwnerNode"), 0);
    entry
}

fn static_empty_owner_dir(account: AccountID) -> STLedgerEntry {
    let keylet = protocol::owner_dir_keylet(acct_id(account));
    let mut entry = STLedgerEntry::new(keylet);
    entry.set_field_h256(sf("sfRootIndex"), keylet.key);
    entry.set_field_v256(
        sf("sfIndexes"),
        protocol::STVector256::from_values(sf("sfIndexes"), Vec::new()),
    );
    entry
}

fn static_mpt_holding(account: AccountID, mpt_id: Uint192, amount: u64) -> STLedgerEntry {
    let mut entry = STLedgerEntry::new(protocol::mptoken_keylet_from_mptid(
        mpt_id,
        acct_id(account),
    ));
    entry.set_account_id(sf("sfAccount"), account);
    entry.set_field_h192(sf("sfMPTokenIssuanceID"), mpt_id);
    entry.set_field_u64(sf("sfMPTAmount"), amount);
    entry.set_field_u32(sf("sfFlags"), 0);
    entry.set_field_u64(sf("sfOwnerNode"), 0);
    entry
}

fn static_vault(
    owner: AccountID,
    pseudo: AccountID,
    sequence: u32,
    asset: protocol::Asset,
    share_id: Uint192,
) -> STLedgerEntry {
    let mut entry = STLedgerEntry::from_type_and_key(
        LedgerEntryType::Vault,
        protocol::vault_keylet(acct_id(owner), sequence).key,
    );
    entry.set_field_u32(sf("sfFlags"), 0);
    entry.set_field_u32(sf("sfSequence"), sequence);
    entry.set_field_u64(sf("sfOwnerNode"), 0);
    entry.set_account_id(sf("sfOwner"), owner);
    entry.set_account_id(sf("sfAccount"), pseudo);
    entry.set_field_issue(
        sf("sfAsset"),
        protocol::STIssue::new_with_asset(sf("sfAsset"), asset),
    );
    let mut zero = protocol::STNumber::from(basics::number::NumberParts::zero());
    zero.associate_asset(asset);
    for field in ["sfAssetsTotal", "sfAssetsAvailable", "sfLossUnrealized"] {
        entry.set_field_number(sf(field), zero.clone());
    }
    entry.set_field_h192(sf("sfShareMPTID"), share_id);
    entry
}

fn static_iou_holding(low: AccountID, high: AccountID, currency: Currency) -> STLedgerEntry {
    let mut entry = STLedgerEntry::from_type_and_key(
        LedgerEntryType::RippleState,
        protocol::line(low, high, currency).key,
    );
    entry.set_field_amount(
        sf("sfBalance"),
        STAmount::from_iou_amount(sf_generic(), IOUAmount::new(), Issue::new(currency, low)),
    );
    entry.set_field_amount(
        sf("sfLowLimit"),
        STAmount::from_iou_amount(
            sf_generic(),
            IOUAmount::from_parts(1_000_000, 0).expect("limit"),
            Issue::new(currency, low),
        ),
    );
    entry.set_field_amount(
        sf("sfHighLimit"),
        STAmount::from_iou_amount(
            sf_generic(),
            IOUAmount::from_parts(1_000_000, 0).expect("limit"),
            Issue::new(currency, high),
        ),
    );
    entry.set_field_u32(sf("sfFlags"), 0);
    entry
}

fn zero_self_withdraw_view(
    asset: protocol::Asset,
    asset_issuance: Option<STLedgerEntry>,
    recipient_holding: Option<STLedgerEntry>,
    cleanup_3_4_enabled: bool,
) -> (ApplyViewImpl<Ledger>, AccountID, Uint256, protocol::Keylet) {
    let recipient = acct(0x91);
    let pseudo = acct(0x92);
    let issuer = acct(0x93);
    let sequence = 9;
    let share_id = static_share_id(pseudo, 1);
    let vault_id = protocol::vault_keylet(acct_id(recipient), sequence).key;
    let mut entries = vec![
        account_root(recipient, 10_000_000_000, 0, 0),
        static_empty_owner_dir(recipient),
        account_root(pseudo, 10_000_000_000, 0, 0),
        account_root(issuer, 10_000_000_000, 0, protocol::lsfDefaultRipple),
        static_vault(recipient, pseudo, sequence, asset, share_id),
        static_mpt_issuance(pseudo, 1, 10),
        static_mpt_holding(recipient, share_id, 10),
    ];
    if let Some(issuance) = asset_issuance {
        entries.push(issuance);
    }
    if let Some(holding) = recipient_holding {
        entries.push(holding);
    }
    let mut ledger = make_ledger(entries);
    let mut features = vec![
        protocol::feature_id("SingleAssetVault"),
        protocol::feature_id("MPTokensV1"),
    ];
    if cleanup_3_4_enabled {
        features.push(protocol::fix_cleanup_3_4_0());
    }
    ledger.set_rules(protocol::Rules::new(features));
    let holding_keylet = match asset {
        protocol::Asset::Issue(issue) => protocol::line(recipient, issue.account, issue.currency),
        protocol::Asset::MPTIssue(issue) => {
            protocol::mptoken_keylet_from_mptid(issue.mpt_id(), acct_id(recipient))
        }
    };
    (
        ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE),
        recipient,
        vault_id,
        holding_keylet,
    )
}

fn assert_zero_self_withdraw_holding_boundary(
    asset: protocol::Asset,
    asset_issuance: Option<STLedgerEntry>,
    existing_holding: Option<STLedgerEntry>,
    cleanup_3_4_enabled: bool,
    label: &str,
) {
    let had_holding = existing_holding.is_some();
    let (mut view, recipient, vault_id, holding_keylet) =
        zero_self_withdraw_view(asset, asset_issuance, existing_holding, cleanup_3_4_enabled);
    let before = view
        .read(holding_keylet.clone())
        .expect("recipient holding read")
        .map(|holding| holding.get_serializer().data().to_vec());
    let share_id = view
        .read(protocol::vault_keylet_from_key(vault_id))
        .expect("vault read")
        .expect("vault exists")
        .get_field_h192(sf("sfShareMPTID"));
    let shares = STAmount::from_mpt_amount(
        sf("sfAmount"),
        protocol::MPTAmount::from_value(10),
        protocol::MPTIssue::new(share_id),
    );

    assert_eq!(
        app::state::vault::apply_vault_withdraw(
            &mut view,
            &vault_withdraw_tx(recipient, vault_id, shares, 1),
        ),
        Ter::TES_SUCCESS,
        "{label}: fully impaired self-withdraw must still burn the shares"
    );

    let after = view.read(holding_keylet).expect("recipient holding read");
    if cleanup_3_4_enabled {
        assert_eq!(
            after.is_some(),
            had_holding,
            "{label}: cleanup must not create a missing zero-value holding"
        );
        if let (Some(before), Some(after)) = (before, after) {
            assert_eq!(
                after.get_serializer().data(),
                before.as_slice(),
                "{label}: cleanup must not mutate an existing zero-value holding"
            );
        }
    } else {
        assert!(
            after.is_some(),
            "{label}: legacy path must create a holding"
        );
        if let (Some(before), Some(after)) = (before, after) {
            assert_eq!(
                after.get_serializer().data(),
                before.as_slice(),
                "{label}: legacy path must not mutate an existing zero-value holding"
            );
        }
    }
}

#[test]
fn vault_withdraw_zero_self_iou_holding_creation_tracks_cleanup_3_4_0() {
    let issuer = acct(0x93);
    let currency = protocol::currency_from_string("USD");
    let asset = protocol::Asset::Issue(Issue::new(currency, issuer));
    for cleanup_3_4_enabled in [false, true] {
        for existing in [false, true] {
            assert_zero_self_withdraw_holding_boundary(
                asset,
                None,
                existing.then(|| static_iou_holding(acct(0x91), issuer, currency)),
                cleanup_3_4_enabled,
                if existing {
                    "IOU existing"
                } else {
                    "IOU missing"
                },
            );
        }
    }
}

#[test]
fn vault_withdraw_zero_self_mpt_holding_creation_tracks_cleanup_3_4_0() {
    let issuer = acct(0x93);
    let asset_id = static_share_id(issuer, 7);
    let asset = protocol::Asset::MPTIssue(protocol::MPTIssue::new(asset_id));
    for cleanup_3_4_enabled in [false, true] {
        for existing in [false, true] {
            assert_zero_self_withdraw_holding_boundary(
                asset,
                Some(static_mpt_issuance(issuer, 7, 0)),
                existing.then(|| static_mpt_holding(acct(0x91), asset_id, 0)),
                cleanup_3_4_enabled,
                if existing {
                    "MPT existing"
                } else {
                    "MPT missing"
                },
            );
        }
    }
}

#[test]
fn vault_withdraw_duplicate_credentials_reject_before_live_mutation() {
    let (mut view, shareholder, destination, issuer, _pseudo, vault_id) =
        vault_withdraw_deposit_auth_fixture();
    let credential_type = b"duplicate-vault-withdraw";
    let credential = accepted_credential_entry(shareholder, issuer, credential_type);
    let credential_id = *credential.key();
    view.insert(Arc::new(credential))
        .expect("credential insert");
    view.insert(Arc::new(credential_preauth_entry(
        destination,
        issuer,
        credential_type,
    )))
    .expect("credential preauth insert");
    let vault_before = view
        .read(protocol::vault_keylet_from_key(vault_id))
        .expect("vault read")
        .expect("vault exists")
        .get_serializer()
        .data()
        .to_vec();

    assert_eq!(
        app::state::vault::apply_vault_withdraw(
            &mut view,
            &vault_withdraw_to_tx(
                shareholder,
                vault_id,
                xrp(1_000_000),
                destination,
                Some(vec![credential_id, credential_id]),
            ),
        ),
        Ter::TEF_INTERNAL,
        "duplicate CredentialIDs must reject before vault state is touched"
    );
    assert_eq!(
        view.read(protocol::vault_keylet_from_key(vault_id))
            .expect("vault read")
            .expect("vault exists")
            .get_serializer()
            .data(),
        vault_before.as_slice(),
    );
}

#[test]
fn vault_withdraw_valid_but_unauthorized_credential_rolls_back_live_apply() {
    let (mut view, shareholder, destination, issuer, _pseudo, vault_id) =
        vault_withdraw_deposit_auth_fixture();
    let credential = accepted_credential_entry(shareholder, issuer, b"unauthorized-vault-withdraw");
    let credential_id = *credential.key();
    view.insert(Arc::new(credential))
        .expect("credential insert");
    let vault_before = view
        .read(protocol::vault_keylet_from_key(vault_id))
        .expect("vault read")
        .expect("vault exists")
        .get_serializer()
        .data()
        .to_vec();

    assert_eq!(
        app::state::vault::apply_vault_withdraw(
            &mut view,
            &vault_withdraw_to_tx(
                shareholder,
                vault_id,
                xrp(1_000_000),
                destination,
                Some(vec![credential_id]),
            ),
        ),
        Ter::TEC_NO_PERMISSION,
    );
    assert_eq!(
        view.read(protocol::vault_keylet_from_key(vault_id))
            .expect("vault read")
            .expect("vault exists")
            .get_serializer()
            .data(),
        vault_before.as_slice(),
        "authorization failure must precede and roll back vault mutation"
    );
}

// #54: exercise the immutable Vault preclaim and its paired live apply path.
// These fixtures intentionally build the MPT asset, vault share issuance, and
// permissioned domain directly so each row pins the real ledger boundary.
fn issue_54_number(asset: protocol::Asset, value: i64) -> protocol::STNumber {
    let mut number = protocol::STNumber::from(RuntimeNumber::from_i64(value));
    number.associate_asset(asset);
    number
}

fn issue_54_accepted_credential(
    subject: AccountID,
    issuer: AccountID,
    credential_type: &[u8],
    expiration: Option<u32>,
) -> STLedgerEntry {
    let mut credential = accepted_credential_entry(subject, issuer, credential_type);
    if let Some(expiration) = expiration {
        credential.set_field_u32(sf("sfExpiration"), expiration);
    }
    credential
}

fn issue_54_permissioned_domain(
    owner: AccountID,
    sequence: u32,
    issuer: AccountID,
    credential_type: &[u8],
) -> STLedgerEntry {
    let mut accepted = protocol::STArray::new(sf("sfAcceptedCredentials"));
    let mut credential = protocol::STObject::make_inner_object(sf("sfCredential"));
    credential.set_account_id(sf("sfIssuer"), issuer);
    credential.set_field_vl(sf("sfCredentialType"), credential_type);
    accepted.push_back(credential);

    let mut domain = STLedgerEntry::new(protocol::permissioned_domain_keylet(
        acct_id(owner),
        sequence,
    ));
    domain.set_account_id(sf("sfOwner"), owner);
    domain.set_field_u32(sf("sfSequence"), sequence);
    domain.set_field_array(sf("sfAcceptedCredentials"), accepted);
    domain.set_field_u64(sf("sfOwnerNode"), 0);
    domain
}

fn issue_54_private_mpt_vault(
    source_expiration: Option<Option<u32>>,
    destination_expiration: Option<Option<u32>>,
    cleanup: bool,
    pseudo_destination: bool,
) -> (
    ApplyViewImpl<Ledger>,
    AccountID,
    AccountID,
    AccountID,
    Uint256,
    Uint192,
) {
    let shareholder = acct(0xA1);
    let destination = acct(0xA2);
    let issuer = acct(0xA3);
    let pseudo = acct(0xA4);
    let vault_sequence = 9;
    let share_id = Uint192::from(protocol::make_mpt_id(1, pseudo));
    let asset_id = Uint192::from(protocol::make_mpt_id(2, issuer));
    let asset = protocol::Asset::MPTIssue(protocol::MPTIssue::new(asset_id));
    let credential_type = b"issue-54-vault-domain";
    let domain = issue_54_permissioned_domain(issuer, 7, issuer, credential_type);
    let domain_id = *domain.key();
    let vault_id = protocol::vault_keylet(acct_id(shareholder), vault_sequence).key;

    let mut vault = static_vault(shareholder, pseudo, vault_sequence, asset, share_id);
    vault.set_field_u32(sf("sfFlags"), tx::VAULT_PRIVATE_FLAG);
    vault.set_field_u8(
        sf("sfWithdrawalPolicy"),
        protocol::VAULT_STRATEGY_FIRST_COME_FIRST_SERVE,
    );
    vault.set_field_number(sf("sfAssetsTotal"), issue_54_number(asset, 10));
    vault.set_field_number(sf("sfAssetsAvailable"), issue_54_number(asset, 10));

    let mut share_issuance = static_mpt_issuance(pseudo, 1, 10);
    share_issuance.set_field_u32(sf("sfFlags"), protocol::lsfMPTCanTransfer);
    share_issuance.set_field_h256(sf("sfDomainID"), domain_id);
    let mut asset_issuance = static_mpt_issuance(issuer, 2, 10);
    asset_issuance.set_field_u32(sf("sfFlags"), protocol::lsfMPTCanTransfer);
    asset_issuance.set_field_h256(sf("sfDomainID"), domain_id);

    let mut destination_root = account_root(destination, 10_000_000_000, 0, 0);
    if pseudo_destination {
        destination_root.set_field_h256(sf("sfVaultID"), Uint256::from_array([0x55; 32]));
    }
    let mut entries = vec![
        account_root(shareholder, 10_000_000_000, 0, 0),
        destination_root,
        account_root(issuer, 10_000_000_000, 0, 0),
        account_root(pseudo, 10_000_000_000, 0, 0),
        static_empty_owner_dir(shareholder),
        vault,
        share_issuance,
        asset_issuance,
        static_mpt_holding(shareholder, share_id, 10),
        static_mpt_holding(pseudo, asset_id, 10),
        static_mpt_holding(destination, asset_id, 0),
        domain,
    ];
    if let Some(expiration) = source_expiration {
        entries.push(issue_54_accepted_credential(
            shareholder,
            issuer,
            credential_type,
            expiration,
        ));
    }
    if let Some(expiration) = destination_expiration {
        entries.push(issue_54_accepted_credential(
            destination,
            issuer,
            credential_type,
            expiration,
        ));
    }

    let mut ledger = make_ledger(entries);
    let mut features = vec![
        protocol::feature_id("SingleAssetVault"),
        protocol::feature_id("MPTokensV1"),
        protocol::feature_id("PermissionedDomains"),
        protocol::feature_id("fixCleanup3_1_3"),
        protocol::feature_id("fixCleanup3_2_0"),
        protocol::feature_id("fixCleanup3_3_0"),
    ];
    if cleanup {
        features.push(protocol::fix_cleanup_3_4_0());
    }
    ledger.set_rules(protocol::Rules::new(features));
    let view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);
    assert!(
        view.read(protocol::mpt_issuance_keylet_from_mptid(asset_id))
            .expect("asset issuance read")
            .is_some(),
        "issue-54 asset issuance must match its MPT ID"
    );
    assert!(
        view.read(protocol::mptoken_keylet_from_mptid(
            asset_id,
            acct_id(destination)
        ))
        .expect("destination asset holding read")
        .is_some(),
        "issue-54 destination must have a strong-auth MPT holding"
    );
    assert!(
        view.read(protocol::mpt_issuance_keylet_from_mptid(share_id))
            .expect("share issuance read")
            .is_some(),
        "issue-54 share issuance must match its MPT ID"
    );
    assert!(
        view.read(protocol::mptoken_keylet_from_mptid(
            share_id,
            acct_id(shareholder)
        ))
        .expect("share holding read")
        .is_some(),
        "issue-54 shareholder must hold vault shares"
    );
    (view, shareholder, destination, issuer, vault_id, share_id)
}

fn issue_54_vault_preclaim(view: &impl ReadView, tx: &STTx) -> Ter {
    tx::run_vault_read_view_preclaim(view, tx, TxType::VAULT_WITHDRAW)
        .expect("VaultWithdraw must have a typed preclaim")
}

fn issue_54_vault_snapshot(
    view: &impl ReadView,
    vault_id: Uint256,
    shareholder: AccountID,
    destination: AccountID,
    share_id: Uint192,
) -> (Vec<u8>, Vec<u8>, Option<Vec<u8>>) {
    let vault = view
        .read(protocol::vault_keylet_from_key(vault_id))
        .expect("vault read")
        .expect("vault exists")
        .get_serializer()
        .data()
        .to_vec();
    let shares = view
        .read(protocol::mptoken_keylet_from_mptid(
            share_id,
            acct_id(shareholder),
        ))
        .expect("share holding read")
        .expect("share holding exists")
        .get_serializer()
        .data()
        .to_vec();
    let destination_asset = view
        .read(protocol::mptoken_keylet_from_mptid(
            Uint192::from(protocol::make_mpt_id(2, acct(0xA3))),
            acct_id(destination),
        ))
        .expect("destination asset holding read")
        .map(|holding| holding.get_serializer().data().to_vec());
    (vault, shares, destination_asset)
}

#[test]
fn issue_54_vault_withdraw_expired_domain_credential_is_tec_expired_and_unchanged() {
    let (view, shareholder, destination, _issuer, vault_id, share_id) =
        issue_54_private_mpt_vault(Some(Some(999)), Some(None), true, false);
    let mut view = view;
    let before = issue_54_vault_snapshot(&view, vault_id, shareholder, destination, share_id);
    let shares = STAmount::from_mpt_amount(
        sf("sfAmount"),
        protocol::MPTAmount::from_value(10),
        protocol::MPTIssue::new(share_id),
    );
    let tx = vault_withdraw_to_tx(shareholder, vault_id, shares, destination, None);

    assert_eq!(issue_54_vault_preclaim(&view, &tx), Ter::TEC_EXPIRED);
    assert_eq!(
        issue_54_vault_snapshot(&view, vault_id, shareholder, destination, share_id),
        before,
        "expired private-domain authorization must not mutate withdrawal state"
    );
}

#[test]
fn issue_54_private_mpt_vault_withdraw_domain_matrix() {
    for (label, source, destination, withdraw_destination, expected) in [
        (
            "authorized third party",
            Some(None),
            Some(None),
            None,
            Ter::TES_SUCCESS,
        ),
        (
            "unauthorized source",
            None,
            Some(None),
            None,
            Ter::TEC_NO_AUTH,
        ),
        (
            "unauthorized third party",
            Some(None),
            None,
            None,
            Ter::TEC_NO_AUTH,
        ),
        (
            "self exception",
            None,
            None,
            Some(acct(0xA1)),
            Ter::TES_SUCCESS,
        ),
        (
            "issuer exception",
            None,
            None,
            Some(acct(0xA3)),
            Ter::TES_SUCCESS,
        ),
    ] {
        let (mut view, shareholder, destination, issuer, vault_id, share_id) =
            issue_54_private_mpt_vault(source, destination, true, false);
        let withdrawal_destination = withdraw_destination.unwrap_or(destination);
        let shares = STAmount::from_mpt_amount(
            sf("sfAmount"),
            protocol::MPTAmount::from_value(10),
            protocol::MPTIssue::new(share_id),
        );
        let tx = vault_withdraw_to_tx(shareholder, vault_id, shares, withdrawal_destination, None);
        let before = issue_54_vault_snapshot(&view, vault_id, shareholder, destination, share_id);
        if label == "authorized third party" {
            let asset_issuance = view
                .read(protocol::mpt_issuance_keylet_from_mptid(Uint192::from(
                    protocol::make_mpt_id(2, issuer),
                )))
                .expect("asset issuance read")
                .expect("asset issuance exists");
            let domain_id = asset_issuance.get_field_h256(sf("sfDomainID"));
            assert_eq!(
                ledger::credential_helpers::valid_domain(&view, domain_id, &shareholder)
                    .expect("source domain read"),
                Ter::TES_SUCCESS,
                "source credential fixture"
            );
            assert_eq!(
                ledger::credential_helpers::valid_domain(&view, domain_id, &destination)
                    .expect("destination domain read"),
                Ter::TES_SUCCESS,
                "destination credential fixture"
            );
            let vault_asset = view
                .read(protocol::vault_keylet_from_key(vault_id))
                .expect("vault read")
                .expect("vault exists")
                .get_field_issue(sf("sfAsset"))
                .asset();
            let protocol::Asset::MPTIssue(vault_issue) = vault_asset else {
                panic!("issue-54 fixture must retain an MPT vault asset");
            };
            assert_eq!(
                ledger::mptoken_helpers::require_auth_mpt_with_type(
                    &view,
                    &vault_issue,
                    &destination,
                    ledger::mptoken_helpers::MPTAuthType::Strong,
                )
                .expect("destination MPT auth read"),
                Ter::TES_SUCCESS,
                "destination MPT strong-auth fixture"
            );
        }

        assert_eq!(issue_54_vault_preclaim(&view, &tx), expected, "{label}");
        if expected == Ter::TES_SUCCESS {
            assert_eq!(
                app::state::vault::apply_vault_withdraw(&mut view, &tx),
                Ter::TES_SUCCESS,
                "{label} live apply"
            );
            if withdrawal_destination == destination {
                assert_eq!(
                    view.read(protocol::mptoken_keylet_from_mptid(
                        Uint192::from(protocol::make_mpt_id(2, issuer)),
                        acct_id(destination),
                    ))
                    .expect("destination asset read")
                    .expect("destination asset holding")
                    .get_field_u64(sf("sfMPTAmount")),
                    10,
                    "{label} pays the authorized destination"
                );
            }
        } else {
            assert_eq!(
                issue_54_vault_snapshot(&view, vault_id, shareholder, destination, share_id),
                before,
                "{label} must not mutate state"
            );
        }
    }
}

#[test]
fn issue_54_vault_withdraw_pseudo_destination_rejects_only_after_cleanup_and_rolls_back() {
    for cleanup in [false, true] {
        let (mut view, shareholder, destination, _issuer, vault_id, share_id) =
            issue_54_private_mpt_vault(Some(None), Some(None), cleanup, true);
        let shares = STAmount::from_mpt_amount(
            sf("sfAmount"),
            protocol::MPTAmount::from_value(10),
            protocol::MPTIssue::new(share_id),
        );
        let tx = vault_withdraw_to_tx(shareholder, vault_id, shares, destination, None);
        let before = issue_54_vault_snapshot(&view, vault_id, shareholder, destination, share_id);
        let expected = if cleanup {
            Ter::TEC_PSEUDO_ACCOUNT
        } else {
            Ter::TES_SUCCESS
        };

        assert_eq!(
            issue_54_vault_preclaim(&view, &tx),
            expected,
            "cleanup={cleanup}"
        );
        if cleanup {
            assert_eq!(
                issue_54_vault_snapshot(&view, vault_id, shareholder, destination, share_id),
                before,
                "cleanup pseudo rejection must roll back exactly"
            );
        } else {
            assert_eq!(
                app::state::vault::apply_vault_withdraw(&mut view, &tx),
                Ter::TES_SUCCESS,
                "legacy accepts the pseudo destination"
            );
        }
    }
}
