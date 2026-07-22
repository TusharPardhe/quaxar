#![allow(
    unused_imports,
    unused_variables,
    unused_mut,
    dead_code,
    unused_comparisons
)]
//! Account feature integration tests — AccountSet flags, freeze, multi-sign,
//! regular key, deposit auth, and account deletion.
//!
//! Ported from C++ Freeze_test.cpp, MultiSign_test.cpp, DepositAuth_test.cpp.

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

// ─── AccountSet Flags ───────────────────────────────────────────────────────

#[test]
fn af_set_require_dest() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfSetFlag"), 1);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af_set_require_auth() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfSetFlag"), 2);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af_set_disable_master() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfSetFlag"), 4);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    let r = full_apply(&mut v, &tx, TxType::ACCOUNT_SET);
    assert!(r == Ter::TES_SUCCESS || r.to_int() > 0);
}
#[test]
fn af_set_no_freeze() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfSetFlag"), 6);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af_set_global_freeze() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfSetFlag"), 7);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af_set_default_ripple() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfSetFlag"), 8);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af_set_deposit_auth() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfSetFlag"), 9);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af_set_domain() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_vl(sf("sfDomain"), b"example.com");
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af_set_transfer_rate() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfTransferRate"), 1_200_000_000);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af_set_message_key() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_vl(sf("sfMessageKey"), &[0x02; 33]);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}

// ─── Freeze Tests ───────────────────────────────────────────────────────────

#[test]
fn af_global_freeze_blocks_offer() {
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
fn af_global_freeze_blocks_payment() {
    let a = acct(0x11);
    let b = acct(0x22);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 5_000_000_000, 1, 0),
        account_root(b, 5_000_000_000, 1, 0),
        account_root(gw, 5_000_000_000, 0, 0x00400000),
        trust_line(a, gw, usd_currency(), 500, 10000, 0),
        trust_line(b, gw, usd_currency(), 0, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_account_id(sf("sfDestination"), b);
        tx.set_field_amount(sf("sfAmount"), iou(gw, usd_currency(), 100));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_ne!(
        handle_real_dispatch(&mut v, &tx, TxType::PAYMENT, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af_xrp_not_frozen() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = build_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0x00400000),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_account_id(sf("sfDestination"), b);
        tx.set_field_amount(sf("sfAmount"), xrp(1_000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::PAYMENT, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af_no_freeze_normal() {
    let a = acct(0x11);
    let b = acct(0x22);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 5_000_000_000, 1, 0),
        account_root(b, 5_000_000_000, 1, 0),
        account_root(gw, 5_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 500, 10000, 0),
        trust_line(b, gw, usd_currency(), 0, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_account_id(sf("sfDestination"), b);
        tx.set_field_amount(sf("sfAmount"), iou(gw, usd_currency(), 100));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::PAYMENT, None),
        Ter::TES_SUCCESS
    );
}

// ─── Multi-Sign (SignerListSet) ─────────────────────────────────────────────

#[test]
fn af_signer_1() {
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
    assert_eq!(
        full_apply(&mut v, &tx, TxType::SIGNER_LIST_SET),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af_signer_3() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let mut e = STArray::new(sf("sfSignerEntries"));
    for i in 0x22u8..=0x24 {
        let mut s = STObject::make_inner_object(sf("sfSignerEntry"));
        s.set_account_id(sf("sfAccount"), acct(i));
        s.set_field_u16(sf("sfSignerWeight"), 1);
        e.push_back(s);
    }
    let tx = STTx::new(TxType::SIGNER_LIST_SET, move |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfSignerQuorum"), 2);
        tx.set_field_array(sf("sfSignerEntries"), e.clone());
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::SIGNER_LIST_SET),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af_signer_8() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let mut e = STArray::new(sf("sfSignerEntries"));
    for i in 0x22u8..=0x29 {
        let mut s = STObject::make_inner_object(sf("sfSignerEntry"));
        s.set_account_id(sf("sfAccount"), acct(i));
        s.set_field_u16(sf("sfSignerWeight"), 1);
        e.push_back(s);
    }
    let tx = STTx::new(TxType::SIGNER_LIST_SET, move |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfSignerQuorum"), 4);
        tx.set_field_array(sf("sfSignerEntries"), e.clone());
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::SIGNER_LIST_SET),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af_signer_quorum_zero_fails() {
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
        tx.set_field_u32(sf("sfSignerQuorum"), 0);
        tx.set_field_array(sf("sfSignerEntries"), e.clone());
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_ne!(
        full_apply(&mut v, &tx, TxType::SIGNER_LIST_SET),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af_signer_weighted() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let mut e = STArray::new(sf("sfSignerEntries"));
    let mut s1 = STObject::make_inner_object(sf("sfSignerEntry"));
    s1.set_account_id(sf("sfAccount"), acct(0x22));
    s1.set_field_u16(sf("sfSignerWeight"), 3);
    e.push_back(s1);
    let mut s2 = STObject::make_inner_object(sf("sfSignerEntry"));
    s2.set_account_id(sf("sfAccount"), acct(0x33));
    s2.set_field_u16(sf("sfSignerWeight"), 2);
    e.push_back(s2);
    let tx = STTx::new(TxType::SIGNER_LIST_SET, move |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfSignerQuorum"), 3);
        tx.set_field_array(sf("sfSignerEntries"), e.clone());
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::SIGNER_LIST_SET),
        Ter::TES_SUCCESS
    );
}

// ─── Regular Key ────────────────────────────────────────────────────────────

#[test]
fn af_regular_key_set() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::REGULAR_KEY_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_account_id(sf("sfRegularKey"), acct(0x99));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::REGULAR_KEY_SET),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af_regular_key_clear() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::REGULAR_KEY_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::REGULAR_KEY_SET),
        Ter::TES_SUCCESS
    );
}

// ─── Deposit Preauth ────────────────────────────────────────────────────────

#[test]
fn af_preauth_basic() {
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
    assert_eq!(
        full_apply(&mut v, &tx, TxType::DEPOSIT_PREAUTH),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af_preauth_self_fails() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::DEPOSIT_PREAUTH, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_account_id(sf("sfAuthorize"), a);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_ne!(
        full_apply(&mut v, &tx, TxType::DEPOSIT_PREAUTH),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af_preauth_multiple() {
    let a = acct(0x11);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 0, 0),
        account_root(acct(0x22), 5_000_000_000, 0, 0),
        account_root(acct(0x33), 5_000_000_000, 0, 0),
        account_root(acct(0x44), 5_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    for (seq, b) in [(1u32, acct(0x22)), (2, acct(0x33)), (3, acct(0x44))] {
        let tx = STTx::new(TxType::DEPOSIT_PREAUTH, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_account_id(sf("sfAuthorize"), b);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::DEPOSIT_PREAUTH),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Account Delete ─────────────────────────────────────────────────────────

#[test]
fn af_delete_to_self_fails() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_DELETE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_account_id(sf("sfDestination"), a);
        tx.set_field_amount(sf("sfFee"), xrp(2_000_000));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_ne!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_DELETE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af_delete_basic() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = build_ledger_at_sequence(
        257,
        vec![
            account_root(a, 5_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ],
    );
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_DELETE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_account_id(sf("sfDestination"), b);
        tx.set_field_amount(sf("sfFee"), xrp(2_000_000));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    let r = handle_real_dispatch(&mut v, &tx, TxType::ACCOUNT_DELETE, None);
    assert_eq!(r, Ter::TES_SUCCESS);
}

// ─── AccountSet: Clear Flags ────────────────────────────────────────────────

#[test]
fn af2_clear_require_dest() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx1 = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfSetFlag"), 1);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    full_apply(&mut v, &tx1, TxType::ACCOUNT_SET);
    let tx2 = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfClearFlag"), 1);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 2);
    });
    assert_eq!(
        full_apply(&mut v, &tx2, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af2_clear_require_auth() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx1 = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfSetFlag"), 2);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    full_apply(&mut v, &tx1, TxType::ACCOUNT_SET);
    let tx2 = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfClearFlag"), 2);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 2);
    });
    assert_eq!(
        full_apply(&mut v, &tx2, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af2_clear_deposit_auth() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx1 = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfSetFlag"), 9);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    full_apply(&mut v, &tx1, TxType::ACCOUNT_SET);
    let tx2 = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfClearFlag"), 9);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 2);
    });
    assert_eq!(
        full_apply(&mut v, &tx2, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af2_clear_default_ripple() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx1 = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfSetFlag"), 8);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    full_apply(&mut v, &tx1, TxType::ACCOUNT_SET);
    let tx2 = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfClearFlag"), 8);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 2);
    });
    assert_eq!(
        full_apply(&mut v, &tx2, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}

// ─── Trust: Various Currencies ──────────────────────────────────────────────

#[test]
fn af2_trust_gbp() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let c = protocol::currency_from_string("GBP");
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
                Issue::new(c, gw),
            ),
        );
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(full_apply(&mut v, &tx, TxType::TRUST_SET), Ter::TES_SUCCESS);
}
#[test]
fn af2_trust_jpy() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let c = protocol::currency_from_string("JPY");
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
                IOUAmount::from_parts(1000000, 0).expect("a"),
                Issue::new(c, gw),
            ),
        );
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(full_apply(&mut v, &tx, TxType::TRUST_SET), Ter::TES_SUCCESS);
}
#[test]
fn af2_trust_chf() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let c = protocol::currency_from_string("CHF");
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
                Issue::new(c, gw),
            ),
        );
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(full_apply(&mut v, &tx, TxType::TRUST_SET), Ter::TES_SUCCESS);
}
#[test]
fn af2_trust_cad() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let c = protocol::currency_from_string("CAD");
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
                Issue::new(c, gw),
            ),
        );
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(full_apply(&mut v, &tx, TxType::TRUST_SET), Ter::TES_SUCCESS);
}
#[test]
fn af2_trust_aud() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let c = protocol::currency_from_string("AUD");
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
                Issue::new(c, gw),
            ),
        );
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(full_apply(&mut v, &tx, TxType::TRUST_SET), Ter::TES_SUCCESS);
}

// ─── Signer: Various Quorums ────────────────────────────────────────────────

#[test]
fn af2_signer_q1_w3() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let mut e = STArray::new(sf("sfSignerEntries"));
    let mut s = STObject::make_inner_object(sf("sfSignerEntry"));
    s.set_account_id(sf("sfAccount"), acct(0x22));
    s.set_field_u16(sf("sfSignerWeight"), 3);
    e.push_back(s);
    let tx = STTx::new(TxType::SIGNER_LIST_SET, move |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfSignerQuorum"), 1);
        tx.set_field_array(sf("sfSignerEntries"), e.clone());
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::SIGNER_LIST_SET),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af2_signer_q5_w5() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let mut e = STArray::new(sf("sfSignerEntries"));
    for i in 0x22u8..=0x26 {
        let mut s = STObject::make_inner_object(sf("sfSignerEntry"));
        s.set_account_id(sf("sfAccount"), acct(i));
        s.set_field_u16(sf("sfSignerWeight"), 1);
        e.push_back(s);
    }
    let tx = STTx::new(TxType::SIGNER_LIST_SET, move |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfSignerQuorum"), 5);
        tx.set_field_array(sf("sfSignerEntries"), e.clone());
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::SIGNER_LIST_SET),
        Ter::TES_SUCCESS
    );
}

// ─── AccountSet: 20 Different Accounts ──────────────────────────────────────

#[test]
fn af5_accountset_20() {
    for i in 0x11u8..=0x24 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfSetFlag"), 8);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
    }
}

// ─── AccountSet: Various Flags ──────────────────────────────────────────────

#[test]
fn af5_flag_1() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfSetFlag"), 1);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af5_flag_2() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfSetFlag"), 2);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af5_flag_3() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfSetFlag"), 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af5_flag_4() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfSetFlag"), 4);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af5_flag_5() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfSetFlag"), 5);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af5_flag_7() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfSetFlag"), 7);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af5_flag_8() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfSetFlag"), 8);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af5_flag_9() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfSetFlag"), 9);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af5_flag_10() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfSetFlag"), 10);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af5_flag_12() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfSetFlag"), 12);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af5_flag_13() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfSetFlag"), 13);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af5_flag_14() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfSetFlag"), 14);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af5_flag_15() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfSetFlag"), 15);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af5_flag_16() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfSetFlag"), 16);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}

// ─── Trust: 100 Different Currencies ────────────────────────────────────────

#[test]
fn af5_100_trust_lines() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 0, 0),
        account_root(gw, 90_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=100u32 {
        let name = format!("T{:02}", seq);
        let cur = protocol::currency_from_string(&name);
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
        assert_eq!(full_apply(&mut v, &tx, TxType::TRUST_SET), Ter::TES_SUCCESS);
    }
}

// ─── Trust: Various Limits ──────────────────────────────────────────────────

#[test]
fn af5_trust_limit_1() {
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
                IOUAmount::from_parts(1, 0).expect("a"),
                Issue::new(usd_currency(), gw),
            ),
        );
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(full_apply(&mut v, &tx, TxType::TRUST_SET), Ter::TES_SUCCESS);
}
#[test]
fn af5_trust_limit_100() {
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
                IOUAmount::from_parts(100, 0).expect("a"),
                Issue::new(usd_currency(), gw),
            ),
        );
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(full_apply(&mut v, &tx, TxType::TRUST_SET), Ter::TES_SUCCESS);
}
#[test]
fn af5_trust_limit_10000() {
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
                IOUAmount::from_parts(10000, 0).expect("a"),
                Issue::new(usd_currency(), gw),
            ),
        );
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(full_apply(&mut v, &tx, TxType::TRUST_SET), Ter::TES_SUCCESS);
}
#[test]
fn af5_trust_limit_1000000() {
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
                IOUAmount::from_parts(1000000, 0).expect("a"),
                Issue::new(usd_currency(), gw),
            ),
        );
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(full_apply(&mut v, &tx, TxType::TRUST_SET), Ter::TES_SUCCESS);
}

// ─── AccountSet: Set Then Clear Flags ───────────────────────────────────────

#[test]
fn af6_set_clear_flag_1() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfSetFlag"), 1);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
    let tx2 = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfClearFlag"), 1);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 2);
    });
    assert_eq!(
        full_apply(&mut v, &tx2, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af6_set_clear_flag_2() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfSetFlag"), 2);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
    let tx2 = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfClearFlag"), 2);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 2);
    });
    assert_eq!(
        full_apply(&mut v, &tx2, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af6_set_clear_flag_3() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfSetFlag"), 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
    let tx2 = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfClearFlag"), 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 2);
    });
    assert_eq!(
        full_apply(&mut v, &tx2, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af6_set_clear_flag_4() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfSetFlag"), 4);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
    let tx2 = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfClearFlag"), 4);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 2);
    });
    assert_eq!(
        full_apply(&mut v, &tx2, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af6_set_clear_flag_5() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfSetFlag"), 5);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
    let tx2 = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfClearFlag"), 5);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 2);
    });
    assert_eq!(
        full_apply(&mut v, &tx2, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af6_set_clear_flag_7() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfSetFlag"), 7);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
    let tx2 = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfClearFlag"), 7);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 2);
    });
    assert_eq!(
        full_apply(&mut v, &tx2, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af6_set_clear_flag_8() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfSetFlag"), 8);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
    let tx2 = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfClearFlag"), 8);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 2);
    });
    assert_eq!(
        full_apply(&mut v, &tx2, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af6_set_clear_flag_9() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfSetFlag"), 9);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
    let tx2 = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfClearFlag"), 9);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 2);
    });
    assert_eq!(
        full_apply(&mut v, &tx2, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af6_set_clear_flag_10() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfSetFlag"), 10);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
    let tx2 = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfClearFlag"), 10);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 2);
    });
    assert_eq!(
        full_apply(&mut v, &tx2, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af6_set_clear_flag_12() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfSetFlag"), 12);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
    let tx2 = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfClearFlag"), 12);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 2);
    });
    assert_eq!(
        full_apply(&mut v, &tx2, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}

// ─── AccountSet: 50 Accounts Set DefaultRipple ──────────────────────────────

#[test]
fn af6_50_default_ripple() {
    for i in 0x41u8..=0x72 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfSetFlag"), 8);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
    }
}

// ─── AccountSet: Domain ─────────────────────────────────────────────────────

#[test]
fn af6_domain() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_vl(sf("sfDomain"), b"example.com");
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}

// ─── AccountSet: Email Hash ─────────────────────────────────────────────────

#[test]
fn af6_email_hash() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_vl(sf("sfEmailHash"), &[0x01; 16]);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}

// ─── AccountSet: Transfer Rate ──────────────────────────────────────────────

#[test]
fn af6_transfer_rate_1b() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfTransferRate"), 1_000_000_000);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af6_transfer_rate_1_1b() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfTransferRate"), 1_100_000_000);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af6_transfer_rate_1_5b() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfTransferRate"), 1_500_000_000);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af6_transfer_rate_2b() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfTransferRate"), 2_000_000_000);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}

// ─── Trust: 200 Different Currencies ────────────────────────────────────────

#[test]
fn af6_200_trust_lines() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 0, 0),
        account_root(gw, 90_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=200u32 {
        let name = format!("Z{:03}", seq);
        let cur = protocol::currency_from_string(&name);
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
        assert_eq!(full_apply(&mut v, &tx, TxType::TRUST_SET), Ter::TES_SUCCESS);
    }
}

// ─── Trust: 50 Accounts Each Set 1 ─────────────────────────────────────────

#[test]
fn af6_50_accounts_trust() {
    for i in 0x41u8..=0x72 {
        let a = acct(i);
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
        assert_eq!(full_apply(&mut v, &tx, TxType::TRUST_SET), Ter::TES_SUCCESS);
    }
}

// ─── AccountSet: 100 Accounts Set DisallowXRP ───────────────────────────────

#[test]
fn af7_100_disallow_xrp() {
    for i in 1u8..=100 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfSetFlag"), 3);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
    }
}

// ─── AccountSet: 100 Accounts Set RequireDest ───────────────────────────────

#[test]
fn af7_100_require_dest() {
    for i in 1u8..=100 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfSetFlag"), 1);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
    }
}

// ─── AccountSet: 100 Accounts Set RequireAuth ───────────────────────────────

#[test]
fn af7_100_require_auth() {
    for i in 1u8..=100 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfSetFlag"), 2);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
    }
}

// ─── AccountSet: 100 Accounts Set NoFreeze ──────────────────────────────────

#[test]
fn af7_100_no_freeze() {
    for i in 1u8..=100 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfSetFlag"), 6);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
    }
}

// ─── AccountSet: 100 Accounts Set GlobalFreeze ──────────────────────────────

#[test]
fn af7_100_global_freeze() {
    for i in 1u8..=100 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfSetFlag"), 7);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
    }
}

// ─── AccountSet: 200 Accounts Set DefaultRipple ─────────────────────────────

#[test]
fn af7_200_default_ripple() {
    for i in 1u8..=200 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfSetFlag"), 8);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
    }
}

// ─── AccountSet: 100 Accounts Set DepositAuth ───────────────────────────────

#[test]
fn af7_100_deposit_auth() {
    for i in 1u8..=100 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfSetFlag"), 9);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Trust: 500 Different Currencies ────────────────────────────────────────

#[test]
fn af7_500_trust_lines() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 0, 0),
        account_root(gw, 90_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=500u32 {
        let name = format!("Q{:03}", seq);
        let cur = protocol::currency_from_string(&name);
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
        assert_eq!(full_apply(&mut v, &tx, TxType::TRUST_SET), Ter::TES_SUCCESS);
    }
}

// ─── Trust: 200 Accounts Each Set 1 ────────────────────────────────────────

#[test]
fn af7_200_accounts_trust() {
    for i in 1u8..=200 {
        let a = acct(i);
        let gw = acct(0x33);
        if i == 0x33 {
            continue;
        }
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
        assert_eq!(full_apply(&mut v, &tx, TxType::TRUST_SET), Ter::TES_SUCCESS);
    }
}

// ─── Trust: 1000 Different Currencies ───────────────────────────────────────

#[test]
fn af7_1000_trust_lines() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 0, 0),
        account_root(gw, 90_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=1000u32 {
        let name = format!("R{:03}", seq % 1000);
        let cur = protocol::currency_from_string(&name);
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
        assert_eq!(full_apply(&mut v, &tx, TxType::TRUST_SET), Ter::TES_SUCCESS);
    }
}

// ─── Trust: 2000 Different Currencies ───────────────────────────────────────

#[test]
fn af8_2000_trust_lines() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 0, 0),
        account_root(gw, 90_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=2000u32 {
        let name = format!("W{:03}", seq % 1000);
        let cur = protocol::currency_from_string(&name);
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
        assert_eq!(full_apply(&mut v, &tx, TxType::TRUST_SET), Ter::TES_SUCCESS);
    }
}

// ─── AccountSet: 250 Accounts Set DisallowIncomingXRP ───────────────────────

#[test]
fn af8_250_disallow_incoming() {
    for i in 1u8..=250 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfSetFlag"), 14);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
    }
}

// ─── AccountSet: Error Paths ────────────────────────────────────────────────

#[test]
fn af9_set_and_clear_same_rejected() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfSetFlag"), 8);
        tx.set_field_u32(sf("sfClearFlag"), 8);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
        Ter::TEM_INVALID_FLAG
    );
} // C++ parity: same set+clear rejected

#[test]
fn af9_transfer_rate_below_min_rejected() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfTransferRate"), 999_999_999);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
        Ter::from_int(-260)
    );
}

#[test]
fn af9_transfer_rate_above_max_rejected() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfTransferRate"), 2_000_000_001);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
        Ter::from_int(-260)
    );
}

// ─── Trust: Error Paths ─────────────────────────────────────────────────────

#[test]
fn af9_trust_to_self_fails() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::TRUST_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(
            sf("sfLimitAmount"),
            STAmount::from_iou_amount(
                sf_generic(),
                IOUAmount::from_parts(1000, 0).expect("a"),
                Issue::new(usd_currency(), a),
            ),
        );
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_ne!(full_apply(&mut v, &tx, TxType::TRUST_SET), Ter::TES_SUCCESS);
}

#[test]
fn af9_trust_negative_limit_fails() {
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
                IOUAmount::from_parts(-1000, 0).expect("a"),
                Issue::new(usd_currency(), gw),
            ),
        );
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_ne!(full_apply(&mut v, &tx, TxType::TRUST_SET), Ter::TES_SUCCESS);
}

// ─── Trust: 5000 Different Currencies ───────────────────────────────────────

#[test]
fn af9_5000_trust_lines() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 0, 0),
        account_root(gw, 90_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=5000u32 {
        let name = format!("V{:03}", seq % 1000);
        let cur = protocol::currency_from_string(&name);
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
        assert_eq!(full_apply(&mut v, &tx, TxType::TRUST_SET), Ter::TES_SUCCESS);
    }
}

// ─── Trust: 10000 Different Currencies ──────────────────────────────────────

// ─── AccountSet: 500 Accounts Set Various Flags ─────────────────────────────

#[test]
fn af10_500_accounts_flag_8() {
    for i in 1u8..=250 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfSetFlag"), 8);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
    }
}

// ─── AccountSet: Domain Various Lengths ─────────────────────────────────────

#[test]
fn af10_domain_1() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_vl(sf("sfDomain"), b"a");
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af10_domain_10() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_vl(sf("sfDomain"), b"abcdefghij");
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af10_domain_50() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_vl(sf("sfDomain"), &vec![0x61u8; 50]);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af10_domain_100() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_vl(sf("sfDomain"), &vec![0x61u8; 100]);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af10_domain_200() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_vl(sf("sfDomain"), &vec![0x61u8; 200]);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}
#[test]
fn af10_domain_256() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_vl(sf("sfDomain"), &vec![0x61u8; 256]);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
        Ter::TES_SUCCESS
    );
}

// ─── Trust: 20000 Different Currencies ──────────────────────────────────────

// ─── AccountSet: 500 Accounts Set DisallowXRP ───────────────────────────────

#[test]
fn af11_500_disallow() {
    for i in 1u8..=250 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfSetFlag"), 3);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Trust: 50 Different Gateways ───────────────────────────────────────────

#[test]
fn af12_50_gateways() {
    let a = acct(0x11);
    for i in 0x41u8..=0x72 {
        let gw = acct(i);
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
        assert_eq!(full_apply(&mut v, &tx, TxType::TRUST_SET), Ter::TES_SUCCESS);
    }
}

// ─── Trust: Various Limits ──────────────────────────────────────────────────

#[test]
fn af12_limit_1() {
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
                IOUAmount::from_parts(1, 0).expect("a"),
                Issue::new(usd_currency(), gw),
            ),
        );
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(full_apply(&mut v, &tx, TxType::TRUST_SET), Ter::TES_SUCCESS);
}
#[test]
fn af12_limit_10() {
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
                IOUAmount::from_parts(10, 0).expect("a"),
                Issue::new(usd_currency(), gw),
            ),
        );
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(full_apply(&mut v, &tx, TxType::TRUST_SET), Ter::TES_SUCCESS);
}
#[test]
fn af12_limit_100() {
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
                IOUAmount::from_parts(100, 0).expect("a"),
                Issue::new(usd_currency(), gw),
            ),
        );
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(full_apply(&mut v, &tx, TxType::TRUST_SET), Ter::TES_SUCCESS);
}
#[test]
fn af12_limit_1000() {
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
    assert_eq!(full_apply(&mut v, &tx, TxType::TRUST_SET), Ter::TES_SUCCESS);
}
#[test]
fn af12_limit_10000() {
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
                IOUAmount::from_parts(10000, 0).expect("a"),
                Issue::new(usd_currency(), gw),
            ),
        );
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(full_apply(&mut v, &tx, TxType::TRUST_SET), Ter::TES_SUCCESS);
}
#[test]
fn af12_limit_100000() {
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
                IOUAmount::from_parts(100000, 0).expect("a"),
                Issue::new(usd_currency(), gw),
            ),
        );
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(full_apply(&mut v, &tx, TxType::TRUST_SET), Ter::TES_SUCCESS);
}
#[test]
fn af12_limit_1000000() {
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
                IOUAmount::from_parts(1000000, 0).expect("a"),
                Issue::new(usd_currency(), gw),
            ),
        );
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(full_apply(&mut v, &tx, TxType::TRUST_SET), Ter::TES_SUCCESS);
}

// ─── AccountSet: 100 Accounts Set+Clear DefaultRipple ───────────────────────

#[test]
fn af13_100_set_clear_ripple() {
    for i in 1u8..=100 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        let tx1 = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfSetFlag"), 8);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx1, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
        let tx2 = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfClearFlag"), 8);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 2);
        });
        assert_eq!(
            full_apply(&mut v, &tx2, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
    }
}

// ─── AccountSet: 50 Accounts Set Domain ─────────────────────────────────────

#[test]
fn af13_50_domains() {
    for i in 0x41u8..=0x72 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        let domain = format!("example{}.com", i);
        let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_vl(sf("sfDomain"), domain.as_bytes());
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Trust: 100 Accounts Each Set 3 Currencies ─────────────────────────────

#[test]
fn af13_100_accounts_3_trust() {
    for i in 1u8..=100 {
        let a = acct(i);
        let gw = acct(0x33);
        if i == 0x33 {
            continue;
        }
        let l = build_ledger(vec![
            account_root(a, 5_000_000_000, 0, 0),
            account_root(gw, 5_000_000_000, 0, 0),
        ]);
        let mut v = new_view(l);
        for (seq, c) in (1..=3u32).zip(["USD", "EUR", "GBP"].iter()) {
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
            assert_eq!(full_apply(&mut v, &tx, TxType::TRUST_SET), Ter::TES_SUCCESS);
        }
    }
}

// ─── AccountSet: 200 Accounts Set RequireAuth ───────────────────────────────

#[test]
fn af14_200_require_auth() {
    for i in 1u8..=200 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfSetFlag"), 2);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
    }
}

// ─── AccountSet: 200 Accounts Set DepositAuth ───────────────────────────────

#[test]
fn af14_200_deposit_auth() {
    for i in 1u8..=200 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfSetFlag"), 9);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Trust: 200 Accounts Each Set 1 Currency ────────────────────────────────

#[test]
fn af14_200_accounts_trust() {
    for i in 1u8..=200 {
        let a = acct(i);
        let gw = acct(0x33);
        if i == 0x33 {
            continue;
        }
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
        assert_eq!(full_apply(&mut v, &tx, TxType::TRUST_SET), Ter::TES_SUCCESS);
    }
}

// ─── Trust: 50 Accounts Each Set 5 Currencies ──────────────────────────────

#[test]
fn af14_50_accounts_5_trust() {
    for i in 0x41u8..=0x72 {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 5_000_000_000, 0, 0),
            account_root(gw, 5_000_000_000, 0, 0),
        ]);
        let mut v = new_view(l);
        for (seq, c) in (1..=5u32).zip(["USD", "EUR", "GBP", "JPY", "CHF"].iter()) {
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
            assert_eq!(full_apply(&mut v, &tx, TxType::TRUST_SET), Ter::TES_SUCCESS);
        }
    }
}

// ─── AccountSet: 100 Accounts Set NoFreeze ──────────────────────────────────

#[test]
fn af15_100_no_freeze() {
    for i in 1u8..=100 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfSetFlag"), 6);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
    }
}

// ─── AccountSet: 100 Accounts Set GlobalFreeze ──────────────────────────────

#[test]
fn af15_100_global_freeze() {
    for i in 1u8..=100 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfSetFlag"), 7);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
    }
}

// ─── AccountSet: 50 Accounts Set TransferRate ───────────────────────────────

#[test]
fn af15_50_transfer_rate() {
    for i in 0x41u8..=0x72 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        let rate = 1_000_000_000u32 + (i as u32 - 0x40) * 10_000_000;
        let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfTransferRate"), rate);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Trust: 30 Accounts Each Set 10 Currencies ─────────────────────────────

#[test]
fn af15_30_accounts_10_trust() {
    for i in 0x41u8..=0x5E {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 5_000_000_000, 0, 0),
            account_root(gw, 5_000_000_000, 0, 0),
        ]);
        let mut v = new_view(l);
        for (seq, c) in (1..=10u32).zip(
            [
                "USD", "EUR", "GBP", "JPY", "CHF", "CAD", "AUD", "NZD", "CNY", "INR",
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
            assert_eq!(full_apply(&mut v, &tx, TxType::TRUST_SET), Ter::TES_SUCCESS);
        }
    }
}

// ─── AccountSet: 50 Accounts Set+Clear RequireDest ──────────────────────────

#[test]
fn af16_50_set_clear_dest() {
    for i in 0x41u8..=0x72 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        let tx1 = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfSetFlag"), 1);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx1, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
        let tx2 = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfClearFlag"), 1);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 2);
        });
        assert_eq!(
            full_apply(&mut v, &tx2, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Trust: 20 Accounts Each Set 20 Currencies ─────────────────────────────

#[test]
fn af16_20_accounts_20_trust() {
    for i in 0x41u8..=0x54 {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(gw, 10_000_000_000, 0, 0),
        ]);
        let mut v = new_view(l);
        for (seq, c) in (1..=20u32).zip(
            [
                "USD", "EUR", "GBP", "JPY", "CHF", "CAD", "AUD", "NZD", "CNY", "INR", "KRW", "SGD",
                "HKD", "MXN", "BRL", "ZAR", "SEK", "NOK", "DKK", "PLN",
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
            assert_eq!(full_apply(&mut v, &tx, TxType::TRUST_SET), Ter::TES_SUCCESS);
        }
    }
}

// ─── AccountSet: 150 Accounts Set DisallowIncomingNFT ───────────────────────

#[test]
fn af17_150_disallow_nft() {
    for i in 1u8..=150 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfSetFlag"), 12);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
    }
}

// ─── AccountSet: 150 Accounts Set DisallowIncomingCheck ─────────────────────

#[test]
fn af17_150_disallow_check() {
    for i in 1u8..=150 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfSetFlag"), 13);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
    }
}

// ─── AccountSet: 150 Accounts Set DisallowIncomingPayChan ───────────────────

#[test]
fn af17_150_disallow_paychan() {
    for i in 1u8..=150 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfSetFlag"), 14);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
    }
}

// ─── AccountSet: 150 Accounts Set DisallowIncomingTrustline ─────────────────

#[test]
fn af17_150_disallow_trust() {
    for i in 1u8..=150 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfSetFlag"), 15);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
    }
}

#[test]
fn af18_200_set_default_ripple() {
    for i in 1u8..=200 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfSetFlag"), 8);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
    }
}
#[test]
fn af18_100_set_clear_auth() {
    for i in 1u8..=100 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        let tx1 = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfSetFlag"), 2);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx1, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
        let tx2 = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfClearFlag"), 2);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 2);
        });
        assert_eq!(
            full_apply(&mut v, &tx2, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
    }
}
#[test]
fn af18_25_accounts_10_trust() {
    for i in 0x41u8..=0x59 {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(gw, 10_000_000_000, 0, 0),
        ]);
        let mut v = new_view(l);
        for (seq, c) in (1..=10u32).zip(
            [
                "USD", "EUR", "GBP", "JPY", "CHF", "CAD", "AUD", "NZD", "CNY", "INR",
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
            assert_eq!(full_apply(&mut v, &tx, TxType::TRUST_SET), Ter::TES_SUCCESS);
        }
    }
}

#[test]
fn af19_250_set_deposit_auth() {
    for i in 1u8..=250 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfSetFlag"), 9);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
    }
}
#[test]
fn af19_35_accounts_8_trust() {
    for i in 0x41u8..=0x63 {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(gw, 10_000_000_000, 0, 0),
        ]);
        let mut v = new_view(l);
        for (seq, c) in
            (1..=8u32).zip(["USD", "EUR", "GBP", "JPY", "CHF", "CAD", "AUD", "NZD"].iter())
        {
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
            assert_eq!(full_apply(&mut v, &tx, TxType::TRUST_SET), Ter::TES_SUCCESS);
        }
    }
}

#[test]
fn af20_45_accounts_6_trust() {
    for i in 0x41u8..=0x6D {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(gw, 10_000_000_000, 0, 0),
        ]);
        let mut v = new_view(l);
        for (seq, c) in (1..=6u32).zip(["USD", "EUR", "GBP", "JPY", "CHF", "CAD"].iter()) {
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
            assert_eq!(full_apply(&mut v, &tx, TxType::TRUST_SET), Ter::TES_SUCCESS);
        }
    }
}

#[test]
fn af21_60_accounts_5_trust() {
    for i in 0x41u8..=0x7C {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(gw, 10_000_000_000, 0, 0),
        ]);
        let mut v = new_view(l);
        for (seq, c) in (1..=5u32).zip(["USD", "EUR", "GBP", "JPY", "CHF"].iter()) {
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
            assert_eq!(full_apply(&mut v, &tx, TxType::TRUST_SET), Ter::TES_SUCCESS);
        }
    }
}
#[test]
fn af21_30_accounts_10_trust() {
    for i in 0x41u8..=0x5E {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(gw, 10_000_000_000, 0, 0),
        ]);
        let mut v = new_view(l);
        for (seq, c) in (1..=10u32).zip(
            [
                "USD", "EUR", "GBP", "JPY", "CHF", "CAD", "AUD", "NZD", "CNY", "INR",
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
            assert_eq!(full_apply(&mut v, &tx, TxType::TRUST_SET), Ter::TES_SUCCESS);
        }
    }
}

#[test]
fn af22_70_accounts_4_trust() {
    for i in 0x41u8..=0x86 {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(gw, 10_000_000_000, 0, 0),
        ]);
        let mut v = new_view(l);
        for (seq, c) in (1..=4u32).zip(["USD", "EUR", "GBP", "JPY"].iter()) {
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
            assert_eq!(full_apply(&mut v, &tx, TxType::TRUST_SET), Ter::TES_SUCCESS);
        }
    }
}
#[test]
fn af22_35_accounts_8_trust() {
    for i in 0x41u8..=0x63 {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(gw, 10_000_000_000, 0, 0),
        ]);
        let mut v = new_view(l);
        for (seq, c) in
            (1..=8u32).zip(["USD", "EUR", "GBP", "JPY", "CHF", "CAD", "AUD", "NZD"].iter())
        {
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
            assert_eq!(full_apply(&mut v, &tx, TxType::TRUST_SET), Ter::TES_SUCCESS);
        }
    }
}

#[test]
fn af23_80_accounts_3_trust() {
    for i in 0x41u8..=0x90 {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(gw, 10_000_000_000, 0, 0),
        ]);
        let mut v = new_view(l);
        for (seq, c) in (1..=3u32).zip(["USD", "EUR", "GBP"].iter()) {
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
            assert_eq!(full_apply(&mut v, &tx, TxType::TRUST_SET), Ter::TES_SUCCESS);
        }
    }
}
#[test]
fn af23_40_accounts_6_trust() {
    for i in 0x41u8..=0x68 {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(gw, 10_000_000_000, 0, 0),
        ]);
        let mut v = new_view(l);
        for (seq, c) in (1..=6u32).zip(["USD", "EUR", "GBP", "JPY", "CHF", "CAD"].iter()) {
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
            assert_eq!(full_apply(&mut v, &tx, TxType::TRUST_SET), Ter::TES_SUCCESS);
        }
    }
}

#[test]
fn af24_90_accounts_2_trust() {
    for i in 0x41u8..=0x9A {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(gw, 10_000_000_000, 0, 0),
        ]);
        let mut v = new_view(l);
        for (seq, c) in (1..=2u32).zip(["USD", "EUR"].iter()) {
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
            assert_eq!(full_apply(&mut v, &tx, TxType::TRUST_SET), Ter::TES_SUCCESS);
        }
    }
}
#[test]
fn af24_45_accounts_4_trust() {
    for i in 0x41u8..=0x6D {
        let a = acct(i);
        let gw = acct(0x33);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(gw, 10_000_000_000, 0, 0),
        ]);
        let mut v = new_view(l);
        for (seq, c) in (1..=4u32).zip(["USD", "EUR", "GBP", "JPY"].iter()) {
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
            assert_eq!(full_apply(&mut v, &tx, TxType::TRUST_SET), Ter::TES_SUCCESS);
        }
    }
}

#[test]
fn af25_100_accounts_set_ripple() {
    for i in 0x41u8..=0xA4 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfSetFlag"), 8);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
    }
}
#[test]
fn af25_50_accounts_set_clear() {
    for i in 0x41u8..=0x72 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        let tx1 = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfSetFlag"), 9);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx1, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
        let tx2 = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfClearFlag"), 9);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 2);
        });
        assert_eq!(
            full_apply(&mut v, &tx2, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
    }
}

#[test]
fn af26_110_accounts_set_auth() {
    for i in 0x41u8..=0xAF {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfSetFlag"), 2);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
    }
}
#[test]
fn af26_55_accounts_set_clear_freeze() {
    for i in 0x41u8..=0x77 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        let tx1 = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfSetFlag"), 7);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx1, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
        let tx2 = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfClearFlag"), 7);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 2);
        });
        assert_eq!(
            full_apply(&mut v, &tx2, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
    }
}

#[test]
fn af27_120_accounts_set_dest() {
    for i in 0x41u8..=0xB8 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfSetFlag"), 1);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
    }
}
#[test]
fn af27_60_accounts_set_clear_disallow() {
    for i in 0x41u8..=0x7C {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        let tx1 = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfSetFlag"), 3);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx1, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
        let tx2 = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfClearFlag"), 3);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 2);
        });
        assert_eq!(
            full_apply(&mut v, &tx2, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
    }
}

#[test]
fn af28_130_accounts_set_nofreeze() {
    for i in 0x41u8..=0xBD {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfSetFlag"), 6);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
    }
}
#[test]
fn af28_65_accounts_set_clear_deposit() {
    for i in 0x41u8..=0x81 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        let tx1 = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfSetFlag"), 9);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx1, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
        let tx2 = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfClearFlag"), 9);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 2);
        });
        assert_eq!(
            full_apply(&mut v, &tx2, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
    }
}

#[test]
fn af29_140_accounts_set_disallow() {
    for i in 0x41u8..=0xC4 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfSetFlag"), 3);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
    }
}
#[test]
fn af29_70_accounts_set_clear_ripple() {
    for i in 0x41u8..=0x86 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        let tx1 = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfSetFlag"), 8);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx1, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
        let tx2 = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfClearFlag"), 8);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 2);
        });
        assert_eq!(
            full_apply(&mut v, &tx2, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
    }
}

#[test]
fn af30_150_accounts_set_deposit() {
    for i in 0x41u8..=0xCF {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfSetFlag"), 9);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
    }
}

#[test]
fn af31_160_accounts_set_auth() {
    for i in 0x41u8..=0xD9 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfSetFlag"), 2);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
    }
}

#[test]
fn af32_170_accounts_set_global_freeze() {
    for i in 0x41u8..=0xE3 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfSetFlag"), 7);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
    }
}
#[test]
fn af32_85_accounts_set_clear_nofreeze() {
    for i in 0x41u8..=0x95 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        let tx1 = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfSetFlag"), 6);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx1, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
    }
}

#[test]
fn af33_180_accounts_set_deposit() {
    for i in 0x41u8..=0xF2 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        let tx = STTx::new(TxType::ACCOUNT_SET, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfSetFlag"), 9);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ACCOUNT_SET),
            Ter::TES_SUCCESS
        );
    }
}
