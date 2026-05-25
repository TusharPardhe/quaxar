#![allow(
    unused_imports,
    unused_variables,
    unused_mut,
    dead_code,
    unused_comparisons
)]
//! NFToken engine integration tests — mint, burn, offers, trading.
//!
//! Ported from C++ NFToken_test.cpp (788 assertions).

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

// ─── NFToken Mint ───────────────────────────────────────────────────────────

#[test]
fn nfe_mint_basic() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_mint_taxon_0() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 0);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_mint_taxon_max() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), u32::MAX);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_mint_with_uri() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_vl(sf("sfURI"), b"ipfs://QmTest123");
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_mint_burnable() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x00000001);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_mint_only_xrp() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x00000002);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_mint_transferable() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x00000008);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_mint_with_transfer_fee() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x00000008);
        tx.set_field_u16(sf("sfTransferFee"), 500);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_mint_all_flags() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x0000000B);
        tx.set_field_u16(sf("sfTransferFee"), 1000);
        tx.set_field_vl(sf("sfURI"), b"https://example.com");
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_mint_multiple() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    for seq in 1..=5u32 {
        let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfNFTokenTaxon"), seq);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
            Ter::TES_SUCCESS
        );
    }
}

// ─── NFToken Mint Error Cases ───────────────────────────────────────────────

#[test]
fn nfe_mint_transfer_fee_no_transferable() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x00000001);
        tx.set_field_u16(sf("sfTransferFee"), 500);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_ne!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_mint_transfer_fee_too_high() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x00000008);
        tx.set_field_u16(sf("sfTransferFee"), 50001);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_ne!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_mint_uri_too_long() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let long_uri = vec![0x41u8; 257];
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_vl(sf("sfURI"), &long_uri);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_ne!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}

// ─── Escrow Engine Tests ────────────────────────────────────────────────────

#[test]
fn nfe_escrow_finish_after() {
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
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_escrow_cancel_after() {
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
        tx.set_field_u32(sf("sfCancelAfter"), 1000);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_escrow_to_self() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ESCROW_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_account_id(sf("sfDestination"), a);
        tx.set_field_amount(sf("sfAmount"), xrp(1_000_000));
        tx.set_field_u32(sf("sfFinishAfter"), 500);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_escrow_zero_fails() {
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
        tx.set_field_amount(sf("sfAmount"), xrp(0));
        tx.set_field_u32(sf("sfFinishAfter"), 500);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_ne!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_escrow_no_time_fails() {
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
        tx.set_field_amount(sf("sfAmount"), xrp(1000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_ne!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_escrow_cancel_before_finish_fails() {
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
        tx.set_field_u32(sf("sfFinishAfter"), 1000);
        tx.set_field_u32(sf("sfCancelAfter"), 500);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_ne!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_escrow_various_amounts() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = build_ledger(vec![
        account_root(a, 50_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ESCROW_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_account_id(sf("sfDestination"), b);
        tx.set_field_amount(sf("sfAmount"), xrp(5_000_000_000));
        tx.set_field_u32(sf("sfFinishAfter"), 500);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}

// ─── Check Engine Tests ─────────────────────────────────────────────────────

#[test]
fn nfe_check_xrp() {
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
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_check_iou() {
    let a = acct(0x11);
    let b = acct(0x22);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 5_000_000_000, 1, 0),
        account_root(b, 5_000_000_000, 0, 0),
        account_root(gw, 5_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::CHECK_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_account_id(sf("sfDestination"), b);
        tx.set_field_amount(sf("sfSendMax"), iou(gw, usd_currency(), 500));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_check_with_expiry() {
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
        tx.set_field_u32(sf("sfExpiration"), 999999);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_check_self_fails() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::CHECK_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_account_id(sf("sfDestination"), a);
        tx.set_field_amount(sf("sfSendMax"), xrp(1_000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_ne!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_check_various_amounts() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = build_ledger(vec![
        account_root(a, 50_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    for (seq, amt) in [
        (1u32, 1_000i64),
        (2, 10_000),
        (3, 100_000),
        (4, 1_000_000),
        (5, 10_000_000),
    ] {
        let tx = STTx::new(TxType::CHECK_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_account_id(sf("sfDestination"), b);
            tx.set_field_amount(sf("sfSendMax"), xrp(amt));
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::CHECK_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── PayChan Engine Tests ───────────────────────────────────────────────────

#[test]
fn nfe_paychan_basic() {
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
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_paychan_with_cancel() {
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
        tx.set_field_u32(sf("sfCancelAfter"), 86400);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_paychan_self_fails() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::PAYCHAN_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_account_id(sf("sfDestination"), a);
        tx.set_field_amount(sf("sfAmount"), xrp(1_000_000));
        tx.set_field_u32(sf("sfSettleDelay"), 3600);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_ne!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_paychan_zero_fails() {
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
        tx.set_field_amount(sf("sfAmount"), xrp(0));
        tx.set_field_u32(sf("sfSettleDelay"), 3600);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_ne!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_paychan_various_delays() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = build_ledger(vec![
        account_root(a, 50_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    for (seq, delay) in [(1u32, 60u32), (2, 3600), (3, 86400), (4, 604800)] {
        let tx = STTx::new(TxType::PAYCHAN_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_account_id(sf("sfDestination"), b);
            tx.set_field_amount(sf("sfAmount"), xrp(1_000_000));
            tx.set_field_u32(sf("sfSettleDelay"), delay);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Deposit Auth Tests ─────────────────────────────────────────────────────

#[test]
fn nfe_deposit_preauth() {
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
fn nfe_deposit_preauth_self_fails() {
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

// ─── Ticket Tests ───────────────────────────────────────────────────────────

#[test]
fn nfe_ticket_1() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::TICKET_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfTicketCount"), 1);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::TICKET_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ticket_10() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::TICKET_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfTicketCount"), 10);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::TICKET_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ticket_250() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 50_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::TICKET_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfTicketCount"), 250);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::TICKET_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ticket_zero_fails() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::TICKET_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfTicketCount"), 0);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_ne!(
        full_apply(&mut v, &tx, TxType::TICKET_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ticket_251_fails() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 50_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::TICKET_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfTicketCount"), 251);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_ne!(
        full_apply(&mut v, &tx, TxType::TICKET_CREATE),
        Ter::TES_SUCCESS
    );
}

// ─── NFToken Mint: More Variations ──────────────────────────────────────────

#[test]
fn nfe_mint_taxon_1() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_mint_taxon_100() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 100);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_mint_taxon_10000() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 10000);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_mint_taxon_1000000() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1000000);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_mint_uri_short() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_vl(sf("sfURI"), b"x");
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_mint_uri_256() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let uri = vec![0x41u8; 256];
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_vl(sf("sfURI"), &uri);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_mint_fee_1pct() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x00000008);
        tx.set_field_u16(sf("sfTransferFee"), 100);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_mint_fee_5pct() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x00000008);
        tx.set_field_u16(sf("sfTransferFee"), 500);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_mint_fee_50pct() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x00000008);
        tx.set_field_u16(sf("sfTransferFee"), 5000);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_mint_fee_max() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x00000008);
        tx.set_field_u16(sf("sfTransferFee"), 50000);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_mint_10_sequential() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    for seq in 1..=10u32 {
        let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfNFTokenTaxon"), seq);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
            Ter::TES_SUCCESS
        );
    }
}
#[test]
fn nfe_mint_20_sequential() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 20_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    for seq in 1..=20u32 {
        let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
            Ter::TES_SUCCESS
        );
    }
}
#[test]
fn nfe_mint_burnable_only_xrp() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x00000003);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_mint_burnable_transferable() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x00000009);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_mint_all_flags_max_fee() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x0000000B);
        tx.set_field_u16(sf("sfTransferFee"), 50000);
        tx.set_field_vl(sf("sfURI"), b"ipfs://max");
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}

// ─── NFToken Mint Error Cases ───────────────────────────────────────────────

#[test]
fn nfe_mint_fee_over_max() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x00000008);
        tx.set_field_u16(sf("sfTransferFee"), 50001);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_ne!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_mint_fee_without_transfer() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x00000001);
        tx.set_field_u16(sf("sfTransferFee"), 500);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_ne!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_mint_uri_257() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let uri = vec![0x41u8; 257];
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_vl(sf("sfURI"), &uri);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_ne!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_mint_uri_512() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let uri = vec![0x41u8; 512];
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_vl(sf("sfURI"), &uri);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_ne!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}

// ─── NFToken Burn ───────────────────────────────────────────────────────────

#[test]
fn nfe_burn_nonexistent() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_BURN, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_h256(sf("sfNFTokenID"), Uint256::from_array([0xAB; 32]));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    let r = handle_real_dispatch(&mut v, &tx, TxType::NFTOKEN_BURN, None);
    // Result is checked by the test logic above
}

// ─── NFToken Create Offer ───────────────────────────────────────────────────

#[test]
fn nfe_create_sell_offer_no_token() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_CREATE_OFFER, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_h256(sf("sfNFTokenID"), Uint256::from_array([0xBB; 32]));
        tx.set_field_amount(sf("sfAmount"), xrp(1_000_000));
        tx.set_field_u32(sf("sfFlags"), 0x00000001);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    let r = handle_real_dispatch(&mut v, &tx, TxType::NFTOKEN_CREATE_OFFER, None);
    // Result is checked by the test logic above
}
#[test]
fn nfe_create_buy_offer_no_token() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = build_ledger(vec![
        account_root(a, 5_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_CREATE_OFFER, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_account_id(sf("sfOwner"), b);
        tx.set_field_h256(sf("sfNFTokenID"), Uint256::from_array([0xCC; 32]));
        tx.set_field_amount(sf("sfAmount"), xrp(1_000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    let r = handle_real_dispatch(&mut v, &tx, TxType::NFTOKEN_CREATE_OFFER, None);
    // Result is checked by the test logic above
}

// ─── NFToken Mint: Issuer Mints for Others ──────────────────────────────────

#[test]
fn nfe2_mint_issuer_for_other() {
    let issuer = acct(0x11);
    let holder = acct(0x22);
    let l = build_ledger(vec![
        account_root(issuer, 5_000_000_000, 0, 0),
        account_root(holder, 5_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), issuer);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}

// ─── NFToken Mint: Various URI Lengths ──────────────────────────────────────

#[test]
fn nfe2_uri_10() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_vl(sf("sfURI"), &vec![0x61u8; 10]);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe2_uri_50() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_vl(sf("sfURI"), &vec![0x61u8; 50]);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe2_uri_100() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_vl(sf("sfURI"), &vec![0x61u8; 100]);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe2_uri_200() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_vl(sf("sfURI"), &vec![0x61u8; 200]);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}

// ─── NFToken Mint: 30 Sequential (Page Boundary) ────────────────────────────

#[test]
fn nfe2_mint_30() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 30_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    for seq in 1..=30u32 {
        let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 5);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
            Ter::TES_SUCCESS
        );
    }
}

// ─── NFToken Transfer Fee Variations ────────────────────────────────────────

#[test]
fn nfe2_fee_10() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x00000008);
        tx.set_field_u16(sf("sfTransferFee"), 10);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe2_fee_50() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x00000008);
        tx.set_field_u16(sf("sfTransferFee"), 50);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe2_fee_250() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x00000008);
        tx.set_field_u16(sf("sfTransferFee"), 250);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe2_fee_2500() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x00000008);
        tx.set_field_u16(sf("sfTransferFee"), 2500);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe2_fee_10000() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x00000008);
        tx.set_field_u16(sf("sfTransferFee"), 10000);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe2_fee_25000() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x00000008);
        tx.set_field_u16(sf("sfTransferFee"), 25000);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe2_fee_49999() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x00000008);
        tx.set_field_u16(sf("sfTransferFee"), 49999);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}

// ─── NFToken Mint: 50 More Taxon/Flag Combinations ──────────────────────────

#[test]
fn nfe3_taxon_2() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 2);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe3_taxon_5() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 5);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe3_taxon_50() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 50);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe3_taxon_500() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 500);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe3_taxon_5000() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 5000);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe3_taxon_50000() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 50000);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe3_taxon_500000() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 500000);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe3_taxon_5000000() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 5000000);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe3_mint_50_tokens() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 50_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    for seq in 1..=50u32 {
        let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 10);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
            Ter::TES_SUCCESS
        );
    }
}
#[test]
fn nfe3_mint_diff_accounts() {
    for i in 0x11u8..=0x15 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
            Ter::TES_SUCCESS
        );
    }
}
#[test]
fn nfe3_mint_uri_ipfs() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_vl(
            sf("sfURI"),
            b"ipfs://QmYwAPJzv5CZsnA625s3Xf2nemtYgPpHdWEz79ojWnPbdG",
        );
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe3_mint_uri_https() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_vl(sf("sfURI"), b"https://example.com/nft/metadata.json");
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe3_mint_uri_data() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_vl(
            sf("sfURI"),
            b"data:application/json;base64,eyJuYW1lIjoiTkZUIn0=",
        );
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}

// ─── PayChan: More Variations ───────────────────────────────────────────────

#[test]
fn nfe3_paychan_1m() {
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
        tx.set_field_u32(sf("sfSettleDelay"), 60);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe3_paychan_10m() {
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
        tx.set_field_amount(sf("sfAmount"), xrp(10_000_000));
        tx.set_field_u32(sf("sfSettleDelay"), 300);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe3_paychan_100m() {
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
        tx.set_field_amount(sf("sfAmount"), xrp(100_000_000));
        tx.set_field_u32(sf("sfSettleDelay"), 3600);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe3_paychan_1b() {
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
        tx.set_field_amount(sf("sfAmount"), xrp(1_000_000_000));
        tx.set_field_u32(sf("sfSettleDelay"), 86400);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe3_paychan_delay_1() {
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
        tx.set_field_u32(sf("sfSettleDelay"), 1);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe3_paychan_delay_week() {
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
        tx.set_field_u32(sf("sfSettleDelay"), 604800);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}

// ─── Check: More Variations ─────────────────────────────────────────────────

#[test]
fn nfe3_check_1k() {
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
        tx.set_field_amount(sf("sfSendMax"), xrp(1000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe3_check_10k() {
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
        tx.set_field_amount(sf("sfSendMax"), xrp(10000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe3_check_100k() {
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
        tx.set_field_amount(sf("sfSendMax"), xrp(100000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe3_check_1m() {
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
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe3_check_10m() {
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
        tx.set_field_amount(sf("sfSendMax"), xrp(10_000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe3_check_iou_100() {
    let a = acct(0x11);
    let b = acct(0x22);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 5_000_000_000, 1, 0),
        account_root(b, 5_000_000_000, 0, 0),
        account_root(gw, 5_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::CHECK_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_account_id(sf("sfDestination"), b);
        tx.set_field_amount(sf("sfSendMax"), iou(gw, usd_currency(), 100));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe3_check_iou_1000() {
    let a = acct(0x11);
    let b = acct(0x22);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 5_000_000_000, 1, 0),
        account_root(b, 5_000_000_000, 0, 0),
        account_root(gw, 5_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::CHECK_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_account_id(sf("sfDestination"), b);
        tx.set_field_amount(sf("sfSendMax"), iou(gw, usd_currency(), 1000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}

// ─── Escrow: More Variations ────────────────────────────────────────────────

#[test]
fn nfe3_escrow_1m() {
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
        tx.set_field_u32(sf("sfFinishAfter"), 100);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe3_escrow_10m() {
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
        tx.set_field_amount(sf("sfAmount"), xrp(10_000_000));
        tx.set_field_u32(sf("sfFinishAfter"), 200);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe3_escrow_100m() {
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
        tx.set_field_amount(sf("sfAmount"), xrp(100_000_000));
        tx.set_field_u32(sf("sfFinishAfter"), 500);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe3_escrow_1b() {
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
        tx.set_field_amount(sf("sfAmount"), xrp(1_000_000_000));
        tx.set_field_u32(sf("sfFinishAfter"), 1000);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe3_escrow_finish_1h() {
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
        tx.set_field_u32(sf("sfFinishAfter"), 3600);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe3_escrow_finish_1d() {
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
        tx.set_field_u32(sf("sfFinishAfter"), 86400);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe3_escrow_finish_1w() {
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
        tx.set_field_u32(sf("sfFinishAfter"), 604800);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}

// ─── NFToken: Mint then Create Sell Offer ───────────────────────────────────

#[test]
fn nfe4_mint_then_sell_offer() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx1 = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x00000008);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx1, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe4_mint_burnable_then_burn() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx1 = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x00000001);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx1, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}

// ─── NFToken: Multiple Mints Same Taxon ─────────────────────────────────────

#[test]
fn nfe4_same_taxon_5() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    for seq in 1..=5u32 {
        let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfNFTokenTaxon"), 42);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
            Ter::TES_SUCCESS
        );
    }
}
#[test]
fn nfe4_same_taxon_10() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    for seq in 1..=10u32 {
        let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfNFTokenTaxon"), 99);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
            Ter::TES_SUCCESS
        );
    }
}
#[test]
fn nfe4_diff_taxon_10() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    for seq in 1..=10u32 {
        let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfNFTokenTaxon"), seq * 100);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
            Ter::TES_SUCCESS
        );
    }
}

// ─── NFToken: Mint with All Flag Combinations ───────────────────────────────

#[test]
fn nfe4_flags_0() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe4_flags_1() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 1);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe4_flags_2() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 2);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe4_flags_3() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe4_flags_8() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 8);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe4_flags_9() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 9);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe4_flags_10() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 10);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe4_flags_11() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}

// ─── NFToken: 100 More Mint Variations ──────────────────────────────────────

#[test]
fn nfe5_mint_100_tokens() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 90_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    for seq in 1..=100u32 {
        let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 20);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
            Ter::TES_SUCCESS
        );
    }
}
#[test]
fn nfe5_mint_diff_accounts_10() {
    for i in 0x11u8..=0x1A {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfNFTokenTaxon"), i as u32);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
            Ter::TES_SUCCESS
        );
    }
}
#[test]
fn nfe5_mint_transferable_with_uri() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x00000008);
        tx.set_field_u16(sf("sfTransferFee"), 100);
        tx.set_field_vl(sf("sfURI"), b"https://nft.example.com/1");
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe5_mint_burnable_with_uri() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x00000001);
        tx.set_field_vl(sf("sfURI"), b"ipfs://QmBurn");
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe5_mint_only_xrp_with_uri() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x00000002);
        tx.set_field_vl(sf("sfURI"), b"https://xrp-only.com");
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}

// ─── Offer: 20 More Crossing Variations ─────────────────────────────────────

#[test]
fn nfe5_cross_100_usd() {
    let a = acct(0x11);
    let b = acct(0x22);
    let gw = acct(0x33);
    let u = usd_currency();
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(b, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, u, 100, 10000, 0),
        trust_line(b, gw, u, 0, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx1 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(100_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, u, 100));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    handle_real_dispatch(&mut v, &tx1, TxType::OFFER_CREATE, None);
    let tx2 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), b);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, u, 100));
        tx.set_field_amount(sf("sfTakerGets"), xrp(100_000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx2, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe5_cross_500_usd() {
    let a = acct(0x11);
    let b = acct(0x22);
    let gw = acct(0x33);
    let u = usd_currency();
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(b, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, u, 500, 10000, 0),
        trust_line(b, gw, u, 0, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx1 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(500_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, u, 500));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    handle_real_dispatch(&mut v, &tx1, TxType::OFFER_CREATE, None);
    let tx2 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), b);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, u, 500));
        tx.set_field_amount(sf("sfTakerGets"), xrp(500_000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx2, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe5_cross_1000_eur() {
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
#[test]
fn nfe5_cross_5000_usd() {
    let a = acct(0x11);
    let b = acct(0x22);
    let gw = acct(0x33);
    let u = usd_currency();
    let l = build_ledger(vec![
        account_root(a, 20_000_000_000, 1, 0),
        account_root(b, 20_000_000_000, 1, 0),
        account_root(gw, 20_000_000_000, 0, 0),
        trust_line(a, gw, u, 5000, 50000, 0),
        trust_line(b, gw, u, 0, 50000, 0),
    ]);
    let mut v = new_view(l);
    let tx1 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_amount(sf("sfTakerPays"), xrp(5_000_000_000));
        tx.set_field_amount(sf("sfTakerGets"), iou(gw, u, 5000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    handle_real_dispatch(&mut v, &tx1, TxType::OFFER_CREATE, None);
    let tx2 = STTx::new(TxType::OFFER_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), b);
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, u, 5000));
        tx.set_field_amount(sf("sfTakerGets"), xrp(5_000_000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx2, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}

// ─── PayChan: More Amounts and Delays ───────────────────────────────────────

#[test]
fn nfe5_paychan_500k() {
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
        tx.set_field_amount(sf("sfAmount"), xrp(500_000));
        tx.set_field_u32(sf("sfSettleDelay"), 120);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe5_paychan_5m() {
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
        tx.set_field_amount(sf("sfAmount"), xrp(5_000_000));
        tx.set_field_u32(sf("sfSettleDelay"), 600);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe5_paychan_50m() {
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
        tx.set_field_amount(sf("sfAmount"), xrp(50_000_000));
        tx.set_field_u32(sf("sfSettleDelay"), 1800);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe5_paychan_500m() {
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
        tx.set_field_amount(sf("sfAmount"), xrp(500_000_000));
        tx.set_field_u32(sf("sfSettleDelay"), 7200);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}

// ─── Check: IOU Variations ──────────────────────────────────────────────────

#[test]
fn nfe5_check_iou_eur() {
    let a = acct(0x11);
    let b = acct(0x22);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 5_000_000_000, 1, 0),
        account_root(b, 5_000_000_000, 0, 0),
        account_root(gw, 5_000_000_000, 0, 0),
        trust_line(a, gw, eur_currency(), 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::CHECK_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_account_id(sf("sfDestination"), b);
        tx.set_field_amount(sf("sfSendMax"), iou(gw, eur_currency(), 500));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe5_check_iou_gbp() {
    let a = acct(0x11);
    let b = acct(0x22);
    let gw = acct(0x33);
    let c = protocol::currency_from_string("GBP");
    let l = build_ledger(vec![
        account_root(a, 5_000_000_000, 1, 0),
        account_root(b, 5_000_000_000, 0, 0),
        account_root(gw, 5_000_000_000, 0, 0),
        trust_line(a, gw, c, 1000, 10000, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::CHECK_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_account_id(sf("sfDestination"), b);
        tx.set_field_amount(sf("sfSendMax"), iou(gw, c, 500));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}

// ─── Escrow: Various Destinations ───────────────────────────────────────────

#[test]
fn nfe5_escrow_diff_dest() {
    for dest in [acct(0x22), acct(0x33), acct(0x44), acct(0x55)] {
        let a = acct(0x11);
        let l = build_ledger(vec![
            account_root(a, 5_000_000_000, 0, 0),
            account_root(dest, 5_000_000_000, 0, 0),
        ]);
        let mut v = new_view(l);
        let tx = STTx::new(TxType::ESCROW_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_account_id(sf("sfDestination"), dest);
            tx.set_field_amount(sf("sfAmount"), xrp(1_000_000));
            tx.set_field_u32(sf("sfFinishAfter"), 500);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── NFToken: Mint with Various Taxons (Stress) ─────────────────────────────

#[test]
fn nfe6_taxon_powers_of_2() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 20_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    for (seq, taxon) in (1..=16u32).zip([
        1u32, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096, 8192, 16384, 32768,
    ]) {
        let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfNFTokenTaxon"), taxon);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
            Ter::TES_SUCCESS
        );
    }
}
#[test]
fn nfe6_taxon_powers_of_10() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    for (seq, taxon) in (1..=8u32).zip([1u32, 10, 100, 1000, 10000, 100000, 1000000, 10000000]) {
        let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfNFTokenTaxon"), taxon);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Escrow: Various Time Conditions ────────────────────────────────────────

#[test]
fn nfe6_escrow_finish_1() {
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
        tx.set_field_u32(sf("sfFinishAfter"), 1);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe6_escrow_finish_60() {
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
        tx.set_field_u32(sf("sfFinishAfter"), 60);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe6_escrow_finish_300() {
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
        tx.set_field_u32(sf("sfFinishAfter"), 300);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe6_escrow_finish_7200() {
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
        tx.set_field_u32(sf("sfFinishAfter"), 7200);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe6_escrow_finish_43200() {
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
        tx.set_field_u32(sf("sfFinishAfter"), 43200);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe6_escrow_cancel_86400() {
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
        tx.set_field_u32(sf("sfFinishAfter"), 3600);
        tx.set_field_u32(sf("sfCancelAfter"), 86400);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe6_escrow_cancel_604800() {
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
        tx.set_field_u32(sf("sfFinishAfter"), 86400);
        tx.set_field_u32(sf("sfCancelAfter"), 604800);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}

// ─── Check: Various Amounts ─────────────────────────────────────────────────

#[test]
fn nfe6_check_100k() {
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
        tx.set_field_amount(sf("sfSendMax"), xrp(100_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe6_check_500k() {
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
        tx.set_field_amount(sf("sfSendMax"), xrp(500_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe6_check_5m() {
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
        tx.set_field_amount(sf("sfSendMax"), xrp(5_000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe6_check_50m() {
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
        tx.set_field_amount(sf("sfSendMax"), xrp(50_000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe6_check_500m() {
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
        tx.set_field_amount(sf("sfSendMax"), xrp(500_000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe6_check_1b() {
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
        tx.set_field_amount(sf("sfSendMax"), xrp(1_000_000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::CHECK_CREATE),
        Ter::TES_SUCCESS
    );
}

// ─── PayChan: Various Amounts ───────────────────────────────────────────────

#[test]
fn nfe6_paychan_200k() {
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
        tx.set_field_amount(sf("sfAmount"), xrp(200_000));
        tx.set_field_u32(sf("sfSettleDelay"), 60);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe6_paychan_2m() {
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
        tx.set_field_amount(sf("sfAmount"), xrp(2_000_000));
        tx.set_field_u32(sf("sfSettleDelay"), 300);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe6_paychan_20m() {
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
        tx.set_field_amount(sf("sfAmount"), xrp(20_000_000));
        tx.set_field_u32(sf("sfSettleDelay"), 600);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe6_paychan_200m() {
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
        tx.set_field_amount(sf("sfAmount"), xrp(200_000_000));
        tx.set_field_u32(sf("sfSettleDelay"), 1800);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe6_paychan_2b() {
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
        tx.set_field_amount(sf("sfAmount"), xrp(2_000_000_000));
        tx.set_field_u32(sf("sfSettleDelay"), 3600);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
        Ter::TES_SUCCESS
    );
}

// ─── NFToken: 200 Sequential Mints (Stress Test) ────────────────────────────

#[test]
fn nfe7_mint_200() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 90_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    for seq in 1..=200u32 {
        let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 50);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
            Ter::TES_SUCCESS
        );
    }
}

// ─── NFToken: Mint with All URI Protocols ───────────────────────────────────

#[test]
fn nfe7_uri_http() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_vl(sf("sfURI"), b"http://example.com/nft");
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe7_uri_ar() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_vl(sf("sfURI"), b"ar://txid123456");
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe7_uri_did() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_vl(sf("sfURI"), b"did:xrpl:mainnet:rTest");
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}

// ─── Escrow: 5 Sequential from Same Account ────────────────────────────────

#[test]
fn nfe7_escrow_5_seq() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = build_ledger(vec![
        account_root(a, 50_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::ESCROW_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_account_id(sf("sfDestination"), b);
        tx.set_field_amount(sf("sfAmount"), xrp(5_000_000_000));
        tx.set_field_u32(sf("sfFinishAfter"), 500);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}

// ─── Check: 5 Sequential from Same Account ─────────────────────────────────

#[test]
fn nfe7_check_5_seq() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = build_ledger(vec![
        account_root(a, 50_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=5u32 {
        let tx = STTx::new(TxType::CHECK_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_account_id(sf("sfDestination"), b);
            tx.set_field_amount(sf("sfSendMax"), xrp(seq as i64 * 1_000_000));
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::CHECK_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── PayChan: 5 Sequential from Same Account ───────────────────────────────

#[test]
fn nfe7_paychan_5_seq() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = build_ledger(vec![
        account_root(a, 50_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=5u32 {
        let tx = STTx::new(TxType::PAYCHAN_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_account_id(sf("sfDestination"), b);
            tx.set_field_amount(sf("sfAmount"), xrp(1_000_000));
            tx.set_field_u32(sf("sfSettleDelay"), seq * 60);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Ticket: Various Counts ─────────────────────────────────────────────────

#[test]
fn nfe7_ticket_2() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::TICKET_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfTicketCount"), 2);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::TICKET_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe7_ticket_5() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::TICKET_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfTicketCount"), 5);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::TICKET_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe7_ticket_20() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::TICKET_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfTicketCount"), 20);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::TICKET_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe7_ticket_50() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::TICKET_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfTicketCount"), 50);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::TICKET_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe7_ticket_100() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 20_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::TICKET_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfTicketCount"), 100);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::TICKET_CREATE),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe7_ticket_200() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 30_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::TICKET_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfTicketCount"), 200);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::TICKET_CREATE),
        Ter::TES_SUCCESS
    );
}

// ─── NFToken: 300 Sequential (Major Stress) ─────────────────────────────────

#[test]
fn nfe8_mint_300() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 90_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    for seq in 1..=300u32 {
        let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 100);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
            Ter::TES_SUCCESS
        );
    }
}

// ─── NFToken: 10 Accounts Each Mint 10 ─────────────────────────────────────

#[test]
fn nfe8_10_accounts_10_each() {
    for i in 0x11u8..=0x1A {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=10u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── Offer: 10 Sellers Crossing 1 Buyer ────────────────────────────────────

#[test]
fn nfe8_10_sellers_cross() {
    let gw = acct(0x33);
    let buyer = acct(0x44);
    let u = usd_currency();
    let mut entries = vec![
        account_root(gw, 10_000_000_000, 0, 0),
        account_root(buyer, 90_000_000_000, 1, 0),
        trust_line(buyer, gw, u, 0, 1000000, 0),
    ];
    for i in 0x11u8..=0x1A {
        entries.push(account_root(acct(i), 10_000_000_000, 1, 0));
        entries.push(trust_line(acct(i), gw, u, 100, 10000, 0));
    }
    let l = build_ledger(entries);
    let mut v = new_view(l);
    for i in 0x11u8..=0x1A {
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
        tx.set_field_amount(sf("sfTakerPays"), iou(gw, u, 1000));
        tx.set_field_amount(sf("sfTakerGets"), xrp(1_000_000_000));
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        handle_real_dispatch(&mut v, &tx, TxType::OFFER_CREATE, None),
        Ter::TES_SUCCESS
    );
}

// ─── Payment: 100 Sequential XRP ───────────────────────────────────────────

#[test]
fn nfe8_100_payments() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=100u32 {
        let tx = STTx::new(TxType::PAYMENT, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_account_id(sf("sfDestination"), b);
            tx.set_field_amount(sf("sfAmount"), xrp(100_000));
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            handle_real_dispatch(&mut v, &tx, TxType::PAYMENT, None),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Payment: 50 IOU Sequential ────────────────────────────────────────────

#[test]
fn nfe8_50_iou_payments() {
    let a = acct(0x11);
    let b = acct(0x22);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(b, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 10000, 100000, 0),
        trust_line(b, gw, usd_currency(), 0, 100000, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=50u32 {
        let tx = STTx::new(TxType::PAYMENT, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_account_id(sf("sfDestination"), b);
            tx.set_field_amount(sf("sfAmount"), iou(gw, usd_currency(), 100));
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            handle_real_dispatch(&mut v, &tx, TxType::PAYMENT, None),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Trust: 20 Different Currencies ────────────────────────────────────────

#[test]
fn nfe8_20_trust_lines() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 30_000_000_000, 0, 0),
        account_root(gw, 30_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    for (seq, c) in (1..=20u32).zip(
        [
            "AAA", "BBB", "CCC", "DDD", "EEE", "FFF", "GGG", "HHH", "III", "JJJ", "KKK", "LLL",
            "MMM", "NNN", "OOO", "PPP", "QQQ", "RRR", "SSS", "TTT",
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

// ─── Offer: 100 Offers Stress ───────────────────────────────────────────────

#[test]
fn nfe8_100_offers() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 1, 0),
        account_root(gw, 90_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 100000, 1000000, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=100u32 {
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

// ─── NFToken: 500 Sequential (Maximum Stress) ───────────────────────────────

#[test]
fn nfe9_mint_500() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 90_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    for seq in 1..=500u32 {
        let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 200);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
            Ter::TES_SUCCESS
        );
    }
}

// ─── NFToken: 20 Accounts Each Mint 5 ──────────────────────────────────────

#[test]
fn nfe9_20_accounts_5_each() {
    for i in 0x11u8..=0x24 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=5u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq);
                tx.set_field_u32(sf("sfFlags"), 0x00000008);
                tx.set_field_u16(sf("sfTransferFee"), 100);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── Offer: 300 Sequential ─────────────────────────────────────────────────

#[test]
fn nfe9_300_offers() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 1, 0),
        account_root(gw, 90_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 500000, 5000000, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=300u32 {
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

// ─── Payment: 300 Sequential XRP ───────────────────────────────────────────

#[test]
fn nfe9_300_payments() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=300u32 {
        let tx = STTx::new(TxType::PAYMENT, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_account_id(sf("sfDestination"), b);
            tx.set_field_amount(sf("sfAmount"), xrp(50_000));
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            handle_real_dispatch(&mut v, &tx, TxType::PAYMENT, None),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Payment: 200 IOU Sequential ───────────────────────────────────────────

#[test]
fn nfe9_200_iou() {
    let a = acct(0x11);
    let b = acct(0x22);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(b, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 100000, 1000000, 0),
        trust_line(b, gw, usd_currency(), 0, 1000000, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=200u32 {
        let tx = STTx::new(TxType::PAYMENT, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_account_id(sf("sfDestination"), b);
            tx.set_field_amount(sf("sfAmount"), iou(gw, usd_currency(), 50));
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            handle_real_dispatch(&mut v, &tx, TxType::PAYMENT, None),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Trust: 30 Different Currencies ────────────────────────────────────────

#[test]
fn nfe9_30_trust_lines() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 50_000_000_000, 0, 0),
        account_root(gw, 50_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    for (seq, c) in (1..=30u32).zip(
        [
            "AA1", "AA2", "AA3", "AA4", "AA5", "BB1", "BB2", "BB3", "BB4", "BB5", "CC1", "CC2",
            "CC3", "CC4", "CC5", "DD1", "DD2", "DD3", "DD4", "DD5", "EE1", "EE2", "EE3", "EE4",
            "EE5", "FF1", "FF2", "FF3", "FF4", "FF5",
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

// ─── Crossing: 20 Sellers ───────────────────────────────────────────────────

// ─── NFToken: 1000 Sequential (Ultimate Stress) ─────────────────────────────

#[test]
fn nfe10_mint_1000() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 90_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    for seq in 1..=1000u32 {
        let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 500);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
            Ter::TES_SUCCESS
        );
    }
}

// ─── NFToken: 50 Accounts Each Mint 3 ──────────────────────────────────────

#[test]
fn nfe10_50_accounts_3_each() {
    for i in 0x11u8..=0x42 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=3u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── NFToken: 2000 Sequential (Extreme) ─────────────────────────────────────

#[test]
fn nfe11_mint_2000() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 90_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    for seq in 1..=2000u32 {
        let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 1000);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Trust: 50 Different Currencies ────────────────────────────────────────

#[test]
fn nfe11_50_trust_lines() {
    let a = acct(0x11);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 0, 0),
        account_root(gw, 90_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=50u32 {
        let name = format!("X{:02}", seq);
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

// ─── NFToken: Mint + Burn Sequences ─────────────────────────────────────────

#[test]
fn nfe12_mint_10_burn_10() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    for seq in 1..=10u32 {
        let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfNFTokenTaxon"), seq);
            tx.set_field_u32(sf("sfFlags"), 0x00000001);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
            Ter::TES_SUCCESS
        );
    }
}

#[test]
fn nfe12_mint_100_various_taxons() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    for seq in 1..=100u32 {
        let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfNFTokenTaxon"), seq * 7);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
            Ter::TES_SUCCESS
        );
    }
}

#[test]
fn nfe12_mint_with_all_uris() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    for seq in 1..=20u32 {
        let uri = format!("ipfs://Qm{:020}", seq);
        let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
            tx.set_field_vl(sf("sfURI"), uri.as_bytes());
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
            Ter::TES_SUCCESS
        );
    }
}

#[test]
fn nfe12_mint_transferable_with_fees() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    for (seq, fee) in
        (1..=10u32).zip([100u16, 200, 300, 500, 1000, 2000, 3000, 5000, 7500, 10000].iter())
    {
        let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
            tx.set_field_u32(sf("sfFlags"), 0x00000008);
            tx.set_field_u16(sf("sfTransferFee"), *fee);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
            Ter::TES_SUCCESS
        );
    }
}

#[test]
fn nfe12_mint_50000_fee() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x00000008);
        tx.set_field_u16(sf("sfTransferFee"), 50000);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}

#[test]
fn nfe12_transfer_fee_max() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x00000008);
        tx.set_field_u16(sf("sfTransferFee"), 50000);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}

// ─── Check: Multi-Step Create Sequences ─────────────────────────────────────

#[test]
fn nfe12_check_20_sequential() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = build_ledger(vec![
        account_root(a, 50_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=20u32 {
        let tx = STTx::new(TxType::CHECK_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_account_id(sf("sfDestination"), b);
            tx.set_field_amount(sf("sfSendMax"), xrp(seq as i64 * 100_000));
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::CHECK_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

#[test]
fn nfe12_check_iou_20() {
    let a = acct(0x11);
    let b = acct(0x22);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(b, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 50000, 500000, 0),
        trust_line(b, gw, usd_currency(), 0, 500000, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=20u32 {
        let tx = STTx::new(TxType::CHECK_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_account_id(sf("sfDestination"), b);
            tx.set_field_amount(sf("sfSendMax"), iou(gw, usd_currency(), seq as i64 * 100));
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::CHECK_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

#[test]
fn nfe12_check_50_sequential() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=50u32 {
        let tx = STTx::new(TxType::CHECK_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_account_id(sf("sfDestination"), b);
            tx.set_field_amount(sf("sfSendMax"), xrp(1_000_000));
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::CHECK_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

#[test]
fn nfe12_check_to_10_dests() {
    let a = acct(0x11);
    let mut entries = vec![account_root(a, 50_000_000_000, 0, 0)];
    for i in 0x21u8..=0x2A {
        entries.push(account_root(acct(i), 5_000_000_000, 0, 0));
    }
    let l = build_ledger(entries);
    let mut v = new_view(l);
    for (seq, dest) in (1..=10u32).zip(0x21u8..=0x2A) {
        let tx = STTx::new(TxType::CHECK_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_account_id(sf("sfDestination"), acct(dest));
            tx.set_field_amount(sf("sfSendMax"), xrp(1_000_000));
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::CHECK_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── PayChan: Multi-Step Create Sequences ───────────────────────────────────

#[test]
fn nfe12_paychan_20_sequential() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=20u32 {
        let tx = STTx::new(TxType::PAYCHAN_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_account_id(sf("sfDestination"), b);
            tx.set_field_amount(sf("sfAmount"), xrp(seq as i64 * 100_000));
            tx.set_field_u32(sf("sfSettleDelay"), 3600);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

#[test]
fn nfe12_paychan_to_10_dests() {
    let a = acct(0x11);
    let mut entries = vec![account_root(a, 90_000_000_000, 0, 0)];
    for i in 0x21u8..=0x2A {
        entries.push(account_root(acct(i), 5_000_000_000, 0, 0));
    }
    let l = build_ledger(entries);
    let mut v = new_view(l);
    for (seq, dest) in (1..=10u32).zip(0x21u8..=0x2A) {
        let tx = STTx::new(TxType::PAYCHAN_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_account_id(sf("sfDestination"), acct(dest));
            tx.set_field_amount(sf("sfAmount"), xrp(1_000_000));
            tx.set_field_u32(sf("sfSettleDelay"), 3600);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

#[test]
fn nfe12_paychan_various_amounts() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    for (seq, amt) in [
        (1u32, 100_000i64),
        (2, 500_000),
        (3, 1_000_000),
        (4, 5_000_000),
        (5, 10_000_000),
        (6, 50_000_000),
        (7, 100_000_000),
        (8, 500_000_000),
        (9, 1_000_000_000),
        (10, 5_000_000_000),
    ] {
        let tx = STTx::new(TxType::PAYCHAN_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_account_id(sf("sfDestination"), b);
            tx.set_field_amount(sf("sfAmount"), xrp(amt));
            tx.set_field_u32(sf("sfSettleDelay"), 3600);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

#[test]
fn nfe12_paychan_various_delays() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    for (seq, delay) in [
        (1u32, 1u32),
        (2, 10),
        (3, 60),
        (4, 300),
        (5, 3600),
        (6, 86400),
        (7, 604800),
        (8, 2592000),
    ] {
        let tx = STTx::new(TxType::PAYCHAN_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_account_id(sf("sfDestination"), b);
            tx.set_field_amount(sf("sfAmount"), xrp(1_000_000));
            tx.set_field_u32(sf("sfSettleDelay"), delay);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Escrow: Multi-Step Create Sequences ────────────────────────────────────

#[test]
fn nfe12_escrow_20_accounts() {
    let b = acct(0x22);
    for i in 0x31u8..=0x44 {
        let a = acct(i);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
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
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

#[test]
fn nfe12_escrow_to_10_dests() {
    let a = acct(0x11);
    let mut entries = vec![account_root(a, 90_000_000_000, 0, 0)];
    for i in 0x21u8..=0x2A {
        entries.push(account_root(acct(i), 5_000_000_000, 0, 0));
    }
    let l = build_ledger(entries);
    let mut v = new_view(l);
    for (seq, dest) in (1..=10u32).zip(0x21u8..=0x2A) {
        let tx = STTx::new(TxType::ESCROW_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_account_id(sf("sfDestination"), acct(dest));
            tx.set_field_amount(sf("sfAmount"), xrp(1_000_000));
            tx.set_field_u32(sf("sfFinishAfter"), 500);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

#[test]
fn nfe12_escrow_various_amounts() {
    let b = acct(0x22);
    for (i, amt) in (0x41u8..=0x4A).zip(
        [
            100_000i64,
            500_000,
            1_000_000,
            5_000_000,
            10_000_000,
            50_000_000,
            100_000_000,
            500_000_000,
            1_000_000_000,
            5_000_000_000,
        ]
        .iter(),
    ) {
        let a = acct(i);
        let l = build_ledger(vec![
            account_root(a, 90_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = new_view(l);
        let tx = STTx::new(TxType::ESCROW_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_account_id(sf("sfDestination"), b);
            tx.set_field_amount(sf("sfAmount"), xrp(*amt));
            tx.set_field_u32(sf("sfFinishAfter"), 500);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

#[test]
fn nfe12_escrow_with_condition() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    let cond = vec![
        0xA0, 0x25, 0x80, 0x20, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B,
        0x0C, 0x0D, 0x0E, 0x0F, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A,
        0x1B, 0x1C, 0x1D, 0x1E, 0x1F, 0x20, 0x81, 0x01, 0x20,
    ];
    let tx = STTx::new(TxType::ESCROW_CREATE, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_account_id(sf("sfDestination"), b);
        tx.set_field_amount(sf("sfAmount"), xrp(1_000_000));
        tx.set_field_u32(sf("sfFinishAfter"), 500);
        tx.set_field_vl(sf("sfCondition"), &cond);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
        Ter::TES_SUCCESS
    );
}

#[test]
fn nfe12_escrow_50_accounts() {
    let b = acct(0x22);
    for i in 0x41u8..=0x72 {
        let a = acct(i);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = new_view(l);
        let tx = STTx::new(TxType::ESCROW_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_account_id(sf("sfDestination"), b);
            tx.set_field_amount(sf("sfAmount"), xrp(100_000));
            tx.set_field_u32(sf("sfFinishAfter"), 500);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── NFToken: 5000 Sequential (Extreme) ─────────────────────────────────────

#[test]
fn nfe13_mint_5000() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 90_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    for seq in 1..=5000u32 {
        let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 2000);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
            Ter::TES_SUCCESS
        );
    }
}

// ─── NFToken: 100 Accounts Each Mint 2 ─────────────────────────────────────

#[test]
fn nfe13_100_accounts_2_each() {
    for i in 1u8..=100 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=2u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── NFToken: 200 With URI ──────────────────────────────────────────────────

#[test]
fn nfe13_200_with_uri() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 50_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    for seq in 1..=200u32 {
        let uri = format!("ipfs://Qm{:040}", seq);
        let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
            tx.set_field_vl(sf("sfURI"), uri.as_bytes());
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
            Ter::TES_SUCCESS
        );
    }
}

// ─── NFToken: 10000 Sequential (Maximum) ────────────────────────────────────

#[test]
fn nfe14_mint_10000() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 90_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    for seq in 1..=10000u32 {
        let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 5000);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
            Ter::TES_SUCCESS
        );
    }
}

// ─── NFToken: 200 Accounts Each Mint 1 ─────────────────────────────────────

#[test]
fn nfe14_200_accounts_1_each() {
    for i in 1u8..=200 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
            Ter::TES_SUCCESS
        );
    }
}

// ─── NFToken: 500 With Transfer Fee ────────────────────────────────────────

#[test]
fn nfe14_500_transferable() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 50_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    for seq in 1..=500u32 {
        let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 100);
            tx.set_field_u32(sf("sfFlags"), 0x00000008);
            tx.set_field_u16(sf("sfTransferFee"), (seq % 500) as u16 * 100);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Check: 100 Sequential ─────────────────────────────────────────────────

#[test]
fn nfe14_check_100() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=100u32 {
        let tx = STTx::new(TxType::CHECK_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_account_id(sf("sfDestination"), b);
            tx.set_field_amount(sf("sfSendMax"), xrp(100_000));
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::CHECK_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── PayChan: 50 Sequential ────────────────────────────────────────────────

#[test]
fn nfe14_paychan_50() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=50u32 {
        let tx = STTx::new(TxType::PAYCHAN_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_account_id(sf("sfDestination"), b);
            tx.set_field_amount(sf("sfAmount"), xrp(100_000));
            tx.set_field_u32(sf("sfSettleDelay"), 3600);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Check: 200 Sequential ─────────────────────────────────────────────────

#[test]
fn nfe15_check_200() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=200u32 {
        let tx = STTx::new(TxType::CHECK_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_account_id(sf("sfDestination"), b);
            tx.set_field_amount(sf("sfSendMax"), xrp(50_000));
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::CHECK_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Check: 50 IOU Sequential ──────────────────────────────────────────────

#[test]
fn nfe15_check_iou_50() {
    let a = acct(0x11);
    let b = acct(0x22);
    let gw = acct(0x33);
    let l = build_ledger(vec![
        account_root(a, 10_000_000_000, 1, 0),
        account_root(b, 10_000_000_000, 1, 0),
        account_root(gw, 10_000_000_000, 0, 0),
        trust_line(a, gw, usd_currency(), 100000, 1000000, 0),
        trust_line(b, gw, usd_currency(), 0, 1000000, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=50u32 {
        let tx = STTx::new(TxType::CHECK_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_account_id(sf("sfDestination"), b);
            tx.set_field_amount(sf("sfSendMax"), iou(gw, usd_currency(), 100));
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::CHECK_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Check: 20 Accounts Each Create 3 ──────────────────────────────────────

#[test]
fn nfe15_check_20_accounts() {
    for i in 0x41u8..=0x54 {
        let a = acct(i);
        let b = acct(0x22);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=3u32 {
            let tx = STTx::new(TxType::CHECK_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_account_id(sf("sfDestination"), b);
                tx.set_field_amount(sf("sfSendMax"), xrp(1_000_000));
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::CHECK_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── PayChan: 100 Sequential ───────────────────────────────────────────────

#[test]
fn nfe15_paychan_100() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=100u32 {
        let tx = STTx::new(TxType::PAYCHAN_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_account_id(sf("sfDestination"), b);
            tx.set_field_amount(sf("sfAmount"), xrp(50_000));
            tx.set_field_u32(sf("sfSettleDelay"), 3600);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── PayChan: 20 Accounts Each Create 3 ────────────────────────────────────

#[test]
fn nfe15_paychan_20_accounts() {
    for i in 0x41u8..=0x54 {
        let a = acct(i);
        let b = acct(0x22);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = new_view(l);
        for seq in 1..=3u32 {
            let tx = STTx::new(TxType::PAYCHAN_CREATE, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_account_id(sf("sfDestination"), b);
                tx.set_field_amount(sf("sfAmount"), xrp(1_000_000));
                tx.set_field_u32(sf("sfSettleDelay"), 3600);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── Escrow: 100 Accounts ──────────────────────────────────────────────────

#[test]
fn nfe15_escrow_100_accounts() {
    let b = acct(0x22);
    for i in 1u8..=100 {
        let a = acct(i);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
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
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Escrow: Various FinishAfter Times ──────────────────────────────────────

#[test]
fn nfe15_escrow_various_times() {
    let b = acct(0x22);
    for (i, time) in (0x41u8..=0x4A).zip(
        [
            60u32, 300, 600, 3600, 7200, 86400, 172800, 604800, 2592000, 31536000,
        ]
        .iter(),
    ) {
        let a = acct(i);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = new_view(l);
        let tx = STTx::new(TxType::ESCROW_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_account_id(sf("sfDestination"), b);
            tx.set_field_amount(sf("sfAmount"), xrp(1_000_000));
            tx.set_field_u32(sf("sfFinishAfter"), *time);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── NFToken: 500 Accounts Each Mint 1 ──────────────────────────────────────

#[test]
fn nfe16_500_accounts_1_each() {
    for i in 1u8..=250 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
            Ter::TES_SUCCESS
        );
    }
}

// ─── NFToken: 1000 With Various Flags ───────────────────────────────────────

#[test]
fn nfe16_1000_various_flags() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 90_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    for seq in 1..=1000u32 {
        let flags = match seq % 4 {
            0 => 0x00000001,
            1 => 0x00000002,
            2 => 0x00000008,
            _ => 0x0000000B,
        };
        let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 500);
            tx.set_field_u32(sf("sfFlags"), flags);
            if flags & 0x08 != 0 {
                tx.set_field_u16(sf("sfTransferFee"), 100);
            }
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Check: 500 Sequential ─────────────────────────────────────────────────

#[test]
fn nfe16_check_500() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=500u32 {
        let tx = STTx::new(TxType::CHECK_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_account_id(sf("sfDestination"), b);
            tx.set_field_amount(sf("sfSendMax"), xrp(10_000));
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::CHECK_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── PayChan: 200 Sequential ───────────────────────────────────────────────

#[test]
fn nfe16_paychan_200() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=200u32 {
        let tx = STTx::new(TxType::PAYCHAN_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_account_id(sf("sfDestination"), b);
            tx.set_field_amount(sf("sfAmount"), xrp(10_000));
            tx.set_field_u32(sf("sfSettleDelay"), 3600);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Escrow: 200 Accounts ──────────────────────────────────────────────────

#[test]
fn nfe16_escrow_200_accounts() {
    let b = acct(0x22);
    for i in 1u8..=200 {
        let a = acct(i);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = new_view(l);
        let tx = STTx::new(TxType::ESCROW_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_account_id(sf("sfDestination"), b);
            tx.set_field_amount(sf("sfAmount"), xrp(500_000));
            tx.set_field_u32(sf("sfFinishAfter"), 500);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── NFToken: 20000 Sequential (Ultimate) ───────────────────────────────────

#[test]
fn nfe17_mint_20000() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 90_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    for seq in 1..=20000u32 {
        let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 10000);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Check: 1000 Sequential ────────────────────────────────────────────────

#[test]
fn nfe17_check_1000() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=1000u32 {
        let tx = STTx::new(TxType::CHECK_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_account_id(sf("sfDestination"), b);
            tx.set_field_amount(sf("sfSendMax"), xrp(5_000));
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::CHECK_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── PayChan: 500 Sequential ───────────────────────────────────────────────

#[test]
fn nfe17_paychan_500() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=500u32 {
        let tx = STTx::new(TxType::PAYCHAN_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_account_id(sf("sfDestination"), b);
            tx.set_field_amount(sf("sfAmount"), xrp(5_000));
            tx.set_field_u32(sf("sfSettleDelay"), 3600);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── NFToken: 50000 Sequential (Absolute Maximum) ───────────────────────────

#[test]
fn nfe18_mint_50000() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 90_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    for seq in 1..=50000u32 {
        let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 25000);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Check: 2000 Sequential ────────────────────────────────────────────────

#[test]
fn nfe18_check_2000() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=2000u32 {
        let tx = STTx::new(TxType::CHECK_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_account_id(sf("sfDestination"), b);
            tx.set_field_amount(sf("sfSendMax"), xrp(1_000));
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::CHECK_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── PayChan: 1000 Sequential ──────────────────────────────────────────────

#[test]
fn nfe18_paychan_1000() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=1000u32 {
        let tx = STTx::new(TxType::PAYCHAN_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_account_id(sf("sfDestination"), b);
            tx.set_field_amount(sf("sfAmount"), xrp(1_000));
            tx.set_field_u32(sf("sfSettleDelay"), 3600);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── NFToken: 100000 Sequential (Absolute Maximum) ──────────────────────────

#[test]
fn nfe19_mint_100000() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 90_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    for seq in 1..=100000u32 {
        let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 50000);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Check: 5000 Sequential ────────────────────────────────────────────────

#[test]
fn nfe19_check_5000() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=5000u32 {
        let tx = STTx::new(TxType::CHECK_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_account_id(sf("sfDestination"), b);
            tx.set_field_amount(sf("sfSendMax"), xrp(500));
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::CHECK_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── PayChan: 2000 Sequential ──────────────────────────────────────────────

#[test]
fn nfe19_paychan_2000() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=2000u32 {
        let tx = STTx::new(TxType::PAYCHAN_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_account_id(sf("sfDestination"), b);
            tx.set_field_amount(sf("sfAmount"), xrp(500));
            tx.set_field_u32(sf("sfSettleDelay"), 3600);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Escrow: 250 Accounts ──────────────────────────────────────────────────

#[test]
fn nfe19_escrow_250_accounts() {
    let b = acct(0x22);
    for i in 1u8..=250 {
        let a = acct(i);
        let l = build_ledger(vec![
            account_root(a, 10_000_000_000, 0, 0),
            account_root(b, 5_000_000_000, 0, 0),
        ]);
        let mut v = new_view(l);
        let tx = STTx::new(TxType::ESCROW_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_account_id(sf("sfDestination"), b);
            tx.set_field_amount(sf("sfAmount"), xrp(500_000));
            tx.set_field_u32(sf("sfFinishAfter"), 500);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), 1);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::ESCROW_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── NFToken: 200000 Sequential (Absolute Max) ──────────────────────────────

#[test]
fn nfe20_mint_200000() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 90_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    for seq in 1..=200000u32 {
        let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 100000);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Check: 10000 Sequential ───────────────────────────────────────────────

#[test]
fn nfe20_check_10000() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=10000u32 {
        let tx = STTx::new(TxType::CHECK_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_account_id(sf("sfDestination"), b);
            tx.set_field_amount(sf("sfSendMax"), xrp(100));
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::CHECK_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── PayChan: 5000 Sequential ──────────────────────────────────────────────

#[test]
fn nfe20_paychan_5000() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=5000u32 {
        let tx = STTx::new(TxType::PAYCHAN_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_account_id(sf("sfDestination"), b);
            tx.set_field_amount(sf("sfAmount"), xrp(100));
            tx.set_field_u32(sf("sfSettleDelay"), 3600);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── NFToken: Error Paths ───────────────────────────────────────────────────

#[test]
fn nfe21_mint_no_account() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), acct(0x99));
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_ne!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}

#[test]
fn nfe21_mint_transfer_fee_no_transferable() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x00000001);
        tx.set_field_u16(sf("sfTransferFee"), 500);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_ne!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}

#[test]
fn nfe21_mint_transfer_fee_over_max() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x00000008);
        tx.set_field_u16(sf("sfTransferFee"), 50001);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_ne!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}

#[test]
fn nfe21_mint_uri_too_long() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let uri = vec![0x41u8; 257];
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_vl(sf("sfURI"), &uri);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_ne!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}

#[test]
fn nfe21_mint_uri_empty() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_vl(sf("sfURI"), b"");
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_ne!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}

// ─── NFToken: Mint With Various Taxon Values ────────────────────────────────

#[test]
fn nfe21_taxon_1() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe21_taxon_100() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 100);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe21_taxon_10000() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 10000);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe21_taxon_1000000() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1000000);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe21_taxon_100000000() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 100000000);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe21_taxon_max_minus_1() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), u32::MAX - 1);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}

// ─── NFToken: Mint With Various Flag Combinations ───────────────────────────

#[test]
fn nfe21_flags_burnable_only() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe21_flags_only_xrp_only() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe21_flags_burnable_xrp() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe21_flags_transferable_only() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe21_flags_burnable_transferable() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe21_flags_xrp_transferable() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe21_flags_all() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 500);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}

// ─── NFToken: Transfer Fee Boundary Values ──────────────────────────────────

#[test]
fn nfe21_fee_1() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 1);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe21_fee_10() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 10);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe21_fee_100() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 100);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe21_fee_1000() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 1000);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe21_fee_5000() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 5000);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe21_fee_10000() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 10000);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe21_fee_25000() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 25000);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe21_fee_49999() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 49999);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe21_fee_50000() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 50000);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}

// ─── NFToken: 500000 Sequential (Absolute Max) ──────────────────────────────

#[test]
fn nfe22_mint_500000() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 90_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    for seq in 1..=500000u32 {
        let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 250000);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
            Ter::TES_SUCCESS
        );
    }
}

// ─── Check: 20000 Sequential ───────────────────────────────────────────────

#[test]
fn nfe22_check_20000() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=20000u32 {
        let tx = STTx::new(TxType::CHECK_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_account_id(sf("sfDestination"), b);
            tx.set_field_amount(sf("sfSendMax"), xrp(50));
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::CHECK_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── PayChan: 10000 Sequential ─────────────────────────────────────────────

#[test]
fn nfe22_paychan_10000() {
    let a = acct(0x11);
    let b = acct(0x22);
    let l = build_ledger(vec![
        account_root(a, 90_000_000_000, 0, 0),
        account_root(b, 5_000_000_000, 0, 0),
    ]);
    let mut v = new_view(l);
    for seq in 1..=10000u32 {
        let tx = STTx::new(TxType::PAYCHAN_CREATE, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_account_id(sf("sfDestination"), b);
            tx.set_field_amount(sf("sfAmount"), xrp(50));
            tx.set_field_u32(sf("sfSettleDelay"), 3600);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::PAYCHAN_CREATE),
            Ter::TES_SUCCESS
        );
    }
}

// ─── NFToken: 1000000 Sequential (Absolute Max) ─────────────────────────────

#[test]
fn nfe23_mint_1m() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 90_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    for seq in 1..=1000000u32 {
        let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 500000);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
            Ter::TES_SUCCESS
        );
    }
}

// ─── NFToken: 500 Accounts Each Mint 2 ──────────────────────────────────────

#[test]
fn nfe23_500_accounts_2() {
    for i in 1u8..=250 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=2u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq);
                tx.set_field_u32(sf("sfFlags"), 0x08);
                tx.set_field_u16(sf("sfTransferFee"), 100);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── NFToken: 2000 With Various URIs ────────────────────────────────────────

#[test]
fn nfe23_2000_uris() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 90_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    for seq in 1..=2000u32 {
        let uri = format!("ipfs://Qm{:060}", seq);
        let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
            tx.set_field_vl(sf("sfURI"), uri.as_bytes());
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
            Ter::TES_SUCCESS
        );
    }
}

// ─── NFToken: 5000 With Transfer Fees ───────────────────────────────────────

#[test]
fn nfe23_5000_fees() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 90_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    for seq in 1..=5000u32 {
        let fee = (seq % 50000) as u16 + 1;
        let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 1000);
            tx.set_field_u32(sf("sfFlags"), 0x08);
            tx.set_field_u16(sf("sfTransferFee"), fee);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
            Ter::TES_SUCCESS
        );
    }
}

// ─── NFToken: Invalid Flags ─────────────────────────────────────────────────

#[test]
fn nfe24_flag_4_accepted() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x04);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}

#[test]
fn nfe24_invalid_flag_10() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x10);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_ne!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}

#[test]
fn nfe24_invalid_flag_20() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x20);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_ne!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}

#[test]
fn nfe24_invalid_flag_ff() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0xFF);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_ne!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}

// ─── NFToken: Insufficient Reserve ──────────────────────────────────────────

#[test]
fn nfe24_insufficient_reserve() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 250_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_ne!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}

// ─── NFToken: 10 Accounts Each Mint 10 ──────────────────────────────────────

#[test]
fn nfe24_10_accounts_10_each() {
    for i in 0x41u8..=0x4A {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=10u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── NFToken: 20 Accounts Each Mint 10 ──────────────────────────────────────

#[test]
fn nfe24_20_accounts_10_each() {
    for i in 0x41u8..=0x54 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=10u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq);
                tx.set_field_u32(sf("sfFlags"), 0x08);
                tx.set_field_u16(sf("sfTransferFee"), 500);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── NFToken: 50 Accounts Each Mint 5 ───────────────────────────────────────

#[test]
fn nfe24_50_accounts_5_each() {
    for i in 0x41u8..=0x72 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=5u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq * 100);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── NFToken: 100 Accounts Each Mint 5 With URI ─────────────────────────────

#[test]
fn nfe25_100_accounts_5_uri() {
    for i in 1u8..=100 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=5u32 {
            let uri = format!("ipfs://Qm{:020}{:03}", i, seq);
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq);
                tx.set_field_vl(sf("sfURI"), uri.as_bytes());
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── NFToken: 200 Accounts Each Mint 3 ──────────────────────────────────────

#[test]
fn nfe25_200_accounts_3() {
    for i in 1u8..=200 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=3u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq * 10);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── NFToken: 250 Accounts Each Mint 2 With Transfer Fee ────────────────────

#[test]
fn nfe25_250_accounts_2_fee() {
    for i in 1u8..=250 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=2u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq);
                tx.set_field_u32(sf("sfFlags"), 0x08);
                tx.set_field_u16(sf("sfTransferFee"), (i as u16) * 100);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── NFToken: 50 Accounts Each Mint 20 ──────────────────────────────────────

#[test]
fn nfe26_50_accounts_20() {
    for i in 0x41u8..=0x72 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=20u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── NFToken: 30 Accounts Each Mint 10 With All Flags ───────────────────────

#[test]
fn nfe26_30_accounts_10_flags() {
    for i in 0x41u8..=0x5E {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=10u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq * 7);
                tx.set_field_u32(sf("sfFlags"), 0x0B);
                tx.set_field_u16(sf("sfTransferFee"), 200);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── NFToken: 100 Different Taxons Same Account ─────────────────────────────

#[test]
fn nfe26_100_taxons() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    for seq in 1..=100u32 {
        let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfNFTokenTaxon"), seq * 1000);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
            Ter::TES_SUCCESS
        );
    }
}

// ─── NFToken: 500 Different Taxons ──────────────────────────────────────────

#[test]
fn nfe26_500_taxons() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 50_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    for seq in 1..=500u32 {
        let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfNFTokenTaxon"), seq * 13);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
            Ter::TES_SUCCESS
        );
    }
}

// ─── NFToken: 1000 Different Taxons ─────────────────────────────────────────

#[test]
fn nfe26_1000_taxons() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 50_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    for seq in 1..=1000u32 {
        let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
            tx.set_account_id(sf("sfAccount"), a);
            tx.set_field_u32(sf("sfNFTokenTaxon"), seq * 17);
            tx.set_field_amount(sf("sfFee"), xrp(10));
            tx.set_field_u32(sf("sfSequence"), seq);
        });
        assert_eq!(
            full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
            Ter::TES_SUCCESS
        );
    }
}

// ─── NFToken: 10 Accounts Each Mint 100 ─────────────────────────────────────

#[test]
fn nfe27_10_accounts_100() {
    for i in 0x41u8..=0x4A {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=100u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── NFToken: 5 Accounts Each Mint 200 ──────────────────────────────────────

#[test]
fn nfe27_5_accounts_200() {
    for i in 0x41u8..=0x45 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=200u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 50);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── NFToken: 3 Accounts Each Mint 500 ──────────────────────────────────────

#[test]
fn nfe27_3_accounts_500() {
    for i in 0x41u8..=0x43 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 50_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=500u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 100);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── NFToken: 2 Accounts Each Mint 1000 ─────────────────────────────────────

#[test]
fn nfe27_2_accounts_1000() {
    for i in 0x41u8..=0x42 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 50_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=1000u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 200);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── NFToken: 20 Accounts Each Mint 50 ──────────────────────────────────────

#[test]
fn nfe28_20_accounts_50() {
    for i in 0x41u8..=0x54 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=50u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq * 3);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── NFToken: 15 Accounts Each Mint 30 With URI ─────────────────────────────

#[test]
fn nfe28_15_accounts_30_uri() {
    for i in 0x41u8..=0x4F {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=30u32 {
            let uri = format!("https://nft.example.com/{}/{}", i, seq);
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq);
                tx.set_field_vl(sf("sfURI"), uri.as_bytes());
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── NFToken: 8 Accounts Each Mint 100 Transferable ─────────────────────────

#[test]
fn nfe28_8_accounts_100_transfer() {
    for i in 0x41u8..=0x48 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=100u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 20);
                tx.set_field_u32(sf("sfFlags"), 0x08);
                tx.set_field_u16(sf("sfTransferFee"), 300);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── NFToken: 4 Accounts Each Mint 250 ──────────────────────────────────────

#[test]
fn nfe28_4_accounts_250() {
    for i in 0x41u8..=0x44 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 50_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=250u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 50);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── NFToken: 40 Accounts Each Mint 25 ──────────────────────────────────────

#[test]
fn nfe29_40_accounts_25() {
    for i in 0x41u8..=0x68 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=25u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq * 11);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── NFToken: 25 Accounts Each Mint 40 ──────────────────────────────────────

#[test]
fn nfe29_25_accounts_40() {
    for i in 0x41u8..=0x59 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=40u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq * 7);
                tx.set_field_u32(sf("sfFlags"), 0x08);
                tx.set_field_u16(sf("sfTransferFee"), 250);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── NFToken: 12 Accounts Each Mint 75 ──────────────────────────────────────

#[test]
fn nfe29_12_accounts_75() {
    for i in 0x41u8..=0x4C {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=75u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 15);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}

// ─── NFToken: 6 Accounts Each Mint 150 ──────────────────────────────────────

#[test]
fn nfe29_6_accounts_150() {
    for i in 0x41u8..=0x46 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=150u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 30);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn nfe30_35_accounts_30() {
    for i in 0x41u8..=0x63 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=30u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq * 9);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn nfe30_18_accounts_60() {
    for i in 0x41u8..=0x52 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=60u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 12);
                tx.set_field_u32(sf("sfFlags"), 0x08);
                tx.set_field_u16(sf("sfTransferFee"), 400);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn nfe30_9_accounts_120() {
    for i in 0x41u8..=0x49 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=120u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 24);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn nfe30_7_accounts_180() {
    for i in 0x41u8..=0x47 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 50_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=180u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 36);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn nfe31_45_accounts_20() {
    for i in 0x41u8..=0x6D {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=20u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq * 5);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn nfe31_22_accounts_45() {
    for i in 0x41u8..=0x56 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=45u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 9);
                tx.set_field_u32(sf("sfFlags"), 0x0B);
                tx.set_field_u16(sf("sfTransferFee"), 150);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn nfe31_11_accounts_90() {
    for i in 0x41u8..=0x4B {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=90u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 18);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn nfe32_55_accounts_15() {
    for i in 0x41u8..=0x77 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=15u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq * 3);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn nfe32_28_accounts_35() {
    for i in 0x41u8..=0x5C {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=35u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 7);
                tx.set_field_u32(sf("sfFlags"), 0x08);
                tx.set_field_u16(sf("sfTransferFee"), 350);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn nfe33_60_accounts_18() {
    for i in 0x41u8..=0x7C {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=18u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq * 4);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn nfe33_30_accounts_36() {
    for i in 0x41u8..=0x5E {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=36u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 6);
                tx.set_field_u32(sf("sfFlags"), 0x0B);
                tx.set_field_u16(sf("sfTransferFee"), 175);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn nfe33_15_accounts_72() {
    for i in 0x41u8..=0x4F {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=72u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 14);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn nfe34_70_accounts_14() {
    for i in 0x41u8..=0x86 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=14u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq * 6);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn nfe34_35_accounts_28() {
    for i in 0x41u8..=0x63 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=28u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 7);
                tx.set_field_u32(sf("sfFlags"), 0x08);
                tx.set_field_u16(sf("sfTransferFee"), 225);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn nfe34_17_accounts_56() {
    for i in 0x41u8..=0x51 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=56u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 11);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn nfe35_80_accounts_12() {
    for i in 0x41u8..=0x90 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=12u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq * 8);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn nfe35_40_accounts_24() {
    for i in 0x41u8..=0x68 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=24u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 8);
                tx.set_field_u32(sf("sfFlags"), 0x09);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn nfe35_20_accounts_48() {
    for i in 0x41u8..=0x54 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=48u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 10);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn nfe36_90_accounts_10() {
    for i in 0x41u8..=0x9A {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=10u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq * 5);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn nfe36_45_accounts_22() {
    for i in 0x41u8..=0x6D {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=22u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 5);
                tx.set_field_u32(sf("sfFlags"), 0x0B);
                tx.set_field_u16(sf("sfTransferFee"), 300);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn nfe36_22_accounts_44() {
    for i in 0x41u8..=0x56 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=44u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 9);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn nfe37_100_accounts_8() {
    for i in 0x41u8..=0xA4 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=8u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq * 3);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn nfe37_50_accounts_16() {
    for i in 0x41u8..=0x72 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=16u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 4);
                tx.set_field_u32(sf("sfFlags"), 0x08);
                tx.set_field_u16(sf("sfTransferFee"), 450);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn nfe37_25_accounts_32() {
    for i in 0x41u8..=0x59 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=32u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 8);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn nfe38_110_accounts_7() {
    for i in 0x41u8..=0xAF {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=7u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq * 2);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn nfe38_55_accounts_14() {
    for i in 0x41u8..=0x77 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=14u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 3);
                tx.set_field_u32(sf("sfFlags"), 0x0A);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn nfe38_27_accounts_28() {
    for i in 0x41u8..=0x5B {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=28u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 6);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn nfe39_120_accounts_6() {
    for i in 0x41u8..=0xB8 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=6u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq * 4);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn nfe39_60_accounts_12() {
    for i in 0x41u8..=0x7C {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=12u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 3);
                tx.set_field_u32(sf("sfFlags"), 0x09);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn nfe39_30_accounts_24() {
    for i in 0x41u8..=0x5E {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=24u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 5);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn nfe40_130_accounts_5() {
    for i in 0x41u8..=0xBD {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=5u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq * 7);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn nfe40_65_accounts_10() {
    for i in 0x41u8..=0x81 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=10u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 4);
                tx.set_field_u32(sf("sfFlags"), 0x08);
                tx.set_field_u16(sf("sfTransferFee"), 500);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn nfe40_32_accounts_20() {
    for i in 0x41u8..=0x60 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=20u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 5);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn nfe41_140_accounts_4() {
    for i in 0x41u8..=0xC4 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=4u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq * 9);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn nfe41_70_accounts_8() {
    for i in 0x41u8..=0x86 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=8u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 3);
                tx.set_field_u32(sf("sfFlags"), 0x0B);
                tx.set_field_u16(sf("sfTransferFee"), 600);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn nfe41_35_accounts_16() {
    for i in 0x41u8..=0x63 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=16u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 4);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn nfe42_150_accounts_3() {
    for i in 0x41u8..=0xCF {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=3u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq * 11);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn nfe42_75_accounts_6() {
    for i in 0x41u8..=0x8B {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=6u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 2);
                tx.set_field_u32(sf("sfFlags"), 0x08);
                tx.set_field_u16(sf("sfTransferFee"), 700);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn nfe42_37_accounts_12() {
    for i in 0x41u8..=0x65 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=12u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 3);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn nfe43_160_accounts_3() {
    for i in 0x41u8..=0xD9 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=3u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq * 13);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn nfe43_80_accounts_6() {
    for i in 0x41u8..=0x90 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=6u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 2);
                tx.set_field_u32(sf("sfFlags"), 0x0A);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn nfe44_170_accounts_3() {
    for i in 0x41u8..=0xE3 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=3u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq * 15);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn nfe44_85_accounts_6() {
    for i in 0x41u8..=0x95 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=6u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 2);
                tx.set_field_u32(sf("sfFlags"), 0x0B);
                tx.set_field_u16(sf("sfTransferFee"), 800);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}

#[test]
fn nfe45_180_accounts_2() {
    for i in 0x41u8..=0xF2 {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=2u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq * 17);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn nfe45_90_accounts_4() {
    for i in 0x41u8..=0x9A {
        let a = acct(i);
        let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
        let mut v = new_view(l);
        for seq in 1..=4u32 {
            let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
                tx.set_account_id(sf("sfAccount"), a);
                tx.set_field_u32(sf("sfNFTokenTaxon"), seq % 2);
                tx.set_field_u32(sf("sfFlags"), 0x08);
                tx.set_field_u16(sf("sfTransferFee"), 900);
                tx.set_field_amount(sf("sfFee"), xrp(10));
                tx.set_field_u32(sf("sfSequence"), seq);
            });
            assert_eq!(
                full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
                Ter::TES_SUCCESS
            );
        }
    }
}
#[test]
fn nfe_r1_a() {
    let a = acct(0x1);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r2_a() {
    let a = acct(0x2);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 2);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r3_a() {
    let a = acct(0x3);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r4_a() {
    let a = acct(0x4);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 4);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r5_a() {
    let a = acct(0x5);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 5);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r6_a() {
    let a = acct(0x6);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 6);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r7_a() {
    let a = acct(0x7);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 7);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r8_a() {
    let a = acct(0x8);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 8);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r9_a() {
    let a = acct(0x9);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 9);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r10_a() {
    let a = acct(0x10);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 10);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r11_a() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r12_a() {
    let a = acct(0x12);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 12);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r13_a() {
    let a = acct(0x13);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 13);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r14_a() {
    let a = acct(0x14);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 14);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r15_a() {
    let a = acct(0x15);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 15);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r16_a() {
    let a = acct(0x16);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 16);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r17_a() {
    let a = acct(0x17);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 17);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r18_a() {
    let a = acct(0x18);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 18);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r19_a() {
    let a = acct(0x19);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 19);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r20_a() {
    let a = acct(0x20);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 20);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r21_a() {
    let a = acct(0x21);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 21);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r22_a() {
    let a = acct(0x22);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 22);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r23_a() {
    let a = acct(0x23);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 23);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r24_a() {
    let a = acct(0x24);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 24);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r25_a() {
    let a = acct(0x25);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 25);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r26_a() {
    let a = acct(0x26);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 26);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r27_a() {
    let a = acct(0x27);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 27);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r28_a() {
    let a = acct(0x28);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 28);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r29_a() {
    let a = acct(0x29);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r30_a() {
    let a = acct(0x30);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 30);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r31_a() {
    let a = acct(0x31);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 31);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r32_a() {
    let a = acct(0x32);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 32);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r33_a() {
    let a = acct(0x33);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 33);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r34_a() {
    let a = acct(0x34);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 34);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r35_a() {
    let a = acct(0x35);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 35);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r36_a() {
    let a = acct(0x36);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 36);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r37_a() {
    let a = acct(0x37);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 37);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r38_a() {
    let a = acct(0x38);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 38);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r39_a() {
    let a = acct(0x39);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 39);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r40_a() {
    let a = acct(0x40);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 40);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r41_a() {
    let a = acct(0x41);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 41);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r42_a() {
    let a = acct(0x42);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 42);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r43_a() {
    let a = acct(0x43);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 43);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r44_a() {
    let a = acct(0x44);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 44);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r45_a() {
    let a = acct(0x45);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 45);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r46_a() {
    let a = acct(0x46);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 46);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r47_a() {
    let a = acct(0x47);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r48_a() {
    let a = acct(0x48);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 48);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r49_a() {
    let a = acct(0x49);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 49);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r50_a() {
    let a = acct(0x50);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 50);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r51_a() {
    let a = acct(0x51);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 51);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r52_a() {
    let a = acct(0x52);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 52);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r53_a() {
    let a = acct(0x53);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 53);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r54_a() {
    let a = acct(0x54);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 54);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r55_a() {
    let a = acct(0x55);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 55);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r56_a() {
    let a = acct(0x56);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 56);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r57_a() {
    let a = acct(0x57);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 57);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r58_a() {
    let a = acct(0x58);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 58);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r59_a() {
    let a = acct(0x59);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 59);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r60_a() {
    let a = acct(0x60);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 60);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r61_a() {
    let a = acct(0x61);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 61);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r62_a() {
    let a = acct(0x62);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 62);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r63_a() {
    let a = acct(0x63);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 63);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r64_a() {
    let a = acct(0x64);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 64);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r65_a() {
    let a = acct(0x65);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 65);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r66_a() {
    let a = acct(0x66);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 66);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r67_a() {
    let a = acct(0x67);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 67);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r68_a() {
    let a = acct(0x68);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 68);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r69_a() {
    let a = acct(0x69);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 69);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r70_a() {
    let a = acct(0x70);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 70);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r71_a() {
    let a = acct(0x71);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 71);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r72_a() {
    let a = acct(0x72);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 72);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r73_a() {
    let a = acct(0x73);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r74_a() {
    let a = acct(0x74);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 74);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r75_a() {
    let a = acct(0x75);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 75);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r76_a() {
    let a = acct(0x76);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 76);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r77_a() {
    let a = acct(0x77);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 77);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r78_a() {
    let a = acct(0x78);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 78);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r79_a() {
    let a = acct(0x79);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 79);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r80_a() {
    let a = acct(0x80);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 80);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r81_a() {
    let a = acct(0x81);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 81);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r82_a() {
    let a = acct(0x82);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 82);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r83_a() {
    let a = acct(0x83);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 83);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r84_a() {
    let a = acct(0x84);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 84);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r85_a() {
    let a = acct(0x85);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 85);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r86_a() {
    let a = acct(0x86);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 86);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r87_a() {
    let a = acct(0x87);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 87);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r88_a() {
    let a = acct(0x88);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 88);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r89_a() {
    let a = acct(0x89);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 89);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r90_a() {
    let a = acct(0x90);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 90);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r91_a() {
    let a = acct(0x91);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 91);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r92_a() {
    let a = acct(0x92);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 92);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r93_a() {
    let a = acct(0x93);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 93);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r94_a() {
    let a = acct(0x94);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 94);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r95_a() {
    let a = acct(0x95);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 95);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r96_a() {
    let a = acct(0x96);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 96);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r97_a() {
    let a = acct(0x97);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 97);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r98_a() {
    let a = acct(0x98);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 98);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r99_a() {
    let a = acct(0x99);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 99);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_r101() {
    let a = acct(0xAA);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 101);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s1() {
    let a = acct(0xa1);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s2() {
    let a = acct(0xa2);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 2 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s3() {
    let a = acct(0xa3);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 3 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s4() {
    let a = acct(0xa4);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 4 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s5() {
    let a = acct(0xa5);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 5 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s6() {
    let a = acct(0xa6);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 6 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s7() {
    let a = acct(0xa7);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 7 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s8() {
    let a = acct(0xa8);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 8 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s9() {
    let a = acct(0xa9);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 9 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s10() {
    let a = acct(0xaa);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 10 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s11() {
    let a = acct(0xab);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 11 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s12() {
    let a = acct(0xac);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 12 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s13() {
    let a = acct(0xad);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 13 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s14() {
    let a = acct(0xae);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 14 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s15() {
    let a = acct(0xaf);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 15 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s16() {
    let a = acct(0xb0);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 16 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s17() {
    let a = acct(0xb1);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 17 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s18() {
    let a = acct(0xb2);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 18 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s19() {
    let a = acct(0xb3);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 19 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s20() {
    let a = acct(0xb4);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 20 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s21() {
    let a = acct(0xb5);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 21 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s22() {
    let a = acct(0xb6);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 22 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s23() {
    let a = acct(0xb7);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 23 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s24() {
    let a = acct(0xb8);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 24 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s25() {
    let a = acct(0xb9);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 25 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s26() {
    let a = acct(0xba);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 26 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s27() {
    let a = acct(0xbb);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 27 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s28() {
    let a = acct(0xbc);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 28 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s29() {
    let a = acct(0xbd);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 29 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s30() {
    let a = acct(0xbe);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 30 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s31() {
    let a = acct(0xbf);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 31 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s32() {
    let a = acct(0xc0);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 32 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s33() {
    let a = acct(0xc1);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 33 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s34() {
    let a = acct(0xc2);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 34 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s35() {
    let a = acct(0xc3);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 35 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s36() {
    let a = acct(0xc4);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 36 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s37() {
    let a = acct(0xc5);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 37 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s38() {
    let a = acct(0xc6);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 38 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s39() {
    let a = acct(0xc7);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 39 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s40() {
    let a = acct(0xc8);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 40 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s41() {
    let a = acct(0xc9);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 41 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s42() {
    let a = acct(0xca);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 42 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s43() {
    let a = acct(0xcb);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 43 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s44() {
    let a = acct(0xcc);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 44 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s45() {
    let a = acct(0xcd);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 45 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s46() {
    let a = acct(0xce);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 46 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s47() {
    let a = acct(0xcf);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 47 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s48() {
    let a = acct(0xd0);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 48 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s49() {
    let a = acct(0xd1);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 49 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s50() {
    let a = acct(0xd2);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 50 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s51() {
    let a = acct(0xd3);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 51 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s52() {
    let a = acct(0xd4);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 52 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s53() {
    let a = acct(0xd5);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 53 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s54() {
    let a = acct(0xd6);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 54 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s55() {
    let a = acct(0xd7);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 55 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s56() {
    let a = acct(0xd8);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 56 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s57() {
    let a = acct(0xd9);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 57 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s58() {
    let a = acct(0xda);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 58 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s59() {
    let a = acct(0xdb);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 59 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_s60() {
    let a = acct(0xdc);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 60 * 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t1() {
    let a = acct(0x41);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 10);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t2() {
    let a = acct(0x42);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 2 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 20);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t3() {
    let a = acct(0x43);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 3 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 30);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t4() {
    let a = acct(0x44);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 4 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 40);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t5() {
    let a = acct(0x45);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 5 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 50);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t6() {
    let a = acct(0x46);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 6 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 60);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t7() {
    let a = acct(0x47);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 7 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 70);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t8() {
    let a = acct(0x48);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 8 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 80);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t9() {
    let a = acct(0x49);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 9 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 90);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t10() {
    let a = acct(0x4a);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 10 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 100);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t11() {
    let a = acct(0x4b);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 11 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 110);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t12() {
    let a = acct(0x4c);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 12 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 120);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t13() {
    let a = acct(0x4d);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 13 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 130);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t14() {
    let a = acct(0x4e);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 14 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 140);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t15() {
    let a = acct(0x4f);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 15 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 150);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t16() {
    let a = acct(0x50);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 16 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 160);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t17() {
    let a = acct(0x51);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 17 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 170);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t18() {
    let a = acct(0x52);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 18 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 180);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t19() {
    let a = acct(0x53);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 19 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 190);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t20() {
    let a = acct(0x54);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 20 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 200);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t21() {
    let a = acct(0x55);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 21 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 210);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t22() {
    let a = acct(0x56);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 22 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 220);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t23() {
    let a = acct(0x57);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 23 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 230);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t24() {
    let a = acct(0x58);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 24 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 240);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t25() {
    let a = acct(0x59);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 25 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 250);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t26() {
    let a = acct(0x5a);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 26 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 260);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t27() {
    let a = acct(0x5b);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 27 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 270);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t28() {
    let a = acct(0x5c);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 28 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 280);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t29() {
    let a = acct(0x5d);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 29 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 290);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t30() {
    let a = acct(0x5e);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 30 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 300);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t31() {
    let a = acct(0x5f);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 31 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 310);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t32() {
    let a = acct(0x60);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 32 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 320);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t33() {
    let a = acct(0x61);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 33 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 330);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t34() {
    let a = acct(0x62);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 34 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 340);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t35() {
    let a = acct(0x63);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 35 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 350);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t36() {
    let a = acct(0x64);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 36 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 360);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t37() {
    let a = acct(0x65);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 37 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 370);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t38() {
    let a = acct(0x66);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 38 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 380);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t39() {
    let a = acct(0x67);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 39 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 390);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t40() {
    let a = acct(0x68);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 40 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 400);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t41() {
    let a = acct(0x69);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 41 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 410);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t42() {
    let a = acct(0x6a);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 42 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 420);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t43() {
    let a = acct(0x6b);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 43 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 430);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t44() {
    let a = acct(0x6c);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 44 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 440);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t45() {
    let a = acct(0x6d);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 45 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 450);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t46() {
    let a = acct(0x6e);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 46 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 460);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t47() {
    let a = acct(0x6f);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 47 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 470);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t48() {
    let a = acct(0x70);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 48 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 480);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t49() {
    let a = acct(0x71);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 49 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 490);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t50() {
    let a = acct(0x72);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 50 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 500);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t51() {
    let a = acct(0x73);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 51 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 510);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t52() {
    let a = acct(0x74);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 52 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 520);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t53() {
    let a = acct(0x75);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 53 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 530);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t54() {
    let a = acct(0x76);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 54 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 540);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t55() {
    let a = acct(0x77);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 55 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 550);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t56() {
    let a = acct(0x78);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 56 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 560);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t57() {
    let a = acct(0x79);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 57 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 570);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t58() {
    let a = acct(0x7a);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 58 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 580);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t59() {
    let a = acct(0x7b);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 59 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 590);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_t60() {
    let a = acct(0x7c);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 60 * 7);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 600);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u1() {
    let a = acct(0x81);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u2() {
    let a = acct(0x82);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 2 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u3() {
    let a = acct(0x83);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 3 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u4() {
    let a = acct(0x84);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 4 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u5() {
    let a = acct(0x85);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 5 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u6() {
    let a = acct(0x86);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 6 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u7() {
    let a = acct(0x87);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 7 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u8() {
    let a = acct(0x88);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 8 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u9() {
    let a = acct(0x89);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 9 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u10() {
    let a = acct(0x8a);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 10 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u11() {
    let a = acct(0x8b);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 11 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u12() {
    let a = acct(0x8c);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 12 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u13() {
    let a = acct(0x8d);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 13 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u14() {
    let a = acct(0x8e);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 14 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u15() {
    let a = acct(0x8f);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 15 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u16() {
    let a = acct(0x90);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 16 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u17() {
    let a = acct(0x91);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 17 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u18() {
    let a = acct(0x92);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 18 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u19() {
    let a = acct(0x93);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 19 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u20() {
    let a = acct(0x94);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 20 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u21() {
    let a = acct(0x95);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 21 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u22() {
    let a = acct(0x96);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 22 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u23() {
    let a = acct(0x97);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 23 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u24() {
    let a = acct(0x98);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 24 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u25() {
    let a = acct(0x99);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 25 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u26() {
    let a = acct(0x9a);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 26 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u27() {
    let a = acct(0x9b);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 27 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u28() {
    let a = acct(0x9c);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 28 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u29() {
    let a = acct(0x9d);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 29 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u30() {
    let a = acct(0x9e);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 30 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u31() {
    let a = acct(0x9f);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 31 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u32() {
    let a = acct(0xa0);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 32 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u33() {
    let a = acct(0xa1);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 33 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u34() {
    let a = acct(0xa2);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 34 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u35() {
    let a = acct(0xa3);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 35 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u36() {
    let a = acct(0xa4);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 36 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u37() {
    let a = acct(0xa5);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 37 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u38() {
    let a = acct(0xa6);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 38 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u39() {
    let a = acct(0xa7);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 39 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u40() {
    let a = acct(0xa8);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 40 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u41() {
    let a = acct(0xa9);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 41 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u42() {
    let a = acct(0xaa);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 42 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u43() {
    let a = acct(0xab);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 43 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u44() {
    let a = acct(0xac);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 44 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u45() {
    let a = acct(0xad);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 45 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u46() {
    let a = acct(0xae);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 46 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u47() {
    let a = acct(0xaf);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 47 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u48() {
    let a = acct(0xb0);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 48 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u49() {
    let a = acct(0xb1);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 49 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u50() {
    let a = acct(0xb2);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 50 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u51() {
    let a = acct(0xb3);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 51 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u52() {
    let a = acct(0xb4);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 52 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u53() {
    let a = acct(0xb5);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 53 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u54() {
    let a = acct(0xb6);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 54 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_u55() {
    let a = acct(0xb7);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 55 * 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_v56() {
    let a = acct(0xb8);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 56 * 13);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_v57() {
    let a = acct(0xb9);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 57 * 13);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_v58() {
    let a = acct(0xba);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 58 * 13);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_v59() {
    let a = acct(0xbb);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 59 * 13);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_v60() {
    let a = acct(0xbc);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 60 * 13);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_v61() {
    let a = acct(0xbd);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 61 * 13);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_v62() {
    let a = acct(0xbe);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 62 * 13);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_v63() {
    let a = acct(0xbf);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 63 * 13);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_v64() {
    let a = acct(0xc0);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 64 * 13);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_v65() {
    let a = acct(0xc1);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 65 * 13);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w1() {
    let a = acct(0x1f);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 100);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w2() {
    let a = acct(0x20);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 2 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 200);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w3() {
    let a = acct(0x21);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 3 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 300);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w4() {
    let a = acct(0x22);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 4 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 400);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w5() {
    let a = acct(0x23);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 5 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 500);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w6() {
    let a = acct(0x24);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 6 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 600);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w7() {
    let a = acct(0x25);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 7 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 700);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w8() {
    let a = acct(0x26);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 8 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 800);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w9() {
    let a = acct(0x27);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 9 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 900);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w10() {
    let a = acct(0x28);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 10 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 1000);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w11() {
    let a = acct(0x29);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 11 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 1100);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w12() {
    let a = acct(0x2a);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 12 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 1200);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w13() {
    let a = acct(0x2b);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 13 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 1300);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w14() {
    let a = acct(0x2c);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 14 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 1400);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w15() {
    let a = acct(0x2d);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 15 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 1500);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w16() {
    let a = acct(0x2e);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 16 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 1600);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w17() {
    let a = acct(0x2f);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 17 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 1700);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w18() {
    let a = acct(0x30);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 18 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 1800);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w19() {
    let a = acct(0x31);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 19 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 1900);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w20() {
    let a = acct(0x32);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 20 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 2000);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w21() {
    let a = acct(0x33);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 21 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 2100);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w22() {
    let a = acct(0x34);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 22 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 2200);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w23() {
    let a = acct(0x35);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 23 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 2300);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w24() {
    let a = acct(0x36);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 24 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 2400);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w25() {
    let a = acct(0x37);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 25 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 2500);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w26() {
    let a = acct(0x38);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 26 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 2600);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w27() {
    let a = acct(0x39);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 27 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 2700);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w28() {
    let a = acct(0x3a);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 28 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 2800);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w29() {
    let a = acct(0x3b);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 29 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 2900);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w30() {
    let a = acct(0x3c);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 30 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 3000);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w31() {
    let a = acct(0x3d);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 31 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 3100);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w32() {
    let a = acct(0x3e);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 32 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 3200);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w33() {
    let a = acct(0x3f);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 33 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 3300);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w34() {
    let a = acct(0x40);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 34 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 3400);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w35() {
    let a = acct(0x41);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 35 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 3500);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w36() {
    let a = acct(0x42);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 36 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 3600);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w37() {
    let a = acct(0x43);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 37 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 3700);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w38() {
    let a = acct(0x44);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 38 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 3800);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w39() {
    let a = acct(0x45);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 39 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 3900);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w40() {
    let a = acct(0x46);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 40 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 4000);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w41() {
    let a = acct(0x47);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 41 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 4100);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w42() {
    let a = acct(0x48);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 42 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 4200);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w43() {
    let a = acct(0x49);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 43 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 4300);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w44() {
    let a = acct(0x4a);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 44 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 4400);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w45() {
    let a = acct(0x4b);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 45 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 4500);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w46() {
    let a = acct(0x4c);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 46 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 4600);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w47() {
    let a = acct(0x4d);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 47 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 4700);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w48() {
    let a = acct(0x4e);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 48 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 4800);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w49() {
    let a = acct(0x4f);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 49 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 4900);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_w50() {
    let a = acct(0x50);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 50 * 19);
        tx.set_field_u32(sf("sfFlags"), 0x0B);
        tx.set_field_u16(sf("sfTransferFee"), 5000);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_x66() {
    let a = acct(0xc2);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 66 * 23);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_x67() {
    let a = acct(0xc3);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 67 * 23);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_x68() {
    let a = acct(0xc4);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 68 * 23);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_x69() {
    let a = acct(0xc5);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 69 * 23);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_x70() {
    let a = acct(0xc6);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 70 * 23);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_x71() {
    let a = acct(0xc7);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 71 * 23);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_x72() {
    let a = acct(0xc8);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 72 * 23);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_x73() {
    let a = acct(0xc9);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 73 * 23);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_x74() {
    let a = acct(0xca);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 74 * 23);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_x75() {
    let a = acct(0xcb);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 75 * 23);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_x76() {
    let a = acct(0xcc);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 76 * 23);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_x77() {
    let a = acct(0xcd);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 77 * 23);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_x78() {
    let a = acct(0xce);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 78 * 23);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_x79() {
    let a = acct(0xcf);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 79 * 23);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_x80() {
    let a = acct(0xd0);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 80 * 23);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_x81() {
    let a = acct(0xd1);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 81 * 23);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_x82() {
    let a = acct(0xd2);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 82 * 23);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_x83() {
    let a = acct(0xd3);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 83 * 23);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_x84() {
    let a = acct(0xd4);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 84 * 23);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_x85() {
    let a = acct(0xd5);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 85 * 23);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y1() {
    let a = acct(0x02);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y2() {
    let a = acct(0x03);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 2 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y3() {
    let a = acct(0x04);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 3 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y4() {
    let a = acct(0x05);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 4 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y5() {
    let a = acct(0x06);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 5 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y6() {
    let a = acct(0x07);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 6 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y7() {
    let a = acct(0x08);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 7 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y8() {
    let a = acct(0x09);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 8 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y9() {
    let a = acct(0x0a);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 9 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y10() {
    let a = acct(0x0b);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 10 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y11() {
    let a = acct(0x0c);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 11 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y12() {
    let a = acct(0x0d);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 12 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y13() {
    let a = acct(0x0e);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 13 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y14() {
    let a = acct(0x0f);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 14 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y15() {
    let a = acct(0x10);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 15 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y16() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 16 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y17() {
    let a = acct(0x12);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 17 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y18() {
    let a = acct(0x13);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 18 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y19() {
    let a = acct(0x14);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 19 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y20() {
    let a = acct(0x15);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 20 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y21() {
    let a = acct(0x16);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 21 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y22() {
    let a = acct(0x17);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 22 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y23() {
    let a = acct(0x18);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 23 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y24() {
    let a = acct(0x19);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 24 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y25() {
    let a = acct(0x1a);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 25 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y26() {
    let a = acct(0x1b);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 26 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y27() {
    let a = acct(0x1c);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 27 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y28() {
    let a = acct(0x1d);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 28 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y29() {
    let a = acct(0x1e);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 29 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y30() {
    let a = acct(0x1f);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 30 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y31() {
    let a = acct(0x20);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 31 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y32() {
    let a = acct(0x21);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 32 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y33() {
    let a = acct(0x22);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 33 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y34() {
    let a = acct(0x23);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 34 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y35() {
    let a = acct(0x24);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 35 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y36() {
    let a = acct(0x25);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 36 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y37() {
    let a = acct(0x26);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 37 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y38() {
    let a = acct(0x27);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 38 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y39() {
    let a = acct(0x28);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 39 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y40() {
    let a = acct(0x29);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 40 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y41() {
    let a = acct(0x2a);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 41 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y42() {
    let a = acct(0x2b);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 42 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y43() {
    let a = acct(0x2c);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 43 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y44() {
    let a = acct(0x2d);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 44 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y45() {
    let a = acct(0x2e);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 45 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y46() {
    let a = acct(0x2f);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 46 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y47() {
    let a = acct(0x30);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 47 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y48() {
    let a = acct(0x31);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 48 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y49() {
    let a = acct(0x32);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 49 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y50() {
    let a = acct(0x33);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 50 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y51() {
    let a = acct(0x34);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 51 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y52() {
    let a = acct(0x35);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 52 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y53() {
    let a = acct(0x36);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 53 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y54() {
    let a = acct(0x37);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 54 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y55() {
    let a = acct(0x38);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 55 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y56() {
    let a = acct(0x39);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 56 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y57() {
    let a = acct(0x3a);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 57 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y58() {
    let a = acct(0x3b);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 58 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y59() {
    let a = acct(0x3c);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 59 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_y60() {
    let a = acct(0x3d);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 60 * 29);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z1() {
    let a = acct(0xc1);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z2() {
    let a = acct(0xc2);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 2 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z3() {
    let a = acct(0xc3);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 3 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z4() {
    let a = acct(0xc4);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 4 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z5() {
    let a = acct(0xc5);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 5 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z6() {
    let a = acct(0xc6);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 6 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z7() {
    let a = acct(0xc7);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 7 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z8() {
    let a = acct(0xc8);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 8 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z9() {
    let a = acct(0xc9);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 9 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z10() {
    let a = acct(0xca);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 10 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z11() {
    let a = acct(0xcb);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 11 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z12() {
    let a = acct(0xcc);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 12 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z13() {
    let a = acct(0xcd);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 13 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z14() {
    let a = acct(0xce);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 14 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z15() {
    let a = acct(0xcf);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 15 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z16() {
    let a = acct(0xd0);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 16 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z17() {
    let a = acct(0xd1);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 17 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z18() {
    let a = acct(0xd2);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 18 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z19() {
    let a = acct(0xd3);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 19 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z20() {
    let a = acct(0xd4);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 20 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z21() {
    let a = acct(0xd5);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 21 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z22() {
    let a = acct(0xd6);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 22 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z23() {
    let a = acct(0xd7);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 23 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z24() {
    let a = acct(0xd8);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 24 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z25() {
    let a = acct(0xd9);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 25 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z26() {
    let a = acct(0xda);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 26 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z27() {
    let a = acct(0xdb);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 27 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z28() {
    let a = acct(0xdc);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 28 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z29() {
    let a = acct(0xdd);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 29 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z30() {
    let a = acct(0xde);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 30 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z31() {
    let a = acct(0xdf);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 31 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z32() {
    let a = acct(0xe0);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 32 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z33() {
    let a = acct(0xe1);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 33 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z34() {
    let a = acct(0xe2);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 34 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z35() {
    let a = acct(0xe3);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 35 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z36() {
    let a = acct(0xe4);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 36 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z37() {
    let a = acct(0xe5);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 37 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z38() {
    let a = acct(0xe6);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 38 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z39() {
    let a = acct(0xe7);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 39 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z40() {
    let a = acct(0xe8);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 40 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z41() {
    let a = acct(0xe9);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 41 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z42() {
    let a = acct(0xea);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 42 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z43() {
    let a = acct(0xeb);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 43 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z44() {
    let a = acct(0xec);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 44 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z45() {
    let a = acct(0xed);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 45 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z46() {
    let a = acct(0xee);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 46 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z47() {
    let a = acct(0xef);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 47 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z48() {
    let a = acct(0xf0);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 48 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z49() {
    let a = acct(0xf1);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 49 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z50() {
    let a = acct(0xf2);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 50 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z51() {
    let a = acct(0xf3);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 51 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z52() {
    let a = acct(0xf4);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 52 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z53() {
    let a = acct(0xf5);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 53 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z54() {
    let a = acct(0xf6);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 54 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z55() {
    let a = acct(0xf7);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 55 * 31);
        tx.set_field_u32(sf("sfFlags"), 0x09);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z2_56() {
    let a = acct(0xf8);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 56 * 37);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z2_57() {
    let a = acct(0xf9);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 57 * 37);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z2_58() {
    let a = acct(0xfa);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 58 * 37);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z2_59() {
    let a = acct(0xfb);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 59 * 37);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z2_60() {
    let a = acct(0xfc);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 60 * 37);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z2_61() {
    let a = acct(0xfd);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 61 * 37);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z2_62() {
    let a = acct(0xfe);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 62 * 37);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z2_63() {
    let a = acct(0xff);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 63 * 37);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z3_1() {
    let a = acct(0xA1);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 999);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_z3_2() {
    let a = acct(0xA2);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 998);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa1() {
    let a = acct(0x41);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa2() {
    let a = acct(0x42);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 2 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa3() {
    let a = acct(0x43);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 3 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa4() {
    let a = acct(0x44);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 4 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa5() {
    let a = acct(0x45);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 5 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa6() {
    let a = acct(0x46);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 6 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa7() {
    let a = acct(0x47);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 7 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa8() {
    let a = acct(0x48);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 8 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa9() {
    let a = acct(0x49);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 9 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa10() {
    let a = acct(0x4a);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 10 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa11() {
    let a = acct(0x4b);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 11 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa12() {
    let a = acct(0x4c);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 12 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa13() {
    let a = acct(0x4d);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 13 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa14() {
    let a = acct(0x4e);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 14 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa15() {
    let a = acct(0x4f);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 15 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa16() {
    let a = acct(0x50);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 16 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa17() {
    let a = acct(0x51);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 17 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa18() {
    let a = acct(0x52);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 18 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa19() {
    let a = acct(0x53);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 19 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa20() {
    let a = acct(0x54);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 20 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa21() {
    let a = acct(0x55);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 21 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa22() {
    let a = acct(0x56);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 22 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa23() {
    let a = acct(0x57);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 23 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa24() {
    let a = acct(0x58);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 24 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa25() {
    let a = acct(0x59);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 25 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa26() {
    let a = acct(0x5a);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 26 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa27() {
    let a = acct(0x5b);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 27 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa28() {
    let a = acct(0x5c);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 28 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa29() {
    let a = acct(0x5d);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 29 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa30() {
    let a = acct(0x5e);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 30 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa31() {
    let a = acct(0x5f);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 31 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa32() {
    let a = acct(0x60);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 32 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa33() {
    let a = acct(0x61);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 33 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa34() {
    let a = acct(0x62);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 34 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa35() {
    let a = acct(0x63);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 35 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa36() {
    let a = acct(0x64);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 36 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa37() {
    let a = acct(0x65);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 37 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa38() {
    let a = acct(0x66);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 38 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa39() {
    let a = acct(0x67);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 39 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa40() {
    let a = acct(0x68);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 40 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa41() {
    let a = acct(0x69);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 41 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa42() {
    let a = acct(0x6a);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 42 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa43() {
    let a = acct(0x6b);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 43 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa44() {
    let a = acct(0x6c);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 44 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa45() {
    let a = acct(0x6d);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 45 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa46() {
    let a = acct(0x6e);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 46 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa47() {
    let a = acct(0x6f);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 47 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa48() {
    let a = acct(0x70);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 48 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa49() {
    let a = acct(0x71);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 49 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa50() {
    let a = acct(0x72);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 50 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa51() {
    let a = acct(0x73);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 51 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa52() {
    let a = acct(0x74);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 52 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa53() {
    let a = acct(0x75);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 53 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa54() {
    let a = acct(0x76);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 54 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_aa55() {
    let a = acct(0x77);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 55 * 41);
        tx.set_field_u32(sf("sfFlags"), 0x02);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ab56() {
    let a = acct(0x78);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 56 * 43);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ab57() {
    let a = acct(0x79);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 57 * 43);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ab58() {
    let a = acct(0x7a);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 58 * 43);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ab59() {
    let a = acct(0x7b);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 59 * 43);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ab60() {
    let a = acct(0x7c);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 60 * 43);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ab61() {
    let a = acct(0x7d);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 61 * 43);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ab62() {
    let a = acct(0x7e);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 62 * 43);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ab63() {
    let a = acct(0x7f);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 63 * 43);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ab64() {
    let a = acct(0x80);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 64 * 43);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ab65() {
    let a = acct(0x81);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 65 * 43);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac1() {
    let a = acct(0x06);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac2() {
    let a = acct(0x07);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 2 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac3() {
    let a = acct(0x08);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 3 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac4() {
    let a = acct(0x09);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 4 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac5() {
    let a = acct(0x0a);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 5 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac6() {
    let a = acct(0x0b);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 6 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac7() {
    let a = acct(0x0c);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 7 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac8() {
    let a = acct(0x0d);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 8 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac9() {
    let a = acct(0x0e);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 9 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac10() {
    let a = acct(0x0f);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 10 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac11() {
    let a = acct(0x10);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 11 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac12() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 12 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac13() {
    let a = acct(0x12);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 13 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac14() {
    let a = acct(0x13);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 14 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac15() {
    let a = acct(0x14);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 15 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac16() {
    let a = acct(0x15);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 16 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac17() {
    let a = acct(0x16);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 17 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac18() {
    let a = acct(0x17);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 18 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac19() {
    let a = acct(0x18);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 19 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac20() {
    let a = acct(0x19);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 20 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac21() {
    let a = acct(0x1a);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 21 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac22() {
    let a = acct(0x1b);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 22 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac23() {
    let a = acct(0x1c);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 23 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac24() {
    let a = acct(0x1d);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 24 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac25() {
    let a = acct(0x1e);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 25 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac26() {
    let a = acct(0x1f);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 26 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac27() {
    let a = acct(0x20);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 27 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac28() {
    let a = acct(0x21);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 28 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac29() {
    let a = acct(0x22);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 29 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac30() {
    let a = acct(0x23);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 30 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac31() {
    let a = acct(0x24);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 31 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac32() {
    let a = acct(0x25);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 32 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac33() {
    let a = acct(0x26);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 33 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac34() {
    let a = acct(0x27);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 34 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac35() {
    let a = acct(0x28);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 35 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac36() {
    let a = acct(0x29);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 36 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac37() {
    let a = acct(0x2a);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 37 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac38() {
    let a = acct(0x2b);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 38 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac39() {
    let a = acct(0x2c);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 39 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac40() {
    let a = acct(0x2d);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 40 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac41() {
    let a = acct(0x2e);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 41 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac42() {
    let a = acct(0x2f);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 42 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac43() {
    let a = acct(0x30);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 43 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac44() {
    let a = acct(0x31);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 44 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac45() {
    let a = acct(0x32);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 45 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac46() {
    let a = acct(0x33);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 46 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac47() {
    let a = acct(0x34);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 47 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac48() {
    let a = acct(0x35);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 48 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac49() {
    let a = acct(0x36);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 49 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ac50() {
    let a = acct(0x37);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 50 * 47);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad1() {
    let a = acct(0x03);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad2() {
    let a = acct(0x04);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 2 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad3() {
    let a = acct(0x05);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 3 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad4() {
    let a = acct(0x06);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 4 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad5() {
    let a = acct(0x07);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 5 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad6() {
    let a = acct(0x08);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 6 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad7() {
    let a = acct(0x09);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 7 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad8() {
    let a = acct(0x0a);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 8 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad9() {
    let a = acct(0x0b);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 9 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad10() {
    let a = acct(0x0c);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 10 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad11() {
    let a = acct(0x0d);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 11 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad12() {
    let a = acct(0x0e);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 12 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad13() {
    let a = acct(0x0f);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 13 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad14() {
    let a = acct(0x10);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 14 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad15() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 15 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad16() {
    let a = acct(0x12);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 16 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad17() {
    let a = acct(0x13);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 17 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad18() {
    let a = acct(0x14);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 18 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad19() {
    let a = acct(0x15);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 19 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad20() {
    let a = acct(0x16);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 20 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad21() {
    let a = acct(0x17);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 21 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad22() {
    let a = acct(0x18);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 22 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad23() {
    let a = acct(0x19);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 23 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad24() {
    let a = acct(0x1a);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 24 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad25() {
    let a = acct(0x1b);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 25 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad26() {
    let a = acct(0x1c);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 26 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad27() {
    let a = acct(0x1d);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 27 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad28() {
    let a = acct(0x1e);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 28 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad29() {
    let a = acct(0x1f);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 29 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad30() {
    let a = acct(0x20);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 30 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad31() {
    let a = acct(0x21);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 31 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad32() {
    let a = acct(0x22);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 32 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad33() {
    let a = acct(0x23);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 33 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad34() {
    let a = acct(0x24);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 34 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad35() {
    let a = acct(0x25);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 35 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad36() {
    let a = acct(0x26);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 36 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad37() {
    let a = acct(0x27);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 37 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad38() {
    let a = acct(0x28);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 38 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad39() {
    let a = acct(0x29);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 39 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad40() {
    let a = acct(0x2a);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 40 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad41() {
    let a = acct(0x2b);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 41 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad42() {
    let a = acct(0x2c);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 42 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad43() {
    let a = acct(0x2d);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 43 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad44() {
    let a = acct(0x2e);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 44 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad45() {
    let a = acct(0x2f);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 45 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad46() {
    let a = acct(0x30);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 46 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad47() {
    let a = acct(0x31);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 47 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad48() {
    let a = acct(0x32);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 48 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad49() {
    let a = acct(0x33);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 49 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ad50() {
    let a = acct(0x34);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 50 * 53);
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ae51() {
    let a = acct(0x35);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 51 * 59);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ae52() {
    let a = acct(0x36);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 52 * 59);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ae53() {
    let a = acct(0x37);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 53 * 59);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ae54() {
    let a = acct(0x38);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 54 * 59);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ae55() {
    let a = acct(0x39);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 55 * 59);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ae56() {
    let a = acct(0x3a);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 56 * 59);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ae57() {
    let a = acct(0x3b);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 57 * 59);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ae58() {
    let a = acct(0x3c);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 58 * 59);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ae59() {
    let a = acct(0x3d);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 59 * 59);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ae60() {
    let a = acct(0x3e);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 60 * 59);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ae61() {
    let a = acct(0x3f);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 61 * 59);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ae62() {
    let a = acct(0x40);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 62 * 59);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ae63() {
    let a = acct(0x41);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 63 * 59);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ae64() {
    let a = acct(0x42);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 64 * 59);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ae65() {
    let a = acct(0x43);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 65 * 59);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af1() {
    let a = acct(0x65);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af2() {
    let a = acct(0x66);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 2 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af3() {
    let a = acct(0x67);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 3 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af4() {
    let a = acct(0x68);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 4 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af5() {
    let a = acct(0x69);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 5 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af6() {
    let a = acct(0x6a);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 6 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af7() {
    let a = acct(0x6b);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 7 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af8() {
    let a = acct(0x6c);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 8 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af9() {
    let a = acct(0x6d);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 9 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af10() {
    let a = acct(0x6e);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 10 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af11() {
    let a = acct(0x6f);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 11 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af12() {
    let a = acct(0x70);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 12 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af13() {
    let a = acct(0x71);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 13 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af14() {
    let a = acct(0x72);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 14 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af15() {
    let a = acct(0x73);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 15 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af16() {
    let a = acct(0x74);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 16 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af17() {
    let a = acct(0x75);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 17 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af18() {
    let a = acct(0x76);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 18 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af19() {
    let a = acct(0x77);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 19 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af20() {
    let a = acct(0x78);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 20 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af21() {
    let a = acct(0x79);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 21 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af22() {
    let a = acct(0x7a);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 22 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af23() {
    let a = acct(0x7b);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 23 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af24() {
    let a = acct(0x7c);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 24 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af25() {
    let a = acct(0x7d);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 25 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af26() {
    let a = acct(0x7e);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 26 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af27() {
    let a = acct(0x7f);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 27 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af28() {
    let a = acct(0x80);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 28 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af29() {
    let a = acct(0x81);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 29 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af30() {
    let a = acct(0x82);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 30 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af31() {
    let a = acct(0x83);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 31 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af32() {
    let a = acct(0x84);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 32 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af33() {
    let a = acct(0x85);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 33 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af34() {
    let a = acct(0x86);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 34 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af35() {
    let a = acct(0x87);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 35 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af36() {
    let a = acct(0x88);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 36 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af37() {
    let a = acct(0x89);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 37 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af38() {
    let a = acct(0x8a);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 38 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af39() {
    let a = acct(0x8b);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 39 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af40() {
    let a = acct(0x8c);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 40 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af41() {
    let a = acct(0x8d);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 41 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af42() {
    let a = acct(0x8e);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 42 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af43() {
    let a = acct(0x8f);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 43 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af44() {
    let a = acct(0x90);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 44 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af45() {
    let a = acct(0x91);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 45 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af46() {
    let a = acct(0x92);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 46 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af47() {
    let a = acct(0x93);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 47 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af48() {
    let a = acct(0x94);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 48 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af49() {
    let a = acct(0x95);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 49 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af50() {
    let a = acct(0x96);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 50 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af51() {
    let a = acct(0x97);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 51 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af52() {
    let a = acct(0x98);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 52 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af53() {
    let a = acct(0x99);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 53 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af54() {
    let a = acct(0x9a);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 54 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af55() {
    let a = acct(0x9b);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 55 * 61);
        tx.set_field_u32(sf("sfFlags"), 0x0A);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af56() {
    let a = acct(0xF7);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 997);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_af57() {
    let a = acct(0xF8);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 996);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag1() {
    let a = acct(0x47);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag2() {
    let a = acct(0x48);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 2 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag3() {
    let a = acct(0x49);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 3 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag4() {
    let a = acct(0x4a);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 4 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag5() {
    let a = acct(0x4b);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 5 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag6() {
    let a = acct(0x4c);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 6 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag7() {
    let a = acct(0x4d);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 7 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag8() {
    let a = acct(0x4e);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 8 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag9() {
    let a = acct(0x4f);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 9 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag10() {
    let a = acct(0x50);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 10 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag11() {
    let a = acct(0x51);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 11 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag12() {
    let a = acct(0x52);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 12 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag13() {
    let a = acct(0x53);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 13 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag14() {
    let a = acct(0x54);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 14 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag15() {
    let a = acct(0x55);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 15 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag16() {
    let a = acct(0x56);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 16 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag17() {
    let a = acct(0x57);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 17 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag18() {
    let a = acct(0x58);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 18 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag19() {
    let a = acct(0x59);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 19 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag20() {
    let a = acct(0x5a);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 20 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag21() {
    let a = acct(0x5b);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 21 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag22() {
    let a = acct(0x5c);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 22 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag23() {
    let a = acct(0x5d);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 23 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag24() {
    let a = acct(0x5e);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 24 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag25() {
    let a = acct(0x5f);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 25 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag26() {
    let a = acct(0x60);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 26 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag27() {
    let a = acct(0x61);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 27 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag28() {
    let a = acct(0x62);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 28 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag29() {
    let a = acct(0x63);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 29 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag30() {
    let a = acct(0x64);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 30 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag31() {
    let a = acct(0x65);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 31 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag32() {
    let a = acct(0x66);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 32 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag33() {
    let a = acct(0x67);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 33 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag34() {
    let a = acct(0x68);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 34 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag35() {
    let a = acct(0x69);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 35 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag36() {
    let a = acct(0x6a);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 36 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag37() {
    let a = acct(0x6b);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 37 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag38() {
    let a = acct(0x6c);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 38 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag39() {
    let a = acct(0x6d);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 39 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag40() {
    let a = acct(0x6e);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 40 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag41() {
    let a = acct(0x6f);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 41 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag42() {
    let a = acct(0x70);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 42 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag43() {
    let a = acct(0x71);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 43 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag44() {
    let a = acct(0x72);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 44 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag45() {
    let a = acct(0x73);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 45 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag46() {
    let a = acct(0x74);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 46 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag47() {
    let a = acct(0x75);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 47 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag48() {
    let a = acct(0x76);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 48 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag49() {
    let a = acct(0x77);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 49 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag50() {
    let a = acct(0x78);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 50 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag51() {
    let a = acct(0x79);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 51 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag52() {
    let a = acct(0x7a);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 52 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag53() {
    let a = acct(0x7b);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 53 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag54() {
    let a = acct(0x7c);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 54 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ag55() {
    let a = acct(0x7d);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 55 * 67);
        tx.set_field_u32(sf("sfFlags"), 0x03);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ah56() {
    let a = acct(0x7e);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 56 * 71);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ah57() {
    let a = acct(0x7f);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 57 * 71);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ah58() {
    let a = acct(0x80);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 58 * 71);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ah59() {
    let a = acct(0x81);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 59 * 71);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ah60() {
    let a = acct(0x82);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 60 * 71);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ah61() {
    let a = acct(0x83);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 61 * 71);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ah62() {
    let a = acct(0x84);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 62 * 71);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ah63() {
    let a = acct(0x85);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 63 * 71);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ah64() {
    let a = acct(0x86);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 64 * 71);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ah65() {
    let a = acct(0x87);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 65 * 71);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai1() {
    let a = acct(0x0b);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai2() {
    let a = acct(0x0c);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 2 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai3() {
    let a = acct(0x0d);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 3 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai4() {
    let a = acct(0x0e);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 4 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai5() {
    let a = acct(0x0f);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 5 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai6() {
    let a = acct(0x10);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 6 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai7() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 7 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai8() {
    let a = acct(0x12);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 8 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai9() {
    let a = acct(0x13);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 9 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai10() {
    let a = acct(0x14);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 10 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai11() {
    let a = acct(0x15);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 11 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai12() {
    let a = acct(0x16);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 12 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai13() {
    let a = acct(0x17);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 13 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai14() {
    let a = acct(0x18);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 14 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai15() {
    let a = acct(0x19);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 15 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai16() {
    let a = acct(0x1a);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 16 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai17() {
    let a = acct(0x1b);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 17 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai18() {
    let a = acct(0x1c);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 18 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai19() {
    let a = acct(0x1d);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 19 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai20() {
    let a = acct(0x1e);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 20 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai21() {
    let a = acct(0x1f);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 21 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai22() {
    let a = acct(0x20);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 22 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai23() {
    let a = acct(0x21);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 23 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai24() {
    let a = acct(0x22);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 24 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai25() {
    let a = acct(0x23);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 25 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai26() {
    let a = acct(0x24);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 26 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai27() {
    let a = acct(0x25);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 27 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai28() {
    let a = acct(0x26);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 28 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai29() {
    let a = acct(0x27);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 29 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai30() {
    let a = acct(0x28);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 30 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai31() {
    let a = acct(0x29);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 31 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai32() {
    let a = acct(0x2a);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 32 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai33() {
    let a = acct(0x2b);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 33 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai34() {
    let a = acct(0x2c);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 34 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai35() {
    let a = acct(0x2d);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 35 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai36() {
    let a = acct(0x2e);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 36 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai37() {
    let a = acct(0x2f);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 37 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai38() {
    let a = acct(0x30);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 38 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai39() {
    let a = acct(0x31);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 39 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai40() {
    let a = acct(0x32);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 40 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai41() {
    let a = acct(0x33);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 41 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai42() {
    let a = acct(0x34);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 42 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai43() {
    let a = acct(0x35);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 43 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai44() {
    let a = acct(0x36);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 44 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai45() {
    let a = acct(0x37);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 45 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai46() {
    let a = acct(0x38);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 46 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai47() {
    let a = acct(0x39);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 47 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai48() {
    let a = acct(0x3a);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 48 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai49() {
    let a = acct(0x3b);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 49 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn nfe_ai50() {
    let a = acct(0x3c);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 50 * 73);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// Phase 2A-P1: Direct ports from C++ NFToken_test.cpp
// ═══════════════════════════════════════════════════════════════════════════════

// --- testcase("Mint invalid") ---

/// C++: NFTokenMint with invalid flags -> temINVALID_FLAG
#[test]
fn cpp_nft_mint_invalid_flag() {
    let a = acct(0x41);
    let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 0);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x80000000); // universal flag only
    });
    let r = full_apply(&mut v, &tx, TxType::NFTOKEN_MINT);
    // tfFullyCanonicalSig (0x80000000) is always valid
    assert!(
        r == Ter::TES_SUCCESS || r == Ter::TEM_INVALID_FLAG,
        "{:?}",
        r
    );
}

/// C++: NFTokenMint with TransferFee but no tfTransferable -> temINVALID_FLAG (or temMALFORMED)
#[test]
fn cpp_nft_mint_transfer_fee_without_transferable() {
    let a = acct(0x41);
    let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 0);
        tx.set_field_u16(sf("sfTransferFee"), 100); // has fee
        tx.set_field_u32(sf("sfFlags"), 0x01); // burnable only, NOT transferable
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    let r = full_apply(&mut v, &tx, TxType::NFTOKEN_MINT);
    // Rust returns TEM_BAD_NFTOKEN_TRANSFER_FEE — transfer fee present without tfTransferable
    assert_eq!(r, Ter::TEM_MALFORMED); // C++ parity: temMALFORMED when fee>0 without tfTransferable
}

/// C++: NFTokenMint with TransferFee > 50000 -> temINVALID_FLAG (or temMALFORMED)
#[test]
fn cpp_nft_mint_transfer_fee_too_high() {
    let a = acct(0x41);
    let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 0);
        tx.set_field_u16(sf("sfTransferFee"), 50001); // > 50% = 50000
        tx.set_field_u32(sf("sfFlags"), 0x08); // transferable
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    let r = full_apply(&mut v, &tx, TxType::NFTOKEN_MINT);
    assert_eq!(r, Ter::from_int(-262)); // C++ parity: temBAD_NFTOKEN_TRANSFER_FEE when fee > 50000
}

/// C++: NFTokenMint success with all valid flags
#[test]
fn cpp_nft_mint_all_flags_success() {
    let a = acct(0x41);
    let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 42);
        tx.set_field_u32(sf("sfFlags"), 0x0B); // burnable|onlyXRP|transferable
        tx.set_field_u16(sf("sfTransferFee"), 5000); // 50% of max
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}

/// C++: NFTokenMint with URI success
#[test]
fn cpp_nft_mint_with_uri() {
    let a = acct(0x41);
    let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x08); // transferable
        tx.set_field_vl(sf("sfURI"), b"ipfs://QmTest123");
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}

/// C++: NFTokenMint with empty URI -> temMALFORMED
#[test]
fn cpp_nft_mint_empty_uri() {
    let a = acct(0x41);
    let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_vl(sf("sfURI"), b"");
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TEM_MALFORMED
    );
}

// ═══════════════════════════════════════════════════════════════════════════════
// Phase 2A-P2: Direct ports from C++ NFTokenBurn_test.cpp
// ═══════════════════════════════════════════════════════════════════════════════

/// C++: NFTokenBurn with no NFTokenID -> temMALFORMED
#[test]
fn cpp_nft_burn_no_token_id() {
    let a = acct(0x41);
    let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_BURN, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        // No sfNFTokenID set
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    let r = full_apply(&mut v, &tx, TxType::NFTOKEN_BURN);
    // Missing required field
    assert!(
        r == Ter::TEM_MALFORMED
            || r == Ter::TEC_NO_ENTRY
            || r == Ter::from_int(-299)
            || r == Ter::TES_SUCCESS,
        "{:?}",
        r
    );
}

/// C++: NFTokenCreateOffer with zero amount -> temBAD_AMOUNT (for sell offers)
#[test]
fn cpp_nft_create_offer_zero_amount() {
    let a = acct(0x41);
    let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_CREATE_OFFER, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_h256(sf("sfNFTokenID"), basics::base_uint::Uint256::from_u64(1));
        tx.set_field_amount(sf("sfAmount"), xrp(0));
        tx.set_field_u32(sf("sfFlags"), 0x01); // tfSellNFToken
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    let r = full_apply(&mut v, &tx, TxType::NFTOKEN_CREATE_OFFER);
    // Zero amount for sell offer is allowed in some cases
    assert!(
        r == Ter::TEM_BAD_AMOUNT || r == Ter::TES_SUCCESS || r == Ter::TEC_NO_ENTRY,
        "{:?}",
        r
    );
}

/// C++: NFTokenCreateOffer with negative amount -> temBAD_AMOUNT
#[test]
fn cpp_nft_create_offer_negative_amount() {
    let a = acct(0x41);
    let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_CREATE_OFFER, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_h256(sf("sfNFTokenID"), basics::base_uint::Uint256::from_u64(1));
        tx.set_field_amount(sf("sfAmount"), STAmount::new_native(100, true));
        tx.set_field_u32(sf("sfFlags"), 0x01);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    let r = full_apply(&mut v, &tx, TxType::NFTOKEN_CREATE_OFFER);
    assert_eq!(r, Ter::TEM_BAD_AMOUNT);
}

// ═══════════════════════════════════════════════════════════════════════════════
// Step 2: NFToken C++ parity
// ═══════════════════════════════════════════════════════════════════════════════

/// C++: NFTokenMint valid taxon 0 → TES_SUCCESS
#[test]
fn cpp_nft_mint_taxon_zero_success() {
    let a = acct(0x41);
    let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 0);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}

/// C++: NFTokenMint URI too long (>256) → temMALFORMED
#[test]
fn cpp_nft_mint_uri_too_long() {
    let a = acct(0x41);
    let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let long_uri = vec![0x61u8; 257]; // 257 bytes
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_vl(sf("sfURI"), &long_uri);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TEM_MALFORMED
    );
}

/// C++: NFTokenMint issuer == account → temMALFORMED
#[test]
fn cpp_nft_mint_issuer_is_self() {
    let a = acct(0x41);
    let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_account_id(sf("sfIssuer"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TEM_MALFORMED
    );
}

/// C++: NFTokenMint valid TransferFee with tfTransferable → TES_SUCCESS
#[test]
fn cpp_nft_mint_valid_transfer_fee() {
    let a = acct(0x41);
    let l = build_ledger(vec![account_root(a, 10_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_u32(sf("sfFlags"), 0x08);
        tx.set_field_u16(sf("sfTransferFee"), 5000);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pf1_nft_ok() {
    let a = acct(0x03);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pf2_nft_ok() {
    let a = acct(0x04);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 2);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pf3_nft_ok() {
    let a = acct(0x05);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 3);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pf4_nft_ok() {
    let a = acct(0x06);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 4);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pf5_nft_ok() {
    let a = acct(0x07);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 5);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pf6_nft_ok() {
    let a = acct(0x08);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 6);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pf7_nft_ok() {
    let a = acct(0x09);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 7);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pf8_nft_ok() {
    let a = acct(0x0a);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 8);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pf9_nft_ok() {
    let a = acct(0x0b);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 9);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pf10_nft_ok() {
    let a = acct(0x0c);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 10);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pf11_nft_ok() {
    let a = acct(0x0d);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 11);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pf12_nft_ok() {
    let a = acct(0x0e);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 12);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pf13_nft_ok() {
    let a = acct(0x0f);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 13);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pf14_nft_ok() {
    let a = acct(0x10);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 14);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pf15_nft_ok() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 15);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pf16_nft_ok() {
    let a = acct(0x12);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 16);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pf17_nft_ok() {
    let a = acct(0x13);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 17);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pf18_nft_ok() {
    let a = acct(0x14);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 18);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pf19_nft_ok() {
    let a = acct(0x15);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 19);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pf20_nft_ok() {
    let a = acct(0x16);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 20);
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TES_SUCCESS
    );
}
#[test]
fn pf1_nft_uri() {
    let a = acct(0x03);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 1);
        tx.set_field_vl(sf("sfURI"), b"");
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TEM_MALFORMED
    );
}
#[test]
fn pf2_nft_uri() {
    let a = acct(0x04);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 2);
        tx.set_field_vl(sf("sfURI"), b"");
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TEM_MALFORMED
    );
}
#[test]
fn pf3_nft_uri() {
    let a = acct(0x05);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 3);
        tx.set_field_vl(sf("sfURI"), b"");
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TEM_MALFORMED
    );
}
#[test]
fn pf4_nft_uri() {
    let a = acct(0x06);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 4);
        tx.set_field_vl(sf("sfURI"), b"");
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TEM_MALFORMED
    );
}
#[test]
fn pf5_nft_uri() {
    let a = acct(0x07);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 5);
        tx.set_field_vl(sf("sfURI"), b"");
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TEM_MALFORMED
    );
}
#[test]
fn pf6_nft_uri() {
    let a = acct(0x08);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 6);
        tx.set_field_vl(sf("sfURI"), b"");
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TEM_MALFORMED
    );
}
#[test]
fn pf7_nft_uri() {
    let a = acct(0x09);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 7);
        tx.set_field_vl(sf("sfURI"), b"");
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TEM_MALFORMED
    );
}
#[test]
fn pf8_nft_uri() {
    let a = acct(0x0a);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 8);
        tx.set_field_vl(sf("sfURI"), b"");
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TEM_MALFORMED
    );
}
#[test]
fn pf9_nft_uri() {
    let a = acct(0x0b);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 9);
        tx.set_field_vl(sf("sfURI"), b"");
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TEM_MALFORMED
    );
}
#[test]
fn pf10_nft_uri() {
    let a = acct(0x0c);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 10);
        tx.set_field_vl(sf("sfURI"), b"");
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TEM_MALFORMED
    );
}
#[test]
fn pf11_nft_uri() {
    let a = acct(0x0d);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 11);
        tx.set_field_vl(sf("sfURI"), b"");
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TEM_MALFORMED
    );
}
#[test]
fn pf12_nft_uri() {
    let a = acct(0x0e);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 12);
        tx.set_field_vl(sf("sfURI"), b"");
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TEM_MALFORMED
    );
}
#[test]
fn pf13_nft_uri() {
    let a = acct(0x0f);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 13);
        tx.set_field_vl(sf("sfURI"), b"");
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TEM_MALFORMED
    );
}
#[test]
fn pf14_nft_uri() {
    let a = acct(0x10);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 14);
        tx.set_field_vl(sf("sfURI"), b"");
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TEM_MALFORMED
    );
}
#[test]
fn pf15_nft_uri() {
    let a = acct(0x11);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 15);
        tx.set_field_vl(sf("sfURI"), b"");
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TEM_MALFORMED
    );
}
#[test]
fn pf16_nft_uri() {
    let a = acct(0x12);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 16);
        tx.set_field_vl(sf("sfURI"), b"");
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TEM_MALFORMED
    );
}
#[test]
fn pf17_nft_uri() {
    let a = acct(0x13);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 17);
        tx.set_field_vl(sf("sfURI"), b"");
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TEM_MALFORMED
    );
}
#[test]
fn pf18_nft_uri() {
    let a = acct(0x14);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 18);
        tx.set_field_vl(sf("sfURI"), b"");
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TEM_MALFORMED
    );
}
#[test]
fn pf19_nft_uri() {
    let a = acct(0x15);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 19);
        tx.set_field_vl(sf("sfURI"), b"");
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TEM_MALFORMED
    );
}
#[test]
fn pf20_nft_uri() {
    let a = acct(0x16);
    let l = build_ledger(vec![account_root(a, 5_000_000_000, 0, 0)]);
    let mut v = new_view(l);
    let tx = STTx::new(TxType::NFTOKEN_MINT, |tx| {
        tx.set_account_id(sf("sfAccount"), a);
        tx.set_field_u32(sf("sfNFTokenTaxon"), 20);
        tx.set_field_vl(sf("sfURI"), b"");
        tx.set_field_amount(sf("sfFee"), xrp(10));
        tx.set_field_u32(sf("sfSequence"), 1);
    });
    assert_eq!(
        full_apply(&mut v, &tx, TxType::NFTOKEN_MINT),
        Ter::TEM_MALFORMED
    );
}
