#![allow(
    unused_imports,
    unused_variables,
    unused_mut,
    dead_code,
    unused_comparisons
)]
//! Directory and owner count integration tests — exercises directory insert,
//! owner count tracking, and ledger entry lifecycle.
//!
//! Ported from C++ Directory_test.cpp and View_test.cpp.

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

// ─── Owner Count Tracking ───────────────────────────────────────────────────

#[test]
fn dir_trust_increases_owner() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(gw, 5_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::TRUST_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(
            sf("sfLimitAmount"),
            STAmount::from_iou_amount(
                sf_generic(),
                IOUAmount::from_parts(1000, 0).expect("a"),
                Issue::new(usd_currency(), gw),
            ),
        );
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    full_apply(&mut v, &tx, TxType::TRUST_SET);
    let k = protocol::account_keylet(acct_id(a));
    let sle = v.peek(k).ok().flatten().unwrap();
    assert!(sle.get_field_u32(sf("sfOwnerCount")) >= 1);
}
#[test]
fn dir_offer_increases_owner() {
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
    handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None);
    let k = protocol::account_keylet(acct_id(a));
    let sle = v.peek(k).ok().flatten().unwrap();
    assert!(sle.get_field_u32(sf("sfOwnerCount")) >= 2);
}
#[test]
fn dir_escrow_increases_owner() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = build_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ESCROW_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_account_id(sf("sfDestination"), b);
        tx.set_field_amount(sf("sfAmount"), xrp(1_000_000));
        tx.set_field_u32(sf("sfFinishAfter"), 500);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    full_apply(&mut v, &tx, TxType::ESCROW_CREATE);
    let k = protocol::account_keylet(acct_id(a));
    let sle = v.peek(k).ok().flatten().unwrap();
    assert!(sle.get_field_u32(sf("sfOwnerCount")) >= 1);
}
#[test]
fn dir_paychan_increases_owner() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = build_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::PAYCHAN_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_account_id(sf("sfDestination"), b);
        tx.set_field_amount(sf("sfAmount"), xrp(1_000_000));
        tx.set_field_u32(sf("sfSettleDelay"), 3600);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE);
    let k = protocol::account_keylet(acct_id(a));
    let sle = v.peek(k).ok().flatten().unwrap();
    assert!(sle.get_field_u32(sf("sfOwnerCount")) >= 1);
}
#[test]
fn dir_check_increases_owner() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = build_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::CHECK_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_account_id(sf("sfDestination"), b);
        tx.set_field_amount(sf("sfSendMax"), xrp(1_000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    full_apply(&mut v, &tx, TxType::CHECK_CREATE);
    let k = protocol::account_keylet(acct_id(a));
    let sle = v.peek(k).ok().flatten().unwrap();
    assert!(sle.get_field_u32(sf("sfOwnerCount")) >= 1);
}
#[test]
fn dir_ticket_increases_owner() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::TICKET_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfTicketCount"), 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    full_apply(&mut v, &tx, TxType::TICKET_CREATE);
    let k = protocol::account_keylet(acct_id(a));
    let sle = v.peek(k).ok().flatten().unwrap();
    assert!(sle.get_field_u32(sf("sfOwnerCount")) >= 3);
}
#[test]
fn dir_signer_increases_owner() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let mut e = STArray::new(sf("sfSignerEntries"));
    let mut s = STObject::make_inner_object(sf("sfSignerEntry"));
    s.set_account_id(sf("sfAccount"), acct(0x22));
    s.set_field_u16(sf("sfSignerWeight"), 1);
    e.push_back(s);
    let tx = STTx::new(TxType::SIGNER_LIST_SET, move |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfSignerQuorum"), 1);
        tx.set_field_array(sf("sfSignerEntries"), e.clone());
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    full_apply(&mut v, &tx, TxType::SIGNER_LIST_SET);
    let k = protocol::account_keylet(acct_id(a));
    let sle = v.peek(k).ok().flatten().unwrap();
    assert!(sle.get_field_u32(sf("sfOwnerCount")) >= 1);
}
#[test]
fn dir_nft_increases_owner() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    full_apply(&mut v, &tx, TxType::NFTOKEN_MINT);
    let k = protocol::account_keylet(acct_id(a));
    let sle = v.peek(k).ok().flatten().unwrap();
    assert!(sle.get_field_u32(sf("sfOwnerCount")) >= 1);
}
#[test]
fn dir_deposit_preauth_increases_owner() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = build_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::DEPOSIT_PREAUTH, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_account_id(sf("sfAuthorize"), b);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    full_apply(&mut v, &tx, TxType::DEPOSIT_PREAUTH);
    let k = protocol::account_keylet(acct_id(a));
    let sle = v.peek(k).ok().flatten().unwrap();
    assert!(sle.get_field_u32(sf("sfOwnerCount")) >= 1);
}

// ─── Multiple Trust Lines (Directory Growth) ────────────────────────────────

#[test]
fn dir_2_trust_lines() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 0, 0),
        account_root(gw, 10_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    for (seq, c) in [(1u32, usd_currency()), (2, eur_currency())] {
        let tx = STTx::new(TxType::TRUST_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_amount(
                sf("sfLimitAmount"),
                STAmount::from_iou_amount(
                    sf_generic(),
                    IOUAmount::from_parts(1000, 0).expect("a"),
                    Issue::new(c, gw),
                ),
            );
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        full_apply(&mut v, &tx, TxType::TRUST_SET);
    }
    let k = protocol::account_keylet(acct_id(a));
    let sle = v.peek(k).ok().flatten().unwrap();
    assert!(sle.get_field_u32(sf("sfOwnerCount")) >= 2);
}
#[test]
fn dir_5_trust_lines() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 0, 0),
        account_root(gw, 10_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    for (seq, c) in (1..=5u32).zip(["AAA", "BBB", "CCC", "DDD", "EEE"].iter()) {
        let cur = protocol::currency_from_string(c);
        let tx = STTx::new(TxType::TRUST_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_amount(
                sf("sfLimitAmount"),
                STAmount::from_iou_amount(
                    sf_generic(),
                    IOUAmount::from_parts(1000, 0).expect("a"),
                    Issue::new(cur, gw),
                ),
            );
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        full_apply(&mut v, &tx, TxType::TRUST_SET);
    }
    let k = protocol::account_keylet(acct_id(a));
    let sle = v.peek(k).ok().flatten().unwrap();
    assert!(sle.get_field_u32(sf("sfOwnerCount")) >= 5);
}
#[test]
fn dir_10_trust_lines() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 20_000_000_000, 0, 0),
        account_root(gw, 20_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    for (seq, c) in (1..=10u32).zip(
        [
            "AAA", "BBB", "CCC", "DDD", "EEE", "FFF", "GGG", "HHH", "III", "JJJ",
        ]
        .iter(),
    ) {
        let cur = protocol::currency_from_string(c);
        let tx = STTx::new(TxType::TRUST_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_amount(
                sf("sfLimitAmount"),
                STAmount::from_iou_amount(
                    sf_generic(),
                    IOUAmount::from_parts(1000, 0).expect("a"),
                    Issue::new(cur, gw),
                ),
            );
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        full_apply(&mut v, &tx, TxType::TRUST_SET);
    }
    let k = protocol::account_keylet(acct_id(a));
    let sle = v.peek(k).ok().flatten().unwrap();
    assert!(sle.get_field_u32(sf("sfOwnerCount")) >= 10);
}

// ─── Offer Cancel Decreases Owner Count ─────────────────────────────────────

#[test]
fn dir_offer_cancel_decreases() {
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
    let before = v
        .peek(protocol::account_keylet(acct_id(a)))
        .ok()
        .flatten()
        .unwrap()
        .get_field_u32(sf("sfOwnerCount"));
    let tx2 = STTx::new(TxType::OFFER_CANCEL, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfOfferSequence"), 1);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 2);
    });
    handle_real_dispatch(&mut v, &tx2, TxType::OFFER_CANCEL, None);
    let after = v
        .peek(protocol::account_keylet(acct_id(a)))
        .ok()
        .flatten()
        .unwrap()
        .get_field_u32(sf("sfOwnerCount"));
    assert!(after < before);
}

// ─── Ledger Entry Existence After Creation ──────────────────────────────────

#[test]
fn dir_trust_line_exists() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(gw, 5_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::TRUST_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(
            sf("sfLimitAmount"),
            STAmount::from_iou_amount(
                sf_generic(),
                IOUAmount::from_parts(1000, 0).expect("a"),
                Issue::new(usd_currency(), gw),
            ),
        );
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    full_apply(&mut v, &tx, TxType::TRUST_SET);
    assert!(
        v.peek(protocol::line(a, gw, usd_currency()))
            .ok()
            .flatten()
            .is_some()
    );
}
#[test]
fn dir_offer_exists() {
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
    handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None);
    assert!(
        v.peek(protocol::offer_keylet(acct_id(a), 1))
            .ok()
            .flatten()
            .is_some()
    );
}
#[test]
fn dir_escrow_exists() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = build_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ESCROW_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_account_id(sf("sfDestination"), b);
        tx.set_field_amount(sf("sfAmount"), xrp(1_000_000));
        tx.set_field_u32(sf("sfFinishAfter"), 500);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    full_apply(&mut v, &tx, TxType::ESCROW_CREATE);
    assert!(
        v.peek(protocol::escrow_keylet(acct_id(a), 1))
            .ok()
            .flatten()
            .is_some()
    );
}
#[test]
fn dir_check_exists() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = build_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::CHECK_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_account_id(sf("sfDestination"), b);
        tx.set_field_amount(sf("sfSendMax"), xrp(1_000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    full_apply(&mut v, &tx, TxType::CHECK_CREATE);
    assert!(
        v.peek(protocol::check_keylet(acct_id(a), 1))
            .ok()
            .flatten()
            .is_some()
    );
}
#[test]
fn dir_paychan_exists() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = build_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::PAYCHAN_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_account_id(sf("sfDestination"), b);
        tx.set_field_amount(sf("sfAmount"), xrp(1_000_000));
        tx.set_field_u32(sf("sfSettleDelay"), 3600);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE);
    assert!(
        v.peek(protocol::pay_channel_keylet(acct_id(a), acct_id(b), 1))
            .ok()
            .flatten()
            .is_some()
    );
}
#[test]
fn dir_deposit_preauth_exists() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = build_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::DEPOSIT_PREAUTH, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_account_id(sf("sfAuthorize"), b);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    full_apply(&mut v, &tx, TxType::DEPOSIT_PREAUTH);
    assert!(
        v.peek(protocol::deposit_preauth_keylet(acct_id(a), acct_id(b)))
            .ok()
            .flatten()
            .is_some()
    );
}

// ─── Offer Removed After Crossing ──────────────────────────────────────────

#[test]
fn dir_crossed_offer_removed() {
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
    assert!(
        v.peek(protocol::offer_keylet(acct_id(a), 1))
            .ok()
            .flatten()
            .is_none()
    );
}
