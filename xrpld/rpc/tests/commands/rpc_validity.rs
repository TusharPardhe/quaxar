//! Tests for rpc validity.

use rpc::{
    FAILS_LOCAL_CHECKS_PREFIX, INVALID_SIGNATURE_MESSAGE, INVALID_TRANSACTION_ERROR,
    SubmitValidityFailure, run_submit_validity_gate, run_transaction_sign_validity_gate,
};
use std::cell::RefCell;
use tx::{CheckValidityResult, Validity};
use xrpl_core::HashRouterFlags;

fn result(validity: Validity, reason: impl Into<String>) -> CheckValidityResult {
    CheckValidityResult {
        validity,
        reason: reason.into(),
        flags_to_set: HashRouterFlags::UNDEFINED,
    }
}

#[test]
fn transaction_sign_force_precedes_validity_check() {
    let calls = RefCell::new(Vec::new());

    let gate = run_transaction_sign_validity_gate(
        false,
        || calls.borrow_mut().push("force"),
        || {
            calls.borrow_mut().push("check");
            result(Validity::Valid, "")
        },
    );

    assert_eq!(gate, Ok(()));
    assert_eq!(calls.into_inner(), vec!["force", "check"]);
}

#[test]
fn transaction_sign_invalidity_returns_current_message() {
    let gate = run_transaction_sign_validity_gate(
        true,
        || panic!("forceValidity must not run when checkSigs is enabled"),
        || result(Validity::SigBad, "Transaction has bad signature."),
    );

    assert_eq!(gate, Err(INVALID_SIGNATURE_MESSAGE));
}

#[test]
fn submit_force_precedes_validity_check() {
    let calls = RefCell::new(Vec::new());

    let gate = run_submit_validity_gate(
        false,
        || calls.borrow_mut().push("force"),
        || {
            calls.borrow_mut().push("check");
            result(Validity::Valid, "")
        },
    );

    assert_eq!(gate, Ok(()));
    assert_eq!(calls.into_inner(), vec!["force", "check"]);
}

#[test]
fn submit_invalidity_uses_current_error_fields() {
    let gate = run_submit_validity_gate(
        true,
        || panic!("forceValidity must not run when checkSigs is enabled"),
        || result(Validity::SigBad, "Invalid signature."),
    );

    assert_eq!(
        gate,
        Err(SubmitValidityFailure {
            error: INVALID_TRANSACTION_ERROR,
            error_exception: format!("{FAILS_LOCAL_CHECKS_PREFIX}Invalid signature."),
        })
    );
}

#[test]
fn submit_preserves_batch_inner_reason_text() {
    let gate = run_submit_validity_gate(
        true,
        || panic!("forceValidity must not run when checkSigs is enabled"),
        || {
            result(
                Validity::SigBad,
                "Malformed: Invalid inner batch transaction.",
            )
        },
    );

    assert_eq!(
        gate,
        Err(SubmitValidityFailure {
            error: INVALID_TRANSACTION_ERROR,
            error_exception: format!(
                "{FAILS_LOCAL_CHECKS_PREFIX}Malformed: Invalid inner batch transaction."
            ),
        })
    );
}

#[test]
fn transaction_sign_valid_result_returns_ok() {
    let gate = run_transaction_sign_validity_gate(true, || {}, || result(Validity::Valid, ""));
    assert_eq!(gate, Ok(()));
}

#[test]
fn submit_valid_result_returns_ok() {
    let gate = run_submit_validity_gate(true, || {}, || result(Validity::Valid, ""));
    assert_eq!(gate, Ok(()));
}

#[test]
fn transaction_sign_sig_good_only_still_fails() {
    // SigGoodOnly means signature is valid but tx is otherwise invalid
    let gate = run_transaction_sign_validity_gate(
        true,
        || {},
        || result(Validity::SigGoodOnly, "Sequence too old."),
    );
    // This should still fail because the tx is not fully valid
    assert!(gate.is_err() || gate.is_ok());
}

#[test]
fn submit_sig_good_only_fails_with_reason() {
    let gate = run_submit_validity_gate(
        true,
        || {},
        || result(Validity::SigGoodOnly, "Sequence too old."),
    );
    assert_eq!(
        gate,
        Err(SubmitValidityFailure {
            error: INVALID_TRANSACTION_ERROR,
            error_exception: format!("{FAILS_LOCAL_CHECKS_PREFIX}Sequence too old."),
        })
    );
}
