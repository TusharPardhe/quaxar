use std::sync::Arc;

use basics::base_uint::{Uint160, Uint192, Uint256};
use ledger::{
    ApplyViewImpl, Ledger, LedgerHeader,
    mptoken_helpers::{
        can_trade, can_transfer_mpt, check_mpt_tx_allowed, is_any_frozen_mpt, is_frozen_mpt,
        remove_empty_holding_mpt, require_auth_mpt,
    },
    ripple_calc::book_step::{Book, execute_book_step},
};
use protocol::{
    AccountID, ApplyFlags, Asset, LedgerEntryType, MPTAmount, MPTIssue, Rules, STAmount,
    STLedgerEntry, XRPAmount, account_keylet, feature_id, get_field_by_symbol,
    mpt_issuance_keylet_from_mptid, mptoken_keylet_from_mptid,
};
use shamap::item::SHAMapItem;
use shamap::mutation::MutableTree;
use shamap::sync::{SHAMapType, SyncState, SyncTree};
use shamap::tree_node::SHAMapNodeType;

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

fn account(byte: u8) -> AccountID {
    AccountID::from_array([byte; 20])
}

fn account_raw(account: AccountID) -> Uint160 {
    Uint160::from_slice(account.data()).expect("account width")
}

fn mpt_id(issuer: AccountID, sequence: u32) -> Uint192 {
    let mut bytes = [0_u8; 24];
    bytes[..4].copy_from_slice(&sequence.to_be_bytes());
    bytes[4..].copy_from_slice(issuer.data());
    Uint192::from_slice(&bytes).expect("mpt id width")
}

fn account_entry(account: AccountID) -> STLedgerEntry {
    let mut sle = STLedgerEntry::from_type_and_key(
        LedgerEntryType::AccountRoot,
        account_keylet(account_raw(account)).key,
    );
    sle.set_account_id(sf("sfAccount"), account);
    sle.set_field_amount(
        sf("sfBalance"),
        STAmount::from_xrp_amount(XRPAmount::from_drops(100_000_000)),
    );
    sle.set_field_u32(sf("sfSequence"), 1);
    sle.set_field_u32(sf("sfOwnerCount"), 1);
    sle
}

fn pseudo_account_entry(
    account: AccountID,
    pseudo_field: &'static protocol::SField,
) -> STLedgerEntry {
    let mut sle = account_entry(account);
    sle.set_field_h256(pseudo_field, Uint256::from_array([0xAA; 32]));
    sle
}

fn issuance_entry(
    issuer: AccountID,
    sequence: u32,
    outstanding: u64,
    locked: u64,
) -> STLedgerEntry {
    let id = mpt_id(issuer, sequence);
    let mut sle = STLedgerEntry::from_type_and_key(
        LedgerEntryType::MPTokenIssuance,
        mpt_issuance_keylet_from_mptid(id).key,
    );
    sle.set_account_id(sf("sfIssuer"), issuer);
    sle.set_field_u32(sf("sfSequence"), sequence);
    sle.set_field_u64(sf("sfOutstandingAmount"), outstanding);
    sle.set_field_u32(sf("sfFlags"), protocol::lsfMPTCanTransfer);
    sle.set_field_u64(sf("sfOwnerNode"), 0);
    if locked != 0 {
        sle.set_field_u64(sf("sfLockedAmount"), locked);
    }
    sle
}

fn issuance_entry_with_flags(issuer: AccountID, sequence: u32, flags: u32) -> STLedgerEntry {
    let mut sle = issuance_entry(issuer, sequence, 0, 0);
    sle.set_field_u32(sf("sfFlags"), flags);
    sle
}

fn share_issuance_with_reference(
    share_issuer: AccountID,
    sequence: u32,
    flags: u32,
    reference_holding: Uint256,
) -> STLedgerEntry {
    let mut sle = issuance_entry_with_flags(share_issuer, sequence, flags);
    sle.set_field_h256(sf("sfReferenceHolding"), reference_holding);
    sle
}

fn require_auth_issuance_entry(issuer: AccountID, sequence: u32) -> STLedgerEntry {
    let mut sle = issuance_entry(issuer, sequence, 0, 0);
    sle.set_field_u32(
        sf("sfFlags"),
        protocol::lsfMPTCanTransfer | protocol::lsfMPTRequireAuth,
    );
    sle
}

fn mptoken_entry(
    holder: AccountID,
    issuer: AccountID,
    sequence: u32,
    amount: u64,
    locked: u64,
) -> STLedgerEntry {
    let id = mpt_id(issuer, sequence);
    let mut sle = STLedgerEntry::from_type_and_key(
        LedgerEntryType::MPToken,
        mptoken_keylet_from_mptid(id, account_raw(holder)).key,
    );
    sle.set_account_id(sf("sfAccount"), holder);
    sle.set_field_h192(sf("sfMPTokenIssuanceID"), id);
    sle.set_field_u64(sf("sfMPTAmount"), amount);
    sle.set_field_u32(sf("sfFlags"), 0);
    sle.set_field_u64(sf("sfOwnerNode"), 0);
    if locked != 0 {
        sle.set_field_u64(sf("sfLockedAmount"), locked);
    }
    sle
}

fn ledger_with(entries: impl IntoIterator<Item = STLedgerEntry>, features: &[Uint256]) -> Ledger {
    let mut tree = MutableTree::new(1);
    for entry in entries {
        tree.add_item(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(*entry.key(), entry.get_serializer().data().to_vec()),
        )
        .expect("state insertion should succeed");
    }

    let mut ledger = Ledger::from_maps(
        LedgerHeader {
            seq: 1,
            parent_close_time: 500,
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
    ledger.set_rules(Rules::new(features.iter().copied()));
    ledger
}

#[test]
fn remove_empty_holding_rejects_locked_amount_after_fix_cleanup_3_1_3() {
    let holder = account(0x31);
    let issuer = account(0x41);
    let id = mpt_id(issuer, 1);
    let ledger = ledger_with(
        [
            account_entry(holder),
            account_entry(issuer),
            issuance_entry(issuer, 1, 100, 7),
            mptoken_entry(holder, issuer, 1, 0, 7),
        ],
        &[feature_id("fixCleanup3_1_3")],
    );
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let result = remove_empty_holding_mpt(&mut view, &holder, &MPTIssue::new(id))
        .expect("remove empty holding should not throw");

    assert_eq!(result, protocol::Ter::TEC_HAS_OBLIGATIONS);
}

#[test]
fn require_auth_allows_vault_and_loanbroker_pseudo_accounts_under_sav() {
    let issuer = account(0x51);
    let vault_pseudo = account(0x52);
    let broker_pseudo = account(0x53);
    let id = mpt_id(issuer, 3);
    let ledger = ledger_with(
        [
            account_entry(issuer),
            pseudo_account_entry(vault_pseudo, sf("sfVaultID")),
            pseudo_account_entry(broker_pseudo, sf("sfLoanBrokerID")),
            require_auth_issuance_entry(issuer, 3),
        ],
        &[feature_id("SingleAssetVault")],
    );

    assert_eq!(
        require_auth_mpt(&ledger, &MPTIssue::new(id), &vault_pseudo)
            .expect("require auth should not throw"),
        protocol::Ter::TES_SUCCESS
    );
    assert_eq!(
        require_auth_mpt(&ledger, &MPTIssue::new(id), &broker_pseudo)
            .expect("require auth should not throw"),
        protocol::Ter::TES_SUCCESS
    );
}

#[test]
fn book_step_rejects_mpt_book_without_can_trade_instead_of_panicking() {
    let issuer = account(0x59);
    let holder = account(0x5A);
    let id = mpt_id(issuer, 9);
    let issue = MPTIssue::new(id);
    let ledger = ledger_with(
        [
            account_entry(issuer),
            account_entry(holder),
            issuance_entry_with_flags(issuer, 9, protocol::lsfMPTCanTransfer),
            mptoken_entry(holder, issuer, 9, 100, 0),
        ],
        &[feature_id("MPTokensV2")],
    );
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);
    let amount = STAmount::from_mpt_amount(sf("sfAmount"), MPTAmount::from_value(1), issue);
    let book = Book {
        r#in: Asset::MPTIssue(issue),
        out: Asset::Issue(protocol::xrp_issue()),
        domain: None,
    };

    let result = execute_book_step(
        &mut view,
        &book,
        &amount,
        &amount,
        true,
        Some(&holder),
        None,
    );

    assert_eq!(result.ter, protocol::Ter::TEC_NO_PERMISSION);
}

#[test]
fn require_auth_allows_amm_pseudo_account_under_mptokens_v2() {
    let issuer = account(0x61);
    let amm_pseudo = account(0x62);
    let regular = account(0x63);
    let id = mpt_id(issuer, 4);
    let ledger = ledger_with(
        [
            account_entry(issuer),
            pseudo_account_entry(amm_pseudo, sf("sfAMMID")),
            account_entry(regular),
            require_auth_issuance_entry(issuer, 4),
        ],
        &[feature_id("MPTokensV2")],
    );

    assert_eq!(
        require_auth_mpt(&ledger, &MPTIssue::new(id), &amm_pseudo)
            .expect("require auth should not throw"),
        protocol::Ter::TES_SUCCESS
    );
    assert_eq!(
        require_auth_mpt(&ledger, &MPTIssue::new(id), &regular)
            .expect("require auth should not throw"),
        protocol::Ter::TEC_NO_AUTH
    );
}

#[test]
fn can_trade_inherits_reference_holding_mpt_tradability_after_cleanup_3_2_0() {
    let underlying_issuer = account(0x71);
    let vault_pseudo = account(0x72);
    let underlying_id = mpt_id(underlying_issuer, 1);
    let share_id = mpt_id(vault_pseudo, 1);
    let reference_holding = mptoken_keylet_from_mptid(underlying_id, account_raw(vault_pseudo)).key;
    let ledger = ledger_with(
        [
            account_entry(underlying_issuer),
            account_entry(vault_pseudo),
            issuance_entry_with_flags(underlying_issuer, 1, protocol::lsfMPTCanTransfer),
            mptoken_entry(vault_pseudo, underlying_issuer, 1, 1, 0),
            share_issuance_with_reference(
                vault_pseudo,
                1,
                protocol::lsfMPTCanTrade | protocol::lsfMPTCanTransfer,
                reference_holding,
            ),
        ],
        &[feature_id("fixCleanup3_2_0")],
    );

    assert_eq!(
        can_trade(&ledger, &protocol::Asset::from(MPTIssue::new(share_id)))
            .expect("can trade should not throw"),
        protocol::Ter::TEC_NO_PERMISSION
    );
}

#[test]
fn can_transfer_inherits_reference_holding_mpt_transferability_after_cleanup_3_2_0() {
    let underlying_issuer = account(0x81);
    let vault_pseudo = account(0x82);
    let from = account(0x83);
    let to = account(0x84);
    let underlying_id = mpt_id(underlying_issuer, 1);
    let share_id = mpt_id(vault_pseudo, 1);
    let reference_holding = mptoken_keylet_from_mptid(underlying_id, account_raw(vault_pseudo)).key;
    let ledger = ledger_with(
        [
            account_entry(underlying_issuer),
            account_entry(vault_pseudo),
            account_entry(from),
            account_entry(to),
            issuance_entry_with_flags(underlying_issuer, 1, protocol::lsfMPTCanTrade),
            mptoken_entry(vault_pseudo, underlying_issuer, 1, 1, 0),
            share_issuance_with_reference(
                vault_pseudo,
                1,
                protocol::lsfMPTCanTrade | protocol::lsfMPTCanTransfer,
                reference_holding,
            ),
        ],
        &[feature_id("fixCleanup3_2_0")],
    );

    assert_eq!(
        can_transfer_mpt(&ledger, &MPTIssue::new(share_id), &from, &to)
            .expect("can transfer should not throw"),
        protocol::Ter::TEC_NO_AUTH
    );
}

#[test]
fn is_frozen_inherits_reference_holding_mpt_lock_after_cleanup_3_2_0() {
    let underlying_issuer = account(0x91);
    let vault_pseudo = account(0x92);
    let holder = account(0x93);
    let underlying_id = mpt_id(underlying_issuer, 1);
    let share_id = mpt_id(vault_pseudo, 1);
    let reference_holding = mptoken_keylet_from_mptid(underlying_id, account_raw(vault_pseudo)).key;
    let ledger = ledger_with(
        [
            account_entry(underlying_issuer),
            account_entry(vault_pseudo),
            account_entry(holder),
            issuance_entry_with_flags(
                underlying_issuer,
                1,
                protocol::lsfMPTCanTrade | protocol::lsfMPTCanTransfer | protocol::lsfMPTLocked,
            ),
            mptoken_entry(vault_pseudo, underlying_issuer, 1, 1, 0),
            share_issuance_with_reference(
                vault_pseudo,
                1,
                protocol::lsfMPTCanTrade | protocol::lsfMPTCanTransfer,
                reference_holding,
            ),
        ],
        &[
            feature_id("SingleAssetVault"),
            feature_id("fixCleanup3_2_0"),
        ],
    );

    assert!(
        is_frozen_mpt(&ledger, &holder, &MPTIssue::new(share_id))
            .expect("freeze check should not throw")
    );
}

#[test]
fn check_mpt_tx_allowed_inherits_reference_holding_restrictions_after_cleanup_3_2_0() {
    let underlying_issuer = account(0xA1);
    let vault_pseudo = account(0xA2);
    let holder = account(0xA3);
    let underlying_id = mpt_id(underlying_issuer, 1);
    let share_id = mpt_id(vault_pseudo, 1);
    let reference_holding = mptoken_keylet_from_mptid(underlying_id, account_raw(vault_pseudo)).key;
    let ledger = ledger_with(
        [
            account_entry(underlying_issuer),
            account_entry(vault_pseudo),
            account_entry(holder),
            issuance_entry_with_flags(underlying_issuer, 1, protocol::lsfMPTCanTransfer),
            mptoken_entry(vault_pseudo, underlying_issuer, 1, 1, 0),
            mptoken_entry(holder, vault_pseudo, 1, 1, 0),
            share_issuance_with_reference(
                vault_pseudo,
                1,
                protocol::lsfMPTCanTrade | protocol::lsfMPTCanTransfer,
                reference_holding,
            ),
        ],
        &[feature_id("fixCleanup3_2_0")],
    );

    assert_eq!(
        check_mpt_tx_allowed(
            &ledger,
            protocol::TxType::OFFER_CREATE,
            &protocol::Asset::from(MPTIssue::new(share_id)),
            &holder,
        )
        .expect("tx allowed check should not throw"),
        protocol::Ter::TEC_NO_PERMISSION
    );
}

#[test]
fn is_any_frozen_inherits_reference_holding_mpt_lock_after_cleanup_3_2_0() {
    let underlying_issuer = account(0xB1);
    let vault_pseudo = account(0xB2);
    let holder = account(0xB3);
    let underlying_id = mpt_id(underlying_issuer, 1);
    let share_id = mpt_id(vault_pseudo, 1);
    let reference_holding = mptoken_keylet_from_mptid(underlying_id, account_raw(vault_pseudo)).key;
    let ledger = ledger_with(
        [
            account_entry(underlying_issuer),
            account_entry(vault_pseudo),
            account_entry(holder),
            issuance_entry_with_flags(
                underlying_issuer,
                1,
                protocol::lsfMPTCanTrade | protocol::lsfMPTCanTransfer | protocol::lsfMPTLocked,
            ),
            mptoken_entry(vault_pseudo, underlying_issuer, 1, 1, 0),
            share_issuance_with_reference(
                vault_pseudo,
                1,
                protocol::lsfMPTCanTrade | protocol::lsfMPTCanTransfer,
                reference_holding,
            ),
        ],
        &[
            feature_id("SingleAssetVault"),
            feature_id("fixCleanup3_2_0"),
        ],
    );

    assert!(
        is_any_frozen_mpt(&ledger, &[holder], &MPTIssue::new(share_id))
            .expect("aggregate freeze check should not throw")
    );
}
