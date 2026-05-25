//! Integration tests that pin the narrowed Rust `VaultCreate.cpp::preflight(...)`
//! wrapper to the current C++ behavior.

use protocol::{Ter, trans_token};
use tx::utility::vault_create_preflight::MAX_MPTOKEN_METADATA_LENGTH;
use tx::{
    MAX_DATA_PAYLOAD_LENGTH, VAULT_MAXIMUM_IOU_SCALE, VAULT_STRATEGY_FIRST_COME_FIRST_SERVE,
    VaultCreatePreflightFacts, run_vault_create_preflight,
};

#[test]
fn vault_create_preflight_rejects_invalid_data_lengths() {
    let empty = run_vault_create_preflight(VaultCreatePreflightFacts {
        data_len: Some(0),
        ..VaultCreatePreflightFacts::default()
    });
    let oversized = run_vault_create_preflight(VaultCreatePreflightFacts {
        data_len: Some(MAX_DATA_PAYLOAD_LENGTH + 1),
        ..VaultCreatePreflightFacts::default()
    });

    assert_eq!(empty, Ter::TEM_MALFORMED);
    assert_eq!(oversized, Ter::TEM_MALFORMED);
    assert_eq!(trans_token(empty), "temMALFORMED");
}

#[test]
fn vault_create_preflight_rejects_non_fcfs_withdrawal_policy() {
    let result = run_vault_create_preflight(VaultCreatePreflightFacts {
        withdrawal_policy: Some(0),
        ..VaultCreatePreflightFacts::default()
    });

    assert_eq!(result, Ter::TEM_MALFORMED);
}

#[test]
fn vault_create_preflight_rejects_zero_or_public_domain() {
    let zero_domain = run_vault_create_preflight(VaultCreatePreflightFacts {
        domain_id_present: true,
        domain_id_is_zero: true,
        is_private: true,
        ..VaultCreatePreflightFacts::default()
    });
    let public_domain = run_vault_create_preflight(VaultCreatePreflightFacts {
        domain_id_present: true,
        domain_id_is_zero: false,
        is_private: false,
        ..VaultCreatePreflightFacts::default()
    });

    assert_eq!(zero_domain, Ter::TEM_MALFORMED);
    assert_eq!(public_domain, Ter::TEM_MALFORMED);
}

#[test]
fn vault_create_preflight_rejects_negative_assets_maximum() {
    let result = run_vault_create_preflight(VaultCreatePreflightFacts {
        assets_maximum_is_negative: true,
        ..VaultCreatePreflightFacts::default()
    });

    assert_eq!(result, Ter::TEM_MALFORMED);
}

#[test]
fn vault_create_preflight_rejects_invalid_metadata() {
    let empty = run_vault_create_preflight(VaultCreatePreflightFacts {
        mptoken_metadata_len: Some(0),
        ..VaultCreatePreflightFacts::default()
    });
    let oversized = run_vault_create_preflight(VaultCreatePreflightFacts {
        mptoken_metadata_len: Some(MAX_MPTOKEN_METADATA_LENGTH + 1),
        ..VaultCreatePreflightFacts::default()
    });

    assert_eq!(empty, Ter::TEM_MALFORMED);
    assert_eq!(oversized, Ter::TEM_MALFORMED);
}

#[test]
fn vault_create_preflight_rejects_scale_for_mpt_and_native_assets() {
    let mpt = run_vault_create_preflight(VaultCreatePreflightFacts {
        scale: Some(0),
        asset_is_mpt: true,
        ..VaultCreatePreflightFacts::default()
    });
    let native = run_vault_create_preflight(VaultCreatePreflightFacts {
        scale: Some(0),
        asset_is_native: true,
        ..VaultCreatePreflightFacts::default()
    });

    assert_eq!(mpt, Ter::TEM_MALFORMED);
    assert_eq!(native, Ter::TEM_MALFORMED);
}

#[test]
fn vault_create_preflight_rejects_scale_above_iou_cap() {
    let result = run_vault_create_preflight(VaultCreatePreflightFacts {
        scale: Some(VAULT_MAXIMUM_IOU_SCALE + 1),
        ..VaultCreatePreflightFacts::default()
    });

    assert_eq!(result, Ter::TEM_MALFORMED);
}

#[test]
fn vault_create_preflight_accepts_valid_private_iou_inputs() {
    let result = run_vault_create_preflight(VaultCreatePreflightFacts {
        data_len: Some(MAX_DATA_PAYLOAD_LENGTH),
        withdrawal_policy: Some(VAULT_STRATEGY_FIRST_COME_FIRST_SERVE),
        domain_id_present: true,
        domain_id_is_zero: false,
        is_private: true,
        mptoken_metadata_len: Some(MAX_MPTOKEN_METADATA_LENGTH),
        scale: Some(VAULT_MAXIMUM_IOU_SCALE),
        ..VaultCreatePreflightFacts::default()
    });

    assert_eq!(result, Ter::TES_SUCCESS);
}
