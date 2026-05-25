//! Regression tests for transactor bugs from ledgers 104111032-104111035.
//!
//! Bug A: self-payment (Account==Destination) must return tecPATH_DRY
//! Bug B: OfferCreate with zero TakerGets balance must return tecUNFUNDED_OFFER
//! Bug C: ImmediateOrCancel offer not fully filled must return tecKILLED

use app::apply_submit_transactor_shell;
use basics::base_uint::{Uint160, Uint256};
use ledger::{Ledger, LedgerHeader, Sandbox};
use protocol::{
    AccountID, ApplyFlags, Currency, IOUAmount, Issue, LedgerEntryType, STAmount, STLedgerEntry,
    STTx, Ter, TxType, XRPAmount, account_keylet, get_field_by_symbol,
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

fn run(ledger: &Ledger, tx: STTx) -> Ter {
    let base = Arc::new(ledger.clone());
    let mut view = Sandbox::new(base, ApplyFlags::default());
    let txn_type = tx.get_txn_type();
    apply_submit_transactor_shell(&mut view, &tx, txn_type)
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
    // Account has only reserve (200_000 drops), zero liquid XRP after reserve
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
    // Set fees so account_reserve(0) = 200_000 (matching account balance → zero liquid)
    ledger.set_fees(ledger::Fees {
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
