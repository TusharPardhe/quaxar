//! Integration tests that pin the narrowed Rust `VaultSet.cpp::preclaim(...)`
//! shell to the current C++ behavior.

use protocol::{Ter, trans_token};
use tx::{VaultSetPreclaimFacts, run_vault_set_preclaim};

#[test]
fn vault_set_preclaim_rejects_missing_vault() {
    let result = run_vault_set_preclaim(VaultSetPreclaimFacts::default());

    assert_eq!(result, Ter::TEC_NO_ENTRY);
    assert_eq!(trans_token(result), "tecNO_ENTRY");
}

#[test]
fn vault_set_preclaim_rejects_non_owner_submitter() {
    let result = run_vault_set_preclaim(VaultSetPreclaimFacts {
        vault_exists: true,
        ..VaultSetPreclaimFacts::default()
    });

    assert_eq!(result, Ter::TEC_NO_PERMISSION);
}

#[test]
fn vault_set_preclaim_rejects_missing_share_issuance() {
    let result = run_vault_set_preclaim(VaultSetPreclaimFacts {
        vault_exists: true,
        submitter_is_owner: true,
        ..VaultSetPreclaimFacts::default()
    });

    assert_eq!(result, Ter::TEF_INTERNAL);
    assert_eq!(trans_token(result), "tefINTERNAL");
}

#[test]
fn vault_set_preclaim_rejects_domain_updates_on_public_vaults() {
    let result = run_vault_set_preclaim(VaultSetPreclaimFacts {
        vault_exists: true,
        submitter_is_owner: true,
        issuance_exists: true,
        domain_id_present: true,
        ..VaultSetPreclaimFacts::default()
    });

    assert_eq!(result, Ter::TEC_NO_PERMISSION);
}

#[test]
fn vault_set_preclaim_rejects_nonexistent_nonzero_domain() {
    let result = run_vault_set_preclaim(VaultSetPreclaimFacts {
        vault_exists: true,
        submitter_is_owner: true,
        issuance_exists: true,
        domain_id_present: true,
        vault_is_private: true,
        issuance_requires_auth: true,
        ..VaultSetPreclaimFacts::default()
    });

    assert_eq!(result, Ter::TEC_OBJECT_NOT_FOUND);
    assert_eq!(trans_token(result), "tecOBJECT_NOT_FOUND");
}

#[test]
fn vault_set_preclaim_allows_zero_domain_reset_without_domain_lookup() {
    let result = run_vault_set_preclaim(VaultSetPreclaimFacts {
        vault_exists: true,
        submitter_is_owner: true,
        issuance_exists: true,
        domain_id_present: true,
        domain_id_is_zero: true,
        vault_is_private: true,
        issuance_requires_auth: true,
        ..VaultSetPreclaimFacts::default()
    });

    assert_eq!(result, Ter::TES_SUCCESS);
}

#[test]
fn vault_set_preclaim_rejects_non_private_issuance_for_domain_updates() {
    let result = run_vault_set_preclaim(VaultSetPreclaimFacts {
        vault_exists: true,
        submitter_is_owner: true,
        issuance_exists: true,
        domain_id_present: true,
        domain_id_is_zero: true,
        vault_is_private: true,
        issuance_requires_auth: false,
        ..VaultSetPreclaimFacts::default()
    });

    assert_eq!(result, Ter::TEF_INTERNAL);
}

#[test]
fn vault_set_preclaim_accepts_non_domain_updates_without_private_checks() {
    let result = run_vault_set_preclaim(VaultSetPreclaimFacts {
        vault_exists: true,
        submitter_is_owner: true,
        issuance_exists: true,
        ..VaultSetPreclaimFacts::default()
    });

    assert_eq!(result, Ter::TES_SUCCESS);
}
