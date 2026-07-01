//! Integration tests that pin the narrowed Rust `VaultDelete.cpp` front
//! preflight and preclaim shells to the current C++ behavior.

use protocol::{Ter, trans_token};
use tx::{
    VaultDeletePreclaimFacts, VaultDeletePreflightFacts, run_vault_delete_preclaim,
    run_vault_delete_preflight,
};

fn base_preclaim() -> VaultDeletePreclaimFacts {
    VaultDeletePreclaimFacts {
        vault_exists: true,
        submitter_is_owner: true,
        assets_available_is_zero: true,
        assets_total_is_zero: true,
        issuance_exists: true,
        issuance_issuer_matches_pseudo: true,
        outstanding_amount_is_zero: true,
    }
}

#[test]
fn vault_delete_preflight_rejects_zero_vault_id() {
    let result = run_vault_delete_preflight(VaultDeletePreflightFacts {
        vault_id_is_zero: true,
        has_memo_data: false,
        lending_protocol_v1_1_enabled: false,
        memo_data_length_valid: true,
    });

    assert_eq!(result, Ter::TEM_MALFORMED);
}

#[test]
fn vault_delete_preclaim_rejects_missing_vault() {
    let result = run_vault_delete_preclaim(VaultDeletePreclaimFacts::default());

    assert_eq!(result, Ter::TEC_NO_ENTRY);
}

#[test]
fn vault_delete_preclaim_rejects_non_owner_submitter() {
    let result = run_vault_delete_preclaim(VaultDeletePreclaimFacts {
        vault_exists: true,
        ..VaultDeletePreclaimFacts::default()
    });

    assert_eq!(result, Ter::TEC_NO_PERMISSION);
}

#[test]
fn vault_delete_preclaim_rejects_nonzero_assets_available() {
    let result = run_vault_delete_preclaim(VaultDeletePreclaimFacts {
        assets_available_is_zero: false,
        ..base_preclaim()
    });

    assert_eq!(result, Ter::TEC_HAS_OBLIGATIONS);
    assert_eq!(trans_token(result), "tecHAS_OBLIGATIONS");
}

#[test]
fn vault_delete_preclaim_rejects_nonzero_assets_total() {
    let result = run_vault_delete_preclaim(VaultDeletePreclaimFacts {
        assets_total_is_zero: false,
        ..base_preclaim()
    });

    assert_eq!(result, Ter::TEC_HAS_OBLIGATIONS);
}

#[test]
fn vault_delete_preclaim_rejects_missing_share_issuance() {
    let result = run_vault_delete_preclaim(VaultDeletePreclaimFacts {
        issuance_exists: false,
        ..base_preclaim()
    });

    assert_eq!(result, Ter::TEC_OBJECT_NOT_FOUND);
}

#[test]
fn vault_delete_preclaim_rejects_mismatched_issuance_owner() {
    let result = run_vault_delete_preclaim(VaultDeletePreclaimFacts {
        issuance_issuer_matches_pseudo: false,
        ..base_preclaim()
    });

    assert_eq!(result, Ter::TEC_NO_PERMISSION);
}

#[test]
fn vault_delete_preclaim_rejects_nonzero_outstanding_shares() {
    let result = run_vault_delete_preclaim(VaultDeletePreclaimFacts {
        outstanding_amount_is_zero: false,
        ..base_preclaim()
    });

    assert_eq!(result, Ter::TEC_HAS_OBLIGATIONS);
}

#[test]
fn vault_delete_preclaim_accepts_empty_owner_vault() {
    let result = run_vault_delete_preclaim(base_preclaim());

    assert_eq!(result, Ter::TES_SUCCESS);
}
