//! Integration tests that pin the shared transactor preflight key helpers to
//! the current C++ `Transactor.cpp` behavior.

use protocol::{Ter, trans_token};
use tx::{
    ApplyFlags, TransactorPreflightSigningKeyFacts, TransactorPreflightSimulateKeysFacts,
    TransactorPreflightSimulateSignerFacts, run_preflight_check_signing_key,
    run_preflight_check_simulate_keys,
};

#[test]
fn preflight_check_signing_key_rejects_unknown_nonempty_pubkey() {
    let result = run_preflight_check_signing_key(TransactorPreflightSigningKeyFacts {
        signing_pub_key_is_empty: false,
        signing_pub_key_type_known: false,
    });

    assert_eq!(result, Ter::TEM_BAD_SIGNATURE);
    assert_eq!(trans_token(result), "temBAD_SIGNATURE");
}

#[test]
fn preflight_check_signing_key_accepts_empty_or_known_pubkey() {
    let empty = run_preflight_check_signing_key(TransactorPreflightSigningKeyFacts {
        signing_pub_key_is_empty: true,
        signing_pub_key_type_known: false,
    });
    let known = run_preflight_check_signing_key(TransactorPreflightSigningKeyFacts {
        signing_pub_key_is_empty: false,
        signing_pub_key_type_known: true,
    });

    assert_eq!(empty, Ter::TES_SUCCESS);
    assert_eq!(known, Ter::TES_SUCCESS);
}

#[test]
fn preflight_check_simulate_keys_skips_non_dry_run() {
    let result = run_preflight_check_simulate_keys(
        ApplyFlags::NONE,
        &TransactorPreflightSimulateKeysFacts {
            txn_signature_present: true,
            txn_signature_is_empty: false,
            ..TransactorPreflightSimulateKeysFacts::default()
        },
    );

    assert_eq!(result, None);
}

#[test]
fn preflight_check_simulate_keys_rejects_signature_material_during_simulation() {
    let top_level = run_preflight_check_simulate_keys(
        ApplyFlags::DRY_RUN,
        &TransactorPreflightSimulateKeysFacts {
            txn_signature_present: true,
            txn_signature_is_empty: false,
            ..TransactorPreflightSimulateKeysFacts::default()
        },
    );
    let signer = run_preflight_check_simulate_keys(
        ApplyFlags::DRY_RUN,
        &TransactorPreflightSimulateKeysFacts {
            signers_present: true,
            signer_facts: vec![TransactorPreflightSimulateSignerFacts {
                txn_signature_present: true,
                txn_signature_is_empty: false,
            }],
            ..TransactorPreflightSimulateKeysFacts::default()
        },
    );
    let mixed = run_preflight_check_simulate_keys(
        ApplyFlags::DRY_RUN,
        &TransactorPreflightSimulateKeysFacts {
            signers_present: true,
            signing_pub_key_is_empty: false,
            ..TransactorPreflightSimulateKeysFacts::default()
        },
    );

    assert_eq!(top_level, Some(Ter::TEM_INVALID));
    assert_eq!(signer, Some(Ter::TEM_INVALID));
    assert_eq!(mixed, Some(Ter::TEM_INVALID));
}

#[test]
fn preflight_check_simulate_keys_accepts_clean_simulation_shapes() {
    let simple = run_preflight_check_simulate_keys(
        ApplyFlags::DRY_RUN,
        &TransactorPreflightSimulateKeysFacts::default(),
    );
    let multisign = run_preflight_check_simulate_keys(
        ApplyFlags::DRY_RUN,
        &TransactorPreflightSimulateKeysFacts {
            signers_present: true,
            signer_facts: vec![TransactorPreflightSimulateSignerFacts {
                txn_signature_present: true,
                txn_signature_is_empty: true,
            }],
            ..TransactorPreflightSimulateKeysFacts::default()
        },
    );

    assert_eq!(simple, Some(Ter::TES_SUCCESS));
    assert_eq!(multisign, Some(Ter::TES_SUCCESS));
}
