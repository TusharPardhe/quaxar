//! Integration tests that pin the narrowed Rust `STTx.cpp::multiSignHelper`
//! seam to the current C++ behavior.

use protocol::{
    CANNOT_BOTH_SINGLE_AND_MULTI_SIGN_ERROR, DUPLICATE_SIGNERS_ERROR, EMPTY_SIGNING_PUB_KEY_ERROR,
    INVALID_MULTISIGNER_ERROR, INVALID_SIGNERS_ARRAY_SIZE_ERROR, MAX_MULTI_SIGNERS,
    StTxMultiSignObject, StTxMultiSigner, UNSORTED_SIGNERS_ERROR, check_sttx_multi_sign,
};

#[derive(Clone, Copy)]
struct TestSigner {
    account_id: &'static str,
}

impl StTxMultiSigner<&'static str> for TestSigner {
    fn account_id(&self) -> &'static str {
        self.account_id
    }
}

#[derive(Clone)]
struct TestSignatureObject {
    signers_present: bool,
    txn_signature_present: bool,
    signers: Vec<TestSigner>,
}

impl StTxMultiSignObject<&'static str, TestSigner> for TestSignatureObject {
    type Signers = Vec<TestSigner>;

    fn signers_present(&self) -> bool {
        self.signers_present
    }

    fn txn_signature_present(&self) -> bool {
        self.txn_signature_present
    }

    fn signers(&self) -> Self::Signers {
        self.signers.clone()
    }
}

#[test]
fn protocol_sttx_multi_sign_requires_signers_for_empty_signing_pub_key() {
    let result = check_sttx_multi_sign(
        &TestSignatureObject {
            signers_present: false,
            txn_signature_present: false,
            signers: vec![],
        },
        None,
        |_| Ok(()),
        |account_id| format!("r{account_id}"),
    );

    assert_eq!(result, Err(EMPTY_SIGNING_PUB_KEY_ERROR.to_owned()));
}

#[test]
fn protocol_sttx_multi_sign_rejects_single_and_multi_sign_mix() {
    let result = check_sttx_multi_sign(
        &TestSignatureObject {
            signers_present: true,
            txn_signature_present: true,
            signers: vec![TestSigner {
                account_id: "alice",
            }],
        },
        None,
        |_| Ok(()),
        |account_id| format!("r{account_id}"),
    );

    assert_eq!(
        result,
        Err(CANNOT_BOTH_SINGLE_AND_MULTI_SIGN_ERROR.to_owned())
    );
}

#[test]
fn protocol_sttx_multi_sign_rejects_invalid_signers_array_size() {
    let too_many_signers = (0..=MAX_MULTI_SIGNERS)
        .map(|_| TestSigner {
            account_id: "alice",
        })
        .collect::<Vec<_>>();

    let empty_result = check_sttx_multi_sign(
        &TestSignatureObject {
            signers_present: true,
            txn_signature_present: false,
            signers: vec![],
        },
        None,
        |_| Ok(()),
        |account_id| format!("r{account_id}"),
    );

    let too_many_result = check_sttx_multi_sign(
        &TestSignatureObject {
            signers_present: true,
            txn_signature_present: false,
            signers: too_many_signers,
        },
        None,
        |_| Ok(()),
        |account_id| format!("r{account_id}"),
    );

    assert_eq!(
        empty_result,
        Err(INVALID_SIGNERS_ARRAY_SIZE_ERROR.to_owned())
    );
    assert_eq!(
        too_many_result,
        Err(INVALID_SIGNERS_ARRAY_SIZE_ERROR.to_owned())
    );
}

#[test]
fn protocol_sttx_multi_sign_rejects_invalid_multisigner() {
    let result = check_sttx_multi_sign(
        &TestSignatureObject {
            signers_present: true,
            txn_signature_present: false,
            signers: vec![TestSigner {
                account_id: "alice",
            }],
        },
        Some(&"alice"),
        |_| Ok(()),
        |account_id| format!("r{account_id}"),
    );

    assert_eq!(result, Err(INVALID_MULTISIGNER_ERROR.to_owned()));
}

#[test]
fn protocol_sttx_multi_sign_rejects_duplicate_signers() {
    let result = check_sttx_multi_sign(
        &TestSignatureObject {
            signers_present: true,
            txn_signature_present: false,
            signers: vec![
                TestSigner {
                    account_id: "alice",
                },
                TestSigner {
                    account_id: "alice",
                },
            ],
        },
        None,
        |_| Ok(()),
        |account_id| format!("r{account_id}"),
    );

    assert_eq!(result, Err(DUPLICATE_SIGNERS_ERROR.to_owned()));
}

#[test]
fn protocol_sttx_multi_sign_rejects_unsorted_signers() {
    let result = check_sttx_multi_sign(
        &TestSignatureObject {
            signers_present: true,
            txn_signature_present: false,
            signers: vec![
                TestSigner { account_id: "bob" },
                TestSigner {
                    account_id: "alice",
                },
            ],
        },
        None,
        |_| Ok(()),
        |account_id| format!("r{account_id}"),
    );

    assert_eq!(result, Err(UNSORTED_SIGNERS_ERROR.to_owned()));
}

#[test]
fn protocol_sttx_multi_sign_formats_invalid_signature() {
    let result = check_sttx_multi_sign(
        &TestSignatureObject {
            signers_present: true,
            txn_signature_present: false,
            signers: vec![TestSigner {
                account_id: "alice",
            }],
        },
        None,
        |_| Err(String::new()),
        |account_id| format!("r{account_id}"),
    );

    assert_eq!(
        result,
        Err("Invalid signature on account ralice.".to_owned())
    );
}

#[test]
fn protocol_sttx_multi_sign_appends_verifier_error_text() {
    let result = check_sttx_multi_sign(
        &TestSignatureObject {
            signers_present: true,
            txn_signature_present: false,
            signers: vec![TestSigner {
                account_id: "alice",
            }],
        },
        None,
        |_| Err(" detail".to_owned()),
        |account_id| format!("r{account_id}"),
    );

    assert_eq!(
        result,
        Err("Invalid signature on account ralice detail.".to_owned())
    );
}

#[test]
fn protocol_sttx_multi_sign_accepts_sorted_valid_signers() {
    let result = check_sttx_multi_sign(
        &TestSignatureObject {
            signers_present: true,
            txn_signature_present: false,
            signers: vec![
                TestSigner {
                    account_id: "alice",
                },
                TestSigner { account_id: "bob" },
            ],
        },
        Some(&"carol"),
        |_| Ok(()),
        |account_id| format!("r{account_id}"),
    );

    assert_eq!(result, Ok(()));
}
