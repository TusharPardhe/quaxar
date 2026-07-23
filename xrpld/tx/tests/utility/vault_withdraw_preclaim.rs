//! Integration tests that pin the higher narrowed Rust
//! `VaultWithdraw.cpp::preclaim(...)` wrapper to the current C++ behavior.

use std::{cell::Cell, rc::Rc};

use protocol::{Ter, trans_token};
use tx::{
    VaultWithdrawPreclaimFrontFacts, VaultWithdrawPreclaimTailFacts, VaultWithdrawRequireAuthType,
    VaultWithdrawShareBranchResult, run_vault_withdraw_preclaim,
};

#[test]
fn vault_withdraw_preclaim_returns_front_failure_before_tail() {
    let tail_called = Cell::new(false);

    let result = run_vault_withdraw_preclaim(
        VaultWithdrawPreclaimFrontFacts::default(),
        VaultWithdrawPreclaimTailFacts::default(),
        || Ter::TES_SUCCESS,
        || VaultWithdrawShareBranchResult::Success,
        || Ter::TES_SUCCESS,
        |_| {
            tail_called.set(true);
            Ter::TES_SUCCESS
        },
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
    );

    assert_eq!(result, Ter::TEC_NO_ENTRY);
    assert_eq!(trans_token(result), "tecNO_ENTRY");
    assert!(!tail_called.get());
}

#[test]
fn vault_withdraw_preclaim_returns_tail_failure_after_front_success() {
    let result = run_vault_withdraw_preclaim(
        VaultWithdrawPreclaimFrontFacts {
            vault_exists: true,
            amount_asset_matches_vault_asset_or_share: true,
            withdrawal_policy_is_first_come_first_serve: true,
            ..VaultWithdrawPreclaimFrontFacts::default()
        },
        VaultWithdrawPreclaimTailFacts::default(),
        || Ter::TES_SUCCESS,
        || VaultWithdrawShareBranchResult::Success,
        || Ter::TES_SUCCESS,
        |_| Ter::TEC_NO_AUTH,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
    );

    assert_eq!(result, Ter::TEC_NO_AUTH);
    assert_eq!(trans_token(result), "tecNO_AUTH");
}

#[test]
fn vault_withdraw_preclaim_runs_current_cpp_stage_order() {
    let seen = Rc::new(std::cell::RefCell::new(Vec::new()));

    let result = run_vault_withdraw_preclaim(
        VaultWithdrawPreclaimFrontFacts {
            vault_exists: true,
            amount_asset_matches_vault_asset_or_share: true,
            withdrawal_policy_is_first_come_first_serve: true,
            fix_cleanup_3_1_3_enabled: true,
            amount_asset_is_vault_share: true,
            share_issuance_exists: true,
        },
        VaultWithdrawPreclaimTailFacts {
            destination_is_submitter: false,
            ..VaultWithdrawPreclaimTailFacts::default()
        },
        {
            let seen = Rc::clone(&seen);
            move || {
                seen.borrow_mut().push("transfer");
                Ter::TES_SUCCESS
            }
        },
        {
            let seen = Rc::clone(&seen);
            move || {
                seen.borrow_mut().push("share");
                VaultWithdrawShareBranchResult::Success
            }
        },
        {
            let seen = Rc::clone(&seen);
            move || {
                seen.borrow_mut().push("direct");
                Ter::TES_SUCCESS
            }
        },
        {
            let seen = Rc::clone(&seen);
            move |auth_type| {
                seen.borrow_mut().push(match auth_type {
                    VaultWithdrawRequireAuthType::WeakAuth => "auth_weak",
                    VaultWithdrawRequireAuthType::StrongAuth => "auth_strong",
                });
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
        || Ter::TES_SUCCESS,
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        seen.borrow().as_slice(),
        [
            "transfer",
            "share",
            "auth_strong",
            "destination_frozen",
            "share_frozen",
        ]
    );
}
