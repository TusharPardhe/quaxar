//! Integration tests that pin the narrowed Rust `VaultSet.cpp`
//! feature gate and preflight shell to the current C++ behavior.

use protocol::{Ter, trans_token};
use tx::{
    VAULT_SET_MAX_DATA_PAYLOAD_LENGTH, VaultSetPreflightFacts, run_vault_set_check_extra_features,
    run_vault_set_preflight,
};

#[test]
fn vault_set_check_extra_features_skips_domain_gate_without_domain() {
    let result = run_vault_set_check_extra_features(false, || false);

    assert!(result);
}

#[test]
fn vault_set_check_extra_features_requires_permissioned_domains_for_domain_updates() {
    assert!(run_vault_set_check_extra_features(true, || true));
    assert!(!run_vault_set_check_extra_features(true, || false));
}

#[test]
fn vault_set_preflight_rejects_zero_vault_id() {
    let result = run_vault_set_preflight(VaultSetPreflightFacts {
        vault_id_is_zero: true,
        domain_id_present: true,
        ..VaultSetPreflightFacts::default()
    });

    assert_eq!(result, Ter::TEM_MALFORMED);
    assert_eq!(trans_token(result), "temMALFORMED");
}

#[test]
fn vault_set_preflight_rejects_empty_and_oversized_data() {
    let empty = run_vault_set_preflight(VaultSetPreflightFacts {
        data_len: Some(0),
        data_present: true,
        ..VaultSetPreflightFacts::default()
    });
    let oversized = run_vault_set_preflight(VaultSetPreflightFacts {
        data_len: Some(VAULT_SET_MAX_DATA_PAYLOAD_LENGTH + 1),
        data_present: true,
        ..VaultSetPreflightFacts::default()
    });

    assert_eq!(empty, Ter::TEM_MALFORMED);
    assert_eq!(oversized, Ter::TEM_MALFORMED);
}

#[test]
fn vault_set_preflight_rejects_negative_assets_maximum() {
    let result = run_vault_set_preflight(VaultSetPreflightFacts {
        assets_maximum_is_negative: true,
        assets_maximum_present: true,
        ..VaultSetPreflightFacts::default()
    });

    assert_eq!(result, Ter::TEM_MALFORMED);
}

#[test]
fn vault_set_preflight_rejects_noop_updates() {
    let result = run_vault_set_preflight(VaultSetPreflightFacts::default());

    assert_eq!(result, Ter::TEM_MALFORMED);
}

#[test]
fn vault_set_preflight_accepts_any_single_valid_update() {
    let domain = run_vault_set_preflight(VaultSetPreflightFacts {
        domain_id_present: true,
        ..VaultSetPreflightFacts::default()
    });
    let assets_maximum = run_vault_set_preflight(VaultSetPreflightFacts {
        assets_maximum_present: true,
        ..VaultSetPreflightFacts::default()
    });
    let data = run_vault_set_preflight(VaultSetPreflightFacts {
        data_len: Some(VAULT_SET_MAX_DATA_PAYLOAD_LENGTH),
        data_present: true,
        ..VaultSetPreflightFacts::default()
    });

    assert_eq!(domain, Ter::TES_SUCCESS);
    assert_eq!(assets_maximum, Ter::TES_SUCCESS);
    assert_eq!(data, Ter::TES_SUCCESS);
}
