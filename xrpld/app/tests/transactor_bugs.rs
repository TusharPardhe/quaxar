//! Regression tests for transactor bugs from ledgers 104103382-104111035.
//!
//! Bug 1: balance < fee in closed ledger → tecINSUFF_FEE (not terINSUF_FEE_B)
//! Bug 2: FillOrKill OfferCreate not fully filled → tecKILLED (state reset)
//! Bug 3: EscrowFinish on IOU escrow → tecLIMIT_EXCEEDED
//! Bug 4: tfNoRippleDirect payment with dry explicit path → tecPATH_DRY
//! Bug 5: self-payment (Account==Destination) → tecPATH_DRY
//! Bug 6: OfferCreate with zero TakerGets balance → tecUNFUNDED_OFFER
//! Bug 7: ImmediateOrCancel offer not fully filled → tecKILLED

use app::apply_submit_transactor_shell;
use basics::base_uint::{Uint160, Uint256};
use ledger::{Fees, Ledger, LedgerHeader, Sandbox};
use protocol::{
    AccountID, ApplyFlags, Currency, IOUAmount, Issue, LedgerEntryType, MPTAmount, MPTIssue,
    STAmount, STArray, STLedgerEntry, STObject, STTx, Ter, TxType, XRPAmount, account_keylet,
    get_field_by_symbol,
};
use shamap::{
    item::SHAMapItem,
    mutation::MutableTree,
    sync::{SHAMapType, SyncState, SyncTree},
    tree_node::SHAMapNodeType,
};
use std::sync::Arc;

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}
fn raw_id(a: AccountID) -> Uint160 {
    Uint160::from_slice(a.data()).unwrap()
}
fn acct(b: u8) -> AccountID {
    AccountID::from_array([b; 20])
}
fn iou_currency(tag: &[u8; 3]) -> Currency {
    let mut d = [0u8; 20];
    d[12..15].copy_from_slice(tag);
    Currency::from(d)
}
fn iou(mantissa: i64, exponent: i32) -> IOUAmount {
    IOUAmount::from_parts(mantissa, exponent).unwrap_or_default()
}

fn build_ledger(seq: u32, entries: Vec<(Uint256, Vec<u8>)>) -> Ledger {
    let mut tree = MutableTree::new(seq);
    for (key, payload) in entries {
        tree.add_item(SHAMapNodeType::AccountState, SHAMapItem::new(key, payload))
            .unwrap();
    }
    let state_map = SyncTree::from_root_with_type(
        tree.root(),
        SHAMapType::State,
        false,
        seq,
        SyncState::Modifying,
    );
    Ledger::from_maps(
        LedgerHeader {
            seq,
            drops: 100_000_000_000,
            ..LedgerHeader::default()
        },
        state_map,
        SyncTree::new_with_type(SHAMapType::Transaction, false, seq),
    )
}

fn account_entry(account: AccountID, balance_drops: i64, owner_count: u32) -> (Uint256, Vec<u8>) {
    let keylet = account_keylet(raw_id(account));
    let mut sle = STLedgerEntry::from_type_and_key(LedgerEntryType::AccountRoot, keylet.key);
    sle.set_account_id(sf("sfAccount"), account);
    sle.set_field_u32(sf("sfSequence"), 1);
    sle.set_field_amount(
        sf("sfBalance"),
        STAmount::from_xrp_amount(XRPAmount::from_drops(balance_drops)),
    );
    sle.set_field_u32(sf("sfOwnerCount"), owner_count);
    (keylet.key, sle.get_serializer().data().to_vec())
}

fn trust_line_entry(
    low: AccountID,
    high: AccountID,
    currency: Currency,
    balance: IOUAmount,
    limit_low: IOUAmount,
    limit_high: IOUAmount,
) -> (Uint256, Vec<u8>) {
    let keylet = protocol::line(low, high, currency);
    let issue_low = Issue {
        currency,
        account: low,
    };
    let issue_high = Issue {
        currency,
        account: high,
    };
    let mut sle = STLedgerEntry::from_type_and_key(LedgerEntryType::RippleState, keylet.key);
    sle.set_field_amount(
        sf("sfBalance"),
        STAmount::from_iou_amount(sf("sfBalance"), balance, issue_high),
    );
    sle.set_field_amount(
        sf("sfLowLimit"),
        STAmount::from_iou_amount(sf("sfLowLimit"), limit_low, issue_low),
    );
    sle.set_field_amount(
        sf("sfHighLimit"),
        STAmount::from_iou_amount(sf("sfHighLimit"), limit_high, issue_high),
    );
    sle.set_field_u32(sf("sfFlags"), 0);
    (keylet.key, sle.get_serializer().data().to_vec())
}

fn nft_object(nft_id: Uint256) -> STObject {
    let mut token = STObject::make_inner_object(sf("sfNFToken"));
    token.set_field_h256(sf("sfNFTokenID"), nft_id);
    token
}

fn nft_page_entry(key: Uint256, token_ids: &[Uint256]) -> (Uint256, Vec<u8>) {
    let mut sle = STLedgerEntry::from_type_and_key(LedgerEntryType::NFTokenPage, key);
    let mut tokens = STArray::new(sf("sfNFTokens"));
    for token_id in token_ids {
        tokens.push_back(nft_object(*token_id));
    }
    sle.set_field_array(sf("sfNFTokens"), tokens);
    (key, sle.get_serializer().data().to_vec())
}

fn nft_sell_offer_entry(
    owner: AccountID,
    sequence: u32,
    nft_id: Uint256,
    amount: STAmount,
) -> (Uint256, Vec<u8>) {
    let keylet = protocol::nft_offer_keylet_for_owner(raw_id(owner), sequence);
    let mut sle = STLedgerEntry::from_type_and_key(LedgerEntryType::NFTokenOffer, keylet.key);
    sle.set_account_id(sf("sfOwner"), owner);
    sle.set_field_h256(sf("sfNFTokenID"), nft_id);
    sle.set_field_amount(sf("sfAmount"), amount);
    sle.set_field_u32(sf("sfFlags"), protocol::SELL_NF_TOKEN_LEDGER_FLAG);
    sle.set_field_u64(sf("sfOwnerNode"), 0);
    (keylet.key, sle.get_serializer().data().to_vec())
}

fn escrow_entry(
    owner: AccountID,
    sequence: u32,
    destination: AccountID,
    amount: STAmount,
    cancel_after: u32,
) -> (Uint256, Vec<u8>) {
    escrow_entry_with_transfer_rate(owner, sequence, destination, amount, cancel_after, None)
}

fn escrow_entry_with_transfer_rate(
    owner: AccountID,
    sequence: u32,
    destination: AccountID,
    amount: STAmount,
    cancel_after: u32,
    transfer_rate: Option<u32>,
) -> (Uint256, Vec<u8>) {
    let keylet = protocol::escrow_keylet(raw_id(owner), sequence);
    let mut sle = STLedgerEntry::from_type_and_key(LedgerEntryType::Escrow, keylet.key);
    sle.set_account_id(sf("sfAccount"), owner);
    sle.set_account_id(sf("sfDestination"), destination);
    sle.set_field_amount(sf("sfAmount"), amount);
    sle.set_field_u32(sf("sfCancelAfter"), cancel_after);
    if let Some(transfer_rate) = transfer_rate {
        sle.set_field_u32(sf("sfTransferRate"), transfer_rate);
    }
    sle.set_field_u64(sf("sfOwnerNode"), 0);
    (keylet.key, sle.get_serializer().data().to_vec())
}

fn check_entry(
    source: AccountID,
    destination: AccountID,
    sequence: u32,
    send_max: STAmount,
) -> (Uint256, Vec<u8>) {
    let keylet = protocol::check_keylet(raw_id(source), sequence);
    let mut sle = STLedgerEntry::from_type_and_key(LedgerEntryType::Check, keylet.key);
    sle.set_account_id(sf("sfAccount"), source);
    sle.set_account_id(sf("sfDestination"), destination);
    sle.set_field_amount(sf("sfSendMax"), send_max);
    sle.set_field_u32(sf("sfSequence"), sequence);
    sle.set_field_u64(sf("sfOwnerNode"), 0);
    (keylet.key, sle.get_serializer().data().to_vec())
}

fn mpt_id(issuer: AccountID, sequence: u32) -> basics::base_uint::Uint192 {
    let mut bytes = [0_u8; 24];
    bytes[..4].copy_from_slice(&sequence.to_be_bytes());
    bytes[4..].copy_from_slice(issuer.data());
    basics::base_uint::Uint192::from_slice(&bytes).expect("mpt id width")
}

fn mpt_issuance_entry(
    issuer: AccountID,
    sequence: u32,
    outstanding: u64,
    locked: u64,
) -> (Uint256, Vec<u8>) {
    mpt_issuance_entry_with_fee(issuer, sequence, outstanding, locked, None)
}

fn mpt_issuance_entry_with_fee(
    issuer: AccountID,
    sequence: u32,
    outstanding: u64,
    locked: u64,
    transfer_fee: Option<u16>,
) -> (Uint256, Vec<u8>) {
    let id = mpt_id(issuer, sequence);
    let keylet = protocol::mpt_issuance_keylet_from_mptid(id);
    let mut sle = STLedgerEntry::from_type_and_key(LedgerEntryType::MPTokenIssuance, keylet.key);
    sle.set_account_id(sf("sfIssuer"), issuer);
    sle.set_field_u32(sf("sfSequence"), sequence);
    sle.set_field_u64(sf("sfOutstandingAmount"), outstanding);
    sle.set_field_u32(sf("sfFlags"), protocol::lsfMPTCanTransfer);
    sle.set_field_u64(sf("sfOwnerNode"), 0);
    if locked != 0 {
        sle.set_field_u64(sf("sfLockedAmount"), locked);
    }
    if let Some(transfer_fee) = transfer_fee {
        sle.set_field_u16(sf("sfTransferFee"), transfer_fee);
    }
    (keylet.key, sle.get_serializer().data().to_vec())
}

fn mptoken_entry(
    holder: AccountID,
    issuer: AccountID,
    sequence: u32,
    amount: u64,
    locked: u64,
) -> (Uint256, Vec<u8>) {
    let id = mpt_id(issuer, sequence);
    let keylet = protocol::mptoken_keylet_from_mptid(id, raw_id(holder));
    let mut sle = STLedgerEntry::from_type_and_key(LedgerEntryType::MPToken, keylet.key);
    sle.set_account_id(sf("sfAccount"), holder);
    sle.set_field_h192(sf("sfMPTokenIssuanceID"), id);
    sle.set_field_u64(sf("sfMPTAmount"), amount);
    sle.set_field_u32(sf("sfFlags"), 0);
    sle.set_field_u64(sf("sfOwnerNode"), 0);
    if locked != 0 {
        sle.set_field_u64(sf("sfLockedAmount"), locked);
    }
    (keylet.key, sle.get_serializer().data().to_vec())
}

fn run(ledger: &Ledger, tx: STTx) -> Ter {
    let base = Arc::new(ledger.clone());
    let mut view = Sandbox::new(base, ApplyFlags::default());
    let txn_type = tx.get_txn_type();
    apply_submit_transactor_shell(&mut view, &tx, txn_type)
}

fn run_and_apply(ledger: &mut Ledger, tx: STTx) -> Ter {
    let base = Arc::new(ledger.clone());
    let mut view = Sandbox::new(base, ApplyFlags::default());
    let txn_type = tx.get_txn_type();
    let ter = apply_submit_transactor_shell(&mut view, &tx, txn_type);
    view.apply(ledger).expect("sandbox apply");
    ter
}

// ── Bug A: self-payment returns tecPATH_DRY ──────────────────────────────────

#[test]
fn self_payment_iou_to_iou_returns_tec_path_dry() {
    // Mirrors 3f05afd3: Account==Destination, IOU amount, IOU sendmax → tecPATH_DRY
    let account = acct(0x10);
    let issuer = acct(0x20);
    let currency = iou_currency(b"PHX");
    let issue = Issue {
        currency,
        account: issuer,
    };
    let ledger = build_ledger(
        104111034,
        vec![
            account_entry(account, 100_000_000, 0),
            account_entry(issuer, 100_000_000, 0),
        ],
    );
    let tx = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(sf("sfAccount"), account);
        tx.set_account_id(sf("sfDestination"), account); // self-payment
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(sf("sfSequence"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x0002_0000); // tfPartialPayment
        tx.set_field_vl(sf("sfSigningPubKey"), &[0u8; 33]);
        tx.set_field_amount(
            sf("sfAmount"),
            STAmount::from_iou_amount(sf("sfAmount"), iou(1_000_000_000_000_000, 0), issue),
        );
        tx.set_field_amount(
            sf("sfSendMax"),
            STAmount::from_iou_amount(sf("sfSendMax"), iou(1_000_000_000_000_000, 0), issue),
        );
    });
    assert_eq!(
        run(&ledger, tx),
        Ter::TEC_PATH_DRY,
        "self-payment IOU→IOU must return tecPATH_DRY"
    );
}

#[test]
fn self_payment_iou_to_xrp_returns_tec_path_dry() {
    // Mirrors 45d72362: Account==Destination, Amount=XRP, SendMax=IOU → tecPATH_DRY
    let account = acct(0x10);
    let issuer = acct(0x20);
    let currency = iou_currency(b"ARM");
    let issue = Issue {
        currency,
        account: issuer,
    };
    let ledger = build_ledger(
        104111034,
        vec![
            account_entry(account, 100_000_000, 0),
            account_entry(issuer, 100_000_000, 0),
        ],
    );
    let tx = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(sf("sfAccount"), account);
        tx.set_account_id(sf("sfDestination"), account); // self-payment
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(sf("sfSequence"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x0002_0000);
        tx.set_field_vl(sf("sfSigningPubKey"), &[0u8; 33]);
        tx.set_field_amount(
            sf("sfAmount"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(1_330_205)),
        );
        tx.set_field_amount(
            sf("sfSendMax"),
            STAmount::from_iou_amount(sf("sfSendMax"), iou(2_486_691_010_129, -9), issue),
        );
    });
    assert_eq!(
        run(&ledger, tx),
        Ter::TEC_PATH_DRY,
        "self-payment IOU→XRP must return tecPATH_DRY"
    );
}

#[test]
fn escrow_cancel_iou_does_not_credit_xrp_drops() {
    // Mirrors #6171's core safety point for Rust's current IOU unlock surface:
    // non-XRP escrow cancel must restore the IOU, not add the IOU mantissa to XRP.
    let owner = acct(0x10);
    let destination = acct(0x11);
    let issuer = acct(0x20);
    let currency = iou_currency(b"USD");
    let issue = Issue {
        currency,
        account: issuer,
    };
    let amount = STAmount::from_iou_amount(sf("sfAmount"), iou(1_000, 0), issue);
    let mut ledger = build_ledger(
        104111034,
        vec![
            account_entry(owner, 100_000_000, 1),
            account_entry(destination, 100_000_000, 0),
            account_entry(issuer, 100_000_000, 0),
            escrow_entry(owner, 1, destination, amount, 0),
        ],
    );

    let tx = STTx::new(TxType::ESCROW_CANCEL, |tx| {
        tx.set_account_id(sf("sfAccount"), owner);
        tx.set_account_id(sf("sfOwner"), owner);
        tx.set_field_u32(sf("sfOfferSequence"), 1);
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(sf("sfSequence"), 1);
        tx.set_field_vl(sf("sfSigningPubKey"), &[0u8; 33]);
    });

    assert_eq!(run_and_apply(&mut ledger, tx), Ter::TES_SUCCESS);

    let owner_sle = ledger
        .peek(account_keylet(raw_id(owner)))
        .expect("owner read")
        .expect("owner exists");
    assert_eq!(
        owner_sle.get_field_amount(sf("sfBalance")).xrp().drops(),
        99_999_990
    );
    assert!(
        ledger
            .peek(protocol::line(issuer, owner, currency))
            .expect("line read")
            .is_some(),
        "IOU cancel should restore/create the owner trust line"
    );
}

#[test]
fn escrow_cancel_mpt_unlocks_locked_amount_accounting() {
    let owner = acct(0x10);
    let destination = acct(0x11);
    let issuer = acct(0x20);
    let issuance_id = mpt_id(issuer, 1);
    let issue = MPTIssue::new(issuance_id);
    let amount = STAmount::from_mpt_amount(sf("sfAmount"), MPTAmount::from_value(10), issue);
    let mut ledger = build_ledger(
        104111034,
        vec![
            account_entry(owner, 100_000_000, 1),
            account_entry(destination, 100_000_000, 0),
            account_entry(issuer, 100_000_000, 0),
            mpt_issuance_entry(issuer, 1, 100, 10),
            mptoken_entry(owner, issuer, 1, 90, 10),
            escrow_entry(owner, 1, destination, amount, 0),
        ],
    );

    let tx = STTx::new(TxType::ESCROW_CANCEL, |tx| {
        tx.set_account_id(sf("sfAccount"), owner);
        tx.set_account_id(sf("sfOwner"), owner);
        tx.set_field_u32(sf("sfOfferSequence"), 1);
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(sf("sfSequence"), 1);
        tx.set_field_vl(sf("sfSigningPubKey"), &[0u8; 33]);
    });

    assert_eq!(run_and_apply(&mut ledger, tx), Ter::TES_SUCCESS);

    let token = ledger
        .peek(protocol::mptoken_keylet_from_mptid(
            issuance_id,
            raw_id(owner),
        ))
        .expect("token read")
        .expect("token exists");
    assert_eq!(token.get_field_u64(sf("sfMPTAmount")), 100);
    assert!(!token.is_field_present(sf("sfLockedAmount")));

    let issuance = ledger
        .peek(protocol::mpt_issuance_keylet_from_mptid(issuance_id))
        .expect("issuance read")
        .expect("issuance exists");
    assert_eq!(issuance.get_field_u64(sf("sfOutstandingAmount")), 100);
    assert!(!issuance.is_field_present(sf("sfLockedAmount")));
}

#[test]
fn escrow_cancel_mpt_preserves_remaining_locked_amount_accounting() {
    let owner = acct(0x12);
    let destination = acct(0x13);
    let issuer = acct(0x21);
    let issuance_id = mpt_id(issuer, 1);
    let issue = MPTIssue::new(issuance_id);
    let amount = STAmount::from_mpt_amount(sf("sfAmount"), MPTAmount::from_value(10), issue);
    let mut ledger = build_ledger(
        104111034,
        vec![
            account_entry(owner, 100_000_000, 1),
            account_entry(destination, 100_000_000, 0),
            account_entry(issuer, 100_000_000, 0),
            mpt_issuance_entry(issuer, 1, 100, 25),
            mptoken_entry(owner, issuer, 1, 75, 25),
            escrow_entry(owner, 1, destination, amount, 0),
        ],
    );

    let tx = STTx::new(TxType::ESCROW_CANCEL, |tx| {
        tx.set_account_id(sf("sfAccount"), owner);
        tx.set_account_id(sf("sfOwner"), owner);
        tx.set_field_u32(sf("sfOfferSequence"), 1);
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(sf("sfSequence"), 1);
        tx.set_field_vl(sf("sfSigningPubKey"), &[0u8; 33]);
    });

    assert_eq!(run_and_apply(&mut ledger, tx), Ter::TES_SUCCESS);

    let token = ledger
        .peek(protocol::mptoken_keylet_from_mptid(
            issuance_id,
            raw_id(owner),
        ))
        .expect("token read")
        .expect("token exists");
    assert_eq!(token.get_field_u64(sf("sfMPTAmount")), 85);
    assert_eq!(token.get_field_u64(sf("sfLockedAmount")), 15);

    let issuance = ledger
        .peek(protocol::mpt_issuance_keylet_from_mptid(issuance_id))
        .expect("issuance read")
        .expect("issuance exists");
    assert_eq!(issuance.get_field_u64(sf("sfOutstandingAmount")), 100);
    assert_eq!(issuance.get_field_u64(sf("sfLockedAmount")), 15);
}

#[test]
fn escrow_finish_mpt_uses_lower_current_transfer_rate_and_unlocks_gross_amount() {
    let owner = acct(0x16);
    let destination = acct(0x17);
    let issuer = acct(0x23);
    let issuance_id = mpt_id(issuer, 1);
    let issue = MPTIssue::new(issuance_id);
    let amount = STAmount::from_mpt_amount(sf("sfAmount"), MPTAmount::from_value(100), issue);
    let expected_net = protocol::divide_round(&amount, protocol::Rate::new(1_100_000_000), true)
        .mpt()
        .value() as u64;
    let mut ledger = build_ledger(
        104111034,
        vec![
            account_entry(owner, 100_000_000, 1),
            account_entry(destination, 100_000_000, 0),
            account_entry(issuer, 100_000_000, 0),
            mpt_issuance_entry_with_fee(issuer, 1, 1_000, 100, Some(10_000)),
            mptoken_entry(owner, issuer, 1, 900, 100),
            mptoken_entry(destination, issuer, 1, 0, 0),
            escrow_entry_with_transfer_rate(owner, 1, destination, amount, 0, Some(1_200_000_000)),
        ],
    );
    ledger.set_rules(protocol::Rules::new([
        protocol::feature_id("fixTokenEscrowV1"),
        protocol::feature_id("fixCleanup3_2_0"),
    ]));

    let tx = STTx::new(TxType::ESCROW_FINISH, |tx| {
        tx.set_account_id(sf("sfAccount"), destination);
        tx.set_account_id(sf("sfOwner"), owner);
        tx.set_field_u32(sf("sfOfferSequence"), 1);
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(sf("sfSequence"), 1);
        tx.set_field_vl(sf("sfSigningPubKey"), &[0u8; 33]);
    });

    assert_eq!(run_and_apply(&mut ledger, tx), Ter::TES_SUCCESS);

    let destination_token = ledger
        .peek(protocol::mptoken_keylet_from_mptid(
            issuance_id,
            raw_id(destination),
        ))
        .expect("destination token read")
        .expect("destination token exists");
    assert_eq!(
        destination_token.get_field_u64(sf("sfMPTAmount")),
        expected_net
    );

    let owner_token = ledger
        .peek(protocol::mptoken_keylet_from_mptid(
            issuance_id,
            raw_id(owner),
        ))
        .expect("owner token read")
        .expect("owner token exists");
    assert!(!owner_token.is_field_present(sf("sfLockedAmount")));

    let issuance = ledger
        .peek(protocol::mpt_issuance_keylet_from_mptid(issuance_id))
        .expect("issuance read")
        .expect("issuance exists");
    assert!(!issuance.is_field_present(sf("sfLockedAmount")));
    assert_eq!(
        issuance.get_field_u64(sf("sfOutstandingAmount")),
        1_000 - (100 - expected_net)
    );
}

#[test]
fn escrow_create_mpt_locks_holder_and_issuance_amounts() {
    let owner = acct(0x10);
    let destination = acct(0x11);
    let issuer = acct(0x20);
    let issuance_id = mpt_id(issuer, 1);
    let issue = MPTIssue::new(issuance_id);
    let amount = STAmount::from_mpt_amount(sf("sfAmount"), MPTAmount::from_value(10), issue);
    let mut ledger = build_ledger(
        104111034,
        vec![
            account_entry(owner, 100_000_000, 0),
            account_entry(destination, 100_000_000, 0),
            account_entry(issuer, 100_000_000, 0),
            mpt_issuance_entry(issuer, 1, 100, 0),
            mptoken_entry(owner, issuer, 1, 100, 0),
        ],
    );

    let tx = STTx::new(TxType::ESCROW_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), owner);
        tx.set_account_id(sf("sfDestination"), destination);
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(sf("sfSequence"), 1);
        tx.set_field_vl(sf("sfSigningPubKey"), &[0u8; 33]);
        tx.set_field_amount(sf("sfAmount"), amount);
        tx.set_field_u32(sf("sfCancelAfter"), 1);
    });

    assert_eq!(run_and_apply(&mut ledger, tx), Ter::TES_SUCCESS);

    let token = ledger
        .peek(protocol::mptoken_keylet_from_mptid(
            issuance_id,
            raw_id(owner),
        ))
        .expect("token read")
        .expect("token exists");
    assert_eq!(token.get_field_u64(sf("sfMPTAmount")), 90);
    assert_eq!(token.get_field_u64(sf("sfLockedAmount")), 10);

    let issuance = ledger
        .peek(protocol::mpt_issuance_keylet_from_mptid(issuance_id))
        .expect("issuance read")
        .expect("issuance exists");
    assert_eq!(issuance.get_field_u64(sf("sfOutstandingAmount")), 100);
    assert_eq!(issuance.get_field_u64(sf("sfLockedAmount")), 10);
}

#[test]
fn escrow_create_mpt_records_locked_transfer_rate() {
    let owner = acct(0x10);
    let destination = acct(0x11);
    let issuer = acct(0x20);
    let issuance_id = mpt_id(issuer, 1);
    let issue = MPTIssue::new(issuance_id);
    let amount = STAmount::from_mpt_amount(sf("sfAmount"), MPTAmount::from_value(10), issue);
    let mut ledger = build_ledger(
        104111034,
        vec![
            account_entry(owner, 100_000_000, 0),
            account_entry(destination, 100_000_000, 0),
            account_entry(issuer, 100_000_000, 0),
            mpt_issuance_entry_with_fee(issuer, 1, 100, 0, Some(10_000)),
            mptoken_entry(owner, issuer, 1, 100, 0),
        ],
    );

    let tx = STTx::new(TxType::ESCROW_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), owner);
        tx.set_account_id(sf("sfDestination"), destination);
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(sf("sfSequence"), 1);
        tx.set_field_vl(sf("sfSigningPubKey"), &[0u8; 33]);
        tx.set_field_amount(sf("sfAmount"), amount);
        tx.set_field_u32(sf("sfCancelAfter"), 1);
    });

    assert_eq!(run_and_apply(&mut ledger, tx), Ter::TES_SUCCESS);
    let escrow = ledger
        .peek(protocol::escrow_keylet(raw_id(owner), 1))
        .expect("escrow read")
        .expect("escrow exists");
    assert_eq!(escrow.get_field_u32(sf("sfTransferRate")), 1_100_000_000);
}

#[test]
fn escrow_finish_mpt_applies_transfer_fee_and_burns_fee_from_outstanding() {
    let owner = acct(0x10);
    let destination = acct(0x11);
    let issuer = acct(0x20);
    let issuance_id = mpt_id(issuer, 1);
    let issue = MPTIssue::new(issuance_id);
    let amount = STAmount::from_mpt_amount(sf("sfAmount"), MPTAmount::from_value(100), issue);
    let expected_net = protocol::divide_round(&amount, protocol::Rate::new(1_100_000_000), true)
        .mpt()
        .value() as u64;
    let mut ledger = build_ledger(
        104111034,
        vec![
            account_entry(owner, 100_000_000, 1),
            account_entry(destination, 100_000_000, 0),
            account_entry(issuer, 100_000_000, 0),
            mpt_issuance_entry_with_fee(issuer, 1, 1_000, 100, Some(10_000)),
            mptoken_entry(owner, issuer, 1, 900, 100),
            mptoken_entry(destination, issuer, 1, 0, 0),
            escrow_entry_with_transfer_rate(owner, 1, destination, amount, 0, Some(1_100_000_000)),
        ],
    );
    ledger.set_rules(protocol::Rules::new([protocol::feature_id(
        "fixTokenEscrowV1",
    )]));

    let tx = STTx::new(TxType::ESCROW_FINISH, |tx| {
        tx.set_account_id(sf("sfAccount"), destination);
        tx.set_account_id(sf("sfOwner"), owner);
        tx.set_field_u32(sf("sfOfferSequence"), 1);
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(sf("sfSequence"), 1);
        tx.set_field_vl(sf("sfSigningPubKey"), &[0u8; 33]);
    });

    assert_eq!(run_and_apply(&mut ledger, tx), Ter::TES_SUCCESS);

    let destination_token = ledger
        .peek(protocol::mptoken_keylet_from_mptid(
            issuance_id,
            raw_id(destination),
        ))
        .expect("destination token read")
        .expect("destination token exists");
    assert_eq!(
        destination_token.get_field_u64(sf("sfMPTAmount")),
        expected_net
    );

    let owner_token = ledger
        .peek(protocol::mptoken_keylet_from_mptid(
            issuance_id,
            raw_id(owner),
        ))
        .expect("owner token read")
        .expect("owner token exists");
    assert!(!owner_token.is_field_present(sf("sfLockedAmount")));

    let issuance = ledger
        .peek(protocol::mpt_issuance_keylet_from_mptid(issuance_id))
        .expect("issuance read")
        .expect("issuance exists");
    assert!(!issuance.is_field_present(sf("sfLockedAmount")));
    assert_eq!(
        issuance.get_field_u64(sf("sfOutstandingAmount")),
        1_000 - (100 - expected_net)
    );
}

#[test]
fn escrow_finish_mpt_to_issuer_burns_gross_locked_amount() {
    let owner = acct(0x10);
    let issuer = acct(0x20);
    let issuance_id = mpt_id(issuer, 1);
    let issue = MPTIssue::new(issuance_id);
    let amount = STAmount::from_mpt_amount(sf("sfAmount"), MPTAmount::from_value(100), issue);
    let mut ledger = build_ledger(
        104111034,
        vec![
            account_entry(owner, 100_000_000, 1),
            account_entry(issuer, 100_000_000, 0),
            mpt_issuance_entry_with_fee(issuer, 1, 1_000, 100, Some(10_000)),
            mptoken_entry(owner, issuer, 1, 900, 100),
            escrow_entry_with_transfer_rate(owner, 1, issuer, amount, 0, Some(1_100_000_000)),
        ],
    );
    ledger.set_rules(protocol::Rules::new([protocol::feature_id(
        "fixTokenEscrowV1",
    )]));

    let tx = STTx::new(TxType::ESCROW_FINISH, |tx| {
        tx.set_account_id(sf("sfAccount"), issuer);
        tx.set_account_id(sf("sfOwner"), owner);
        tx.set_field_u32(sf("sfOfferSequence"), 1);
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(sf("sfSequence"), 1);
        tx.set_field_vl(sf("sfSigningPubKey"), &[0u8; 33]);
    });

    assert_eq!(run_and_apply(&mut ledger, tx), Ter::TES_SUCCESS);

    let owner_token = ledger
        .peek(protocol::mptoken_keylet_from_mptid(
            issuance_id,
            raw_id(owner),
        ))
        .expect("owner token read")
        .expect("owner token exists");
    assert_eq!(owner_token.get_field_u64(sf("sfMPTAmount")), 900);
    assert!(!owner_token.is_field_present(sf("sfLockedAmount")));

    let issuance = ledger
        .peek(protocol::mpt_issuance_keylet_from_mptid(issuance_id))
        .expect("issuance read")
        .expect("issuance exists");
    assert!(!issuance.is_field_present(sf("sfLockedAmount")));
    assert_eq!(issuance.get_field_u64(sf("sfOutstandingAmount")), 900);
    assert!(
        ledger
            .peek(protocol::mptoken_keylet_from_mptid(
                issuance_id,
                raw_id(issuer),
            ))
            .expect("issuer token read")
            .is_none()
    );
}

#[test]
fn fix_cleanup_3_2_0_rejects_recursive_invalid_mpt_amount_in_tx_array() {
    let account = acct(0x10);
    let issuer = acct(0x20);
    let issuance_id = mpt_id(issuer, 1);
    let issue = MPTIssue::new(issuance_id);
    let mut ledger = build_ledger(104111034, vec![account_entry(account, 100_000_000, 0)]);
    ledger.set_rules(protocol::Rules::new([protocol::feature_id(
        "fixCleanup3_2_0",
    )]));

    let tx = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(sf("sfAccount"), account);
        tx.set_account_id(sf("sfDestination"), acct(0x11));
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(sf("sfSequence"), 1);
        tx.set_field_vl(sf("sfSigningPubKey"), &[0u8; 33]);
        tx.set_field_amount(
            sf("sfAmount"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(1)),
        );

        let mut memo = STObject::make_inner_object(sf("sfMemo"));
        memo.set_field_amount(
            sf("sfAmount"),
            STAmount::from_mpt_amount(sf("sfAmount"), MPTAmount::from_value(-1), issue),
        );
        let mut memos = STArray::new(sf("sfMemos"));
        memos.push_back(memo);
        tx.set_field_array(sf("sfMemos"), memos);
    });

    assert_eq!(run(&ledger, tx), Ter::TEM_BAD_AMOUNT);
}

#[test]
fn check_cash_rejects_legacy_check_with_invalid_mpt_send_max_after_fix_cleanup_3_2_0() {
    let source = acct(0x14);
    let destination = acct(0x15);
    let issuer = acct(0x22);
    let issuance_id = mpt_id(issuer, 1);
    let issue = MPTIssue::new(issuance_id);
    let invalid_send_max =
        STAmount::from_mpt_amount(sf("sfSendMax"), MPTAmount::from_value(-1), issue);
    let valid_amount = STAmount::from_mpt_amount(sf("sfAmount"), MPTAmount::from_value(1), issue);
    let check_key = protocol::check_keylet(raw_id(source), 1).key;
    let mut ledger = build_ledger(
        104111034,
        vec![
            account_entry(source, 100_000_000, 1),
            account_entry(destination, 100_000_000, 0),
            account_entry(issuer, 100_000_000, 0),
            mpt_issuance_entry(issuer, 1, 10, 0),
            mptoken_entry(source, issuer, 1, 10, 0),
            mptoken_entry(destination, issuer, 1, 0, 0),
            check_entry(source, destination, 1, invalid_send_max),
        ],
    );
    ledger.set_rules(protocol::Rules::new([protocol::feature_id(
        "fixCleanup3_2_0",
    )]));

    let tx = STTx::new(TxType::CHECK_CASH, |tx| {
        tx.set_account_id(sf("sfAccount"), destination);
        tx.set_field_h256(sf("sfCheckID"), check_key);
        tx.set_field_amount(sf("sfAmount"), valid_amount);
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(sf("sfSequence"), 1);
        tx.set_field_vl(sf("sfSigningPubKey"), &[0u8; 33]);
    });

    assert_eq!(run(&ledger, tx), Ter::TEF_BAD_LEDGER);
}

#[test]
fn escrow_finish_mpt_create_holding_uses_pre_fee_reserve_balance() {
    let owner = acct(0x10);
    let destination = acct(0x11);
    let issuer = acct(0x20);
    let issuance_id = mpt_id(issuer, 1);
    let issue = MPTIssue::new(issuance_id);
    let amount = STAmount::from_mpt_amount(sf("sfAmount"), MPTAmount::from_value(10), issue);
    let mut ledger = build_ledger(
        104111034,
        vec![
            account_entry(owner, 100_000_000, 1),
            account_entry(destination, 1_200_000, 0),
            account_entry(issuer, 100_000_000, 0),
            mpt_issuance_entry(issuer, 1, 100, 10),
            mptoken_entry(owner, issuer, 1, 90, 10),
            escrow_entry(owner, 1, destination, amount, 0),
        ],
    );

    let tx = STTx::new(TxType::ESCROW_FINISH, |tx| {
        tx.set_account_id(sf("sfAccount"), destination);
        tx.set_account_id(sf("sfOwner"), owner);
        tx.set_field_u32(sf("sfOfferSequence"), 1);
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(sf("sfSequence"), 1);
        tx.set_field_vl(sf("sfSigningPubKey"), &[0u8; 33]);
    });

    assert_eq!(run_and_apply(&mut ledger, tx), Ter::TES_SUCCESS);

    let destination_root = ledger
        .peek(protocol::account_keylet(raw_id(destination)))
        .expect("destination account read")
        .expect("destination account exists");
    assert_eq!(destination_root.get_field_u32(sf("sfOwnerCount")), 1);

    let destination_token = ledger
        .peek(protocol::mptoken_keylet_from_mptid(
            issuance_id,
            raw_id(destination),
        ))
        .expect("destination token read")
        .expect("destination token exists");
    assert_eq!(destination_token.get_field_u64(sf("sfMPTAmount")), 10);
}

// ── Bug B: tecUNFUNDED_OFFER ─────────────────────────────────────────────────

#[test]
fn offer_create_zero_iou_balance_returns_tec_unfunded_offer() {
    // Mirrors f75b24ba: account offers IOU (TakerGets) but has zero IOU balance
    let account = acct(0x10);
    let issuer = acct(0x20);
    let currency = iou_currency(b"OUS");
    let issue = Issue {
        currency,
        account: issuer,
    };
    // No trust line → zero IOU balance
    let ledger = build_ledger(
        104111032,
        vec![
            account_entry(account, 100_000_000, 0),
            account_entry(issuer, 100_000_000, 0),
        ],
    );
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), account);
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(sf("sfSequence"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x0002_0000); // tfPassive
        tx.set_field_vl(sf("sfSigningPubKey"), &[0u8; 33]);
        tx.set_field_amount(
            sf("sfTakerPays"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(7)),
        );
        tx.set_field_amount(
            sf("sfTakerGets"),
            STAmount::from_iou_amount(sf("sfTakerGets"), iou(5_007_888_892_255, -4), issue),
        );
    });
    assert_eq!(
        run(&ledger, tx),
        Ter::TEC_UNFUNDED_OFFER,
        "OfferCreate with zero TakerGets IOU balance must return tecUNFUNDED_OFFER"
    );
}

#[test]
fn offer_create_zero_liquid_xrp_returns_tec_unfunded_offer() {
    // Mirrors 13716ce7: account offers XRP (TakerGets=XRP) but has zero liquid XRP
    let account = acct(0x10);
    let issuer = acct(0x20);
    let currency = iou_currency(b"DRO");
    let issue = Issue {
        currency,
        account: issuer,
    };
    // Account has exactly reserve (200_000 drops), zero liquid XRP after reserve.
    // We set fees so reserve=200_000 to match mainnet.
    let mut ledger = build_ledger(
        104111033,
        vec![
            account_entry(account, 200_000, 0),
            account_entry(issuer, 100_000_000, 0),
            trust_line_entry(
                account,
                issuer,
                currency,
                iou(1_000_000_000_000, -9),
                iou(10_000, 0),
                iou(0, 0),
            ),
        ],
    );
    // Set mainnet-like fees so reserve=200_000 drops
    ledger.set_fees(Fees {
        base: 10,
        reserve: 200_000,
        increment: 50_000,
    });
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), account);
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(12)),
        );
        tx.set_field_u32(sf("sfSequence"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x000a_0000); // tfPassive|tfImmediateOrCancel
        tx.set_field_vl(sf("sfSigningPubKey"), &[0u8; 33]);
        tx.set_field_amount(
            sf("sfTakerPays"),
            STAmount::from_iou_amount(sf("sfTakerPays"), iou(6_505_508_109_500, -13), issue),
        );
        tx.set_field_amount(
            sf("sfTakerGets"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(1_785_546)),
        );
    });
    assert_eq!(
        run(&ledger, tx),
        Ter::TEC_UNFUNDED_OFFER,
        "OfferCreate with zero liquid XRP must return tecUNFUNDED_OFFER"
    );
}

// ── Bug C: ImmediateOrCancel not filled → tecKILLED ──────────────────────────

#[test]
fn offer_create_ioc_no_matching_offers_returns_tec_killed() {
    // Mirrors f7e8826f: tfImmediateOrCancel, no matching offers → tecKILLED
    let account = acct(0x10);
    let issuer = acct(0x20);
    let currency = iou_currency(b"CUL");
    let issue = Issue {
        currency,
        account: issuer,
    };
    // Account has IOU balance (so not tecUNFUNDED_OFFER)
    let ledger = build_ledger(
        104111035,
        vec![
            account_entry(account, 100_000_000, 0),
            account_entry(issuer, 100_000_000, 0),
            trust_line_entry(
                account,
                issuer,
                currency,
                iou(1_000_000_000_000, -9),
                iou(10_000, 0),
                iou(0, 0),
            ),
        ],
    );
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), account);
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(sf("sfSequence"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x0002_0000); // tfImmediateOrCancel
        tx.set_field_vl(sf("sfSigningPubKey"), &[0u8; 33]);
        tx.set_field_amount(
            sf("sfTakerPays"),
            STAmount::from_iou_amount(sf("sfTakerPays"), iou(2_275_889_852_000, -10), issue),
        );
        tx.set_field_amount(
            sf("sfTakerGets"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(1_000_000)),
        );
    });
    assert_eq!(
        run(&ledger, tx),
        Ter::TEC_KILLED,
        "ImmediateOrCancel with no matching offers must return tecKILLED"
    );
}

// ── Bug 1: balance < fee in closed ledger → tecINSUFF_FEE ────────────────────
// Ledger 104103382: 12daf3d4 — Payment where account balance < fee.
// C++ returns tecINSUFF_FEE (claimed, fee capped to balance).
// We were returning terINSUF_FEE_B (retry, no fee burned).

#[test]
fn payment_balance_less_than_fee_returns_tec_insuff_fee() {
    // Account has 10 drops, fee is 15 drops → balance < fee → tecINSUFF_FEE
    let account = acct(0x10);
    let dst = acct(0x20);
    let mut ledger = build_ledger(
        104103382,
        vec![
            account_entry(account, 10, 0), // only 10 drops
            account_entry(dst, 100_000_000, 0),
        ],
    );
    ledger.set_fees(Fees {
        base: 10,
        reserve: 200_000,
        increment: 50_000,
    });

    let tx = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(sf("sfAccount"), account);
        tx.set_account_id(sf("sfDestination"), dst);
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(15)),
        );
        tx.set_field_u32(sf("sfSequence"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x0002_0000); // tfPartialPayment
        tx.set_field_vl(sf("sfSigningPubKey"), &[0u8; 33]);
        tx.set_field_amount(
            sf("sfAmount"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(1_000)),
        );
    });

    // In closed ledger: balance(10) < fee(15) but balance > 0 → tecINSUFF_FEE
    assert_eq!(
        run(&ledger, tx),
        Ter::TEC_INSUFF_FEE,
        "balance < fee in closed ledger must return tecINSUFF_FEE"
    );
}

#[test]
fn payment_zero_balance_returns_ter_insuf_fee_b() {
    // Account has 0 drops → terINSUF_FEE_B (retry, no fee to burn)
    let account = acct(0x10);
    let dst = acct(0x20);
    let ledger = build_ledger(
        104103382,
        vec![
            account_entry(account, 0, 0),
            account_entry(dst, 100_000_000, 0),
        ],
    );

    let tx = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(sf("sfAccount"), account);
        tx.set_account_id(sf("sfDestination"), dst);
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(15)),
        );
        tx.set_field_u32(sf("sfSequence"), 1);
        tx.set_field_u32(sf("sfFlags"), 0);
        tx.set_field_vl(sf("sfSigningPubKey"), &[0u8; 33]);
        tx.set_field_amount(
            sf("sfAmount"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(1_000)),
        );
    });

    assert_eq!(
        run(&ledger, tx),
        Ter::TER_INSUF_FEE_B,
        "zero balance must return terINSUF_FEE_B"
    );
}

// ── Bug 2: FillOrKill OfferCreate → tecKILLED with state reset ───────────────
// Ledger 104103382: 0bf7fbd1 — OfferCreate tfFillOrKill, no matching offers.
// C++ returns tecKILLED. State changes from crossing are discarded.

#[test]
fn offer_create_fok_no_matching_offers_returns_tec_killed() {
    // Mirrors 0bf7fbd1: tfFillOrKill, IOU→XRP, no matching offers → tecKILLED
    let account = acct(0x10);
    let issuer = acct(0x20);
    let currency = iou_currency(b"SCR");
    let issue = Issue {
        currency,
        account: issuer,
    };
    let ledger = build_ledger(
        104103382,
        vec![
            account_entry(account, 100_000_000, 0),
            account_entry(issuer, 100_000_000, 0),
            trust_line_entry(
                account,
                issuer,
                currency,
                iou(5_000_000_000_000, -6),
                iou(10_000_000, 0),
                iou(0, 0),
            ),
        ],
    );

    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), account);
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(15)),
        );
        tx.set_field_u32(sf("sfSequence"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x0004_0000); // tfFillOrKill
        tx.set_field_vl(sf("sfSigningPubKey"), &[0u8; 33]);
        // TakerPays = IOU (what account offers)
        tx.set_field_amount(
            sf("sfTakerPays"),
            STAmount::from_iou_amount(sf("sfTakerPays"), iou(2_230_748_829_406, -6), issue),
        );
        // TakerGets = XRP (what account wants)
        tx.set_field_amount(
            sf("sfTakerGets"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(711_077_348)),
        );
    });

    assert_eq!(
        run(&ledger, tx),
        Ter::TEC_KILLED,
        "FillOrKill OfferCreate with no matching offers must return tecKILLED"
    );
}

// ── Bug 3: EscrowFinish on IOU escrow → tecLIMIT_EXCEEDED ────────────────────
// Ledger 104109073: 3c0c240c — EscrowFinish on a non-native (IOU) escrow.
// C++ returns tecLIMIT_EXCEEDED. We were returning tesSUCCESS.

#[test]
fn escrow_finish_iou_escrow_returns_tec_limit_exceeded() {
    // EscrowFinish where the escrow holds IOU (non-native) → tecLIMIT_EXCEEDED
    use basics::base_uint::Uint160;
    use protocol::{LedgerEntryType, escrow_keylet};

    let account = acct(0x10); // finisher
    let owner = acct(0x20); // escrow owner
    let dst = acct(0x30); // escrow destination
    let issuer = acct(0x40);
    let currency = iou_currency(b"USD");
    let issue = Issue {
        currency,
        account: issuer,
    };
    let offer_seq = 42u32;

    // Build an escrow SLE with non-native amount
    let escrow_kl = escrow_keylet(Uint160::from_slice(owner.data()).unwrap(), offer_seq);
    let mut escrow_sle = STLedgerEntry::from_type_and_key(LedgerEntryType::Escrow, escrow_kl.key);
    escrow_sle.set_account_id(sf("sfAccount"), owner);
    escrow_sle.set_account_id(sf("sfDestination"), dst);
    escrow_sle.set_field_u32(sf("sfFinishAfter"), 0); // no time lock
    // Non-native amount (IOU)
    escrow_sle.set_field_amount(
        sf("sfAmount"),
        STAmount::from_iou_amount(sf("sfAmount"), iou(1_000_000_000_000, -9), issue),
    );
    escrow_sle.set_field_u64(sf("sfOwnerNode"), 0);

    let ledger = build_ledger(
        104109073,
        vec![
            account_entry(account, 100_000_000, 0),
            account_entry(owner, 100_000_000, 1),
            account_entry(dst, 100_000_000, 0),
            account_entry(issuer, 100_000_000, 0),
            (escrow_kl.key, escrow_sle.get_serializer().data().to_vec()),
        ],
    );

    let tx = STTx::new(TxType::ESCROW_FINISH, |tx| {
        tx.set_account_id(sf("sfAccount"), account);
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(12)),
        );
        tx.set_field_u32(sf("sfSequence"), 1);
        tx.set_field_u32(sf("sfFlags"), 0);
        tx.set_field_vl(sf("sfSigningPubKey"), &[0u8; 33]);
        tx.set_account_id(sf("sfOwner"), owner);
        tx.set_field_u32(sf("sfOfferSequence"), offer_seq);
    });

    assert_eq!(
        run(&ledger, tx),
        Ter::TEC_LIMIT_EXCEEDED,
        "EscrowFinish on IOU escrow must return tecLIMIT_EXCEEDED"
    );
}

// ── Bug 4: tfNoRippleDirect with dry explicit path → tecPATH_DRY ─────────────
// Ledger 104109074: 06cfc67b — Payment flags=0x30000 (tfPartialPayment|tfNoRippleDirect)
// XRP→IOU with explicit path, no liquidity → tecPATH_DRY (not tecPATH_PARTIAL).

#[test]
fn payment_no_ripple_direct_dry_explicit_path_returns_tec_path_dry() {
    // Mirrors 06cfc67b: tfPartialPayment|tfNoRippleDirect, XRP sendmax, IOU amount,
    // explicit path through empty order book → tecPATH_DRY
    let account = acct(0x10);
    let dst = acct(0x20);
    let issuer = acct(0x30);
    let currency = iou_currency(b"PHX");
    let issue = Issue {
        currency,
        account: issuer,
    };

    let ledger = build_ledger(
        104109074,
        vec![
            account_entry(account, 100_000_000, 0),
            account_entry(dst, 100_000_000, 0),
            account_entry(issuer, 100_000_000, 0),
        ],
    );

    // Build an explicit path: XRP → PHOENIX IOU (through empty book)
    let mut path = protocol::STPath::new();
    path.push_back(protocol::STPathElement::from_optionals(
        None,
        Some(protocol::PathAsset::Currency(currency)),
        Some(issuer),
    ));

    let mut paths = protocol::STPathSet::new(sf("sfPaths"));
    paths.push_back(path);

    let tx = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(sf("sfAccount"), account);
        tx.set_account_id(sf("sfDestination"), dst);
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(sf("sfSequence"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x0003_0000); // tfPartialPayment | tfNoRippleDirect
        tx.set_field_vl(sf("sfSigningPubKey"), &[0u8; 33]);
        // Amount = huge IOU (can't be delivered)
        tx.set_field_amount(
            sf("sfAmount"),
            STAmount::from_iou_amount(sf("sfAmount"), iou(1_000_000_000_000_000, 0), issue),
        );
        // SendMax = XRP
        tx.set_field_amount(
            sf("sfSendMax"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(781_254)),
        );
        tx.set_field_path_set(sf("sfPaths"), paths);
    });

    assert_eq!(
        run(&ledger, tx),
        Ter::TEC_PATH_DRY,
        "tfNoRippleDirect with dry explicit path must return tecPATH_DRY (not tecPATH_PARTIAL)"
    );
}

// ── Bug 8: XRP→IOU payment delivers 0 instead of partial (cross-type arithmetic) ─
// Ledger 104111699: c3a2b63a — XRP→GiB payment, SendMax=1482694 XRP, Amount=25.22 GiB,
// DeliverMin=25.02 GiB. Book has offers. We deliver 0 (tecPATH_DRY), C++ delivers
// ~14.83 GiB (tecPATH_PARTIAL, below DeliverMin).
// Root cause: compute_offer_consumption used cross-type multiply(XRP, IOU) which
// overflowed. Fix: use div_round(in_limit, rate, out_asset) like C++ ceilIn.

#[test]
fn xrp_to_iou_payment_delivers_partial_not_path_dry() {
    // Minimal setup: account with XRP, issuer, one offer in the XRP/GiB book.
    // The offer: TakerPays=16366110 XRP, TakerGets=163.66 GiB.
    // Payment: SendMax=1482694 XRP, Amount=25.22 GiB, DeliverMin=25.02 GiB.
    // Expected: we deliver ~14.83 GiB → tesSUCCESS from flow engine →
    //   tecPATH_PARTIAL from payment.rs (14.83 < DeliverMin=25.02).
    use basics::base_uint::Uint160;
    use protocol::{Book, LedgerEntryType, offer_keylet, owner_dir_keylet, quality_keylet};

    let sender = acct(0x10);
    let dst = acct(0x20);
    let issuer = acct(0x30);
    let offer_owner = issuer; // offer_owner IS the issuer → unlimited GiB funds
    let currency = iou_currency(b"GiB");
    let issue = Issue {
        currency,
        account: issuer,
    };

    // Build offer: TakerPays=16366110 XRP, TakerGets=163.66 GiB
    let offer_seq = 1u32;
    let offer_kl = offer_keylet(Uint160::from_slice(offer_owner.data()).unwrap(), offer_seq);
    let book = Book::new(protocol::xrp_issue(), issue, None);
    let book_base = protocol::book_keylet(book);
    let rate = {
        // quality = TakerPays / TakerGets encoded as u64
        // For simplicity use a round rate: 100000 XRP/GiB
        // TakerPays=100000, TakerGets=1 → quality = 100000 * 10^(56-mantissa_bits)
        // Use getRate equivalent: encode as (exponent+100) << 56 | mantissa
        // rate = 100000 = 1e5, mantissa=1000000000000000, exp=-10 → (90 << 56) | 1000000000000000
        let mantissa: u64 = 1_000_000_000_000_000;
        let exp: u64 = 90; // exponent = 90 - 100 = -10, so value = 1e15 * 10^-10 = 1e5
        (exp << 56) | mantissa
    };
    let quality_dir_kl = quality_keylet(book_base, rate);

    let mut offer_sle = STLedgerEntry::from_type_and_key(LedgerEntryType::Offer, offer_kl.key);
    offer_sle.set_account_id(sf("sfAccount"), offer_owner);
    offer_sle.set_field_u32(sf("sfSequence"), offer_seq);
    offer_sle.set_field_amount(
        sf("sfTakerPays"),
        STAmount::from_xrp_amount(XRPAmount::from_drops(16_366_110)),
    );
    offer_sle.set_field_amount(
        sf("sfTakerGets"),
        STAmount::from_iou_amount(sf("sfTakerGets"), iou(163_661_276_470_588, -15), issue),
    );
    offer_sle.set_field_h256(sf("sfBookDirectory"), quality_dir_kl.key);
    offer_sle.set_field_u64(sf("sfOwnerNode"), 0);
    offer_sle.set_field_u64(sf("sfBookNode"), 0);

    // Book directory page with the offer
    let mut book_dir_sle =
        STLedgerEntry::from_type_and_key(LedgerEntryType::DirectoryNode, quality_dir_kl.key);
    book_dir_sle.set_field_h256(sf("sfRootIndex"), book_base.key);
    book_dir_sle.set_field_u64(sf("sfExchangeRate"), rate);
    {
        use protocol::STVector256;
        let indexes = STVector256::from_values(sf("sfIndexes"), vec![offer_kl.key]);
        book_dir_sle.set_field_v256(sf("sfIndexes"), indexes);
    }

    // Owner directory for offer_owner
    let owner_dir_kl = owner_dir_keylet(Uint160::from_slice(offer_owner.data()).unwrap());
    let mut owner_dir_sle =
        STLedgerEntry::from_type_and_key(LedgerEntryType::DirectoryNode, owner_dir_kl.key);
    {
        use protocol::STVector256;
        let indexes = STVector256::from_values(sf("sfIndexes"), vec![offer_kl.key]);
        owner_dir_sle.set_field_v256(sf("sfIndexes"), indexes);
    }

    let ledger = build_ledger(
        104111699,
        vec![
            account_entry(sender, 10_000_000, 0), // 10 XRP
            account_entry(dst, 1_000_000, 0),
            account_entry(issuer, 100_000_000, 1), // issuer == offer_owner
            // Trust line: dst can receive GiB
            trust_line_entry(dst, issuer, currency, iou(0, 0), iou(1_000, 0), iou(0, 0)),
            (offer_kl.key, offer_sle.get_serializer().data().to_vec()),
            (
                quality_dir_kl.key,
                book_dir_sle.get_serializer().data().to_vec(),
            ),
            (
                owner_dir_kl.key,
                owner_dir_sle.get_serializer().data().to_vec(),
            ),
        ],
    );

    // Payment: SendMax=1482694 XRP, Amount=25.22 GiB, DeliverMin=25.02 GiB
    // With the offer at rate ~100000 XRP/GiB, 1482694 XRP buys ~14.83 GiB.
    // 14.83 < DeliverMin=25.02 → tecPATH_PARTIAL (not tecPATH_DRY).
    let tx = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(sf("sfAccount"), sender);
        tx.set_account_id(sf("sfDestination"), dst);
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(sf("sfSequence"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x8002_0000); // tfPartialPayment + high bit
        tx.set_field_vl(sf("sfSigningPubKey"), &[0u8; 33]);
        tx.set_field_amount(
            sf("sfAmount"),
            STAmount::from_iou_amount(sf("sfAmount"), iou(25_225_490_050_000, -12), issue),
        );
        tx.set_field_amount(
            sf("sfSendMax"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(1_482_694)),
        );
        tx.set_field_amount(
            sf("sfDeliverMin"),
            STAmount::from_iou_amount(sf("sfDeliverMin"), iou(25_023_686_129_600, -12), issue),
        );
    });

    let result = run(&ledger, tx);
    assert_ne!(
        result,
        Ter::TEC_PATH_DRY,
        "XRP→IOU payment with partial liquidity must NOT return tecPATH_DRY"
    );
    // Should be tecPATH_PARTIAL (delivered something but below DeliverMin)
    // or tesSUCCESS if we happen to deliver above DeliverMin
    assert!(
        result == Ter::TEC_PATH_PARTIAL || result == Ter::TES_SUCCESS,
        "XRP→IOU payment with partial liquidity must return tecPATH_PARTIAL or tesSUCCESS, got {:?}",
        result
    );
}

// ── Offer amount rounding and cross-type precision tests ──────────────────────
// These test the cross_type_scale fix in compute_offer_consumption for all
// direction combinations: XRP→IOU (limitStepIn), IOU→XRP (limitOut/limitStepIn),
// and IOU→IOU (both branches).

/// Helper: build a ledger with an offer in the book.
/// offer_owner is the issuer of out_issue (unlimited funds).
fn ledger_with_offer(
    seq: u32,
    sender: AccountID,
    dst: AccountID,
    offer_owner: AccountID, // must be issuer of out_issue for unlimited funds
    offer_taker_pays: STAmount, // what offer owner wants (in)
    offer_taker_gets: STAmount, // what offer owner gives (out)
) -> Ledger {
    use basics::base_uint::Uint160;
    use protocol::{Book, LedgerEntryType, offer_keylet, owner_dir_keylet, quality_keylet};

    let offer_seq = 1u32;
    let offer_kl = offer_keylet(Uint160::from_slice(offer_owner.data()).unwrap(), offer_seq);

    // Compute book and quality
    let in_issue = if offer_taker_pays.native() {
        protocol::xrp_issue()
    } else {
        offer_taker_pays.issue()
    };
    let out_issue = if offer_taker_gets.native() {
        protocol::xrp_issue()
    } else {
        offer_taker_gets.clone().issue()
    };
    let book = Book::new(in_issue, out_issue, None);
    let book_base = protocol::book_keylet(book);

    // Encode quality as u64 (exponent << 56 | mantissa)
    let quality = {
        let tp_val = if offer_taker_pays.native() {
            offer_taker_pays.xrp().drops() as f64
        } else {
            offer_taker_pays.mantissa() as f64 * 10f64.powi(offer_taker_pays.exponent())
        };
        let tg_val = if offer_taker_gets.native() {
            offer_taker_gets.xrp().drops() as f64
        } else {
            offer_taker_gets.mantissa() as f64 * 10f64.powi(offer_taker_gets.exponent())
        };
        let rate = tp_val / tg_val;
        let log10 = rate.log10().floor() as i64;
        let mantissa = (rate / 10f64.powi(log10 as i32 - 15)) as u64;
        let exp = (log10 + 100 - 15) as u64;
        (exp << 56) | mantissa
    };
    let quality_dir_kl = quality_keylet(book_base, quality);

    let mut offer_sle = STLedgerEntry::from_type_and_key(LedgerEntryType::Offer, offer_kl.key);
    offer_sle.set_account_id(sf("sfAccount"), offer_owner);
    offer_sle.set_field_u32(sf("sfSequence"), offer_seq);
    offer_sle.set_field_amount(sf("sfTakerPays"), offer_taker_pays);
    offer_sle.set_field_amount(sf("sfTakerGets"), offer_taker_gets.clone());
    offer_sle.set_field_h256(sf("sfBookDirectory"), quality_dir_kl.key);
    offer_sle.set_field_u64(sf("sfOwnerNode"), 0);
    offer_sle.set_field_u64(sf("sfBookNode"), 0);

    let mut book_dir_sle =
        STLedgerEntry::from_type_and_key(LedgerEntryType::DirectoryNode, quality_dir_kl.key);
    book_dir_sle.set_field_h256(sf("sfRootIndex"), book_base.key);
    book_dir_sle.set_field_u64(sf("sfExchangeRate"), quality);
    {
        use protocol::STVector256;
        book_dir_sle.set_field_v256(
            sf("sfIndexes"),
            STVector256::from_values(sf("sfIndexes"), vec![offer_kl.key]),
        );
    }

    let owner_dir_kl = owner_dir_keylet(Uint160::from_slice(offer_owner.data()).unwrap());
    let mut owner_dir_sle =
        STLedgerEntry::from_type_and_key(LedgerEntryType::DirectoryNode, owner_dir_kl.key);
    {
        use protocol::STVector256;
        owner_dir_sle.set_field_v256(
            sf("sfIndexes"),
            STVector256::from_values(sf("sfIndexes"), vec![offer_kl.key]),
        );
    }

    // Trust line for dst to receive out_issue (if IOU)
    let mut entries = vec![
        account_entry(sender, 100_000_000, 0),
        account_entry(dst, 1_000_000, 0),
        account_entry(offer_owner, 100_000_000, 1),
        (offer_kl.key, offer_sle.get_serializer().data().to_vec()),
        (
            quality_dir_kl.key,
            book_dir_sle.get_serializer().data().to_vec(),
        ),
        (
            owner_dir_kl.key,
            owner_dir_sle.get_serializer().data().to_vec(),
        ),
    ];
    if !offer_taker_gets.native() {
        let out_iss = offer_taker_gets.clone().issue();
        entries.push(trust_line_entry(
            dst,
            out_iss.account,
            out_iss.currency,
            iou(0, 0),
            iou(1_000_000, 0),
            iou(0, 0),
        ));
    }

    build_ledger(seq, entries)
}

// ── XRP→IOU: limitStepIn (sender's XRP < offer's TakerPays) ──────────────────
// Real data: c3a2b63a from ledger 104111699
// Offer: TakerPays=16366110 XRP, TakerGets=163.66 GiB
// Payment: SendMax=1482694 XRP, Amount=25.22 GiB, DeliverMin=25.02 GiB
// Expected: delivers ~14.83 GiB → tecPATH_PARTIAL (below DeliverMin)

#[test]
fn xrp_to_iou_limit_step_in_delivers_partial() {
    let sender = acct(0x10);
    let dst = acct(0x20);
    let issuer = acct(0x30); // offer_owner == issuer
    let currency = iou_currency(b"GiB");
    let issue = Issue {
        currency,
        account: issuer,
    };

    let ledger = ledger_with_offer(
        104111699,
        sender,
        dst,
        issuer,
        STAmount::from_xrp_amount(XRPAmount::from_drops(16_366_110)), // TakerPays XRP
        STAmount::from_iou_amount(sf("sfTakerGets"), iou(163_661_276_470_588, -15), issue), // TakerGets GiB
    );

    let tx = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(sf("sfAccount"), sender);
        tx.set_account_id(sf("sfDestination"), dst);
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(sf("sfSequence"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x8002_0000); // tfPartialPayment
        tx.set_field_vl(sf("sfSigningPubKey"), &[0u8; 33]);
        tx.set_field_amount(
            sf("sfAmount"),
            STAmount::from_iou_amount(sf("sfAmount"), iou(25_225_490_050_000, -12), issue),
        );
        tx.set_field_amount(
            sf("sfSendMax"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(1_482_694)),
        );
        tx.set_field_amount(
            sf("sfDeliverMin"),
            STAmount::from_iou_amount(sf("sfDeliverMin"), iou(25_023_686_129_600, -12), issue),
        );
    });

    let result = run(&ledger, tx);
    // We deliver ~14.83 GiB which is below DeliverMin=25.02 → tecPATH_PARTIAL
    assert_ne!(
        result,
        Ter::TEC_PATH_DRY,
        "XRP→IOU limitStepIn must NOT return tecPATH_DRY (must deliver something)"
    );
    assert!(
        result == Ter::TEC_PATH_PARTIAL || result == Ter::TES_SUCCESS,
        "XRP→IOU limitStepIn must return tecPATH_PARTIAL or tesSUCCESS, got {:?}",
        result
    );
}

// ── IOU→XRP: limitStepIn (sender's IOU < offer's TakerGets) ──────────────────
// Real data: c6e6aed6 from ledger 104111701
// Offer in book: TakerPays=IOU (what owner wants), TakerGets=XRP (what owner gives)
// Payment: SendMax=12.52 IOU, Amount=4937 XRP, DeliverMin=4936 XRP
// Expected: delivers some XRP → tecPATH_PARTIAL (below Amount=4937 but above DeliverMin=4936)

#[test]
fn iou_to_xrp_limit_step_in_delivers_partial() {
    let sender = acct(0x10);
    let dst = acct(0x20);
    let issuer = acct(0x30);
    let currency = iou_currency(b"589"); // 353839... currency
    let issue = Issue {
        currency,
        account: issuer,
    };

    // Offer: TakerPays=IOU (owner wants IOU), TakerGets=XRP (owner gives XRP)
    // Rate: ~40 XRP per IOU (4937 XRP / 12.52 IOU ≈ 394 XRP/IOU)
    // With SendMax=12.52 IOU, we can get ~4937 XRP
    let _ledger = ledger_with_offer(
        104111701,
        sender,
        dst,
        issuer,
        // TakerPays = IOU (what offer owner wants to receive)
        STAmount::from_iou_amount(sf("sfTakerPays"), iou(1_252_559_499_500_057, -14), issue),
        // TakerGets = XRP (what offer owner gives)
        STAmount::from_xrp_amount(XRPAmount::from_drops(4_937)),
    );

    // Give sender IOU balance via trust line
    // sender needs to hold IOU to send it
    // Since issuer == offer_owner, sender needs a trust line to issuer
    // Build a custom ledger with trust line for sender
    use protocol::{Book, LedgerEntryType, offer_keylet, owner_dir_keylet, quality_keylet};

    let offer_seq = 1u32;
    let offer_kl = offer_keylet(Uint160::from_slice(issuer.data()).unwrap(), offer_seq);
    let in_issue = issue; // IOU
    let out_issue = protocol::xrp_issue(); // XRP
    let book = Book::new(in_issue, out_issue, None);
    let book_base = protocol::book_keylet(book);
    let quality = {
        let tp_val = 1_252_559_499_500_057f64 * 10f64.powi(-14);
        let tg_val = 4937f64;
        let rate = tp_val / tg_val;
        let log10 = rate.log10().floor() as i64;
        let mantissa = (rate / 10f64.powi(log10 as i32 - 15)) as u64;
        let exp = (log10 + 100 - 15) as u64;
        (exp << 56) | mantissa
    };
    let quality_dir_kl = quality_keylet(book_base, quality);

    let mut offer_sle = STLedgerEntry::from_type_and_key(LedgerEntryType::Offer, offer_kl.key);
    offer_sle.set_account_id(sf("sfAccount"), issuer);
    offer_sle.set_field_u32(sf("sfSequence"), offer_seq);
    offer_sle.set_field_amount(
        sf("sfTakerPays"),
        STAmount::from_iou_amount(sf("sfTakerPays"), iou(1_252_559_499_500_057, -14), issue),
    );
    offer_sle.set_field_amount(
        sf("sfTakerGets"),
        STAmount::from_xrp_amount(XRPAmount::from_drops(4_937)),
    );
    offer_sle.set_field_h256(sf("sfBookDirectory"), quality_dir_kl.key);
    offer_sle.set_field_u64(sf("sfOwnerNode"), 0);
    offer_sle.set_field_u64(sf("sfBookNode"), 0);

    let mut book_dir_sle =
        STLedgerEntry::from_type_and_key(LedgerEntryType::DirectoryNode, quality_dir_kl.key);
    book_dir_sle.set_field_h256(sf("sfRootIndex"), book_base.key);
    book_dir_sle.set_field_u64(sf("sfExchangeRate"), quality);
    {
        book_dir_sle.set_field_v256(
            sf("sfIndexes"),
            protocol::STVector256::from_values(sf("sfIndexes"), vec![offer_kl.key]),
        );
    }
    let owner_dir_kl = owner_dir_keylet(Uint160::from_slice(issuer.data()).unwrap());
    let mut owner_dir_sle =
        STLedgerEntry::from_type_and_key(LedgerEntryType::DirectoryNode, owner_dir_kl.key);
    {
        owner_dir_sle.set_field_v256(
            sf("sfIndexes"),
            protocol::STVector256::from_values(sf("sfIndexes"), vec![offer_kl.key]),
        );
    }

    // sender holds IOU (trust line: sender < issuer lexicographically? acct(0x10) < acct(0x30))
    // low=sender(0x10), high=issuer(0x30). Balance positive = low holds IOU.
    let ledger2 = build_ledger(
        104111701,
        vec![
            account_entry(sender, 1_000_000, 0),
            account_entry(dst, 100_000_000, 0),
            account_entry(issuer, 100_000_000, 1),
            // sender holds 20 IOU (positive balance from low=sender perspective)
            trust_line_entry(
                sender,
                issuer,
                currency,
                iou(2_000_000_000_000_000, -14),
                iou(1_000, 0),
                iou(0, 0),
            ),
            (offer_kl.key, offer_sle.get_serializer().data().to_vec()),
            (
                quality_dir_kl.key,
                book_dir_sle.get_serializer().data().to_vec(),
            ),
            (
                owner_dir_kl.key,
                owner_dir_sle.get_serializer().data().to_vec(),
            ),
        ],
    );

    let tx = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(sf("sfAccount"), sender);
        tx.set_account_id(sf("sfDestination"), dst);
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(sf("sfSequence"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x0002_0000); // tfPartialPayment
        tx.set_field_vl(sf("sfSigningPubKey"), &[0u8; 33]);
        tx.set_field_amount(
            sf("sfAmount"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(4_937)),
        );
        tx.set_field_amount(
            sf("sfSendMax"),
            STAmount::from_iou_amount(sf("sfSendMax"), iou(1_252_559_499_500_057, -14), issue),
        );
        tx.set_field_amount(
            sf("sfDeliverMin"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(4_936)),
        );
    });

    let result = run(&ledger2, tx);
    assert_ne!(
        result,
        Ter::TEC_PATH_DRY,
        "IOU→XRP limitStepIn must NOT return tecPATH_DRY"
    );
    assert!(
        result == Ter::TEC_PATH_PARTIAL || result == Ter::TES_SUCCESS,
        "IOU→XRP limitStepIn must return tecPATH_PARTIAL or tesSUCCESS, got {:?}",
        result
    );
}

// ── OfferCreate partial crossing: XRP→IOU limitStepIn ────────────────────────
// Real data: 10ebe5fa from ledger 104111699
// Taker: TakerPays=476.19 XWLF IOU, TakerGets=19999999 XRP
// Existing offer: TakerPays=2649775829 XRP, TakerGets=88325.86 XWLF
// Taker gives 476.19 XWLF, gets 14285715 XRP (limited by taker's input)
// Expected: tesSUCCESS, offer partially consumed

#[test]
fn offer_create_partial_crossing_xrp_iou_limit_step_in() {
    let taker = acct(0x10);
    let offer_owner = acct(0x20); // issuer of XWLF
    let currency = iou_currency(b"XWL");
    let issue = Issue {
        currency,
        account: offer_owner,
    };

    // Existing offer: TakerPays=2649775829 XRP, TakerGets=88325.86 XWLF
    // (offer owner wants XRP, gives XWLF)
    let _ledger = ledger_with_offer(
        104111699,
        taker,
        taker,
        offer_owner,
        STAmount::from_xrp_amount(XRPAmount::from_drops(2_649_775_829)), // TakerPays XRP
        STAmount::from_iou_amount(sf("sfTakerGets"), iou(8_832_586_096_872_838, -11), issue), // TakerGets XWLF
    );

    // Taker creates offer: TakerPays=476.19 XWLF, TakerGets=19999999 XRP
    // Taker needs XWLF balance — taker IS the issuer? No, offer_owner is issuer.
    // Give taker a trust line with XWLF balance.
    // Since taker(0x10) < offer_owner(0x20): low=taker, high=offer_owner
    // Positive balance = taker holds XWLF
    use basics::base_uint::Uint160;
    use protocol::{Book, LedgerEntryType, offer_keylet, owner_dir_keylet, quality_keylet};

    let existing_offer_seq = 1u32;
    let existing_offer_kl = offer_keylet(
        Uint160::from_slice(offer_owner.data()).unwrap(),
        existing_offer_seq,
    );
    let in_issue = protocol::xrp_issue();
    let out_issue = issue;
    let book = Book::new(in_issue, out_issue, None);
    let book_base = protocol::book_keylet(book);
    let quality = {
        let tp_val = 2_649_775_829f64;
        let tg_val = 8_832_586_096_872_838f64 * 10f64.powi(-11);
        let rate = tp_val / tg_val;
        let log10 = rate.log10().floor() as i64;
        let mantissa = (rate / 10f64.powi(log10 as i32 - 15)) as u64;
        let exp = (log10 + 100 - 15) as u64;
        (exp << 56) | mantissa
    };
    let quality_dir_kl = quality_keylet(book_base, quality);

    let mut offer_sle =
        STLedgerEntry::from_type_and_key(LedgerEntryType::Offer, existing_offer_kl.key);
    offer_sle.set_account_id(sf("sfAccount"), offer_owner);
    offer_sle.set_field_u32(sf("sfSequence"), existing_offer_seq);
    offer_sle.set_field_amount(
        sf("sfTakerPays"),
        STAmount::from_xrp_amount(XRPAmount::from_drops(2_649_775_829)),
    );
    offer_sle.set_field_amount(
        sf("sfTakerGets"),
        STAmount::from_iou_amount(sf("sfTakerGets"), iou(8_832_586_096_872_838, -11), issue),
    );
    offer_sle.set_field_h256(sf("sfBookDirectory"), quality_dir_kl.key);
    offer_sle.set_field_u64(sf("sfOwnerNode"), 0);
    offer_sle.set_field_u64(sf("sfBookNode"), 0);

    let mut book_dir_sle =
        STLedgerEntry::from_type_and_key(LedgerEntryType::DirectoryNode, quality_dir_kl.key);
    book_dir_sle.set_field_h256(sf("sfRootIndex"), book_base.key);
    book_dir_sle.set_field_u64(sf("sfExchangeRate"), quality);
    {
        book_dir_sle.set_field_v256(
            sf("sfIndexes"),
            protocol::STVector256::from_values(sf("sfIndexes"), vec![existing_offer_kl.key]),
        );
    }
    let owner_dir_kl = owner_dir_keylet(Uint160::from_slice(offer_owner.data()).unwrap());
    let mut owner_dir_sle =
        STLedgerEntry::from_type_and_key(LedgerEntryType::DirectoryNode, owner_dir_kl.key);
    {
        owner_dir_sle.set_field_v256(
            sf("sfIndexes"),
            protocol::STVector256::from_values(sf("sfIndexes"), vec![existing_offer_kl.key]),
        );
    }

    let ledger2 = build_ledger(
        104111699,
        vec![
            account_entry(taker, 100_000_000, 0),
            account_entry(offer_owner, 100_000_000, 1),
            // taker holds 1000 XWLF (low=taker(0x10) < high=offer_owner(0x20))
            trust_line_entry(
                taker,
                offer_owner,
                currency,
                iou(1_000_000_000_000_000, -12),
                iou(10_000, 0),
                iou(0, 0),
            ),
            (
                existing_offer_kl.key,
                offer_sle.get_serializer().data().to_vec(),
            ),
            (
                quality_dir_kl.key,
                book_dir_sle.get_serializer().data().to_vec(),
            ),
            (
                owner_dir_kl.key,
                owner_dir_sle.get_serializer().data().to_vec(),
            ),
        ],
    );

    // Taker creates offer: TakerPays=476.19 XWLF, TakerGets=19999999 XRP
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), taker);
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(sf("sfSequence"), 1);
        tx.set_field_u32(sf("sfFlags"), 0);
        tx.set_field_vl(sf("sfSigningPubKey"), &[0u8; 33]);
        tx.set_field_amount(
            sf("sfTakerPays"),
            STAmount::from_iou_amount(sf("sfTakerPays"), iou(4_761_904_760_000_000, -16), issue),
        );
        tx.set_field_amount(
            sf("sfTakerGets"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(19_999_999)),
        );
    });

    let result = run(&ledger2, tx);
    assert_eq!(
        result,
        Ter::TES_SUCCESS,
        "OfferCreate partial crossing XRP→IOU must return tesSUCCESS, got {:?}",
        result
    );
}

// ── Quality threshold: OfferCreate must not cross offers below taker's quality ─
// Real data: 863ab720 from ledger 104112734
// Taker: TakerPays=2684.37 USD, TakerGets=39.09 DSH
// Book best offer: TakerPays=1903.94 USD, TakerGets=27.71 DSH (quality 68.72 USD/DSH)
// Taker quality: 2684.37/39.09 = 68.67 USD/DSH < 68.72 → no crossing, offer placed
// C++ returns tesSUCCESS (offer placed). We were returning tecPATH_PARTIAL.

#[test]
fn offer_create_no_crossing_when_book_quality_worse_than_taker() {
    // Taker offers USD, wants DSH. Book offer has worse quality (asks more USD per DSH).
    // Taker should NOT cross the book offer — just place their own offer.
    let taker = acct(0x10);
    let usd_issuer = acct(0x20); // USD issuer
    let dsh_issuer = acct(0x30); // DSH issuer (== offer_owner for unlimited funds)
    let usd_currency = iou_currency(b"USD");
    let dsh_currency = iou_currency(b"DSH");
    let usd_issue = Issue {
        currency: usd_currency,
        account: usd_issuer,
    };
    let dsh_issue = Issue {
        currency: dsh_currency,
        account: dsh_issuer,
    };

    // Book offer: TakerPays=1903.94 USD (what owner wants), TakerGets=27.71 DSH (what owner gives)
    // Quality = 27.71/1903.94 = 0.01455 DSH/USD
    // Taker quality = 39.09/2684.37 = 0.01456 DSH/USD
    // Since taker quality (0.01456) > book quality (0.01455), taker is offering MORE DSH per USD
    // Wait — let me recalculate. In XRPL book_offers(taker_pays=USD, taker_gets=DSH):
    // The offer owner TakerPays=USD (wants USD), TakerGets=DSH (gives DSH).
    // For crossing: offer gives DSH, taker wants DSH. Offer quality = DSH_given/USD_wanted = 27.71/1903.94.
    // Taker quality = DSH_wanted/USD_given = 39.09/2684.37.
    // 27.71/1903.94 = 0.01455, 39.09/2684.37 = 0.01456.
    // Taker wants 0.01456 DSH/USD but offer only gives 0.01455 DSH/USD → worse → no crossing.

    // Build ledger with the book offer
    let _ledger = ledger_with_offer(
        104112734,
        taker,
        taker,
        dsh_issuer,
        // Existing offer: TakerPays=1903.94 USD (owner wants), TakerGets=27.71 DSH (owner gives)
        STAmount::from_iou_amount(
            sf("sfTakerPays"),
            iou(1_903_940_848_168_491, -12),
            usd_issue,
        ),
        STAmount::from_iou_amount(
            sf("sfTakerGets"),
            iou(2_770_764_676_252_658, -14),
            dsh_issue,
        ),
    );

    // Give taker USD balance (taker(0x10) < usd_issuer(0x20): low=taker, high=usd_issuer)
    // Positive balance = taker holds USD
    use basics::base_uint::Uint160;
    use protocol::{Book, LedgerEntryType, offer_keylet, owner_dir_keylet, quality_keylet};

    let existing_offer_seq = 1u32;
    let existing_offer_kl = offer_keylet(
        Uint160::from_slice(dsh_issuer.data()).unwrap(),
        existing_offer_seq,
    );
    let in_issue = usd_issue;
    let out_issue = dsh_issue;
    let book = Book::new(in_issue, out_issue, None);
    let book_base = protocol::book_keylet(book);
    let quality = {
        let tp_val = 1_903_940_848_168_491f64 * 10f64.powi(-12);
        let tg_val = 2_770_764_676_252_658f64 * 10f64.powi(-14);
        let rate = tp_val / tg_val;
        let log10 = rate.log10().floor() as i64;
        let mantissa = (rate / 10f64.powi(log10 as i32 - 15)) as u64;
        let exp = (log10 + 100 - 15) as u64;
        (exp << 56) | mantissa
    };
    let quality_dir_kl = quality_keylet(book_base, quality);

    let mut offer_sle =
        STLedgerEntry::from_type_and_key(LedgerEntryType::Offer, existing_offer_kl.key);
    offer_sle.set_account_id(sf("sfAccount"), dsh_issuer);
    offer_sle.set_field_u32(sf("sfSequence"), existing_offer_seq);
    offer_sle.set_field_amount(
        sf("sfTakerPays"),
        STAmount::from_iou_amount(
            sf("sfTakerPays"),
            iou(1_903_940_848_168_491, -12),
            usd_issue,
        ),
    );
    offer_sle.set_field_amount(
        sf("sfTakerGets"),
        STAmount::from_iou_amount(
            sf("sfTakerGets"),
            iou(2_770_764_676_252_658, -14),
            dsh_issue,
        ),
    );
    offer_sle.set_field_h256(sf("sfBookDirectory"), quality_dir_kl.key);
    offer_sle.set_field_u64(sf("sfOwnerNode"), 0);
    offer_sle.set_field_u64(sf("sfBookNode"), 0);

    let mut book_dir_sle =
        STLedgerEntry::from_type_and_key(LedgerEntryType::DirectoryNode, quality_dir_kl.key);
    book_dir_sle.set_field_h256(sf("sfRootIndex"), book_base.key);
    book_dir_sle.set_field_u64(sf("sfExchangeRate"), quality);
    {
        book_dir_sle.set_field_v256(
            sf("sfIndexes"),
            protocol::STVector256::from_values(sf("sfIndexes"), vec![existing_offer_kl.key]),
        );
    }
    let owner_dir_kl = owner_dir_keylet(Uint160::from_slice(dsh_issuer.data()).unwrap());
    let mut owner_dir_sle =
        STLedgerEntry::from_type_and_key(LedgerEntryType::DirectoryNode, owner_dir_kl.key);
    {
        owner_dir_sle.set_field_v256(
            sf("sfIndexes"),
            protocol::STVector256::from_values(sf("sfIndexes"), vec![existing_offer_kl.key]),
        );
    }

    let ledger2 = build_ledger(
        104112734,
        vec![
            account_entry(taker, 100_000_000, 0),
            account_entry(usd_issuer, 100_000_000, 0),
            account_entry(dsh_issuer, 100_000_000, 1),
            // taker holds USD (low=taker(0x10) < high=usd_issuer(0x20))
            trust_line_entry(
                taker,
                usd_issuer,
                usd_currency,
                iou(5_000_000_000_000_000, -12),
                iou(100_000, 0),
                iou(0, 0),
            ),
            // taker holds DSH (low=taker(0x10) < high=dsh_issuer(0x30)) — needed for TakerGets
            trust_line_entry(
                taker,
                dsh_issuer,
                dsh_currency,
                iou(1_000_000_000_000_000, -12),
                iou(100_000, 0),
                iou(0, 0),
            ),
            (
                existing_offer_kl.key,
                offer_sle.get_serializer().data().to_vec(),
            ),
            (
                quality_dir_kl.key,
                book_dir_sle.get_serializer().data().to_vec(),
            ),
            (
                owner_dir_kl.key,
                owner_dir_sle.get_serializer().data().to_vec(),
            ),
        ],
    );

    // Taker creates offer: TakerPays=2684.37 USD, TakerGets=39.09 DSH
    // Taker's quality (39.09/2684.37 = 0.01456) > book quality (27.71/1903.94 = 0.01455)
    // → book offer is WORSE than taker's threshold → no crossing → offer placed → tesSUCCESS
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), taker);
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(sf("sfSequence"), 1);
        tx.set_field_u32(sf("sfFlags"), 0);
        tx.set_field_vl(sf("sfSigningPubKey"), &[0u8; 33]);
        tx.set_field_amount(
            sf("sfTakerPays"),
            STAmount::from_iou_amount(
                sf("sfTakerPays"),
                iou(2_684_369_568_329_343, -12),
                usd_issue,
            ),
        );
        tx.set_field_amount(
            sf("sfTakerGets"),
            STAmount::from_iou_amount(
                sf("sfTakerGets"),
                iou(3_908_774_478_791_216, -14),
                dsh_issue,
            ),
        );
    });

    assert_eq!(
        run(&ledger2, tx),
        Ter::TES_SUCCESS,
        "OfferCreate with book quality worse than taker must return tesSUCCESS (offer placed, no crossing)"
    );
}

// ── AMM swap: XRP→IOU via AMM pool ───────────────────────────────────────────
// Real data: fcc1b953 from ledger 104113590
// AMM XRP/ARMA: pool_in=51096941136 XRP, pool_out=88592348.49 ARMA
// Payment: SendMax=110619 XRP, Amount=205.97 ARMA, delivered ~190 ARMA
// C++ uses AMM (no CLOB offers consumed). We must deliver via AMM.

#[test]
fn xrp_to_iou_via_amm_delivers_partial() {
    use protocol::{LedgerEntryType, amm as amm_keylet_fn};

    let sender = acct(0x10); // = ARMA issuer (r319FqohpKLwjtcV2mosyC5sy125fDk4uH)
    let dst = acct(0x20);
    let amm_account = acct(0x30);
    let arma_issuer = sender; // sender IS the issuer
    let currency = iou_currency(b"ARM");
    let issue = Issue {
        currency,
        account: arma_issuer,
    };

    // Build AMM SLE
    let amm_kl = amm_keylet_fn(
        protocol::Asset::Issue(protocol::xrp_issue()),
        protocol::Asset::Issue(issue),
    );
    let mut amm_sle = STLedgerEntry::from_type_and_key(LedgerEntryType::AMM, amm_kl.key);
    amm_sle.set_account_id(sf("sfAccount"), amm_account);
    amm_sle.set_field_u16(sf("sfTradingFee"), 0); // 0 fee for simplicity
    // Set AMM assets
    {
        use protocol::{STIssue, xrp_issue};
        amm_sle.set_field_issue(
            sf("sfAsset"),
            STIssue::new_with_asset(sf("sfAsset"), xrp_issue()),
        );
        amm_sle.set_field_issue(
            sf("sfAsset2"),
            STIssue::new_with_asset(sf("sfAsset2"), issue),
        );
    }

    // AMM account holds XRP (pool_in) and ARMA (pool_out)
    // pool_in = 51,096,941,136 XRP drops, pool_out = 88,592,348 ARMA
    let pool_xrp: i64 = 51_096_941_136;
    let _pool_arma = iou(88_592_348_000_000_000, -9); // 88592348 ARMA

    // Trust line: AMM holds ARMA (low=amm(0x30) < high=arma_issuer(0x10)? No, 0x30 > 0x10)
    // low=arma_issuer(0x10), high=amm_account(0x30)
    // Positive balance = low (arma_issuer) holds ARMA → AMM owes ARMA to issuer
    // We want AMM to hold ARMA: negative balance from low's perspective
    // Actually: AMM holds ARMA means AMM has positive balance from AMM's perspective
    // Since AMM(0x30) > issuer(0x10): AMM is high. Balance positive = low holds.
    // For AMM to hold ARMA: balance should be negative (high holds).
    let amm_arma_balance = iou(-88_592_348_000_000_000, -9); // negative = high(AMM) holds

    let ledger = build_ledger(
        104113590,
        vec![
            account_entry(sender, 100_000_000, 0),
            account_entry(dst, 1_000_000, 0),
            account_entry(amm_account, pool_xrp, 0), // AMM holds XRP
            // Trust line: arma_issuer(low=0x10) ↔ amm_account(high=0x30)
            trust_line_entry(
                arma_issuer,
                amm_account,
                currency,
                amm_arma_balance,
                iou(0, 0),
                iou(1_000_000_000, 0),
            ),
            // dst trust line to receive ARMA
            trust_line_entry(
                dst,
                arma_issuer,
                currency,
                iou(0, 0),
                iou(1_000_000, 0),
                iou(0, 0),
            ),
            (amm_kl.key, amm_sle.get_serializer().data().to_vec()),
        ],
    );

    // Payment: SendMax=110619 XRP, Amount=205.97 ARMA (partial payment)
    let tx = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(sf("sfAccount"), sender);
        tx.set_account_id(sf("sfDestination"), dst);
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(sf("sfSequence"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x0002_0000); // tfPartialPayment
        tx.set_field_vl(sf("sfSigningPubKey"), &[0u8; 33]);
        tx.set_field_amount(
            sf("sfAmount"),
            STAmount::from_iou_amount(sf("sfAmount"), iou(205_967_277_000_000, -9), issue),
        );
        tx.set_field_amount(
            sf("sfSendMax"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(110_619)),
        );
    });

    let result = run(&ledger, tx);
    // With AMM: should deliver ~190 ARMA → tesSUCCESS or tecPATH_PARTIAL
    // Without AMM: tecPATH_DRY (no CLOB offers)
    assert_ne!(
        result,
        Ter::TEC_PATH_DRY,
        "XRP→IOU via AMM must NOT return tecPATH_DRY (AMM should provide liquidity)"
    );
    assert!(
        result == Ter::TES_SUCCESS || result == Ter::TEC_PATH_PARTIAL,
        "XRP→IOU via AMM must return tesSUCCESS or tecPATH_PARTIAL, got {:?}",
        result
    );
}

#[test]
fn offer_create_reuses_funds_freed_by_cancelled_offer() {
    use protocol::{LedgerEntryType, offer_keylet};

    let account = acct(0x55);
    let issuer = acct(0x66);
    let currency = iou_currency(b"USD");
    let issue = Issue {
        currency,
        account: issuer,
    };

    let existing_offer_kl = offer_keylet(raw_id(account), 1);
    let mut existing_offer =
        STLedgerEntry::from_type_and_key(LedgerEntryType::Offer, existing_offer_kl.key);
    existing_offer.set_account_id(sf("sfAccount"), account);
    existing_offer.set_field_u32(sf("sfSequence"), 1);
    existing_offer.set_field_amount(
        sf("sfTakerPays"),
        STAmount::from_xrp_amount(XRPAmount::from_drops(1_000)),
    );
    existing_offer.set_field_amount(
        sf("sfTakerGets"),
        STAmount::from_iou_amount(sf("sfTakerGets"), iou(50_000_000, -6), issue),
    );
    existing_offer.set_field_u64(sf("sfOwnerNode"), 0);
    existing_offer.set_field_u64(sf("sfBookNode"), 0);

    // Owner directory for account (contains the existing offer)
    let owner_dir_kl = protocol::owner_dir_keylet(raw_id(account));
    let mut owner_dir_sle =
        STLedgerEntry::from_type_and_key(LedgerEntryType::DirectoryNode, owner_dir_kl.key);
    {
        use protocol::STVector256;
        let indexes = STVector256::from_values(sf("sfIndexes"), vec![existing_offer_kl.key]);
        owner_dir_sle.set_field_v256(sf("sfIndexes"), indexes);
    }

    // Trust line so account can hold the IOU
    let tl = trust_line_entry(
        account,
        issuer,
        currency,
        iou(50_000_000, -6),    // balance = 50 (from existing offer's TakerGets)
        iou(1_000_000_000, -6), // limit
        iou(0, 0),
    );

    let ledger = build_ledger(
        104117200,
        vec![
            account_entry(account, 100_000_000, 1),
            account_entry(issuer, 100_000_000, 0),
            (
                existing_offer_kl.key,
                existing_offer.get_serializer().data().to_vec(),
            ),
            (
                owner_dir_kl.key,
                owner_dir_sle.get_serializer().data().to_vec(),
            ),
            tl,
        ],
    );

    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), account);
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(sf("sfSequence"), 2);
        tx.set_field_u32(sf("sfOfferSequence"), 1);
        tx.set_field_vl(sf("sfSigningPubKey"), &[0u8; 33]);
        tx.set_field_amount(
            sf("sfTakerPays"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(2_000)),
        );
        tx.set_field_amount(
            sf("sfTakerGets"),
            STAmount::from_iou_amount(sf("sfTakerGets"), iou(50_000_000, -6), issue),
        );
    });

    assert_eq!(
        run(&ledger, tx),
        Ter::TES_SUCCESS,
        "OfferCreate should reuse TakerGets freed by the cancelled offer"
    );
}

#[test]
fn offer_create_fill_or_kill_passive_returns_tec_killed() {
    let account = acct(0x57);
    let issuer = acct(0x67);
    let currency = iou_currency(b"USD");
    let issue = Issue {
        currency,
        account: issuer,
    };
    let ledger = build_ledger(
        104117201,
        vec![
            account_entry(account, 100_000_000, 0),
            account_entry(issuer, 100_000_000, 0),
        ],
    );

    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), account);
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(sf("sfSequence"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x0005_0000);
        tx.set_field_vl(sf("sfSigningPubKey"), &[0u8; 33]);
        tx.set_field_amount(
            sf("sfTakerPays"),
            STAmount::from_iou_amount(sf("sfTakerPays"), iou(10_000_000, -6), issue),
        );
        tx.set_field_amount(
            sf("sfTakerGets"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(1_000)),
        );
    });

    assert_eq!(run(&ledger, tx), Ter::TEC_KILLED);
}

#[test]
fn offer_create_ioc_passive_returns_tec_killed() {
    let account = acct(0x58);
    let issuer = acct(0x68);
    let currency = iou_currency(b"USD");
    let issue = Issue {
        currency,
        account: issuer,
    };
    let ledger = build_ledger(
        104117202,
        vec![
            account_entry(account, 100_000_000, 0),
            account_entry(issuer, 100_000_000, 0),
        ],
    );

    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), account);
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(sf("sfSequence"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x0003_0000);
        tx.set_field_vl(sf("sfSigningPubKey"), &[0u8; 33]);
        tx.set_field_amount(
            sf("sfTakerPays"),
            STAmount::from_iou_amount(sf("sfTakerPays"), iou(10_000_000, -6), issue),
        );
        tx.set_field_amount(
            sf("sfTakerGets"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(1_000)),
        );
    });

    assert_eq!(run(&ledger, tx), Ter::TEC_KILLED);
}

#[test]
fn direct_iou_payment_without_partial_returns_tec_path_partial_when_sender_lacks_balance() {
    let sender = acct(0x70);
    let dst = acct(0x71);
    let issuer = acct(0x72);
    let currency = iou_currency(b"USD");
    let issue = Issue {
        currency,
        account: issuer,
    };
    let ledger = build_ledger(
        104117203,
        vec![
            account_entry(sender, 100_000_000, 0),
            account_entry(dst, 100_000_000, 0),
            account_entry(issuer, 100_000_000, 0),
            trust_line_entry(
                sender,
                issuer,
                currency,
                iou(50_000_000, -6),
                iou(100_000, 0),
                iou(0, 0),
            ),
            trust_line_entry(dst, issuer, currency, iou(0, 0), iou(100_000, 0), iou(0, 0)),
        ],
    );

    let tx = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(sf("sfAccount"), sender);
        tx.set_account_id(sf("sfDestination"), dst);
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(sf("sfSequence"), 1);
        tx.set_field_vl(sf("sfSigningPubKey"), &[0u8; 33]);
        let requested = STAmount::from_iou_amount(sf("sfAmount"), iou(100_000_000, -6), issue);
        tx.set_field_amount(sf("sfAmount"), requested.clone());
        tx.set_field_amount(sf("sfSendMax"), requested);
    });

    assert_eq!(run(&ledger, tx), Ter::TEC_PATH_PARTIAL);
}

#[test]
fn direct_iou_payment_with_partial_delivers_available_balance() {
    let sender = acct(0x73);
    let dst = acct(0x74);
    let issuer = acct(0x75);
    let currency = iou_currency(b"USD");
    let issue = Issue {
        currency,
        account: issuer,
    };
    let ledger = build_ledger(
        104117204,
        vec![
            account_entry(sender, 100_000_000, 0),
            account_entry(dst, 100_000_000, 0),
            account_entry(issuer, 100_000_000, 0),
            trust_line_entry(
                sender,
                issuer,
                currency,
                iou(50_000_000, -6),
                iou(100_000, 0),
                iou(0, 0),
            ),
            trust_line_entry(dst, issuer, currency, iou(0, 0), iou(100_000, 0), iou(0, 0)),
        ],
    );

    let tx = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(sf("sfAccount"), sender);
        tx.set_account_id(sf("sfDestination"), dst);
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(sf("sfSequence"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x0002_0000);
        tx.set_field_vl(sf("sfSigningPubKey"), &[0u8; 33]);
        let requested = STAmount::from_iou_amount(sf("sfAmount"), iou(100_000_000, -6), issue);
        tx.set_field_amount(sf("sfAmount"), requested.clone());
        tx.set_field_amount(sf("sfSendMax"), requested);
    });

    assert_eq!(run(&ledger, tx), Ter::TES_SUCCESS);
}

#[test]
fn nftoken_accept_offer_moves_token_from_successor_page() {
    // C++ nft::locatePage uses view.succ(first, max.next()) and then reads the
    // successor page.  A single-page NFT directory is keyed at the owner's max
    // page, so direct keylet(owner_min, token_id) lookup misses this valid page.
    let buyer = acct(0x81);
    let seller = acct(0x82);
    let nft_id =
        Uint256::from_hex("0008000082828282828282828282828282828282828282820000000000000001")
            .expect("nft id");
    let seller_page_key = protocol::nft_page_max_keylet(raw_id(seller)).key;
    let buyer_page_key = protocol::nft_page_max_keylet(raw_id(buyer)).key;
    let sell_offer_sequence = 7;
    let (sell_offer_key, sell_offer_payload) = nft_sell_offer_entry(
        seller,
        sell_offer_sequence,
        nft_id,
        STAmount::from_xrp_amount(XRPAmount::new()),
    );

    let mut ledger = build_ledger(
        17261854,
        vec![
            account_entry(buyer, 100_000_000, 0),
            account_entry(seller, 100_000_000, 2),
            (sell_offer_key, sell_offer_payload),
            nft_page_entry(seller_page_key, &[nft_id]),
        ],
    );

    let tx = STTx::new(TxType::NFTOKEN_ACCEPT_OFFER, |tx| {
        tx.set_account_id(sf("sfAccount"), buyer);
        tx.set_field_amount(
            sf("sfFee"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(10)),
        );
        tx.set_field_u32(sf("sfSequence"), 1);
        tx.set_field_u32(sf("sfFlags"), 0);
        tx.set_field_vl(sf("sfSigningPubKey"), &[0u8; 33]);
        tx.set_field_h256(sf("sfNFTokenSellOffer"), sell_offer_key);
    });

    let base = Arc::new(ledger.clone());
    let mut view = Sandbox::new(base, ApplyFlags::default());
    let result = apply_submit_transactor_shell(&mut view, &tx, TxType::NFTOKEN_ACCEPT_OFFER);
    assert_eq!(result, Ter::TES_SUCCESS);
    view.apply(&mut ledger).expect("apply sandbox");

    assert!(
        ledger
            .read(protocol::nft_page_keylet(
                protocol::nft_page_min_keylet(raw_id(seller)),
                nft_id
            ))
            .expect("read direct seller page")
            .is_none(),
        "direct seller NFT page key should not have been used"
    );
    assert!(
        ledger
            .read(protocol::nft_page_max_keylet(raw_id(seller)))
            .expect("read seller max page")
            .is_none(),
        "seller's single successor page should be erased after transfer"
    );

    let buyer_page = ledger
        .read(protocol::nft_page_max_keylet(raw_id(buyer)))
        .expect("read buyer max page")
        .expect("buyer successor page should be created");
    let tokens = buyer_page.get_field_array(sf("sfNFTokens"));
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens
            .get(0)
            .expect("buyer token")
            .get_field_h256(sf("sfNFTokenID")),
        nft_id
    );
    assert_eq!(*buyer_page.key(), buyer_page_key);
}
