//! Integration tests that pin the narrowed Rust
//! `VaultWithdraw.cpp::doApply()` transfer tail to the current C++ behavior.

use std::{cell::Cell, rc::Rc};

use protocol::{Ter, trans_token};
use tx::{VaultWithdrawDoApplyTransferTailFacts, run_vault_withdraw_do_apply_transfer_tail};

#[test]
fn vault_withdraw_do_apply_transfer_tail_returns_share_transfer_failure_unchanged() {
    let remove_called = Cell::new(false);
    let associated = Cell::new(false);
    let withdrew = Cell::new(false);

    let result = run_vault_withdraw_do_apply_transfer_tail(
        VaultWithdrawDoApplyTransferTailFacts {
            account: "depositor",
            vault_owner: "vault-owner",
            destination: Some("erin"),
        },
        || Ter::TER_NO_ACCOUNT,
        || {
            remove_called.set(true);
            Ter::TES_SUCCESS
        },
        || {
            associated.set(true);
        },
        |_| {
            withdrew.set(true);
            Ter::TES_SUCCESS
        },
    );

    assert_eq!(result, Ter::TER_NO_ACCOUNT);
    assert_eq!(trans_token(result), "terNO_ACCOUNT");
    assert!(!remove_called.get());
    assert!(!associated.get());
    assert!(!withdrew.get());
}

#[test]
fn vault_withdraw_do_apply_transfer_tail_returns_non_obligation_cleanup_failure_unchanged() {
    let associated = Cell::new(false);
    let withdrew = Cell::new(false);

    let result = run_vault_withdraw_do_apply_transfer_tail(
        VaultWithdrawDoApplyTransferTailFacts {
            account: "depositor",
            vault_owner: "vault-owner",
            destination: Some("erin"),
        },
        || Ter::TES_SUCCESS,
        || Ter::TEC_NO_PERMISSION,
        || {
            associated.set(true);
        },
        |_| {
            withdrew.set(true);
            Ter::TES_SUCCESS
        },
    );

    assert_eq!(result, Ter::TEC_NO_PERMISSION);
    assert_eq!(trans_token(result), "tecNO_PERMISSION");
    assert!(!associated.get());
    assert!(!withdrew.get());
}

#[test]
fn vault_withdraw_do_apply_transfer_tail_ignores_techas_obligations() {
    let steps = Rc::new(std::cell::RefCell::new(Vec::new()));

    let result = run_vault_withdraw_do_apply_transfer_tail(
        VaultWithdrawDoApplyTransferTailFacts {
            account: "depositor",
            vault_owner: "vault-owner",
            destination: Some("erin"),
        },
        {
            let steps = Rc::clone(&steps);
            move || {
                steps.borrow_mut().push("send_shares".to_string());
                Ter::TES_SUCCESS
            }
        },
        {
            let steps = Rc::clone(&steps);
            move || {
                steps.borrow_mut().push("remove_empty_holding".to_string());
                Ter::TEC_HAS_OBLIGATIONS
            }
        },
        {
            let steps = Rc::clone(&steps);
            move || {
                steps.borrow_mut().push("associate_asset".to_string());
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |dst| {
                steps.borrow_mut().push(format!("do_withdraw:{dst}"));
                Ter::TES_SUCCESS
            }
        },
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        steps.borrow().as_slice(),
        [
            "send_shares",
            "remove_empty_holding",
            "associate_asset",
            "do_withdraw:erin",
        ]
    );
}

#[test]
fn vault_withdraw_do_apply_transfer_tail_skips_cleanup_for_owner() {
    let remove_called = Cell::new(false);
    let seen_destination = Cell::new(None);

    let result = run_vault_withdraw_do_apply_transfer_tail(
        VaultWithdrawDoApplyTransferTailFacts {
            account: "vault-owner",
            vault_owner: "vault-owner",
            destination: Some("erin"),
        },
        || Ter::TES_SUCCESS,
        || {
            remove_called.set(true);
            Ter::TES_SUCCESS
        },
        || {},
        |dst| {
            seen_destination.set(Some(*dst));
            Ter::TES_SUCCESS
        },
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert!(!remove_called.get());
    assert_eq!(seen_destination.get(), Some("erin"));
}

#[test]
fn vault_withdraw_do_apply_transfer_tail_defaults_missing_destination_to_submitter() {
    let steps = Rc::new(std::cell::RefCell::new(Vec::new()));

    let result = run_vault_withdraw_do_apply_transfer_tail(
        VaultWithdrawDoApplyTransferTailFacts {
            account: "depositor",
            vault_owner: "vault-owner",
            destination: None,
        },
        {
            let steps = Rc::clone(&steps);
            move || {
                steps.borrow_mut().push("send_shares".to_string());
                Ter::TES_SUCCESS
            }
        },
        {
            let steps = Rc::clone(&steps);
            move || {
                steps.borrow_mut().push("remove_empty_holding".to_string());
                Ter::TES_SUCCESS
            }
        },
        {
            let steps = Rc::clone(&steps);
            move || {
                steps.borrow_mut().push("associate_asset".to_string());
            }
        },
        {
            let steps = Rc::clone(&steps);
            move |dst| {
                steps.borrow_mut().push(format!("do_withdraw:{dst}"));
                Ter::TES_SUCCESS
            }
        },
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        steps.borrow().as_slice(),
        [
            "send_shares",
            "remove_empty_holding",
            "associate_asset",
            "do_withdraw:depositor",
        ]
    );
}
