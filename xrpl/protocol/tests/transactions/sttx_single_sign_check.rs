//! Integration tests that pin the narrowed Rust `STTx.cpp::checkSingleSign(...)`
//! and `checkBatchSingleSign(...)` shells to the current C++ behavior.

use protocol::{
    CANNOT_BOTH_SINGLE_AND_MULTI_SIGN_ERROR, StTxSingleSignObject,
    run_sttx_check_batch_single_sign, run_sttx_check_single_sign,
};

#[derive(Clone, Copy)]
struct TestSignatureObject {
    signers_present: bool,
}

impl StTxSingleSignObject for TestSignatureObject {
    fn signers_present(&self) -> bool {
        self.signers_present
    }
}

#[test]
fn protocol_sttx_check_single_sign_passes_signing_data_to_verifier() {
    let mut seen = None;

    let result = run_sttx_check_single_sign(
        &TestSignatureObject {
            signers_present: false,
        },
        &"txn-signing-data",
        |_, signing_data| {
            seen = Some(*signing_data);
            true
        },
    );

    assert_eq!(result, Ok(()));
    assert_eq!(seen, Some("txn-signing-data"));
}

#[test]
fn protocol_sttx_check_batch_single_sign_passes_batch_message_to_verifier() {
    let mut seen = None;

    let result = run_sttx_check_batch_single_sign(
        &TestSignatureObject {
            signers_present: false,
        },
        &"batch-message",
        |_, batch_message| {
            seen = Some(*batch_message);
            true
        },
    );

    assert_eq!(result, Ok(()));
    assert_eq!(seen, Some("batch-message"));
}

#[test]
fn protocol_sttx_check_single_sign_returns_lower_helper_failure_unchanged() {
    let result = run_sttx_check_single_sign(
        &TestSignatureObject {
            signers_present: true,
        },
        &"txn-signing-data",
        |_, _| true,
    );

    assert_eq!(
        result,
        Err(CANNOT_BOTH_SINGLE_AND_MULTI_SIGN_ERROR.to_owned())
    );
}
