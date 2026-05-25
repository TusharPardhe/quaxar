//! Integration tests that pin the narrowed Rust front half of
//! `VaultWithdraw.cpp::preclaim(...)` to the current C++ behavior.

use std::{cell::Cell, rc::Rc};

use protocol::{Ter, trans_token};
use tx::{
    VaultWithdrawPreclaimFrontFacts, VaultWithdrawShareBranchResult,
    run_vault_withdraw_preclaim_front,
};

#[test]
fn vault_withdraw_preclaim_front_rejects_missing_vault() {
    let can_transfer_called = Cell::new(false);

    let result = run_vault_withdraw_preclaim_front(
        VaultWithdrawPreclaimFrontFacts::default(),
        || {
            can_transfer_called.set(true);
            Ter::TES_SUCCESS
        },
        || VaultWithdrawShareBranchResult::Success,
        || Ter::TES_SUCCESS,
    );

    assert_eq!(result, Ter::TEC_NO_ENTRY);
    assert_eq!(trans_token(result), "tecNO_ENTRY");
    assert!(!can_transfer_called.get());
}

#[test]
fn vault_withdraw_preclaim_front_rejects_asset_mismatch() {
    let can_transfer_called = Cell::new(false);

    let result = run_vault_withdraw_preclaim_front(
        VaultWithdrawPreclaimFrontFacts {
            vault_exists: true,
            ..VaultWithdrawPreclaimFrontFacts::default()
        },
        || {
            can_transfer_called.set(true);
            Ter::TES_SUCCESS
        },
        || VaultWithdrawShareBranchResult::Success,
        || Ter::TES_SUCCESS,
    );

    assert_eq!(result, Ter::TEC_WRONG_ASSET);
    assert!(!can_transfer_called.get());
}

#[test]
fn vault_withdraw_preclaim_front_returns_transfer_failure_first() {
    let policy_checked = Cell::new(false);

    let result = run_vault_withdraw_preclaim_front(
        VaultWithdrawPreclaimFrontFacts {
            vault_exists: true,
            amount_asset_matches_vault_asset_or_share: true,
            ..VaultWithdrawPreclaimFrontFacts::default()
        },
        || Ter::TER_NO_RIPPLE,
        || {
            policy_checked.set(true);
            VaultWithdrawShareBranchResult::Success
        },
        || Ter::TES_SUCCESS,
    );

    assert_eq!(result, Ter::TER_NO_RIPPLE);
    assert_eq!(trans_token(result), "terNO_RIPPLE");
    assert!(!policy_checked.get());
}

#[test]
fn vault_withdraw_preclaim_front_rejects_invalid_withdrawal_policy() {
    let result = run_vault_withdraw_preclaim_front(
        VaultWithdrawPreclaimFrontFacts {
            vault_exists: true,
            amount_asset_matches_vault_asset_or_share: true,
            ..VaultWithdrawPreclaimFrontFacts::default()
        },
        || Ter::TES_SUCCESS,
        || VaultWithdrawShareBranchResult::Success,
        || Ter::TES_SUCCESS,
    );

    assert_eq!(result, Ter::TEF_INTERNAL);
}

#[test]
fn vault_withdraw_preclaim_front_rejects_missing_share_issuance() {
    let share_called = Cell::new(false);

    let result = run_vault_withdraw_preclaim_front(
        VaultWithdrawPreclaimFrontFacts {
            vault_exists: true,
            amount_asset_matches_vault_asset_or_share: true,
            withdrawal_policy_is_first_come_first_serve: true,
            fix_security_3_1_3_enabled: true,
            amount_asset_is_vault_share: true,
            ..VaultWithdrawPreclaimFrontFacts::default()
        },
        || Ter::TES_SUCCESS,
        || {
            share_called.set(true);
            VaultWithdrawShareBranchResult::Success
        },
        || Ter::TES_SUCCESS,
    );

    assert_eq!(result, Ter::TEF_INTERNAL);
    assert!(!share_called.get());
}

#[test]
fn vault_withdraw_preclaim_front_maps_share_branch_failures() {
    let missing_assets = run_vault_withdraw_preclaim_front(
        VaultWithdrawPreclaimFrontFacts {
            vault_exists: true,
            amount_asset_matches_vault_asset_or_share: true,
            withdrawal_policy_is_first_come_first_serve: true,
            fix_security_3_1_3_enabled: true,
            amount_asset_is_vault_share: true,
            share_issuance_exists: true,
        },
        || Ter::TES_SUCCESS,
        || VaultWithdrawShareBranchResult::MissingConvertedAssets,
        || Ter::TES_SUCCESS,
    );
    let overflow = run_vault_withdraw_preclaim_front(
        VaultWithdrawPreclaimFrontFacts {
            vault_exists: true,
            amount_asset_matches_vault_asset_or_share: true,
            withdrawal_policy_is_first_come_first_serve: true,
            fix_security_3_1_3_enabled: true,
            amount_asset_is_vault_share: true,
            share_issuance_exists: true,
        },
        || Ter::TES_SUCCESS,
        || VaultWithdrawShareBranchResult::Overflow,
        || Ter::TES_SUCCESS,
    );
    let can_withdraw_failure = run_vault_withdraw_preclaim_front(
        VaultWithdrawPreclaimFrontFacts {
            vault_exists: true,
            amount_asset_matches_vault_asset_or_share: true,
            withdrawal_policy_is_first_come_first_serve: true,
            fix_security_3_1_3_enabled: true,
            amount_asset_is_vault_share: true,
            share_issuance_exists: true,
        },
        || Ter::TES_SUCCESS,
        || VaultWithdrawShareBranchResult::CanWithdrawFailure(Ter::TEC_NO_PERMISSION),
        || Ter::TES_SUCCESS,
    );

    assert_eq!(missing_assets, Ter::TEF_INTERNAL);
    assert_eq!(overflow, Ter::TEC_PATH_DRY);
    assert_eq!(trans_token(overflow), "tecPATH_DRY");
    assert_eq!(can_withdraw_failure, Ter::TEC_NO_PERMISSION);
}

#[test]
fn vault_withdraw_preclaim_front_uses_direct_branch_when_share_branch_is_inactive() {
    let direct_called = Cell::new(false);

    let result = run_vault_withdraw_preclaim_front(
        VaultWithdrawPreclaimFrontFacts {
            vault_exists: true,
            amount_asset_matches_vault_asset_or_share: true,
            withdrawal_policy_is_first_come_first_serve: true,
            fix_security_3_1_3_enabled: true,
            ..VaultWithdrawPreclaimFrontFacts::default()
        },
        || Ter::TES_SUCCESS,
        || VaultWithdrawShareBranchResult::CanWithdrawFailure(Ter::TEC_NO_PERMISSION),
        || {
            direct_called.set(true);
            Ter::TEC_NO_AUTH
        },
    );

    assert_eq!(result, Ter::TEC_NO_AUTH);
    assert!(direct_called.get());
}

#[test]
fn vault_withdraw_preclaim_front_runs_share_branch_in_current_on_success() {
    let seen = Rc::new(std::cell::RefCell::new(Vec::new()));

    let result = run_vault_withdraw_preclaim_front(
        VaultWithdrawPreclaimFrontFacts {
            vault_exists: true,
            amount_asset_matches_vault_asset_or_share: true,
            withdrawal_policy_is_first_come_first_serve: true,
            fix_security_3_1_3_enabled: true,
            amount_asset_is_vault_share: true,
            share_issuance_exists: true,
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
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(seen.borrow().as_slice(), ["transfer", "share"]);
}

#[test]
fn vault_withdraw_preclaim_front_runs_direct_branch_in_current_on_success() {
    let seen = Rc::new(std::cell::RefCell::new(Vec::new()));

    let result = run_vault_withdraw_preclaim_front(
        VaultWithdrawPreclaimFrontFacts {
            vault_exists: true,
            amount_asset_matches_vault_asset_or_share: true,
            withdrawal_policy_is_first_come_first_serve: true,
            ..VaultWithdrawPreclaimFrontFacts::default()
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
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(seen.borrow().as_slice(), ["transfer", "direct"]);
}
