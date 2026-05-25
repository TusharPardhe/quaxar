//! Integration tests that pin the narrowed Rust tail half of
//! `VaultWithdraw.cpp::preclaim(...)` to the current C++ behavior.

use std::{cell::Cell, rc::Rc};

use protocol::{Ter, trans_token};
use tx::{
    VaultWithdrawPreclaimTailFacts, VaultWithdrawRequireAuthType, run_vault_withdraw_preclaim_tail,
};

#[test]
fn vault_withdraw_preclaim_tail_uses_weakauth_when_destination_is_submitter() {
    let seen = Cell::new(None);

    let result = run_vault_withdraw_preclaim_tail(
        VaultWithdrawPreclaimTailFacts {
            destination_is_submitter: true,
        },
        |auth_type| {
            seen.set(Some(auth_type));
            Ter::TES_SUCCESS
        },
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(seen.get(), Some(VaultWithdrawRequireAuthType::WeakAuth));
}

#[test]
fn vault_withdraw_preclaim_tail_uses_strongauth_when_destination_differs() {
    let seen = Cell::new(None);

    let result = run_vault_withdraw_preclaim_tail(
        VaultWithdrawPreclaimTailFacts::default(),
        |auth_type| {
            seen.set(Some(auth_type));
            Ter::TES_SUCCESS
        },
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(seen.get(), Some(VaultWithdrawRequireAuthType::StrongAuth));
}

#[test]
fn vault_withdraw_preclaim_tail_returns_auth_failure_first() {
    let destination_checked = Cell::new(false);

    let result = run_vault_withdraw_preclaim_tail(
        VaultWithdrawPreclaimTailFacts::default(),
        |_| Ter::TEC_NO_AUTH,
        || {
            destination_checked.set(true);
            Ter::TES_SUCCESS
        },
        || Ter::TES_SUCCESS,
    );

    assert_eq!(result, Ter::TEC_NO_AUTH);
    assert_eq!(trans_token(result), "tecNO_AUTH");
    assert!(!destination_checked.get());
}

#[test]
fn vault_withdraw_preclaim_tail_returns_destination_freeze_failure_before_share_check() {
    let share_checked = Cell::new(false);

    let result = run_vault_withdraw_preclaim_tail(
        VaultWithdrawPreclaimTailFacts::default(),
        |_| Ter::TES_SUCCESS,
        || Ter::TEC_FROZEN,
        || {
            share_checked.set(true);
            Ter::TES_SUCCESS
        },
    );

    assert_eq!(result, Ter::TEC_FROZEN);
    assert_eq!(trans_token(result), "tecFROZEN");
    assert!(!share_checked.get());
}

#[test]
fn vault_withdraw_preclaim_tail_returns_submitter_share_freeze_failure() {
    let result = run_vault_withdraw_preclaim_tail(
        VaultWithdrawPreclaimTailFacts::default(),
        |_| Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TEC_LOCKED,
    );

    assert_eq!(result, Ter::TEC_LOCKED);
    assert_eq!(trans_token(result), "tecLOCKED");
}

#[test]
fn vault_withdraw_preclaim_tail_runs_current_on_success() {
    let seen = Rc::new(std::cell::RefCell::new(Vec::new()));

    let result = run_vault_withdraw_preclaim_tail(
        VaultWithdrawPreclaimTailFacts::default(),
        {
            let seen = Rc::clone(&seen);
            move |_| {
                seen.borrow_mut().push("auth");
                Ter::TES_SUCCESS
            }
        },
        {
            let seen = Rc::clone(&seen);
            move || {
                seen.borrow_mut().push("destination_frozen");
                Ter::TES_SUCCESS
            }
        },
        {
            let seen = Rc::clone(&seen);
            move || {
                seen.borrow_mut().push("share_frozen");
                Ter::TES_SUCCESS
            }
        },
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        seen.borrow().as_slice(),
        ["auth", "destination_frozen", "share_frozen"]
    );
}
