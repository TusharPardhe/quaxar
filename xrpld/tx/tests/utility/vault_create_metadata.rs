//! Integration tests that pin the narrowed Rust `VaultCreate.cpp` metadata
//! helpers to the current C++ behavior.

use std::cell::Cell;

use tx::{
    VAULT_CREATE_FLAGS_MASK, VAULT_PRIVATE_FLAG, VAULT_SHARE_NON_TRANSFERABLE_FLAG,
    get_vault_create_flags_mask, run_vault_create_check_extra_features,
};

#[test]
fn vault_create_check_extra_features_requires_mptokens_before_any_domain_logic() {
    let permissioned_domains_checked = Cell::new(false);

    let result = run_vault_create_check_extra_features(false, true, || {
        permissioned_domains_checked.set(true);
        true
    });

    assert!(!result);
    assert!(!permissioned_domains_checked.get());
}

#[test]
fn vault_create_check_extra_features_skips_permissioned_domain_gate_without_domain() {
    let permissioned_domains_checked = Cell::new(false);

    let result = run_vault_create_check_extra_features(true, false, || {
        permissioned_domains_checked.set(true);
        false
    });

    assert!(result);
    assert!(!permissioned_domains_checked.get());
}

#[test]
fn vault_create_check_extra_features_rejects_domain_without_permissioned_domains() {
    assert!(!run_vault_create_check_extra_features(true, true, || false));
    assert!(run_vault_create_check_extra_features(true, true, || true));
}

#[test]
fn vault_create_flags_mask_txflags() {
    assert_eq!(VAULT_PRIVATE_FLAG, 0x0001_0000);
    assert_eq!(VAULT_SHARE_NON_TRANSFERABLE_FLAG, 0x0002_0000);
    assert_eq!(VAULT_CREATE_FLAGS_MASK, 0x3ffc_ffff);
    assert_eq!(get_vault_create_flags_mask(), 0x3ffc_ffff);
}
