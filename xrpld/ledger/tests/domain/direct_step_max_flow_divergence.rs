//! Reproduction/guard for the 2026-10-01 step-0 strand dry divergence:
//! a self-directed tfPartialPayment IOU->XRP (e.g. tx 01D0D930, FRH->XRP) that
//! the network delivered (tesSUCCESS) but Quaxar dried (tecPATH_DRY). The dry
//! was localized to the strand's first Direct step producing zero because
//! max_payment_flow returned zero. This test verifies that a sender holding a
//! positive IOU balance yields a positive REDEEM flow (the sender can spend the
//! IOU it holds), independent of any trust-line limit.

use basics::base_uint::{Uint160, Uint256};
use ledger::ripple_calc::direct_step::max_payment_flow;
use ledger::{ApplyViewImpl, Ledger, LedgerHeader};
use protocol::{
    AccountID, ApplyFlags, Currency, IOUAmount, Issue, LedgerEntryType, STAmount, STLedgerEntry,
    account_keylet, currency_from_string, get_field_by_symbol, line, sf_generic,
};
use shamap::item::SHAMapItem;
use shamap::mutation::MutableTree;
use shamap::sync::{SHAMapType, SyncState, SyncTree};
use shamap::tree_node::SHAMapNodeType;
use std::sync::Arc;

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

fn iou(mantissa: i64, exponent: i32, issue: Issue) -> STAmount {
    STAmount::from_iou_amount(
        sf_generic(),
        IOUAmount::from_parts(mantissa, exponent).expect("iou"),
        issue,
    )
}

fn build_view_ledger(items: Vec<(Uint256, Vec<u8>)>) -> Ledger {
    let header = LedgerHeader {
        seq: 100,
        close_time: 1_000,
        close_time_resolution: ledger::LEDGER_DEFAULT_TIME_RESOLUTION,
        drops: ledger::INITIAL_XRP_DROPS,
        ..LedgerHeader::default()
    };
    let mut tree = MutableTree::new(100);
    for (key, payload) in &items {
        tree.add_item(
            SHAMapNodeType::AccountState,
            SHAMapItem::new(*key, payload.clone()),
        )
        .expect("state item");
    }
    Ledger::from_maps(
        header,
        SyncTree::from_root_with_type(tree.root(), SHAMapType::State, false, 100, SyncState::Immutable),
        SyncTree::new_with_type(SHAMapType::Transaction, false, 100),
    )
}

fn account_root(account: AccountID) -> (Uint256, Vec<u8>) {
    let key = account_keylet(Uint160::from_slice(account.data()).expect("w")).key;
    let mut e = STLedgerEntry::from_type_and_key(LedgerEntryType::AccountRoot, key);
    e.set_account_id(sf("sfAccount"), account);
    e.set_field_amount(sf("sfBalance"), STAmount::new_native(1_000_000_000, false));
    e.set_field_u32(sf("sfSequence"), 1);
    e.set_field_u32(sf("sfOwnerCount"), 1);
    (key, e.get_serializer().data().to_vec())
}

/// Trust line where `sender` (low) holds +balance of `currency` issued by `issuer` (high).
fn trustline_sender_holds(
    sender: AccountID,
    issuer: AccountID,
    currency: Currency,
    balance_mantissa: i64,
    balance_exp: i32,
) -> (Uint256, Vec<u8>) {
    let (low, high) = if sender < issuer { (sender, issuer) } else { (issuer, sender) };
    let key = line(low, high, currency).key;
    let mut e = STLedgerEntry::from_type_and_key(LedgerEntryType::RippleState, key);
    // sfBalance is from the LOW account's perspective. sender holds +X means:
    // if sender is low, balance = +X; if sender is high, balance = -X.
    let bal = if sender < issuer {
        iou(balance_mantissa, balance_exp, Issue::new(currency, low))
    } else {
        iou(-balance_mantissa, balance_exp, Issue::new(currency, low))
    };
    e.set_field_amount(sf("sfBalance"), bal);
    e.set_field_amount(sf("sfLowLimit"), iou(1_000_000_000, 0, Issue::new(currency, low)));
    e.set_field_amount(sf("sfHighLimit"), iou(1_000_000_000, 0, Issue::new(currency, high)));
    (key, e.get_serializer().data().to_vec())
}

#[test]
fn max_payment_flow_sender_holding_iou_redeems_positive() {
    let sender = AccountID::from_array([0x11; 20]);
    let issuer = AccountID::from_array([0x99; 20]); // issuer > sender
    let currency = currency_from_string("FRH");

    // Sender holds 500 FRH.
    let items = vec![
        account_root(sender),
        account_root(issuer),
        trustline_sender_holds(sender, issuer, currency, 500, 0),
    ];
    let base = Arc::new(build_view_ledger(items));
    let mut view = ApplyViewImpl::new(base, ApplyFlags::NONE);

    let (flow, _dir) = max_payment_flow(&mut view, &sender, &issuer, currency).expect("max flow");
    assert!(
        flow.signum() > 0,
        "sender holding 500 FRH must yield positive redeem flow (step-0 dry divergence); got {flow:?}"
    );
}

#[test]
fn max_payment_flow_sender_high_holding_iou_redeems_positive() {
    // Same, but with sender > issuer so sender is the HIGH account (sign flip).
    let sender = AccountID::from_array([0x99; 20]);
    let issuer = AccountID::from_array([0x11; 20]); // issuer < sender
    let currency = currency_from_string("FRH");

    let items = vec![
        account_root(sender),
        account_root(issuer),
        trustline_sender_holds(sender, issuer, currency, 500, 0),
    ];
    let base = Arc::new(build_view_ledger(items));
    let mut view = ApplyViewImpl::new(base, ApplyFlags::NONE);

    let (flow, _dir) = max_payment_flow(&mut view, &sender, &issuer, currency).expect("max flow");
    assert!(
        flow.signum() > 0,
        "sender(high) holding 500 FRH must yield positive redeem flow; got {flow:?}"
    );
}
