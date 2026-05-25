//! Integration tests that pin the narrowed Rust `STTx.cpp::singleSignHelper`
//! seam to the current C++ behavior.

use protocol::{
    CANNOT_BOTH_SINGLE_AND_MULTI_SIGN_ERROR, INVALID_SIGNATURE_ERROR, StTxSingleSignObject,
    check_sttx_single_sign,
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
fn protocol_sttx_single_sign_rejects_single_and_multi_sign_mix() {
    let result = check_sttx_single_sign(
        &TestSignatureObject {
            signers_present: true,
        },
        |_| true,
    );

    assert_eq!(
        result,
        Err(CANNOT_BOTH_SINGLE_AND_MULTI_SIGN_ERROR.to_owned())
    );
}

#[test]
fn protocol_sttx_single_sign_accepts_valid_signature() {
    let result = check_sttx_single_sign(
        &TestSignatureObject {
            signers_present: false,
        },
        |_| true,
    );

    assert_eq!(result, Ok(()));
}

#[test]
fn protocol_sttx_single_sign_returns_invalid_signature_on_verify_failure() {
    let result = check_sttx_single_sign(
        &TestSignatureObject {
            signers_present: false,
        },
        |_| false,
    );

    assert_eq!(result, Err(INVALID_SIGNATURE_ERROR.to_owned()));
}

#[test]
fn protocol_sttx_single_sign_returns_invalid_signature_on_verify_panic() {
    let result = check_sttx_single_sign(
        &TestSignatureObject {
            signers_present: false,
        },
        |_| panic!("simulated verifier panic"),
    );

    assert_eq!(result, Err(INVALID_SIGNATURE_ERROR.to_owned()));
}
