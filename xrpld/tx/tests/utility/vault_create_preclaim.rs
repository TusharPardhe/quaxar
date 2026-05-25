//! Integration tests that pin the narrowed Rust `VaultCreate.cpp::preclaim(...)`
//! wrapper to the current C++ behavior.

use std::{cell::Cell, rc::Rc};

use protocol::{Ter, trans_token};
use tx::{VaultCreatePreclaimFacts, run_vault_create_preclaim};

#[test]
fn vault_create_preclaim_returns_can_add_holding_failure_first() {
    let pseudo_checked = Cell::new(false);

    let result = run_vault_create_preclaim(
        VaultCreatePreclaimFacts::default(),
        || Ter::TER_NO_RIPPLE,
        || {
            pseudo_checked.set(true);
            false
        },
        || false,
        || true,
        || false,
    );

    assert_eq!(result, Ter::TER_NO_RIPPLE);
    assert_eq!(trans_token(result), "terNO_RIPPLE");
    assert!(!pseudo_checked.get());
}

#[test]
fn vault_create_preclaim_skips_pseudo_account_check_for_native_assets() {
    let pseudo_checked = Cell::new(false);

    let result = run_vault_create_preclaim(
        VaultCreatePreclaimFacts {
            asset_is_native: true,
            ..VaultCreatePreclaimFacts::default()
        },
        || Ter::TES_SUCCESS,
        || {
            pseudo_checked.set(true);
            true
        },
        || false,
        || true,
        || false,
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert!(!pseudo_checked.get());
}

#[test]
fn vault_create_preclaim_rejects_pseudo_account_issuer_for_non_native_assets() {
    let result = run_vault_create_preclaim(
        VaultCreatePreclaimFacts::default(),
        || Ter::TES_SUCCESS,
        || true,
        || false,
        || true,
        || false,
    );

    assert_eq!(result, Ter::TEC_WRONG_ASSET);
    assert_eq!(trans_token(result), "tecWRONG_ASSET");
}

#[test]
fn vault_create_preclaim_maps_frozen_issue_to_tecfrozen() {
    let result = run_vault_create_preclaim(
        VaultCreatePreclaimFacts {
            asset_is_issue: true,
            ..VaultCreatePreclaimFacts::default()
        },
        || Ter::TES_SUCCESS,
        || false,
        || true,
        || true,
        || false,
    );

    assert_eq!(result, Ter::TEC_FROZEN);
}

#[test]
fn vault_create_preclaim_maps_frozen_non_issue_to_teclocked() {
    let result = run_vault_create_preclaim(
        VaultCreatePreclaimFacts::default(),
        || Ter::TES_SUCCESS,
        || false,
        || true,
        || true,
        || false,
    );

    assert_eq!(result, Ter::TEC_LOCKED);
}

#[test]
fn vault_create_preclaim_requires_existing_domain() {
    let result = run_vault_create_preclaim(
        VaultCreatePreclaimFacts {
            domain_id_present: true,
            ..VaultCreatePreclaimFacts::default()
        },
        || Ter::TES_SUCCESS,
        || false,
        || false,
        || false,
        || false,
    );

    assert_eq!(result, Ter::TEC_OBJECT_NOT_FOUND);
    assert_eq!(trans_token(result), "tecOBJECT_NOT_FOUND");
}

#[test]
fn vault_create_preclaim_returns_address_collision() {
    let result = run_vault_create_preclaim(
        VaultCreatePreclaimFacts::default(),
        || Ter::TES_SUCCESS,
        || false,
        || false,
        || true,
        || true,
    );

    assert_eq!(result, Ter::TER_ADDRESS_COLLISION);
    assert_eq!(trans_token(result), "terADDRESS_COLLISION");
}

#[test]
fn vault_create_preclaim_runs_helpers_in_current_on_success() {
    let seen = Rc::new(std::cell::RefCell::new(Vec::new()));

    let result = run_vault_create_preclaim(
        VaultCreatePreclaimFacts {
            domain_id_present: true,
            ..VaultCreatePreclaimFacts::default()
        },
        {
            let seen = Rc::clone(&seen);
            move || {
                seen.borrow_mut().push("holding");
                Ter::TES_SUCCESS
            }
        },
        {
            let seen = Rc::clone(&seen);
            move || {
                seen.borrow_mut().push("pseudo");
                false
            }
        },
        {
            let seen = Rc::clone(&seen);
            move || {
                seen.borrow_mut().push("frozen");
                false
            }
        },
        {
            let seen = Rc::clone(&seen);
            move || {
                seen.borrow_mut().push("domain");
                true
            }
        },
        {
            let seen = Rc::clone(&seen);
            move || {
                seen.borrow_mut().push("address");
                false
            }
        },
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        seen.borrow().as_slice(),
        ["holding", "pseudo", "frozen", "domain", "address"]
    );
}
