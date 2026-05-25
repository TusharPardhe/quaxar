//! Integration tests that pin the narrowed Rust
//! `VaultDeposit.cpp::doApply()` transfer tail to the current C++ behavior.

use std::{cell::Cell, rc::Rc};

use protocol::{Ter, trans_token};
use tx::run_vault_deposit_do_apply_transfer_tail;

#[test]
fn vault_deposit_do_apply_transfer_tail_returns_asset_transfer_failure_unchanged() {
    let negative_checked = Cell::new(false);
    let shares_sent = Cell::new(false);
    let associated = Cell::new(false);

    let result = run_vault_deposit_do_apply_transfer_tail(
        || Ter::TER_NO_RIPPLE,
        || {
            negative_checked.set(true);
            false
        },
        || {
            shares_sent.set(true);
            Ter::TES_SUCCESS
        },
        || {
            associated.set(true);
        },
    );

    assert_eq!(result, Ter::TER_NO_RIPPLE);
    assert_eq!(trans_token(result), "terNO_RIPPLE");
    assert!(!negative_checked.get());
    assert!(!shares_sent.get());
    assert!(!associated.get());
}

#[test]
fn vault_deposit_do_apply_transfer_tail_maps_negative_balance_to_tefinternal() {
    let shares_sent = Cell::new(false);
    let associated = Cell::new(false);

    let result = run_vault_deposit_do_apply_transfer_tail(
        || Ter::TES_SUCCESS,
        || true,
        || {
            shares_sent.set(true);
            Ter::TES_SUCCESS
        },
        || {
            associated.set(true);
        },
    );

    assert_eq!(result, Ter::TEF_INTERNAL);
    assert_eq!(trans_token(result), "tefINTERNAL");
    assert!(!shares_sent.get());
    assert!(!associated.get());
}

#[test]
fn vault_deposit_do_apply_transfer_tail_returns_share_transfer_failure_unchanged() {
    let associated = Cell::new(false);

    let result = run_vault_deposit_do_apply_transfer_tail(
        || Ter::TES_SUCCESS,
        || false,
        || Ter::TEC_NO_PERMISSION,
        || {
            associated.set(true);
        },
    );

    assert_eq!(result, Ter::TEC_NO_PERMISSION);
    assert_eq!(trans_token(result), "tecNO_PERMISSION");
    assert!(!associated.get());
}

#[test]
fn vault_deposit_do_apply_transfer_tail_runs_in_current_on_success() {
    let steps = Rc::new(std::cell::RefCell::new(Vec::new()));

    let result = run_vault_deposit_do_apply_transfer_tail(
        {
            let steps = Rc::clone(&steps);
            move || {
                steps.borrow_mut().push("send-assets");
                Ter::TES_SUCCESS
            }
        },
        {
            let steps = Rc::clone(&steps);
            move || {
                steps.borrow_mut().push("check-negative");
                false
            }
        },
        {
            let steps = Rc::clone(&steps);
            move || {
                steps.borrow_mut().push("send-shares");
                Ter::TES_SUCCESS
            }
        },
        {
            let steps = Rc::clone(&steps);
            move || {
                steps.borrow_mut().push("associate-asset");
            }
        },
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        steps.borrow().as_slice(),
        [
            "send-assets",
            "check-negative",
            "send-shares",
            "associate-asset"
        ]
    );
}
