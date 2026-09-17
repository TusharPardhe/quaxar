use std::sync::Arc;

use basics::base_uint::{Uint160, Uint192, Uint256};
use ledger::{
    ApplyViewImpl, Ledger, LedgerHeader, ReadView,
    flow_engine::strand_builder::to_strands_checked,
    mptoken_helpers::{
        MPTAuthType, can_trade, can_transfer_asset, can_transfer_mpt, check_mpt_tx_allowed,
        is_any_frozen_mpt, is_frozen_mpt, lock_escrow_mpt, remove_empty_holding_mpt,
        require_auth_mpt, require_auth_mpt_with_type, unlock_escrow_mpt,
    },
    ripple_calc::book_step::{Book, execute_book_step},
    ripple_state_helpers::{account_send, account_send_allow_mpt_overflow},
};
use protocol::{
    AccountID, ApplyFlags, Asset, Currency, IOUAmount, Issue, LedgerEntryType, MPTAmount, MPTIssue,
    Rules, STAmount, STArray, STLedgerEntry, STObject, STPathSet, XRPAmount, account_keylet,
    credential_keylet, feature_id, get_field_by_symbol, lsfAccepted,
    mpt_issuance_keylet_from_mptid, mptoken_keylet_from_mptid,
};
use shamap::item::SHAMapItem;
use shamap::mutation::MutableTree;
use shamap::sync::{SHAMapType, SyncState, SyncTree};
use shamap::tree_node::SHAMapNodeType;

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

#[test]
fn mptokens_v2_book_send_allows_only_temporary_aggregate_overflow() {
    let issuer = account(0xC7);
    let holder = account(0xC8);
    let issue = MPTIssue::new(mpt_id(issuer, 13));
    let mut issuance = issuance_entry(issuer, 13, 10, 0);
    issuance.set_field_u64(sf("sfMaximumAmount"), 10);
    let entries = [
        account_entry(issuer),
        account_entry(holder),
        issuance,
        mptoken_entry(holder, issuer, 13, 0, 0),
    ];
    let amount = STAmount::from_mpt_amount(sf("sfAmount"), MPTAmount::from_value(1), issue);

    let mut ordinary = ApplyViewImpl::new(
        Arc::new(ledger_with(entries.clone(), &[feature_id("MPTokensV2")])),
        ApplyFlags::NONE,
    );
    assert_eq!(
        account_send(&mut ordinary, &issuer, &holder, &amount),
        protocol::Ter::TEC_PATH_DRY,
        "ordinary sends may not exceed MaximumAmount",
    );

    let mut book = ApplyViewImpl::new(
        Arc::new(ledger_with(entries.clone(), &[feature_id("MPTokensV2")])),
        ApplyFlags::NONE,
    );
    assert_eq!(
        account_send_allow_mpt_overflow(&mut book, &issuer, &holder, &amount),
        protocol::Ter::TES_SUCCESS,
    );
    assert_eq!(
        book.read(mpt_issuance_keylet_from_mptid(issue.mpt_id()))
            .expect("issuance read")
            .expect("issuance")
            .get_field_u64(sf("sfOutstandingAmount")),
        11,
    );

    let mut pre_v2 = ApplyViewImpl::new(Arc::new(ledger_with(entries, &[])), ApplyFlags::NONE);
    assert_eq!(
        account_send_allow_mpt_overflow(&mut pre_v2, &issuer, &holder, &amount),
        protocol::Ter::TEC_PATH_DRY,
        "the temporary-overflow policy is gated by MPTokensV2",
    );
}

#[test]
fn escrow_lock_and_unlock_enforce_the_signed_mpt_protocol_cap() {
    let issuer = account(0xC1);
    let sender = account(0xC2);
    let receiver = account(0xC3);
    let issue = MPTIssue::new(mpt_id(issuer, 12));
    let max = protocol::MAX_MP_TOKEN_AMOUNT as u64;
    let amount = STAmount::from_mpt_amount(sf("sfAmount"), MPTAmount::from_value(1), issue);

    let lock_ledger = ledger_with(
        [
            account_entry(issuer),
            account_entry(sender),
            issuance_entry(issuer, 12, max, max),
            mptoken_entry(sender, issuer, 12, 1, max),
        ],
        &[],
    );
    let mut lock_view = ApplyViewImpl::new(Arc::new(lock_ledger), ApplyFlags::NONE);
    assert_eq!(
        lock_escrow_mpt(&mut lock_view, &sender, &amount).expect("lock should not throw"),
        protocol::Ter::TEC_INTERNAL,
        "MAX_MPT_AMOUNT + 1 must fail even though it fits in u64",
    );

    let unlock_ledger = ledger_with(
        [
            account_entry(issuer),
            account_entry(sender),
            account_entry(receiver),
            issuance_entry(issuer, 12, max, 1),
            mptoken_entry(sender, issuer, 12, 0, 1),
            mptoken_entry(receiver, issuer, 12, max, 0),
        ],
        &[],
    );
    let mut unlock_view = ApplyViewImpl::new(Arc::new(unlock_ledger), ApplyFlags::NONE);
    assert_eq!(
        unlock_escrow_mpt(
            &mut unlock_view,
            &sender,
            &receiver,
            &amount,
            &amount,
            false,
            None,
            None,
        )
        .expect("unlock should not throw"),
        protocol::Ter::TEC_INTERNAL,
        "receiver credit above MAX_MPT_AMOUNT must fail",
    );
}

#[test]
fn reference_holding_freeze_inheritance_requires_fix_cleanup_3_2_0() {
    let underlying_issuer = account(0xC4);
    let vault_pseudo = account(0xC5);
    let holder = account(0xC6);
    let underlying_id = mpt_id(underlying_issuer, 1);
    let share_id = mpt_id(vault_pseudo, 1);
    let reference = mptoken_keylet_from_mptid(underlying_id, account_raw(vault_pseudo)).key;
    let entries = [
        account_entry(underlying_issuer),
        account_entry(vault_pseudo),
        account_entry(holder),
        issuance_entry_with_flags(
            underlying_issuer,
            1,
            protocol::lsfMPTCanTransfer | protocol::lsfMPTLocked,
        ),
        mptoken_entry(vault_pseudo, underlying_issuer, 1, 1, 0),
        share_issuance_with_reference(vault_pseudo, 1, protocol::lsfMPTCanTransfer, reference),
    ];

    let legacy = ledger_with(entries.clone(), &[feature_id("SingleAssetVault")]);
    assert!(
        !is_frozen_mpt(&legacy, &holder, &MPTIssue::new(share_id)).expect("legacy freeze check"),
        "SingleAssetVault alone must not activate sfReferenceHolding inheritance",
    );
    let fixed = ledger_with(entries, &[feature_id("fixCleanup3_2_0")]);
    assert!(is_frozen_mpt(&fixed, &holder, &MPTIssue::new(share_id)).expect("fixed freeze check"));
}

#[test]
fn recursive_reference_holding_checks_stop_at_protocol_depth_five() {
    assert_eq!(protocol::MAX_ASSET_CHECK_DEPTH, 5);
    let from = account(0xD0);
    let to = account(0xD1);
    let issuers = (0..6).map(|n| account(0xD2 + n)).collect::<Vec<_>>();
    let mut entries = vec![account_entry(from), account_entry(to)];

    for (index, issuer) in issuers.iter().copied().enumerate() {
        entries.push(account_entry(issuer));
        let reference = if index + 1 < issuers.len() {
            let underlying = mpt_id(issuers[index + 1], 1);
            let token = mptoken_entry(issuer, issuers[index + 1], 1, 1, 0);
            let key = *token.key();
            debug_assert_eq!(
                key,
                mptoken_keylet_from_mptid(underlying, account_raw(issuer)).key
            );
            entries.push(token);
            key
        } else {
            Uint256::from_u64(0xDEAD)
        };
        entries.push(share_issuance_with_reference(
            issuer,
            1,
            protocol::lsfMPTCanTransfer,
            reference,
        ));
    }

    let ledger = ledger_with(entries, &[feature_id("fixCleanup3_2_0")]);
    assert_eq!(
        can_transfer_mpt(&ledger, &MPTIssue::new(mpt_id(issuers[0], 1)), &from, &to,)
            .expect("recursive transfer check"),
        protocol::Ter::TEC_INTERNAL,
    );
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

fn vault_entry(id: Uint256, owner: AccountID, pseudo: AccountID, asset: Asset) -> STLedgerEntry {
    let mut sle = STLedgerEntry::new(protocol::vault_keylet_from_key(id));
    sle.set_account_id(sf("sfOwner"), owner);
    sle.set_account_id(sf("sfAccount"), pseudo);
    sle.set_field_issue(
        sf("sfAsset"),
        protocol::STIssue::new_with_asset(sf("sfAsset"), asset),
    );
    sle
}

fn credential_entry(
    subject: AccountID,
    issuer: AccountID,
    credential_type: &[u8],
) -> STLedgerEntry {
    let mut sle = STLedgerEntry::new(credential_keylet(
        account_raw(subject),
        account_raw(issuer),
        credential_type,
    ));
    sle.set_account_id(sf("sfSubject"), subject);
    sle.set_account_id(sf("sfIssuer"), issuer);
    sle.set_field_vl(sf("sfCredentialType"), credential_type);
    sle.set_field_u32(sf("sfFlags"), lsfAccepted);
    sle
}

fn domain_entry(
    id: Uint256,
    owner: AccountID,
    credential_issuer: AccountID,
    credential_type: &[u8],
) -> STLedgerEntry {
    let mut sle = STLedgerEntry::new(protocol::permissioned_domain_keylet_from_id(id));
    sle.set_account_id(sf("sfOwner"), owner);
    let mut credential = STObject::make_inner_object(sf("sfCredential"));
    credential.set_account_id(sf("sfIssuer"), credential_issuer);
    credential.set_field_vl(sf("sfCredentialType"), credential_type);
    let mut credentials = STArray::new(sf("sfAcceptedCredentials"));
    credentials.push_back(credential);
    sle.set_field_array(sf("sfAcceptedCredentials"), credentials);
    sle
}

fn iou_holding(low: AccountID, high: AccountID, currency: Currency) -> STLedgerEntry {
    let mut sle = STLedgerEntry::new(protocol::line(low, high, currency));
    sle.set_field_amount(
        sf("sfLowLimit"),
        STAmount::from_iou_amount(
            sf("sfLowLimit"),
            IOUAmount::new(),
            Issue::new(currency, low),
        ),
    );
    sle.set_field_amount(
        sf("sfHighLimit"),
        STAmount::from_iou_amount(
            sf("sfHighLimit"),
            IOUAmount::new(),
            Issue::new(currency, high),
        ),
    );
    sle.set_field_amount(
        sf("sfBalance"),
        STAmount::from_iou_amount(
            sf("sfBalance"),
            IOUAmount::new(),
            Issue::new(currency, protocol::no_account()),
        ),
    );
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
fn require_auth_modes_distinguish_missing_mpt_holding() {
    let issuer = account(0x21);
    let holder = account(0x22);
    let issue = MPTIssue::new(mpt_id(issuer, 1));
    let ledger = ledger_with(
        [
            account_entry(issuer),
            account_entry(holder),
            issuance_entry_with_flags(issuer, 1, protocol::lsfMPTCanTransfer),
        ],
        &[],
    );

    assert_eq!(
        require_auth_mpt_with_type(&ledger, &issue, &holder, MPTAuthType::Weak)
            .expect("weak auth read"),
        protocol::Ter::TES_SUCCESS,
    );
    for mode in [MPTAuthType::Strong, MPTAuthType::Legacy] {
        assert_eq!(
            require_auth_mpt_with_type(&ledger, &issue, &holder, mode)
                .expect("strong/legacy auth read"),
            protocol::Ter::TEC_NO_AUTH,
        );
    }
}

#[test]
fn require_auth_domain_authorizes_only_weak_missing_holding() {
    let issuer = account(0x23);
    let holder = account(0x24);
    let credential_issuer = account(0x25);
    let domain_id = Uint256::from_array([0xD1; 32]);
    let credential_type = b"member";
    let issue = MPTIssue::new(mpt_id(issuer, 2));
    let mut issuance = require_auth_issuance_entry(issuer, 2);
    issuance.set_field_h256(sf("sfDomainID"), domain_id);
    let ledger = ledger_with(
        [
            account_entry(issuer),
            account_entry(holder),
            account_entry(credential_issuer),
            issuance,
            domain_entry(domain_id, issuer, credential_issuer, credential_type),
            credential_entry(holder, credential_issuer, credential_type),
        ],
        &[feature_id("PermissionedDomains")],
    );

    assert_eq!(
        require_auth_mpt_with_type(&ledger, &issue, &holder, MPTAuthType::Weak)
            .expect("domain weak auth read"),
        protocol::Ter::TES_SUCCESS,
    );
    assert_eq!(
        require_auth_mpt_with_type(&ledger, &issue, &holder, MPTAuthType::Legacy)
            .expect("domain legacy auth read"),
        protocol::Ter::TEC_NO_AUTH,
        "Legacy requires the outer MPToken before consulting DomainID",
    );
}

#[test]
fn require_auth_uses_legacy_vault_issuer_fallback_and_preserves_nested_iou_modes() {
    let vault_owner = account(0x26);
    let iou_issuer = account(0x27);
    let vault_pseudo = account(0x28);
    let holder = account(0x29);
    let vault_id = Uint256::from_array([0xA1; 32]);
    let currency = Currency::from_array([0x55; 20]);
    let share_id = mpt_id(vault_pseudo, 3);
    let mut pseudo_root = pseudo_account_entry(vault_pseudo, sf("sfVaultID"));
    pseudo_root.set_field_h256(sf("sfVaultID"), vault_id);
    let ledger = ledger_with(
        [
            account_entry(vault_owner),
            account_entry(iou_issuer),
            pseudo_root,
            account_entry(holder),
            vault_entry(
                vault_id,
                vault_owner,
                vault_pseudo,
                Asset::Issue(Issue::new(currency, iou_issuer)),
            ),
            issuance_entry_with_flags(vault_pseudo, 3, protocol::lsfMPTCanTransfer),
            mptoken_entry(holder, vault_pseudo, 3, 1, 0),
        ],
        &[feature_id("SingleAssetVault")],
    );
    let share = MPTIssue::new(share_id);

    assert_eq!(
        require_auth_mpt_with_type(&ledger, &share, &holder, MPTAuthType::Legacy)
            .expect("legacy nested IOU auth read"),
        protocol::Ter::TES_SUCCESS,
        "Legacy is weak for the vault's IOU underlying",
    );
    assert_eq!(
        require_auth_mpt_with_type(&ledger, &share, &holder, MPTAuthType::Strong)
            .expect("strong nested IOU auth read"),
        protocol::Ter::TEC_NO_LINE,
        "Strong requires the vault underlying trust line",
    );
}

#[test]
fn require_auth_domain_falls_back_to_explicit_token_authorization() {
    let issuer = account(0x2C);
    let holder = account(0x2D);
    let missing_domain = Uint256::from_array([0xD3; 32]);
    let issue = MPTIssue::new(mpt_id(issuer, 5));
    let mut issuance = require_auth_issuance_entry(issuer, 5);
    issuance.set_field_h256(sf("sfDomainID"), missing_domain);
    let mut token = mptoken_entry(holder, issuer, 5, 1, 0);
    token.set_field_u32(sf("sfFlags"), protocol::lsfMPTAuthorized);
    let ledger = ledger_with(
        [
            account_entry(issuer),
            account_entry(holder),
            issuance,
            token,
        ],
        &[feature_id("PermissionedDomains")],
    );

    assert_eq!(
        require_auth_mpt_with_type(&ledger, &issue, &holder, MPTAuthType::Legacy)
            .expect("explicit token fallback auth read"),
        protocol::Ter::TES_SUCCESS,
        "a DomainID lookup failure must not mask explicit issuer authorization",
    );
}

#[test]
fn require_auth_recurses_into_vault_mpt_underlying() {
    let underlying_issuer = account(0x2E);
    let vault_owner = account(0x2F);
    let vault_pseudo = account(0x30);
    let holder = account(0x31);
    let vault_id = Uint256::from_array([0xA2; 32]);
    let underlying = MPTIssue::new(mpt_id(underlying_issuer, 6));
    let share = MPTIssue::new(mpt_id(vault_pseudo, 7));
    let mut pseudo_root = pseudo_account_entry(vault_pseudo, sf("sfVaultID"));
    pseudo_root.set_field_h256(sf("sfVaultID"), vault_id);
    let ledger = ledger_with(
        [
            account_entry(underlying_issuer),
            account_entry(vault_owner),
            pseudo_root,
            account_entry(holder),
            issuance_entry_with_flags(underlying_issuer, 6, protocol::lsfMPTCanTransfer),
            vault_entry(
                vault_id,
                vault_owner,
                vault_pseudo,
                Asset::MPTIssue(underlying),
            ),
            issuance_entry_with_flags(vault_pseudo, 7, protocol::lsfMPTCanTransfer),
            mptoken_entry(holder, vault_pseudo, 7, 1, 0),
        ],
        &[feature_id("SingleAssetVault")],
    );

    assert_eq!(
        require_auth_mpt_with_type(&ledger, &share, &holder, MPTAuthType::Weak)
            .expect("weak recursive MPT auth read"),
        protocol::Ter::TES_SUCCESS,
    );
    for mode in [MPTAuthType::Legacy, MPTAuthType::Strong] {
        assert_eq!(
            require_auth_mpt_with_type(&ledger, &share, &holder, mode)
                .expect("strong recursive MPT auth read"),
            protocol::Ter::TEC_NO_AUTH,
            "the outer share exists, so failure must come from the missing underlying MPToken",
        );
    }
}

#[test]
fn holder_transfer_credits_receiver_before_sender_debit() {
    let issuer = account(0x32);
    let source = account(0x33);
    let destination = account(0x34);
    let issue = MPTIssue::new(mpt_id(issuer, 8));
    let ledger = ledger_with(
        [
            account_entry(issuer),
            account_entry(source),
            account_entry(destination),
            issuance_entry(issuer, 8, 0, 0),
            mptoken_entry(source, issuer, 8, 0, 0),
            mptoken_entry(destination, issuer, 8, 0, 0),
        ],
        &[],
    );
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);
    let amount = STAmount::from_mpt_amount(sf("sfAmount"), MPTAmount::from_value(1), issue);

    // Production callers execute this primitive in a transaction sandbox, so
    // the failed transfer is rolled back. Reading the child here intentionally
    // exposes the intermediate order: rippled issues to the receiver before
    // attempting the sender's gross redemption.
    assert_eq!(
        ledger::ripple_state_helpers::account_send(&mut view, &source, &destination, &amount,),
        protocol::Ter::TEC_INSUFFICIENT_FUNDS,
    );
    assert_eq!(
        view.read(mptoken_keylet_from_mptid(
            issue.mpt_id(),
            account_raw(destination),
        ))
        .expect("destination token read")
        .expect("destination token")
        .get_field_u64(sf("sfMPTAmount")),
        1,
    );
    assert_eq!(
        view.read(mpt_issuance_keylet_from_mptid(issue.mpt_id()))
            .expect("issuance read")
            .expect("issuance")
            .get_field_u64(sf("sfOutstandingAmount")),
        1,
    );
}

#[test]
fn cleanup_3_3_moves_pseudo_exemption_before_domain_and_holding_checks() {
    let issuer = account(0x2A);
    let pseudo = account(0x2B);
    let missing_domain = Uint256::from_array([0xD2; 32]);
    let issue = MPTIssue::new(mpt_id(issuer, 4));
    let mut issuance = require_auth_issuance_entry(issuer, 4);
    issuance.set_field_h256(sf("sfDomainID"), missing_domain);
    let entries = [
        account_entry(issuer),
        pseudo_account_entry(pseudo, sf("sfAMMID")),
        issuance,
    ];

    let legacy = ledger_with(entries.clone(), &[feature_id("MPTokensV2")]);
    assert_eq!(
        require_auth_mpt_with_type(&legacy, &issue, &pseudo, MPTAuthType::Weak)
            .expect("legacy pseudo auth read"),
        protocol::Ter::TEC_OBJECT_NOT_FOUND,
        "before fixCleanup3_3_0 DomainID failure precedes pseudo exemption",
    );

    let fixed = ledger_with(
        entries,
        &[feature_id("MPTokensV2"), feature_id("fixCleanup3_3_0")],
    );
    assert_eq!(
        require_auth_mpt_with_type(&fixed, &issue, &pseudo, MPTAuthType::Weak)
            .expect("fixed pseudo auth read"),
        protocol::Ter::TES_SUCCESS,
    );
}

#[test]
fn vault_share_can_transfer_dispatches_to_iou_underlying() {
    let underlying_issuer = account(0x70);
    let vault_pseudo = account(0x80);
    let from = account(0x81);
    let to = account(0x82);
    let currency = Currency::from_array([0x66; 20]);
    let holding = iou_holding(underlying_issuer, vault_pseudo, currency);
    let share_id = mpt_id(vault_pseudo, 5);
    let ledger = ledger_with(
        [
            account_entry(underlying_issuer),
            account_entry(vault_pseudo),
            account_entry(from),
            account_entry(to),
            share_issuance_with_reference(
                vault_pseudo,
                5,
                protocol::lsfMPTCanTransfer,
                *holding.key(),
            ),
            holding,
        ],
        &[feature_id("fixCleanup3_2_0")],
    );

    assert_eq!(
        can_transfer_asset(
            &ledger,
            Asset::MPTIssue(MPTIssue::new(share_id)),
            &from,
            &to
        )
        .expect("recursive IOU canTransfer read"),
        protocol::Ter::TER_NO_RIPPLE,
        "both absent IOU lines inherit the issuer's disabled DefaultRipple",
    );
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
fn require_auth_allows_every_metadata_pseudo_account_under_sav() {
    let issuer = account(0x51);
    let id = mpt_id(issuer, 3);
    let pseudo_fields = protocol::all_sfields()
        .iter()
        .filter(|field| field.should_meta(protocol::SField::S_MD_PSEUDO_ACCOUNT))
        .copied()
        .collect::<Vec<_>>();
    assert!(
        !pseudo_fields.is_empty(),
        "the protocol metadata must define pseudo-account discriminators"
    );

    let mut entries = vec![
        account_entry(issuer),
        require_auth_issuance_entry(issuer, 3),
    ];
    let pseudos = pseudo_fields
        .iter()
        .enumerate()
        .map(|(index, field)| {
            let pseudo = account(0x52 + index as u8);
            entries.push(pseudo_account_entry(
                pseudo,
                get_field_by_symbol(field.symbol_name()),
            ));
            pseudo
        })
        .collect::<Vec<_>>();
    let ledger = ledger_with(
        entries,
        &[
            feature_id("SingleAssetVault"),
            feature_id("fixCleanup3_3_0"),
        ],
    );

    for pseudo in pseudos {
        assert_eq!(
            require_auth_mpt(&ledger, &MPTIssue::new(id), &pseudo)
                .expect("require auth should not throw"),
            protocol::Ter::TES_SUCCESS,
            "every metadata-marked discriminator must confer pseudo authorization"
        );
    }
}

#[test]
fn amm_lp_transfer_checks_each_underlying_mpt_for_redemption_and_spendability() {
    let issuer = account(0x54);
    let amm_account = account(0x55);
    let holder = account(0x56);
    let recipient = account(0x57);
    let amm_id = Uint256::from_u64(0xA55);
    let mpt = MPTIssue::new(mpt_id(issuer, 8));
    let mut amm_root = pseudo_account_entry(amm_account, sf("sfAMMID"));
    amm_root.set_field_h256(sf("sfAMMID"), amm_id);
    let mut amm = STLedgerEntry::from_type_and_key(LedgerEntryType::AMM, amm_id);
    amm.set_account_id(sf("sfAccount"), amm_account);
    amm.set_field_issue(
        sf("sfAsset"),
        protocol::STIssue::new_with_asset(sf("sfAsset"), Asset::MPTIssue(mpt)),
    );
    amm.set_field_issue(
        sf("sfAsset2"),
        protocol::STIssue::new_with_asset(sf("sfAsset2"), Asset::Issue(protocol::xrp_issue())),
    );

    let no_transfer = ledger_with(
        [
            account_entry(issuer),
            account_entry(holder),
            account_entry(recipient),
            amm_root.clone(),
            amm.clone(),
            issuance_entry_with_flags(issuer, 8, protocol::lsfMPTCanTrade),
        ],
        &[feature_id("MPTokensV2")],
    );
    assert_eq!(
        ledger::mptoken_helpers::can_transfer_lp_token(
            &no_transfer,
            &holder,
            &amm_account,
            &amm_account,
        )
        .expect("direct redemption transfer check"),
        protocol::Ter::TEC_NO_AUTH,
        "non-transferable pool MPT blocks holder-to-AMM LP redemption"
    );
    assert_eq!(
        ledger::mptoken_helpers::can_transfer_lp_token(
            &no_transfer,
            &holder,
            &recipient,
            &amm_account,
        )
        .expect("LP spendability transfer check"),
        protocol::Ter::TEC_NO_AUTH,
        "the same rule makes the LP token unspendable for a third-party transfer"
    );
    assert_eq!(
        ledger::mptoken_helpers::can_transfer_lp_token(
            &no_transfer,
            &issuer,
            &holder,
            &amm_account,
        )
        .expect("issuer transfer check"),
        protocol::Ter::TES_SUCCESS,
        "the underlying MPT issuer remains exempt"
    );

    let transferable = ledger_with(
        [
            account_entry(issuer),
            account_entry(holder),
            account_entry(recipient),
            amm_root,
            amm,
            issuance_entry_with_flags(
                issuer,
                8,
                protocol::lsfMPTCanTrade | protocol::lsfMPTCanTransfer,
            ),
        ],
        &[feature_id("MPTokensV2")],
    );
    assert_eq!(
        ledger::mptoken_helpers::can_transfer_lp_token(
            &transferable,
            &holder,
            &amm_account,
            &amm_account,
        )
        .expect("transferable direct redemption check"),
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
fn mpt_payment_strand_rejects_holder_without_mptoken() {
    let issuer = account(0x5B);
    let holder = account(0x5C);
    let issue = MPTIssue::new(mpt_id(issuer, 10));
    let ledger = ledger_with(
        [
            account_entry(issuer),
            account_entry(holder),
            issuance_entry_with_flags(issuer, 10, protocol::lsfMPTCanTransfer),
        ],
        &[feature_id("MPTokensV2")],
    );
    let mut view = ApplyViewImpl::new(Arc::new(ledger), ApplyFlags::NONE);

    let (ter, strands) = to_strands_checked(
        &mut view,
        &holder,
        &issuer,
        &Asset::MPTIssue(issue),
        None,
        &STPathSet::new(sf("sfPaths")),
        true,
        false,
        false,
    );

    assert_eq!(ter, protocol::Ter::TEC_NO_AUTH);
    assert!(strands.is_empty());
}

#[test]
fn require_auth_allows_amm_pseudo_account_after_cleanup_3_3() {
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
        &[feature_id("MPTokensV2"), feature_id("fixCleanup3_3_0")],
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
