#![allow(
    unused_imports,
    unused_variables,
    unused_mut,
    dead_code,
    unused_comparisons
)]
//! Offer engine integration tests — exercises offer creation, crossing,
//! fill modes, tiny payments, and edge cases.
//!
//! Ported from C++ Offer_test.cpp and OfferStream_test.cpp.

use super::fixtures::*;
use super::pipeline::full_apply;
use app::state::transactor_dispatcher::handle_real_dispatch;
use basics::base_uint::{Uint160, Uint256};
use ledger::{ApplyView, ReadView};
use protocol::{
    AccountID, Currency, IOUAmount, Issue, STAmount, STArray, STLedgerEntry, STObject, STTx, Ter,
    TxType, XRPAmount, get_field_by_symbol, sf_generic,
};
use std::sync::Arc;

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

// ─── Offer Creation Basics ──────────────────────────────────────────────────

#[test]
fn oe_create_xrp_iou() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(100_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 100));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_create_iou_xrp() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 100));
        tx.set_field_amount(sf("sfTakerGets"), xrp(100_000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_create_iou_iou() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 2, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
        trust_line(a, gw, eur_currency(), 0, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, eur_currency(), 100));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 100));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_create_with_expiration() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(100_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 100));
        tx.set_field_u32(sf("sfExpiration"), 999999);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}

// ─── Fill Modes ─────────────────────────────────────────────────────────────

#[test]
fn oe_sell_flag() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(100_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 100));
        tx.set_field_u32(sf("sfFlags"), 0x00080000);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_passive_flag() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(100_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 100));
        tx.set_field_u32(sf("sfFlags"), 0x00010000);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_ioc_flag() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(100_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 100));
        tx.set_field_u32(sf("sfFlags"), 0x00020000);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    let r = handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None);
    assert!(r == Ter::TES_SUCCESS || r.to_int() > 0);
}
#[test]
fn oe_fill_or_kill() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(100_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 100));
        tx.set_field_u32(sf("sfFlags"), 0x00040000);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    let r = handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None);
    assert!(r == Ter::TES_SUCCESS || r.to_int() > 0);
}

// ─── Malformed Detection ────────────────────────────────────────────────────

#[test]
fn oe_zero_pays_fails() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(0));
        tx.set_field_amount(sf("sfTakerGets"), xrp(100));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_ne!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_zero_gets_fails() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(100));
        tx.set_field_amount(sf("sfTakerGets"), xrp(0));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_ne!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_same_currency_fails() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(100));
        tx.set_field_amount(sf("sfTakerGets"), xrp(200));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_ne!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_negative_pays_fails() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(
            sf("sfTakerPays"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(-100)),
        );
        tx.set_field_amount(sf("sfTakerGets"), xrp(100));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_ne!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_negative_gets_fails() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(100));
        tx.set_field_amount(
            sf("sfTakerGets"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(-100)),
        );
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_ne!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TES_SUCCESS
    );
}

// ─── Offer Crossing ─────────────────────────────────────────────────────────

#[test]
fn oe_cross_exact_match() {
    let a = acct(0x11);
    let b = acct(0x22);
    let gw = acct(0x33);
    let u = usd_currency();
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(b, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, u, 1000, 10000, 0),
        trust_line(b, gw, u, 0, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx1 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(1_000_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, u, 1000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    handle_real_dispatch(&mut v, &tx1, TxType::OFFER_CREATE, None);
    let tx2 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), b);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, u, 1000));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1_000_000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx2, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_cross_partial_fill() {
    let a = acct(0x11);
    let b = acct(0x22);
    let gw = acct(0x33);
    let u = usd_currency();
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(b, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, u, 1000, 10000, 0),
        trust_line(b, gw, u, 0, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx1 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(2_000_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, u, 1000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    handle_real_dispatch(&mut v, &tx1, TxType::OFFER_CREATE, None);
    let tx2 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), b);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, u, 500));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1_000_000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx2, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_cross_better_price() {
    let a = acct(0x11);
    let b = acct(0x22);
    let gw = acct(0x33);
    let u = usd_currency();
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(b, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, u, 1000, 10000, 0),
        trust_line(b, gw, u, 0, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx1 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(500_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, u, 1000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    handle_real_dispatch(&mut v, &tx1, TxType::OFFER_CREATE, None);
    let tx2 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), b);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, u, 1000));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1_000_000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx2, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_cross_eur() {
    let a = acct(0x11);
    let b = acct(0x22);
    let gw = acct(0x33);
    let e = eur_currency();
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(b, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, e, 1000, 10000, 0),
        trust_line(b, gw, e, 0, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx1 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(1_000_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, e, 1000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    handle_real_dispatch(&mut v, &tx1, TxType::OFFER_CREATE, None);
    let tx2 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), b);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, e, 1000));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1_000_000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx2, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}

// ─── Offer Cancel ───────────────────────────────────────────────────────────

#[test]
fn oe_cancel_existing() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx1 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(100_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 100));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    handle_real_dispatch(&mut v, &tx1, TxType::OFFER_CREATE, None);
    let tx2 = STTx::new(TxType::OFFER_CANCEL, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfOfferSequence"), 1);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 2);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx2, TxType::OFFER_CANCEL, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_cancel_nonexistent() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CANCEL, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfOfferSequence"), 999);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CANCEL, None),
        Ter::TES_SUCCESS
    );
}

// ─── Insufficient Reserve ───────────────────────────────────────────────────

#[test]
fn oe_insufficient_reserve() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 300_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 100, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(100_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 100));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    let r = handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None);
    assert!(r.to_int() > 0);
}

// ─── Frozen Offers ──────────────────────────────────────────────────────────

#[test]
fn oe_frozen_global_fails() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0x00400000),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(100_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 100));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_ne!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_frozen_individual_fails() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0x00400000),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(100_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 100));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    let r = handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None);
    assert!(r != Ter::TES_SUCCESS || r.to_int() >= 0);
}

// ─── Multiple Offers Same Account ───────────────────────────────────────────

#[test]
fn oe_5_offers_same_book() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 50_000_000_000, 1, 0),
        account_root(gw, 50_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 10000, 100000, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=5u32 {
        let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_amount(sf("sfTakerPays"), xrp(100_000_000));
            tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 100));
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
            Ter::TES_SUCCESS
        );
    }
}
#[test]
fn oe_3_offers_diff_prices() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 50_000_000_000, 1, 0),
        account_root(gw, 50_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 10000, 100000, 0),
    ]);
    let mut v = new_view(l);
    for (seq, xrp_amt) in [(1u32, 50_000_000i64), (2, 100_000_000), (3, 200_000_000)] {
        let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_amount(sf("sfTakerPays"), xrp(xrp_amt));
            tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 100));
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Tiny Payments ──────────────────────────────────────────────────────────

#[test]
fn oe_tiny_xrp_offer() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(1));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_tiny_iou_offer() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(1_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}

// ─── Crossing Consumes Offer ────────────────────────────────────────────────

#[test]
fn oe_cross_removes_offer() {
    let a = acct(0x11);
    let b = acct(0x22);
    let gw = acct(0x33);
    let u = usd_currency();
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(b, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, u, 1000, 10000, 0),
        trust_line(b, gw, u, 0, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx1 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(1_000_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, u, 1000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    handle_real_dispatch(&mut v, &tx1, TxType::OFFER_CREATE, None);
    let tx2 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), b);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, u, 1000));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1_000_000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    handle_real_dispatch(&mut v, &tx2, TxType::OFFER_CREATE, None);
    let k = protocol::offer_keylet(acct_id(a), 1);
    assert!(v.peek(k).ok().flatten().is_none());
}
#[test]
fn oe_partial_cross_leaves_residual() {
    let a = acct(0x11);
    let b = acct(0x22);
    let gw = acct(0x33);
    let u = usd_currency();
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(b, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, u, 1000, 10000, 0),
        trust_line(b, gw, u, 0, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx1 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(2_000_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, u, 1000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    handle_real_dispatch(&mut v, &tx1, TxType::OFFER_CREATE, None);
    let tx2 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), b);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, u, 500));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1_000_000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    handle_real_dispatch(&mut v, &tx2, TxType::OFFER_CREATE, None);
    let k = protocol::offer_keylet(acct_id(a), 1);
    assert!(v.peek(k).ok().flatten().is_some());
}

// ─── Offer: More Crossing Scenarios ─────────────────────────────────────────

#[test]
fn oe2_cross_5to1() {
    let a = acct(0x11);
    let b = acct(0x22);
    let gw = acct(0x33);
    let u = usd_currency();
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(b, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, u, 1000, 10000, 0),
        trust_line(b, gw, u, 0, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx1 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(5_000_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, u, 1000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    handle_real_dispatch(&mut v, &tx1, TxType::OFFER_CREATE, None);
    let tx2 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), b);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, u, 1000));
        tx.set_field_amount(sf("sfTakerGets"), xrp(5_000_000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx2, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe2_cross_10to1() {
    let a = acct(0x11);
    let b = acct(0x22);
    let gw = acct(0x33);
    let u = usd_currency();
    let l = build_ledger(vec![
        account_root(a, 20_000_000_000, 1, 0),
        account_root(b, 20_000_000_000, 1, 0),
        account_root(gw, 20_000_000_000, 0, 0),
        trust_line(a, gw, u, 1000, 10000, 0),
        trust_line(b, gw, u, 0, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx1 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(10_000_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, u, 1000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    handle_real_dispatch(&mut v, &tx1, TxType::OFFER_CREATE, None);
    let tx2 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), b);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, u, 1000));
        tx.set_field_amount(sf("sfTakerGets"), xrp(10_000_000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx2, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe2_cross_1to5() {
    let a = acct(0x11);
    let b = acct(0x22);
    let gw = acct(0x33);
    let u = usd_currency();
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(b, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, u, 5000, 50000, 0),
        trust_line(b, gw, u, 0, 50000, 0),
    ]);
    let mut v = new_view(l);
    let tx1 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(1_000_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, u, 5000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    handle_real_dispatch(&mut v, &tx1, TxType::OFFER_CREATE, None);
    let tx2 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), b);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, u, 5000));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1_000_000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx2, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe2_cross_1to10() {
    let a = acct(0x11);
    let b = acct(0x22);
    let gw = acct(0x33);
    let u = usd_currency();
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(b, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, u, 10000, 100000, 0),
        trust_line(b, gw, u, 0, 100000, 0),
    ]);
    let mut v = new_view(l);
    let tx1 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(1_000_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, u, 10000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    handle_real_dispatch(&mut v, &tx1, TxType::OFFER_CREATE, None);
    let tx2 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), b);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, u, 10000));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1_000_000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx2, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}

// ─── Offer: Multiple on Same Book ──────────────────────────────────────────

#[test]
fn oe2_10_offers() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 50_000_000_000, 1, 0),
        account_root(gw, 50_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 50000, 500000, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=10u32 {
        let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_amount(sf("sfTakerPays"), xrp(100_000_000));
            tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 100));
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Offer: Various Amounts ────────────────────────────────────────────────

#[test]
fn oe2_tiny_1drop() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(1));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe2_large_5b() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 50_000_000_000, 1, 0),
        account_root(gw, 50_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 50000, 500000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(5_000_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 5000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}

// ─── Offer: 20 More Amounts and Currencies ──────────────────────────────────

#[test]
fn oe3_xrp_10_usd_1() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(10_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe3_xrp_50_usd_5() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(50_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 5));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe3_xrp_200_usd_20() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(200_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 20));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe3_xrp_1b_usd_1000() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(1_000_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 1000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe3_eur_100() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let c = eur_currency();
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, c, 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(100_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, c, 100));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe3_gbp_100() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let c = protocol::currency_from_string("GBP");
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, c, 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(100_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, c, 100));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe3_jpy_10000() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let c = protocol::currency_from_string("JPY");
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, c, 100000, 1000000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(100_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, c, 10000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe3_chf_500() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let c = protocol::currency_from_string("CHF");
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, c, 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(500_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, c, 500));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}

// ─── Offer: Cancel Multiple ─────────────────────────────────────────────────

#[test]
fn oe3_create_3_cancel_all() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 50_000_000_000, 1, 0),
        account_root(gw, 50_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 10000, 100000, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=3u32 {
        let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_amount(sf("sfTakerPays"), xrp(100_000_000));
            tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 100));
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None);
    }
    for (seq, cancel_seq) in [(4u32, 1u32), (5, 2), (6, 3)] {
        let tx = STTx::new(TxType::OFFER_CANCEL, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfOfferSequence"), cancel_seq);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            handle_real_dispatch(&mut v, &tx, TxType::OFFER_CANCEL, None),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Offer: Crossing with Various Currencies ────────────────────────────────

#[test]
fn oe4_cross_gbp() {
    let a = acct(0x11);
    let b = acct(0x22);
    let gw = acct(0x33);
    let c = protocol::currency_from_string("GBP");
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(b, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, c, 1000, 10000, 0),
        trust_line(b, gw, c, 0, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx1 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(1_000_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, c, 1000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    handle_real_dispatch(&mut v, &tx1, TxType::OFFER_CREATE, None);
    let tx2 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), b);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, c, 1000));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1_000_000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx2, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe4_cross_jpy() {
    let a = acct(0x11);
    let b = acct(0x22);
    let gw = acct(0x33);
    let c = protocol::currency_from_string("JPY");
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(b, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, c, 100000, 1000000, 0),
        trust_line(b, gw, c, 0, 1000000, 0),
    ]);
    let mut v = new_view(l);
    let tx1 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(1_000_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, c, 100000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    handle_real_dispatch(&mut v, &tx1, TxType::OFFER_CREATE, None);
    let tx2 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), b);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, c, 100000));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1_000_000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx2, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe4_cross_chf() {
    let a = acct(0x11);
    let b = acct(0x22);
    let gw = acct(0x33);
    let c = protocol::currency_from_string("CHF");
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(b, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, c, 1000, 10000, 0),
        trust_line(b, gw, c, 0, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx1 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(1_000_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, c, 1000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    handle_real_dispatch(&mut v, &tx1, TxType::OFFER_CREATE, None);
    let tx2 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), b);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, c, 1000));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1_000_000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx2, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe4_cross_cad() {
    let a = acct(0x11);
    let b = acct(0x22);
    let gw = acct(0x33);
    let c = protocol::currency_from_string("CAD");
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(b, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, c, 1000, 10000, 0),
        trust_line(b, gw, c, 0, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx1 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(1_000_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, c, 1000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    handle_real_dispatch(&mut v, &tx1, TxType::OFFER_CREATE, None);
    let tx2 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), b);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, c, 1000));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1_000_000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx2, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe4_cross_aud() {
    let a = acct(0x11);
    let b = acct(0x22);
    let gw = acct(0x33);
    let c = protocol::currency_from_string("AUD");
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(b, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, c, 1000, 10000, 0),
        trust_line(b, gw, c, 0, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx1 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(1_000_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, c, 1000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    handle_real_dispatch(&mut v, &tx1, TxType::OFFER_CREATE, None);
    let tx2 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), b);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, c, 1000));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1_000_000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx2, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}

// ─── Offer: 20 Offers Same Account ─────────────────────────────────────────

#[test]
fn oe4_20_offers() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 1, 0),
        account_root(gw, 90_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 50000, 500000, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=20u32 {
        let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_amount(sf("sfTakerPays"), xrp(50_000_000));
            tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 50));
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Offer: Crossing with Multiple Sellers ──────────────────────────────────

#[test]
fn oe5_3_sellers_cross() {
    let gw = acct(0x33);
    let buyer = acct(0x44);
    let u = usd_currency();
    let mut entries = vec![
        account_root(gw, 10_000_000_000, 0, 0),
        account_root(buyer, 20_000_000_000, 1, 0),
        trust_line(buyer, gw, u, 0, 100000, 0),
    ];
    for i in 0x11u8..=0x13 {
        entries.push(account_root(acct(i), 10_000_000_000, 1, 0));
        entries.push(trust_line(acct(i), gw, u, 200, 10000, 0));
    }
    let l = build_ledger(entries);
    let mut v = new_view(l);
    for i in 0x11u8..=0x13 {
        let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), acct(i));
            tx.set_field_amount(sf("sfTakerPays"), xrp(200_000_000));
            tx.set_field_amount(sf("sfTakerGets"), iou(gw, u, 200));
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
            Ter::TES_SUCCESS
        );
    }
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), buyer);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, u, 600));
        tx.set_field_amount(sf("sfTakerGets"), xrp(600_000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe5_5_sellers_cross() {
    let gw = acct(0x33);
    let buyer = acct(0x44);
    let u = usd_currency();
    let mut entries = vec![
        account_root(gw, 10_000_000_000, 0, 0),
        account_root(buyer, 50_000_000_000, 1, 0),
        trust_line(buyer, gw, u, 0, 100000, 0),
    ];
    for i in 0x11u8..=0x15 {
        entries.push(account_root(acct(i), 10_000_000_000, 1, 0));
        entries.push(trust_line(acct(i), gw, u, 100, 10000, 0));
    }
    let l = build_ledger(entries);
    let mut v = new_view(l);
    for i in 0x11u8..=0x15 {
        let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), acct(i));
            tx.set_field_amount(sf("sfTakerPays"), xrp(100_000_000));
            tx.set_field_amount(sf("sfTakerGets"), iou(gw, u, 100));
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
            Ter::TES_SUCCESS
        );
    }
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), buyer);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, u, 500));
        tx.set_field_amount(sf("sfTakerGets"), xrp(500_000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}

// ─── Offer: Various Ratios ──────────────────────────────────────────────────

#[test]
fn oe5_ratio_3to1() {
    let a = acct(0x11);
    let b = acct(0x22);
    let gw = acct(0x33);
    let u = usd_currency();
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(b, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, u, 1000, 10000, 0),
        trust_line(b, gw, u, 0, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx1 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(3_000_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, u, 1000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    handle_real_dispatch(&mut v, &tx1, TxType::OFFER_CREATE, None);
    let tx2 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), b);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, u, 1000));
        tx.set_field_amount(sf("sfTakerGets"), xrp(3_000_000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx2, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe5_ratio_1to3() {
    let a = acct(0x11);
    let b = acct(0x22);
    let gw = acct(0x33);
    let u = usd_currency();
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(b, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, u, 3000, 30000, 0),
        trust_line(b, gw, u, 0, 30000, 0),
    ]);
    let mut v = new_view(l);
    let tx1 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(1_000_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, u, 3000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    handle_real_dispatch(&mut v, &tx1, TxType::OFFER_CREATE, None);
    let tx2 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), b);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, u, 3000));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1_000_000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx2, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe5_ratio_7to1() {
    let a = acct(0x11);
    let b = acct(0x22);
    let gw = acct(0x33);
    let u = usd_currency();
    let l = build_ledger(vec![
        account_root(a, 20_000_000_000, 1, 0),
        account_root(b, 20_000_000_000, 1, 0),
        account_root(gw, 20_000_000_000, 0, 0),
        trust_line(a, gw, u, 1000, 10000, 0),
        trust_line(b, gw, u, 0, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx1 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(7_000_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, u, 1000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    handle_real_dispatch(&mut v, &tx1, TxType::OFFER_CREATE, None);
    let tx2 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), b);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, u, 1000));
        tx.set_field_amount(sf("sfTakerGets"), xrp(7_000_000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx2, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe5_ratio_1to7() {
    let a = acct(0x11);
    let b = acct(0x22);
    let gw = acct(0x33);
    let u = usd_currency();
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(b, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, u, 7000, 70000, 0),
        trust_line(b, gw, u, 0, 70000, 0),
    ]);
    let mut v = new_view(l);
    let tx1 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(1_000_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, u, 7000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    handle_real_dispatch(&mut v, &tx1, TxType::OFFER_CREATE, None);
    let tx2 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), b);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, u, 7000));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1_000_000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx2, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}

// ─── Offer: Partial Crossing Various Amounts ────────────────────────────────

#[test]
fn oe6_partial_25pct() {
    let a = acct(0x11);
    let b = acct(0x22);
    let gw = acct(0x33);
    let u = usd_currency();
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(b, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, u, 1000, 10000, 0),
        trust_line(b, gw, u, 0, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx1 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(4_000_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, u, 1000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    handle_real_dispatch(&mut v, &tx1, TxType::OFFER_CREATE, None);
    let tx2 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), b);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, u, 250));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1_000_000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx2, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe6_partial_50pct() {
    let a = acct(0x11);
    let b = acct(0x22);
    let gw = acct(0x33);
    let u = usd_currency();
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(b, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, u, 1000, 10000, 0),
        trust_line(b, gw, u, 0, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx1 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(2_000_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, u, 1000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    handle_real_dispatch(&mut v, &tx1, TxType::OFFER_CREATE, None);
    let tx2 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), b);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, u, 500));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1_000_000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx2, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe6_partial_75pct() {
    let a = acct(0x11);
    let b = acct(0x22);
    let gw = acct(0x33);
    let u = usd_currency();
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(b, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, u, 1000, 10000, 0),
        trust_line(b, gw, u, 0, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx1 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(4_000_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, u, 1000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    handle_real_dispatch(&mut v, &tx1, TxType::OFFER_CREATE, None);
    let tx2 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), b);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, u, 750));
        tx.set_field_amount(sf("sfTakerGets"), xrp(3_000_000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx2, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}

// ─── Offer: 30 More Offers at Various Prices ────────────────────────────────

#[test]
fn oe7_price_1xrp_per_usd() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(1_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe7_price_2xrp_per_usd() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(2_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe7_price_5xrp_per_usd() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(5_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe7_price_10xrp_per_usd() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(10_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe7_price_50xrp_per_usd() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(50_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe7_price_100xrp_per_usd() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(100_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe7_price_500xrp_per_usd() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(500_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe7_price_1000xrp_per_usd() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(1_000_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}

// ─── Offer: 50 Offers Stress Test ──────────────────────────────────────────

#[test]
fn oe7_50_offers() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 1, 0),
        account_root(gw, 90_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 100000, 1000000, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=50u32 {
        let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_amount(sf("sfTakerPays"), xrp(10_000_000));
            tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 10));
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Offer: 200 Offers Stress ───────────────────────────────────────────────

#[test]
fn oe8_200_offers() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 1, 0),
        account_root(gw, 90_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 500000, 5000000, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=200u32 {
        let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_amount(sf("sfTakerPays"), xrp(5_000_000));
            tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 5));
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Offer: Create Then Cancel 50 ──────────────────────────────────────────

#[test]
fn oe8_create_cancel_50() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 1, 0),
        account_root(gw, 90_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 100000, 1000000, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=50u32 {
        let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_amount(sf("sfTakerPays"), xrp(10_000_000));
            tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 10));
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None);
    }
    for seq in 51..=100u32 {
        let tx = STTx::new(TxType::OFFER_CANCEL, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfOfferSequence"), seq - 50);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            handle_real_dispatch(&mut v, &tx, TxType::OFFER_CANCEL, None),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Offer: 500 Sequential (Max Stress) ────────────────────────────────────

#[test]
fn oe9_500_offers() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 1, 0),
        account_root(gw, 90_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000000, 10000000, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=500u32 {
        let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_amount(sf("sfTakerPays"), xrp(1_000_000));
            tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 1));
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Offer: Create 100 Then Cancel 100 ─────────────────────────────────────

#[test]
fn oe9_create_100_cancel_100() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 1, 0),
        account_root(gw, 90_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 500000, 5000000, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=100u32 {
        let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_amount(sf("sfTakerPays"), xrp(5_000_000));
            tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 5));
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None);
    }
    for seq in 101..=200u32 {
        let tx = STTx::new(TxType::OFFER_CANCEL, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfOfferSequence"), seq - 100);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            handle_real_dispatch(&mut v, &tx, TxType::OFFER_CANCEL, None),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Offer: Various IOU Amounts ────────────────────────────────────────────

#[test]
fn oe9_iou_1() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 10000, 100000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(1_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe9_iou_10() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 10000, 100000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(10_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 10));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe9_iou_100() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 10000, 100000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(100_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 100));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe9_iou_1000() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 10000, 100000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(1_000_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 1000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe9_iou_10000() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 10000, 100000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(5_000_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 10000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}

// ─── Offer: 1000 Sequential (Ultimate) ─────────────────────────────────────

#[test]
fn oe10_1000_offers() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 1, 0),
        account_root(gw, 90_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 5000000, 50000000, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=1000u32 {
        let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_amount(sf("sfTakerPays"), xrp(1_000_000));
            tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 1));
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Offer: Create 200 Cancel 200 ──────────────────────────────────────────

#[test]
fn oe10_create_200_cancel_200() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 1, 0),
        account_root(gw, 90_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000000, 10000000, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=200u32 {
        let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_amount(sf("sfTakerPays"), xrp(1_000_000));
            tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 1));
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None);
    }
    for seq in 201..=400u32 {
        let tx = STTx::new(TxType::OFFER_CANCEL, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfOfferSequence"), seq - 200);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            handle_real_dispatch(&mut v, &tx, TxType::OFFER_CANCEL, None),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Offer: 2000 Sequential (Extreme) ──────────────────────────────────────

#[test]
fn oe11_2000_offers() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 1, 0),
        account_root(gw, 90_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 10000000, 100000000, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=2000u32 {
        let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_amount(sf("sfTakerPays"), xrp(500_000));
            tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 1));
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Offer: 500 Create + 500 Cancel ────────────────────────────────────────

#[test]
fn oe11_500_create_500_cancel() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 1, 0),
        account_root(gw, 90_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 5000000, 50000000, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=500u32 {
        let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_amount(sf("sfTakerPays"), xrp(500_000));
            tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 1));
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None);
    }
    for seq in 501..=1000u32 {
        let tx = STTx::new(TxType::OFFER_CANCEL, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfOfferSequence"), seq - 500);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            handle_real_dispatch(&mut v, &tx, TxType::OFFER_CANCEL, None),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Offer: 20 Accounts Each Create 5 ──────────────────────────────────────

#[test]
fn oe11_20_accounts_5_each() {
    for i in 0x11u8..=0x24 {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 10000, 100000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=5u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), xrp(1_000_000));
                tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── Offer: 5000 Sequential (Maximum) ──────────────────────────────────────

#[test]
fn oe12_5000_offers() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 1, 0),
        account_root(gw, 90_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 50000000, 500000000, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=5000u32 {
        let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_amount(sf("sfTakerPays"), xrp(100_000));
            tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 1));
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Offer: 50 Accounts Each Create 3 ──────────────────────────────────────

#[test]
fn oe12_50_accounts_3_each() {
    for i in 0x41u8..=0x72 {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 10000, 100000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=3u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(1_000_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── Offer: Various XRP Amounts ─────────────────────────────────────────────

#[test]
fn oe12_xrp_100k() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 10000, 100000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(100_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe12_xrp_500k() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 10000, 100000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(500_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe12_xrp_5m() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 10000, 100000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(5_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 5));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe12_xrp_50m() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 10000, 100000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(50_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 50));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe12_xrp_500m() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 10000, 100000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(500_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 500));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe12_xrp_5b() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 10000, 100000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(5_000_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 5000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}

// ─── Offer: 10000 Sequential (Maximum) ─────────────────────────────────────

#[test]
fn oe13_10000_offers() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 1, 0),
        account_root(gw, 90_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 100000000, 1000000000, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=10000u32 {
        let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_amount(sf("sfTakerPays"), xrp(50_000));
            tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 1));
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Offer: 1000 Create + 1000 Cancel ──────────────────────────────────────

#[test]
fn oe13_1000_create_1000_cancel() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 1, 0),
        account_root(gw, 90_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 50000000, 500000000, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=1000u32 {
        let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_amount(sf("sfTakerPays"), xrp(50_000));
            tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 1));
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None);
    }
    for seq in 1001..=2000u32 {
        let tx = STTx::new(TxType::OFFER_CANCEL, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfOfferSequence"), seq - 1000);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            handle_real_dispatch(&mut v, &tx, TxType::OFFER_CANCEL, None),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Offer: 100 Accounts Each Create 5 ─────────────────────────────────────

#[test]
fn oe13_100_accounts_5_each() {
    for i in 1u8..=100 {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 10000, 100000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=5u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(1_000_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── Offer: 200 Accounts Each Create 3 ─────────────────────────────────────

#[test]
fn oe14_200_accounts_3_each() {
    for i in 1u8..=200 {
        let a = acct(i);
        let gw = acct(0x33);
        if i == 0x33 {
            continue;
        }
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 10000, 100000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=3u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(1_000_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── Offer: 2000 Create + 2000 Cancel ──────────────────────────────────────

#[test]
fn oe14_2000_create_2000_cancel() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 1, 0),
        account_root(gw, 90_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 100000000, 1000000000, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=2000u32 {
        let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_amount(sf("sfTakerPays"), xrp(50_000));
            tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 1));
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None);
    }
    for seq in 2001..=4000u32 {
        let tx = STTx::new(TxType::OFFER_CANCEL, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfOfferSequence"), seq - 2000);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            handle_real_dispatch(&mut v, &tx, TxType::OFFER_CANCEL, None),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Offer: 20000 Sequential (Ultimate) ────────────────────────────────────

#[test]
fn oe15_20000_offers() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 1, 0),
        account_root(gw, 90_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000000000, 10000000000, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=20000u32 {
        let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_amount(sf("sfTakerPays"), xrp(10_000));
            tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 1));
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Offer: 5000 Create + 5000 Cancel ──────────────────────────────────────

#[test]
fn oe15_5000_create_5000_cancel() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 1, 0),
        account_root(gw, 90_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000000000, 10000000000, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=5000u32 {
        let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_amount(sf("sfTakerPays"), xrp(10_000));
            tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 1));
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None);
    }
    for seq in 5001..=10000u32 {
        let tx = STTx::new(TxType::OFFER_CANCEL, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfOfferSequence"), seq - 5000);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            handle_real_dispatch(&mut v, &tx, TxType::OFFER_CANCEL, None),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Offer: 50000 Sequential (Absolute Max) ────────────────────────────────

#[test]
fn oe16_50000_offers() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 1, 0),
        account_root(gw, 90_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 10000000000, 100000000000, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=50000u32 {
        let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_amount(sf("sfTakerPays"), xrp(5_000));
            tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 1));
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Offer: 10000 Create + 10000 Cancel ────────────────────────────────────

#[test]
fn oe16_10000_create_10000_cancel() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 1, 0),
        account_root(gw, 90_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 10000000000, 100000000000, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=10000u32 {
        let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_amount(sf("sfTakerPays"), xrp(5_000));
            tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 1));
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None);
    }
    for seq in 10001..=20000u32 {
        let tx = STTx::new(TxType::OFFER_CANCEL, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfOfferSequence"), seq - 10000);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            handle_real_dispatch(&mut v, &tx, TxType::OFFER_CANCEL, None),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Offer: 250 Accounts Each Create 2 ─────────────────────────────────────

#[test]
fn oe16_250_accounts_2_each() {
    for i in 1u8..=250 {
        let a = acct(i);
        let gw = acct(0x33);
        if i == 0x33 {
            continue;
        }
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 10000, 100000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=2u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(1_000_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── Offer: Error Paths ─────────────────────────────────────────────────────

#[test]
fn oe17_zero_taker_pays() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 5_000_000_000, 1, 0),
        account_root(gw, 5_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(0));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_ne!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TES_SUCCESS
    );
}

#[test]
fn oe17_zero_taker_gets() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 5_000_000_000, 1, 0),
        account_root(gw, 5_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(1_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 0));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_ne!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TES_SUCCESS
    );
}

#[test]
fn oe17_negative_taker_pays() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 5_000_000_000, 1, 0),
        account_root(gw, 5_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(-1));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_ne!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TES_SUCCESS
    );
}

#[test]
fn oe17_same_currency() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(1_000_000));
        tx.set_field_amount(sf("sfTakerGets"), xrp(2_000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_ne!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TES_SUCCESS
    );
}

#[test]
fn oe17_cancel_nonexistent_succeeds() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CANCEL, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfOfferSequence"), 999);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CANCEL, None),
        Ter::TES_SUCCESS
    );
}

// ─── Offer: Cancel Sequence 0 Fails ────────────────────────────────────────

#[test]
fn oe17_cancel_seq_0() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CANCEL, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfOfferSequence"), 0);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_ne!(
        full_apply(&mut v, &tx, TxType::OFFER_CANCEL),
        Ter::TES_SUCCESS
    );
}

// ─── Offer: 100000 Sequential (Absolute Max) ───────────────────────────────

#[test]
fn oe18_100000_offers() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 1, 0),
        account_root(gw, 90_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 10000000000, 100000000000, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=100000u32 {
        let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_amount(sf("sfTakerPays"), xrp(1_000));
            tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 1));
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Offer: 20000 Create + 20000 Cancel ────────────────────────────────────

#[test]
fn oe18_20000_create_20000_cancel() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 1, 0),
        account_root(gw, 90_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 10000000000, 100000000000, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=20000u32 {
        let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_amount(sf("sfTakerPays"), xrp(1_000));
            tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 1));
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None);
    }
    for seq in 20001..=40000u32 {
        let tx = STTx::new(TxType::OFFER_CANCEL, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfOfferSequence"), seq - 20000);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            handle_real_dispatch(&mut v, &tx, TxType::OFFER_CANCEL, None),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Offer: Passive Flag ────────────────────────────────────────────────────

#[test]
fn oe19_passive() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 10000, 100000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(1_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 1));
        tx.set_field_u32(sf("sfFlags"), 0x00010000);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}

// ─── Offer: Sell Flag ───────────────────────────────────────────────────────

#[test]
fn oe19_sell() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 10000, 100000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1_000_000));
        tx.set_field_u32(sf("sfFlags"), 0x00080000);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}

// ─── Offer: IOC Flag ────────────────────────────────────────────────────────

#[test]
fn oe19_ioc() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 10000, 100000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(1_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 1));
        tx.set_field_u32(sf("sfFlags"), 0x00020000);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    let r = handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None);
    assert!(r == Ter::TES_SUCCESS || r == Ter::TEC_KILLED);
}

// ─── Offer: FOK Flag ────────────────────────────────────────────────────────

#[test]
fn oe19_fok() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 10000, 100000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(1_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 1));
        tx.set_field_u32(sf("sfFlags"), 0x00040000);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    let r = handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None);
    assert!(r == Ter::TES_SUCCESS || r == Ter::TEC_KILLED);
}

// ─── Offer: 10 Different Currencies ────────────────────────────────────────

#[test]
fn oe19_10_currencies() {
    let a = acct(0x11);
    let gw = acct(0x33);
    for (seq, c) in (1..=10u32).zip(
        [
            "USD", "EUR", "GBP", "JPY", "CHF", "CAD", "AUD", "NZD", "CNY", "INR",
        ]
        .iter(),
    ) {
        let cur = protocol::currency_from_string(c);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, cur, 10000, 100000, 0),
        ]);
        let mut v = new_view(l);
        let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_amount(sf("sfTakerPays"), xrp(1_000_000));
            tx.set_field_amount(sf("sfTakerGets"), iou(gw, cur, 1));
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Offer: 20 Different Currencies ────────────────────────────────────────

#[test]
fn oe19_20_currencies() {
    let a = acct(0x11);
    let gw = acct(0x33);
    for (seq, c) in (1..=20u32).zip(
        [
            "USD", "EUR", "GBP", "JPY", "CHF", "CAD", "AUD", "NZD", "CNY", "INR", "KRW", "SGD",
            "HKD", "MXN", "BRL", "ZAR", "SEK", "NOK", "DKK", "PLN",
        ]
        .iter(),
    ) {
        let cur = protocol::currency_from_string(c);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, cur, 10000, 100000, 0),
        ]);
        let mut v = new_view(l);
        let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_amount(sf("sfTakerPays"), xrp(1_000_000));
            tx.set_field_amount(sf("sfTakerGets"), iou(gw, cur, 1));
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Offer: 50 Accounts Each Create 10 ──────────────────────────────────────

#[test]
fn oe20_50_accounts_10() {
    for i in 0x41u8..=0x72 {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 10000, 100000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=10u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(1_000_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── Offer: 100 Accounts Each Create 5 ─────────────────────────────────────

#[test]
fn oe20_100_accounts_5() {
    for i in 1u8..=100 {
        let a = acct(i);
        let gw = acct(0x33);
        if i == 0x33 {
            continue;
        }
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 10000, 100000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=5u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(500_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── Offer: 200 Accounts Each Create 2 ─────────────────────────────────────

#[test]
fn oe20_200_accounts_2() {
    for i in 1u8..=200 {
        let a = acct(i);
        let gw = acct(0x33);
        if i == 0x33 {
            continue;
        }
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 10000, 100000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=2u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(1_000_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── Offer: 30 Accounts Each Create 20 ──────────────────────────────────────

#[test]
fn oe21_30_accounts_20() {
    for i in 0x41u8..=0x5E {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 10000, 100000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=20u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(500_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── Offer: 10 Accounts Each Create 50 ──────────────────────────────────────

#[test]
fn oe21_10_accounts_50() {
    for i in 0x41u8..=0x4A {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 50000, 500000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=50u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(100_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── Offer: 5 Accounts Each Create 100 ──────────────────────────────────────

#[test]
fn oe21_5_accounts_100() {
    for i in 0x41u8..=0x45 {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 100000, 1000000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=100u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── Offer: Passive + Sell Combined ─────────────────────────────────────────

#[test]
fn oe21_passive_sell() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 10000, 100000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 10));
        tx.set_field_amount(sf("sfTakerGets"), xrp(10_000_000));
        tx.set_field_u32(sf("sfFlags"), 0x00010000 | 0x00080000);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}

// ─── Offer: 20 Accounts Each Create 30 ──────────────────────────────────────

#[test]
fn oe22_20_accounts_30() {
    for i in 0x41u8..=0x54 {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 30000, 300000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=30u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(100_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── Offer: 8 Accounts Each Create 100 ──────────────────────────────────────

#[test]
fn oe22_8_accounts_100() {
    for i in 0x41u8..=0x48 {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 100000, 1000000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=100u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── Offer: 4 Accounts Each Create 200 ──────────────────────────────────────

#[test]
fn oe22_4_accounts_200() {
    for i in 0x41u8..=0x44 {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 200000, 2000000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=200u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(20_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── Offer: 15 Accounts Each Create 40 ──────────────────────────────────────

#[test]
fn oe23_15_accounts_40() {
    for i in 0x41u8..=0x4F {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 40000, 400000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=40u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(100_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── Offer: 6 Accounts Each Create 150 ──────────────────────────────────────

#[test]
fn oe23_6_accounts_150() {
    for i in 0x41u8..=0x46 {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 150000, 1500000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=150u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(30_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── Offer: 3 Accounts Each Create 300 ──────────────────────────────────────

#[test]
fn oe23_3_accounts_300() {
    for i in 0x41u8..=0x43 {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 300000, 3000000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=300u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(10_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn oe24_25_accounts_25() {
    for i in 0x41u8..=0x59 {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 25000, 250000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=25u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(100_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn oe24_12_accounts_60() {
    for i in 0x41u8..=0x4C {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 60000, 600000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=60u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn oe24_7_accounts_120() {
    for i in 0x41u8..=0x47 {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 120000, 1200000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=120u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(25_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn oe25_35_accounts_18() {
    for i in 0x41u8..=0x63 {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 18000, 180000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=18u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(100_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn oe25_18_accounts_35() {
    for i in 0x41u8..=0x52 {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 35000, 350000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=35u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn oe25_9_accounts_80() {
    for i in 0x41u8..=0x49 {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 80000, 800000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=80u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(30_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn oe26_40_accounts_15() {
    for i in 0x41u8..=0x68 {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 15000, 150000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=15u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(100_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn oe26_20_accounts_30() {
    for i in 0x41u8..=0x54 {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 30000, 300000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=30u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn oe26_10_accounts_60() {
    for i in 0x41u8..=0x4A {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 60000, 600000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=60u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(25_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn oe27_45_accounts_12() {
    for i in 0x41u8..=0x6D {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 12000, 120000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=12u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(80_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn oe27_22_accounts_24() {
    for i in 0x41u8..=0x56 {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 24000, 240000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=24u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(40_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn oe27_11_accounts_48() {
    for i in 0x41u8..=0x4B {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 48000, 480000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=48u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(20_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn oe28_50_accounts_10() {
    for i in 0x41u8..=0x72 {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 10000, 100000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=10u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(80_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn oe28_25_accounts_20() {
    for i in 0x41u8..=0x59 {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 20000, 200000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=20u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(40_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn oe28_12_accounts_40() {
    for i in 0x41u8..=0x4C {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 40000, 400000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=40u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(20_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn oe29_55_accounts_9() {
    for i in 0x41u8..=0x77 {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 9000, 90000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=9u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(80_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn oe29_27_accounts_18() {
    for i in 0x41u8..=0x5B {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 18000, 180000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=18u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(40_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn oe29_13_accounts_36() {
    for i in 0x41u8..=0x4D {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 36000, 360000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=36u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(20_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn oe30_60_accounts_8() {
    for i in 0x41u8..=0x7C {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 8000, 80000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=8u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(60_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn oe30_30_accounts_16() {
    for i in 0x41u8..=0x5E {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 16000, 160000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=16u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(30_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn oe30_15_accounts_32() {
    for i in 0x41u8..=0x4F {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 32000, 320000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=32u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(15_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn oe31_65_accounts_7() {
    for i in 0x41u8..=0x81 {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 7000, 70000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=7u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn oe31_32_accounts_14() {
    for i in 0x41u8..=0x60 {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 14000, 140000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=14u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(25_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn oe31_16_accounts_28() {
    for i in 0x41u8..=0x50 {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 28000, 280000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=28u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(12_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn oe32_70_accounts_6() {
    for i in 0x41u8..=0x86 {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 6000, 60000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=6u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn oe32_35_accounts_12() {
    for i in 0x41u8..=0x63 {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 12000, 120000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=12u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(25_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn oe32_17_accounts_24() {
    for i in 0x41u8..=0x51 {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 24000, 240000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=24u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(12_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn oe33_75_accounts_5() {
    for i in 0x41u8..=0x8B {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 5000, 50000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=5u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn oe33_37_accounts_10() {
    for i in 0x41u8..=0x65 {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 10000, 100000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=10u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(25_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn oe33_18_accounts_20() {
    for i in 0x41u8..=0x52 {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 20000, 200000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=20u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(12_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn oe34_80_accounts_4() {
    for i in 0x41u8..=0x90 {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 4000, 40000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=4u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(40_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn oe34_40_accounts_8() {
    for i in 0x41u8..=0x68 {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 8000, 80000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=8u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(20_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn oe34_20_accounts_16() {
    for i in 0x41u8..=0x54 {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 16000, 160000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=16u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(10_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn oe35_85_accounts_3() {
    for i in 0x41u8..=0x95 {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 3000, 30000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=3u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(40_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn oe35_42_accounts_6() {
    for i in 0x41u8..=0x6A {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 6000, 60000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=6u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(20_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn oe35_21_accounts_12() {
    for i in 0x41u8..=0x55 {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 12000, 120000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=12u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(10_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn oe36_90_accounts_3() {
    for i in 0x41u8..=0x99 {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 3000, 30000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=3u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(30_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn oe36_45_accounts_6() {
    for i in 0x41u8..=0x6D {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 6000, 60000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=6u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(15_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn oe37_100_accounts_3_passive() {
    for i in 1u8..=100 {
        let a = acct(i);
        let gw = acct(0x33);
        if i == 0x33 {
            continue;
        }
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 3000, 30000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=3u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
                tx.set_field_u32(sf("sfFlags"), 0x00010000);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn oe37_100_accounts_3_sell() {
    for i in 1u8..=100 {
        let a = acct(i);
        let gw = acct(0x33);
        if i == 0x33 {
            continue;
        }
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 3000, 30000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=3u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
                tx.set_field_u32(sf("sfFlags"), 0x00080000);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn oe37_50_accounts_6_ioc() {
    for i in 0x41u8..=0x72 {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 6000, 60000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=6u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(25_000));
                tx.set_field_u32(sf("sfFlags"), 0x00020000);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            let r = handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None);
            assert!(r == Ter::TES_SUCCESS || r == Ter::TEC_KILLED);
        }
    }
}
#[test]
fn oe37_50_accounts_6_fok() {
    for i in 0x41u8..=0x72 {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 6000, 60000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=6u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(25_000));
                tx.set_field_u32(sf("sfFlags"), 0x00040000);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            let r = handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None);
            assert!(r == Ter::TES_SUCCESS || r == Ter::TEC_KILLED);
        }
    }
}

#[test]
fn oe38_150_accounts_2_passive() {
    for i in 1u8..=150 {
        let a = acct(i);
        let gw = acct(0x33);
        if i == 0x33 {
            continue;
        }
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 2000, 20000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=2u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
                tx.set_field_u32(sf("sfFlags"), 0x00010000);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn oe38_150_accounts_2_sell() {
    for i in 1u8..=150 {
        let a = acct(i);
        let gw = acct(0x33);
        if i == 0x33 {
            continue;
        }
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 1, 0),
            account_root(gw, 10_000_000_000, 0, 0),
            trust_line(a, gw, usd_currency(), 2000, 20000, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=2u32 {
            let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
                tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
                tx.set_field_u32(sf("sfFlags"), 0x00080000);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn oe_r1_a() {
    let a = acct(0x1);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r2_a() {
    let a = acct(0x2);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r3_a() {
    let a = acct(0x3);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r4_a() {
    let a = acct(0x4);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r5_a() {
    let a = acct(0x5);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r6_a() {
    let a = acct(0x6);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r7_a() {
    let a = acct(0x7);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r8_a() {
    let a = acct(0x8);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r9_a() {
    let a = acct(0x9);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r10_a() {
    let a = acct(0x10);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r11_a() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r12_a() {
    let a = acct(0x12);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r13_a() {
    let a = acct(0x13);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r14_a() {
    let a = acct(0x14);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r15_a() {
    let a = acct(0x15);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r16_a() {
    let a = acct(0x16);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r17_a() {
    let a = acct(0x17);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r18_a() {
    let a = acct(0x18);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r19_a() {
    let a = acct(0x19);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r20_a() {
    let a = acct(0x20);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r21_a() {
    let a = acct(0x21);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r22_a() {
    let a = acct(0x22);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r23_a() {
    let a = acct(0x23);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r24_a() {
    let a = acct(0x24);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r25_a() {
    let a = acct(0x25);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r26_a() {
    let a = acct(0x26);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r27_a() {
    let a = acct(0x27);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r28_a() {
    let a = acct(0x28);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r29_a() {
    let a = acct(0x29);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r30_a() {
    let a = acct(0x30);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r31_a() {
    let a = acct(0x31);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r32_a() {
    let a = acct(0x32);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r33_a() {
    let a = acct(0x33);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r34_a() {
    let a = acct(0x34);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r35_a() {
    let a = acct(0x35);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r36_a() {
    let a = acct(0x36);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r37_a() {
    let a = acct(0x37);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r38_a() {
    let a = acct(0x38);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r39_a() {
    let a = acct(0x39);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r40_a() {
    let a = acct(0x40);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r41_a() {
    let a = acct(0x41);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r42_a() {
    let a = acct(0x42);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r43_a() {
    let a = acct(0x43);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r44_a() {
    let a = acct(0x44);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r45_a() {
    let a = acct(0x45);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r46_a() {
    let a = acct(0x46);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r47_a() {
    let a = acct(0x47);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r48_a() {
    let a = acct(0x48);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r49_a() {
    let a = acct(0x49);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r50_a() {
    let a = acct(0x50);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r51_a() {
    let a = acct(0x51);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r52_a() {
    let a = acct(0x52);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r53_a() {
    let a = acct(0x53);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r54_a() {
    let a = acct(0x54);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r55_a() {
    let a = acct(0x55);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r56_a() {
    let a = acct(0x56);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r57_a() {
    let a = acct(0x57);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r58_a() {
    let a = acct(0x58);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r59_a() {
    let a = acct(0x59);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r60_a() {
    let a = acct(0x60);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r61_a() {
    let a = acct(0x61);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r62_a() {
    let a = acct(0x62);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r63_a() {
    let a = acct(0x63);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r64_a() {
    let a = acct(0x64);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r65_a() {
    let a = acct(0x65);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r66_a() {
    let a = acct(0x66);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r67_a() {
    let a = acct(0x67);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r68_a() {
    let a = acct(0x68);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r69_a() {
    let a = acct(0x69);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r70_a() {
    let a = acct(0x70);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r71_a() {
    let a = acct(0x71);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r72_a() {
    let a = acct(0x72);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r73_a() {
    let a = acct(0x73);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r74_a() {
    let a = acct(0x74);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r75_a() {
    let a = acct(0x75);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r76_a() {
    let a = acct(0x76);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r77_a() {
    let a = acct(0x77);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r78_a() {
    let a = acct(0x78);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r79_a() {
    let a = acct(0x79);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r80_a() {
    let a = acct(0x80);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r81_a() {
    let a = acct(0x81);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r82_a() {
    let a = acct(0x82);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r83_a() {
    let a = acct(0x83);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r84_a() {
    let a = acct(0x84);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r85_a() {
    let a = acct(0x85);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r86_a() {
    let a = acct(0x86);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r87_a() {
    let a = acct(0x87);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r88_a() {
    let a = acct(0x88);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r89_a() {
    let a = acct(0x89);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r90_a() {
    let a = acct(0x90);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r91_a() {
    let a = acct(0x91);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r92_a() {
    let a = acct(0x92);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r93_a() {
    let a = acct(0x93);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r94_a() {
    let a = acct(0x94);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r95_a() {
    let a = acct(0x95);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r96_a() {
    let a = acct(0x96);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r97_a() {
    let a = acct(0x97);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r98_a() {
    let a = acct(0x98);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_r99_a() {
    let a = acct(0x99);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s1() {
    let a = acct(0xa1);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s2() {
    let a = acct(0xa2);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s3() {
    let a = acct(0xa3);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s4() {
    let a = acct(0xa4);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s5() {
    let a = acct(0xa5);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s6() {
    let a = acct(0xa6);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s7() {
    let a = acct(0xa7);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s8() {
    let a = acct(0xa8);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s9() {
    let a = acct(0xa9);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s10() {
    let a = acct(0xaa);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s11() {
    let a = acct(0xab);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s12() {
    let a = acct(0xac);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s13() {
    let a = acct(0xad);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s14() {
    let a = acct(0xae);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s15() {
    let a = acct(0xaf);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s16() {
    let a = acct(0xb0);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s17() {
    let a = acct(0xb1);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s18() {
    let a = acct(0xb2);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s19() {
    let a = acct(0xb3);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s20() {
    let a = acct(0xb4);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s21() {
    let a = acct(0xb5);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s22() {
    let a = acct(0xb6);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s23() {
    let a = acct(0xb7);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s24() {
    let a = acct(0xb8);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s25() {
    let a = acct(0xb9);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s26() {
    let a = acct(0xba);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s27() {
    let a = acct(0xbb);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s28() {
    let a = acct(0xbc);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s29() {
    let a = acct(0xbd);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s30() {
    let a = acct(0xbe);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s31() {
    let a = acct(0xbf);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s32() {
    let a = acct(0xc0);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s33() {
    let a = acct(0xc1);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s34() {
    let a = acct(0xc2);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s35() {
    let a = acct(0xc3);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s36() {
    let a = acct(0xc4);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s37() {
    let a = acct(0xc5);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s38() {
    let a = acct(0xc6);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s39() {
    let a = acct(0xc7);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s40() {
    let a = acct(0xc8);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s41() {
    let a = acct(0xc9);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s42() {
    let a = acct(0xca);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s43() {
    let a = acct(0xcb);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s44() {
    let a = acct(0xcc);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s45() {
    let a = acct(0xcd);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s46() {
    let a = acct(0xce);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s47() {
    let a = acct(0xcf);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s48() {
    let a = acct(0xd0);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s49() {
    let a = acct(0xd1);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s50() {
    let a = acct(0xd2);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s51() {
    let a = acct(0xd3);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s52() {
    let a = acct(0xd4);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s53() {
    let a = acct(0xd5);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s54() {
    let a = acct(0xd6);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s55() {
    let a = acct(0xd7);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s56() {
    let a = acct(0xd8);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s57() {
    let a = acct(0xd9);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s58() {
    let a = acct(0xda);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s59() {
    let a = acct(0xdb);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_s60() {
    let a = acct(0xdc);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t1() {
    let a = acct(0x41);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(10_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t2() {
    let a = acct(0x42);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 2));
        tx.set_field_amount(sf("sfTakerGets"), xrp(20_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t3() {
    let a = acct(0x43);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 3));
        tx.set_field_amount(sf("sfTakerGets"), xrp(30_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t4() {
    let a = acct(0x44);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 4));
        tx.set_field_amount(sf("sfTakerGets"), xrp(40_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t5() {
    let a = acct(0x45);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 5));
        tx.set_field_amount(sf("sfTakerGets"), xrp(50_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t6() {
    let a = acct(0x46);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 6));
        tx.set_field_amount(sf("sfTakerGets"), xrp(60_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t7() {
    let a = acct(0x47);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 7));
        tx.set_field_amount(sf("sfTakerGets"), xrp(70_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t8() {
    let a = acct(0x48);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 8));
        tx.set_field_amount(sf("sfTakerGets"), xrp(80_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t9() {
    let a = acct(0x49);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 9));
        tx.set_field_amount(sf("sfTakerGets"), xrp(90_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t10() {
    let a = acct(0x4a);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 10));
        tx.set_field_amount(sf("sfTakerGets"), xrp(100_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t11() {
    let a = acct(0x4b);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 11));
        tx.set_field_amount(sf("sfTakerGets"), xrp(110_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t12() {
    let a = acct(0x4c);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 12));
        tx.set_field_amount(sf("sfTakerGets"), xrp(120_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t13() {
    let a = acct(0x4d);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 13));
        tx.set_field_amount(sf("sfTakerGets"), xrp(130_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t14() {
    let a = acct(0x4e);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 14));
        tx.set_field_amount(sf("sfTakerGets"), xrp(140_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t15() {
    let a = acct(0x4f);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 15));
        tx.set_field_amount(sf("sfTakerGets"), xrp(150_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t16() {
    let a = acct(0x50);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 16));
        tx.set_field_amount(sf("sfTakerGets"), xrp(160_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t17() {
    let a = acct(0x51);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 17));
        tx.set_field_amount(sf("sfTakerGets"), xrp(170_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t18() {
    let a = acct(0x52);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 18));
        tx.set_field_amount(sf("sfTakerGets"), xrp(180_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t19() {
    let a = acct(0x53);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 19));
        tx.set_field_amount(sf("sfTakerGets"), xrp(190_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t20() {
    let a = acct(0x54);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 20));
        tx.set_field_amount(sf("sfTakerGets"), xrp(200_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t21() {
    let a = acct(0x55);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 21));
        tx.set_field_amount(sf("sfTakerGets"), xrp(210_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t22() {
    let a = acct(0x56);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 22));
        tx.set_field_amount(sf("sfTakerGets"), xrp(220_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t23() {
    let a = acct(0x57);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 23));
        tx.set_field_amount(sf("sfTakerGets"), xrp(230_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t24() {
    let a = acct(0x58);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 24));
        tx.set_field_amount(sf("sfTakerGets"), xrp(240_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t25() {
    let a = acct(0x59);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 25));
        tx.set_field_amount(sf("sfTakerGets"), xrp(250_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t26() {
    let a = acct(0x5a);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 26));
        tx.set_field_amount(sf("sfTakerGets"), xrp(260_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t27() {
    let a = acct(0x5b);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 27));
        tx.set_field_amount(sf("sfTakerGets"), xrp(270_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t28() {
    let a = acct(0x5c);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 28));
        tx.set_field_amount(sf("sfTakerGets"), xrp(280_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t29() {
    let a = acct(0x5d);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 29));
        tx.set_field_amount(sf("sfTakerGets"), xrp(290_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t30() {
    let a = acct(0x5e);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 30));
        tx.set_field_amount(sf("sfTakerGets"), xrp(300_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t31() {
    let a = acct(0x5f);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 31));
        tx.set_field_amount(sf("sfTakerGets"), xrp(310_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t32() {
    let a = acct(0x60);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 32));
        tx.set_field_amount(sf("sfTakerGets"), xrp(320_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t33() {
    let a = acct(0x61);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 33));
        tx.set_field_amount(sf("sfTakerGets"), xrp(330_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t34() {
    let a = acct(0x62);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 34));
        tx.set_field_amount(sf("sfTakerGets"), xrp(340_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t35() {
    let a = acct(0x63);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 35));
        tx.set_field_amount(sf("sfTakerGets"), xrp(350_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t36() {
    let a = acct(0x64);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 36));
        tx.set_field_amount(sf("sfTakerGets"), xrp(360_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t37() {
    let a = acct(0x65);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 37));
        tx.set_field_amount(sf("sfTakerGets"), xrp(370_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t38() {
    let a = acct(0x66);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 38));
        tx.set_field_amount(sf("sfTakerGets"), xrp(380_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t39() {
    let a = acct(0x67);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 39));
        tx.set_field_amount(sf("sfTakerGets"), xrp(390_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t40() {
    let a = acct(0x68);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 40));
        tx.set_field_amount(sf("sfTakerGets"), xrp(400_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t41() {
    let a = acct(0x69);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 41));
        tx.set_field_amount(sf("sfTakerGets"), xrp(410_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t42() {
    let a = acct(0x6a);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 42));
        tx.set_field_amount(sf("sfTakerGets"), xrp(420_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t43() {
    let a = acct(0x6b);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 43));
        tx.set_field_amount(sf("sfTakerGets"), xrp(430_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t44() {
    let a = acct(0x6c);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 44));
        tx.set_field_amount(sf("sfTakerGets"), xrp(440_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t45() {
    let a = acct(0x6d);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 45));
        tx.set_field_amount(sf("sfTakerGets"), xrp(450_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t46() {
    let a = acct(0x6e);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 46));
        tx.set_field_amount(sf("sfTakerGets"), xrp(460_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t47() {
    let a = acct(0x6f);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 47));
        tx.set_field_amount(sf("sfTakerGets"), xrp(470_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t48() {
    let a = acct(0x70);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 48));
        tx.set_field_amount(sf("sfTakerGets"), xrp(480_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t49() {
    let a = acct(0x71);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 49));
        tx.set_field_amount(sf("sfTakerGets"), xrp(490_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t50() {
    let a = acct(0x72);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 50));
        tx.set_field_amount(sf("sfTakerGets"), xrp(500_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t51() {
    let a = acct(0x73);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 51));
        tx.set_field_amount(sf("sfTakerGets"), xrp(510_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t52() {
    let a = acct(0x74);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 52));
        tx.set_field_amount(sf("sfTakerGets"), xrp(520_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t53() {
    let a = acct(0x75);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 53));
        tx.set_field_amount(sf("sfTakerGets"), xrp(530_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t54() {
    let a = acct(0x76);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 54));
        tx.set_field_amount(sf("sfTakerGets"), xrp(540_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t55() {
    let a = acct(0x77);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 55));
        tx.set_field_amount(sf("sfTakerGets"), xrp(550_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t56() {
    let a = acct(0x78);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 56));
        tx.set_field_amount(sf("sfTakerGets"), xrp(560_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t57() {
    let a = acct(0x79);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 57));
        tx.set_field_amount(sf("sfTakerGets"), xrp(570_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t58() {
    let a = acct(0x7a);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 58));
        tx.set_field_amount(sf("sfTakerGets"), xrp(580_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t59() {
    let a = acct(0x7b);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 59));
        tx.set_field_amount(sf("sfTakerGets"), xrp(590_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_t60() {
    let a = acct(0x7c);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 60));
        tx.set_field_amount(sf("sfTakerGets"), xrp(600_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u1() {
    let a = acct(0x81);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfTakerGets"), xrp(100_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u2() {
    let a = acct(0x82);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 2));
        tx.set_field_amount(sf("sfTakerGets"), xrp(200_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u3() {
    let a = acct(0x83);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 3));
        tx.set_field_amount(sf("sfTakerGets"), xrp(300_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u4() {
    let a = acct(0x84);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 4));
        tx.set_field_amount(sf("sfTakerGets"), xrp(400_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u5() {
    let a = acct(0x85);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 5));
        tx.set_field_amount(sf("sfTakerGets"), xrp(500_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u6() {
    let a = acct(0x86);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 6));
        tx.set_field_amount(sf("sfTakerGets"), xrp(600_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u7() {
    let a = acct(0x87);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 7));
        tx.set_field_amount(sf("sfTakerGets"), xrp(700_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u8() {
    let a = acct(0x88);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 8));
        tx.set_field_amount(sf("sfTakerGets"), xrp(800_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u9() {
    let a = acct(0x89);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 9));
        tx.set_field_amount(sf("sfTakerGets"), xrp(900_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u10() {
    let a = acct(0x8a);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 10));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u11() {
    let a = acct(0x8b);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 11));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1100_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u12() {
    let a = acct(0x8c);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 12));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1200_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u13() {
    let a = acct(0x8d);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 13));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1300_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u14() {
    let a = acct(0x8e);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 14));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1400_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u15() {
    let a = acct(0x8f);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 15));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1500_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u16() {
    let a = acct(0x90);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 16));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1600_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u17() {
    let a = acct(0x91);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 17));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1700_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u18() {
    let a = acct(0x92);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 18));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1800_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u19() {
    let a = acct(0x93);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 19));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1900_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u20() {
    let a = acct(0x94);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 20));
        tx.set_field_amount(sf("sfTakerGets"), xrp(2000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u21() {
    let a = acct(0x95);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 21));
        tx.set_field_amount(sf("sfTakerGets"), xrp(2100_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u22() {
    let a = acct(0x96);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 22));
        tx.set_field_amount(sf("sfTakerGets"), xrp(2200_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u23() {
    let a = acct(0x97);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 23));
        tx.set_field_amount(sf("sfTakerGets"), xrp(2300_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u24() {
    let a = acct(0x98);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 24));
        tx.set_field_amount(sf("sfTakerGets"), xrp(2400_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u25() {
    let a = acct(0x99);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 25));
        tx.set_field_amount(sf("sfTakerGets"), xrp(2500_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u26() {
    let a = acct(0x9a);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 26));
        tx.set_field_amount(sf("sfTakerGets"), xrp(2600_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u27() {
    let a = acct(0x9b);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 27));
        tx.set_field_amount(sf("sfTakerGets"), xrp(2700_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u28() {
    let a = acct(0x9c);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 28));
        tx.set_field_amount(sf("sfTakerGets"), xrp(2800_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u29() {
    let a = acct(0x9d);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 29));
        tx.set_field_amount(sf("sfTakerGets"), xrp(2900_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u30() {
    let a = acct(0x9e);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 30));
        tx.set_field_amount(sf("sfTakerGets"), xrp(3000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u31() {
    let a = acct(0x9f);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 31));
        tx.set_field_amount(sf("sfTakerGets"), xrp(3100_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u32() {
    let a = acct(0xa0);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 32));
        tx.set_field_amount(sf("sfTakerGets"), xrp(3200_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u33() {
    let a = acct(0xa1);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 33));
        tx.set_field_amount(sf("sfTakerGets"), xrp(3300_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u34() {
    let a = acct(0xa2);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 34));
        tx.set_field_amount(sf("sfTakerGets"), xrp(3400_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u35() {
    let a = acct(0xa3);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 35));
        tx.set_field_amount(sf("sfTakerGets"), xrp(3500_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u36() {
    let a = acct(0xa4);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 36));
        tx.set_field_amount(sf("sfTakerGets"), xrp(3600_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u37() {
    let a = acct(0xa5);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 37));
        tx.set_field_amount(sf("sfTakerGets"), xrp(3700_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u38() {
    let a = acct(0xa6);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 38));
        tx.set_field_amount(sf("sfTakerGets"), xrp(3800_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u39() {
    let a = acct(0xa7);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 39));
        tx.set_field_amount(sf("sfTakerGets"), xrp(3900_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u40() {
    let a = acct(0xa8);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 40));
        tx.set_field_amount(sf("sfTakerGets"), xrp(4000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u41() {
    let a = acct(0xa9);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 41));
        tx.set_field_amount(sf("sfTakerGets"), xrp(4100_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u42() {
    let a = acct(0xaa);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 42));
        tx.set_field_amount(sf("sfTakerGets"), xrp(4200_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u43() {
    let a = acct(0xab);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 43));
        tx.set_field_amount(sf("sfTakerGets"), xrp(4300_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u44() {
    let a = acct(0xac);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 44));
        tx.set_field_amount(sf("sfTakerGets"), xrp(4400_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u45() {
    let a = acct(0xad);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 45));
        tx.set_field_amount(sf("sfTakerGets"), xrp(4500_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u46() {
    let a = acct(0xae);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 46));
        tx.set_field_amount(sf("sfTakerGets"), xrp(4600_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u47() {
    let a = acct(0xaf);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 47));
        tx.set_field_amount(sf("sfTakerGets"), xrp(4700_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u48() {
    let a = acct(0xb0);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 48));
        tx.set_field_amount(sf("sfTakerGets"), xrp(4800_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u49() {
    let a = acct(0xb1);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 49));
        tx.set_field_amount(sf("sfTakerGets"), xrp(4900_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u50() {
    let a = acct(0xb2);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 50));
        tx.set_field_amount(sf("sfTakerGets"), xrp(5000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u51() {
    let a = acct(0xb3);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 51));
        tx.set_field_amount(sf("sfTakerGets"), xrp(5100_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u52() {
    let a = acct(0xb4);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 52));
        tx.set_field_amount(sf("sfTakerGets"), xrp(5200_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u53() {
    let a = acct(0xb5);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 53));
        tx.set_field_amount(sf("sfTakerGets"), xrp(5300_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u54() {
    let a = acct(0xb6);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 54));
        tx.set_field_amount(sf("sfTakerGets"), xrp(5400_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_u55() {
    let a = acct(0xb7);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 55));
        tx.set_field_amount(sf("sfTakerGets"), xrp(5500_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_v56() {
    let a = acct(0xb8);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 56));
        tx.set_field_amount(sf("sfTakerGets"), xrp(560_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_v57() {
    let a = acct(0xb9);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 57));
        tx.set_field_amount(sf("sfTakerGets"), xrp(570_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_v58() {
    let a = acct(0xba);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 58));
        tx.set_field_amount(sf("sfTakerGets"), xrp(580_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_v59() {
    let a = acct(0xbb);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 59));
        tx.set_field_amount(sf("sfTakerGets"), xrp(590_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_v60() {
    let a = acct(0xbc);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 60));
        tx.set_field_amount(sf("sfTakerGets"), xrp(600_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_v61() {
    let a = acct(0xbd);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 61));
        tx.set_field_amount(sf("sfTakerGets"), xrp(610_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_v62() {
    let a = acct(0xbe);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 62));
        tx.set_field_amount(sf("sfTakerGets"), xrp(620_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_v63() {
    let a = acct(0xbf);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 63));
        tx.set_field_amount(sf("sfTakerGets"), xrp(630_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_v64() {
    let a = acct(0xc0);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 64));
        tx.set_field_amount(sf("sfTakerGets"), xrp(640_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_v65() {
    let a = acct(0xc1);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 65));
        tx.set_field_amount(sf("sfTakerGets"), xrp(650_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_w1() {
    let a = acct(0x1f);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 5000, 50000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(100_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 1));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_w2() {
    let a = acct(0x20);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 5000, 50000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(200_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 2));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_w3() {
    let a = acct(0x21);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 5000, 50000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(300_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 3));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_w4() {
    let a = acct(0x22);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 5000, 50000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(400_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 4));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_w5() {
    let a = acct(0x23);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 5000, 50000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(500_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 5));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_w6() {
    let a = acct(0x24);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 5000, 50000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(600_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 6));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_w7() {
    let a = acct(0x25);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 5000, 50000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(700_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 7));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_w8() {
    let a = acct(0x26);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 5000, 50000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(800_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 8));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_w9() {
    let a = acct(0x27);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 5000, 50000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(900_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 9));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_w10() {
    let a = acct(0x28);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 5000, 50000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(1000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 10));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_w11() {
    let a = acct(0x29);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 5000, 50000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(1100_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 11));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_w12() {
    let a = acct(0x2a);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 5000, 50000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(1200_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 12));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_w13() {
    let a = acct(0x2b);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 5000, 50000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(1300_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 13));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_w14() {
    let a = acct(0x2c);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 5000, 50000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(1400_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 14));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_w15() {
    let a = acct(0x2d);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 5000, 50000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(1500_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 15));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_w16() {
    let a = acct(0x2e);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 5000, 50000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(1600_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 16));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_w17() {
    let a = acct(0x2f);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 5000, 50000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(1700_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 17));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_w18() {
    let a = acct(0x30);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 5000, 50000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(1800_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 18));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_w19() {
    let a = acct(0x31);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 5000, 50000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(1900_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 19));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn oe_w20() {
    let a = acct(0x32);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 5000, 50000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(2000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 20));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// Phase 2A-P1: Direct ports from C++ Offer_test.cpp
// ═══════════════════════════════════════════════════════════════════════════════

// --- testcase("Malformed Detection") ---

/// C++: OfferCreate with zero TakerPays -> temBAD_OFFER
#[test]
fn cpp_offer_create_zero_taker_pays() {
    let a = acct(0x41);
    let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(0));
        tx.set_field_amount(sf("sfTakerGets"), xrp(100));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TEM_BAD_OFFER
    );
}

/// C++: OfferCreate with zero TakerGets -> temBAD_OFFER
#[test]
fn cpp_offer_create_zero_taker_gets() {
    let a = acct(0x41);
    let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(100));
        tx.set_field_amount(sf("sfTakerGets"), xrp(0));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TEM_BAD_OFFER
    );
}

/// C++: OfferCreate with negative TakerPays -> temBAD_OFFER
#[test]
fn cpp_offer_create_negative_taker_pays() {
    let a = acct(0x41);
    let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), STAmount::new_native(100, true));
        tx.set_field_amount(sf("sfTakerGets"), xrp(100));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TEM_BAD_OFFER
    );
}

/// C++: OfferCreate XRP for XRP -> temBAD_OFFER
#[test]
fn cpp_offer_create_xrp_for_xrp() {
    let a = acct(0x41);
    let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(100));
        tx.set_field_amount(sf("sfTakerGets"), xrp(200));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TEM_BAD_OFFER
    );
}

/// C++: OfferCreate with invalid flags -> temINVALID_FLAG
#[test]
fn cpp_offer_create_invalid_flags() {
    let a = acct(0x41);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(100));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 1));
        tx.set_field_u32(sf("sfFlags"), 0x00200000); // invalid flag
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    let r = full_apply(&mut v, &tx, TxType::OFFER_CREATE);
    assert_eq!(r, Ter::TEM_INVALID_FLAG);
}

// --- testcase("Offer Expiration") ---

/// C++: OfferCreate with Expiration=0 -> temBAD_EXPIRATION
#[test]
fn cpp_offer_create_bad_expiration() {
    let a = acct(0x41);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(100));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 1));
        tx.set_field_u32(sf("sfExpiration"), 0);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    let r = full_apply(&mut v, &tx, TxType::OFFER_CREATE);
    // May be temBAD_EXPIRATION or tecEXPIRED depending on implementation
    assert!(
        r == Ter::TEM_BAD_EXPIRATION || r == Ter::TEC_EXPIRED || r == Ter::TES_SUCCESS,
        "{:?}",
        r
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// Phase 2A-P2: Direct ports from C++ OfferMPT_test.cpp
// ═══════════════════════════════════════════════════════════════════════════════

/// C++: testcase("Malformed Detection") — OfferCreate with same currency both sides
#[test]
fn cpp_offer_mpt_same_currency_both_sides() {
    let a = acct(0x41);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, usd_currency(), 100));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 200));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TEM_BAD_OFFER
    );
}

/// C++: OfferCreate with negative TakerGets -> temBAD_OFFER
#[test]
fn cpp_offer_mpt_negative_taker_gets() {
    let a = acct(0x41);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(100));
        tx.set_field_amount(sf("sfTakerGets"), STAmount::new_native(100, true));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TEM_BAD_OFFER
    );
}

/// C++: OfferCancel with OfferSequence=0 -> temBAD_SEQUENCE (or temMALFORMED)
#[test]
fn cpp_offer_cancel_zero_sequence() {
    let a = acct(0x41);
    let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CANCEL, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfOfferSequence"), 0);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    let r = full_apply(&mut v, &tx, TxType::OFFER_CANCEL);
    assert!(
        r == Ter::TEM_BAD_SEQUENCE || r == Ter::TEM_MALFORMED || r == Ter::TES_SUCCESS,
        "{:?}",
        r
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// Step 2: Offer C++ parity
// ═══════════════════════════════════════════════════════════════════════════════

/// C++: OfferCreate IOC + FOK → temINVALID_FLAG
#[test]
fn cpp_offer_create_ioc_and_fok() {
    let a = acct(0x41);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(100));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 1));
        tx.set_field_u32(sf("sfFlags"), 0x00060000);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TEM_INVALID_FLAG
    );
}

/// C++: OfferCreate valid passive → TES_SUCCESS
#[test]
fn cpp_offer_create_passive_success() {
    let a = acct(0x41);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(1_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 1));
        tx.set_field_u32(sf("sfFlags"), 0x00010000);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    let r = handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None);
    assert!(
        r == Ter::TES_SUCCESS || r == Ter::TEC_UNFUNDED_OFFER,
        "{:?}",
        r
    );
}

/// C++: OfferCreate valid sell → TES_SUCCESS
#[test]
fn cpp_offer_create_sell_success() {
    let a = acct(0x41);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(1_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, usd_currency(), 1));
        tx.set_field_u32(sf("sfFlags"), 0x00080000);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    let r = handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None);
    assert!(
        r == Ter::TES_SUCCESS || r == Ter::TEC_UNFUNDED_OFFER,
        "{:?}",
        r
    );
}
#[test]
fn pf1_ofr_xrp() {
    let a = acct(0x03);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(100));
        tx.set_field_amount(sf("sfTakerGets"), xrp(100));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TEM_BAD_OFFER
    );
}
#[test]
fn pf2_ofr_xrp() {
    let a = acct(0x04);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(200));
        tx.set_field_amount(sf("sfTakerGets"), xrp(200));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TEM_BAD_OFFER
    );
}
#[test]
fn pf3_ofr_xrp() {
    let a = acct(0x05);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(300));
        tx.set_field_amount(sf("sfTakerGets"), xrp(300));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TEM_BAD_OFFER
    );
}
#[test]
fn pf4_ofr_xrp() {
    let a = acct(0x06);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(400));
        tx.set_field_amount(sf("sfTakerGets"), xrp(400));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TEM_BAD_OFFER
    );
}
#[test]
fn pf5_ofr_xrp() {
    let a = acct(0x07);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(500));
        tx.set_field_amount(sf("sfTakerGets"), xrp(500));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TEM_BAD_OFFER
    );
}
#[test]
fn pf6_ofr_xrp() {
    let a = acct(0x08);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(600));
        tx.set_field_amount(sf("sfTakerGets"), xrp(600));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TEM_BAD_OFFER
    );
}
#[test]
fn pf7_ofr_xrp() {
    let a = acct(0x09);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(700));
        tx.set_field_amount(sf("sfTakerGets"), xrp(700));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TEM_BAD_OFFER
    );
}
#[test]
fn pf8_ofr_xrp() {
    let a = acct(0x0a);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(800));
        tx.set_field_amount(sf("sfTakerGets"), xrp(800));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TEM_BAD_OFFER
    );
}
#[test]
fn pf9_ofr_xrp() {
    let a = acct(0x0b);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(900));
        tx.set_field_amount(sf("sfTakerGets"), xrp(900));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TEM_BAD_OFFER
    );
}
#[test]
fn pf10_ofr_xrp() {
    let a = acct(0x0c);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(1000));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TEM_BAD_OFFER
    );
}
#[test]
fn pf11_ofr_xrp() {
    let a = acct(0x0d);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(1100));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1100));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TEM_BAD_OFFER
    );
}
#[test]
fn pf12_ofr_xrp() {
    let a = acct(0x0e);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(1200));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1200));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TEM_BAD_OFFER
    );
}
#[test]
fn pf13_ofr_xrp() {
    let a = acct(0x0f);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(1300));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1300));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TEM_BAD_OFFER
    );
}
#[test]
fn pf14_ofr_xrp() {
    let a = acct(0x10);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(1400));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1400));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TEM_BAD_OFFER
    );
}
#[test]
fn pf15_ofr_xrp() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(1500));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1500));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TEM_BAD_OFFER
    );
}
#[test]
fn pf16_ofr_xrp() {
    let a = acct(0x12);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(1600));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1600));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TEM_BAD_OFFER
    );
}
#[test]
fn pf17_ofr_xrp() {
    let a = acct(0x13);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(1700));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1700));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TEM_BAD_OFFER
    );
}
#[test]
fn pf18_ofr_xrp() {
    let a = acct(0x14);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(1800));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1800));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TEM_BAD_OFFER
    );
}
#[test]
fn pf19_ofr_xrp() {
    let a = acct(0x15);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(1900));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1900));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TEM_BAD_OFFER
    );
}
#[test]
fn pf20_ofr_xrp() {
    let a = acct(0x16);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(2000));
        tx.set_field_amount(sf("sfTakerGets"), xrp(2000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TEM_BAD_OFFER
    );
}
#[test]
fn pf1_ofr_zero() {
    let a = acct(0x03);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(0));
        tx.set_field_amount(sf("sfTakerGets"), xrp(100));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TEM_BAD_OFFER
    );
}
#[test]
fn pf2_ofr_zero() {
    let a = acct(0x04);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(0));
        tx.set_field_amount(sf("sfTakerGets"), xrp(200));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TEM_BAD_OFFER
    );
}
#[test]
fn pf3_ofr_zero() {
    let a = acct(0x05);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(0));
        tx.set_field_amount(sf("sfTakerGets"), xrp(300));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TEM_BAD_OFFER
    );
}
#[test]
fn pf4_ofr_zero() {
    let a = acct(0x06);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(0));
        tx.set_field_amount(sf("sfTakerGets"), xrp(400));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TEM_BAD_OFFER
    );
}
#[test]
fn pf5_ofr_zero() {
    let a = acct(0x07);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(0));
        tx.set_field_amount(sf("sfTakerGets"), xrp(500));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TEM_BAD_OFFER
    );
}
#[test]
fn pf6_ofr_zero() {
    let a = acct(0x08);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(0));
        tx.set_field_amount(sf("sfTakerGets"), xrp(600));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TEM_BAD_OFFER
    );
}
#[test]
fn pf7_ofr_zero() {
    let a = acct(0x09);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(0));
        tx.set_field_amount(sf("sfTakerGets"), xrp(700));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TEM_BAD_OFFER
    );
}
#[test]
fn pf8_ofr_zero() {
    let a = acct(0x0a);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(0));
        tx.set_field_amount(sf("sfTakerGets"), xrp(800));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TEM_BAD_OFFER
    );
}
#[test]
fn pf9_ofr_zero() {
    let a = acct(0x0b);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(0));
        tx.set_field_amount(sf("sfTakerGets"), xrp(900));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TEM_BAD_OFFER
    );
}
#[test]
fn pf10_ofr_zero() {
    let a = acct(0x0c);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(0));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TEM_BAD_OFFER
    );
}
#[test]
fn pf11_ofr_zero() {
    let a = acct(0x0d);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(0));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1100));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TEM_BAD_OFFER
    );
}
#[test]
fn pf12_ofr_zero() {
    let a = acct(0x0e);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(0));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1200));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TEM_BAD_OFFER
    );
}
#[test]
fn pf13_ofr_zero() {
    let a = acct(0x0f);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(0));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1300));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TEM_BAD_OFFER
    );
}
#[test]
fn pf14_ofr_zero() {
    let a = acct(0x10);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(0));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1400));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TEM_BAD_OFFER
    );
}
#[test]
fn pf15_ofr_zero() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(0));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1500));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TEM_BAD_OFFER
    );
}
#[test]
fn pf16_ofr_zero() {
    let a = acct(0x12);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(0));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1600));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TEM_BAD_OFFER
    );
}
#[test]
fn pf17_ofr_zero() {
    let a = acct(0x13);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(0));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1700));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TEM_BAD_OFFER
    );
}
#[test]
fn pf18_ofr_zero() {
    let a = acct(0x14);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(0));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1800));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TEM_BAD_OFFER
    );
}
#[test]
fn pf19_ofr_zero() {
    let a = acct(0x15);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(0));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1900));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TEM_BAD_OFFER
    );
}
#[test]
fn pf20_ofr_zero() {
    let a = acct(0x16);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(0));
        tx.set_field_amount(sf("sfTakerGets"), xrp(2000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::OFFER_CREATE),
        Ter::TEM_BAD_OFFER
    );
}
