//! Integration tests that pin the narrowed Rust `VaultDeposit.cpp::preclaim(...)`
//! wrapper to the current C++ behavior.

use std::{cell::Cell, rc::Rc};

use protocol::{Ter, trans_token};
use tx::{VaultDepositPreclaimFacts, run_vault_deposit_preclaim};

#[test]
fn vault_deposit_preclaim_rejects_missing_vault() {
    let can_transfer_called = Cell::new(false);

    let result = run_vault_deposit_preclaim(
        VaultDepositPreclaimFacts::default(),
        || {
            can_transfer_called.set(true);
            Ter::TES_SUCCESS
        },
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
    );

    assert_eq!(result, Ter::TEC_NO_ENTRY);
    assert_eq!(trans_token(result), "tecNO_ENTRY");
    assert!(!can_transfer_called.get());
}

#[test]
fn vault_deposit_preclaim_rejects_asset_mismatch_before_helper_calls() {
    let can_transfer_called = Cell::new(false);

    let result = run_vault_deposit_preclaim(
        VaultDepositPreclaimFacts {
            vault_exists: true,
            ..VaultDepositPreclaimFacts::default()
        },
        || {
            can_transfer_called.set(true);
            Ter::TES_SUCCESS
        },
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
    );

    assert_eq!(result, Ter::TEC_WRONG_ASSET);
    assert!(!can_transfer_called.get());
}

#[test]
fn vault_deposit_preclaim_returns_transfer_failure_first() {
    let domain_checked = Cell::new(false);

    let result = run_vault_deposit_preclaim(
        VaultDepositPreclaimFacts {
            vault_exists: true,
            deposited_asset_matches_vault_asset: true,
            ..VaultDepositPreclaimFacts::default()
        },
        || Ter::TER_NO_RIPPLE,
        || {
            domain_checked.set(true);
            Ter::TES_SUCCESS
        },
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
    );

    assert_eq!(result, Ter::TER_NO_RIPPLE);
    assert_eq!(trans_token(result), "terNO_RIPPLE");
    assert!(!domain_checked.get());
}

#[test]
fn vault_deposit_preclaim_rejects_vault_share_asset_overlap() {
    let result = run_vault_deposit_preclaim(
        VaultDepositPreclaimFacts {
            vault_exists: true,
            deposited_asset_matches_vault_asset: true,
            vault_share_matches_vault_asset: true,
            ..VaultDepositPreclaimFacts::default()
        },
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
    );

    assert_eq!(result, Ter::TEF_INTERNAL);
}

#[test]
fn vault_deposit_preclaim_maps_frozen_asset_to_tecfrozen_or_teclocked() {
    let issue = run_vault_deposit_preclaim(
        VaultDepositPreclaimFacts {
            vault_exists: true,
            deposited_asset_matches_vault_asset: true,
            issuance_exists: true,
            vault_asset_is_issue: true,
            vault_asset_frozen_for_account: true,
            ..VaultDepositPreclaimFacts::default()
        },
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
    );
    let non_issue = run_vault_deposit_preclaim(
        VaultDepositPreclaimFacts {
            vault_exists: true,
            deposited_asset_matches_vault_asset: true,
            issuance_exists: true,
            vault_asset_frozen_for_account: true,
            ..VaultDepositPreclaimFacts::default()
        },
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
    );

    assert_eq!(issue, Ter::TEC_FROZEN);
    assert_eq!(non_issue, Ter::TEC_LOCKED);
}

#[test]
fn vault_deposit_preclaim_rejects_frozen_shares() {
    let result = run_vault_deposit_preclaim(
        VaultDepositPreclaimFacts {
            vault_exists: true,
            deposited_asset_matches_vault_asset: true,
            issuance_exists: true,
            vault_share_frozen_for_account: true,
            ..VaultDepositPreclaimFacts::default()
        },
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
    );

    assert_eq!(result, Ter::TEC_LOCKED);
}

#[test]
fn vault_deposit_preclaim_requires_domain_for_private_non_owner_vaults() {
    let valid_domain_called = Cell::new(false);

    let result = run_vault_deposit_preclaim(
        VaultDepositPreclaimFacts {
            vault_exists: true,
            deposited_asset_matches_vault_asset: true,
            issuance_exists: true,
            vault_is_private: true,
            ..VaultDepositPreclaimFacts::default()
        },
        || Ter::TES_SUCCESS,
        || {
            valid_domain_called.set(true);
            Ter::TES_SUCCESS
        },
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
    );

    assert_eq!(result, Ter::TEC_NO_AUTH);
    assert!(!valid_domain_called.get());
}

#[test]
fn vault_deposit_preclaim_suppresses_tecexpired_from_domain_check() {
    let require_auth_called = Cell::new(false);

    let result = run_vault_deposit_preclaim(
        VaultDepositPreclaimFacts {
            vault_exists: true,
            deposited_asset_matches_vault_asset: true,
            issuance_exists: true,
            vault_is_private: true,
            domain_id_present: true,
            account_holds_sufficient_assets: true,
            ..VaultDepositPreclaimFacts::default()
        },
        || Ter::TES_SUCCESS,
        || Ter::TEC_EXPIRED,
        || {
            require_auth_called.set(true);
            Ter::TES_SUCCESS
        },
        || Ter::TES_SUCCESS,
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert!(require_auth_called.get());
}

#[test]
fn vault_deposit_preclaim_returns_domain_or_auth_or_balance_failures() {
    let domain_failure = run_vault_deposit_preclaim(
        VaultDepositPreclaimFacts {
            vault_exists: true,
            deposited_asset_matches_vault_asset: true,
            issuance_exists: true,
            vault_is_private: true,
            domain_id_present: true,
            ..VaultDepositPreclaimFacts::default()
        },
        || Ter::TES_SUCCESS,
        || Ter::TEC_OBJECT_NOT_FOUND,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
    );
    let auth_failure = run_vault_deposit_preclaim(
        VaultDepositPreclaimFacts {
            vault_exists: true,
            deposited_asset_matches_vault_asset: true,
            issuance_exists: true,
            account_holds_sufficient_assets: true,
            ..VaultDepositPreclaimFacts::default()
        },
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TER_NO_ACCOUNT,
        || Ter::TES_SUCCESS,
    );
    let insufficient_funds = run_vault_deposit_preclaim(
        VaultDepositPreclaimFacts {
            vault_exists: true,
            deposited_asset_matches_vault_asset: true,
            issuance_exists: true,
            ..VaultDepositPreclaimFacts::default()
        },
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
    );

    assert_eq!(domain_failure, Ter::TEC_OBJECT_NOT_FOUND);
    assert_eq!(auth_failure, Ter::TER_NO_ACCOUNT);
    assert_eq!(insufficient_funds, Ter::TEC_INSUFFICIENT_FUNDS);
}

#[test]
fn vault_deposit_preclaim_runs_helpers_in_current_on_success() {
    let seen = Rc::new(std::cell::RefCell::new(Vec::new()));

    let result = run_vault_deposit_preclaim(
        VaultDepositPreclaimFacts {
            vault_exists: true,
            deposited_asset_matches_vault_asset: true,
            issuance_exists: true,
            vault_is_private: true,
            domain_id_present: true,
            account_holds_sufficient_assets: true,
            ..VaultDepositPreclaimFacts::default()
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
                seen.borrow_mut().push("domain");
                Ter::TEC_EXPIRED
            }
        },
        {
            let seen = Rc::clone(&seen);
            move || {
                seen.borrow_mut().push("auth");
                Ter::TES_SUCCESS
            }
        },
        || Ter::TES_SUCCESS,
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(seen.borrow().as_slice(), ["transfer", "domain", "auth"]);
}
