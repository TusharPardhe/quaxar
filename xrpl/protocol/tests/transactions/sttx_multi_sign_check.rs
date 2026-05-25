//! Integration tests that pin the narrowed Rust `STTx.cpp::checkMultiSign(...)`
//! and `checkBatchMultiSign(...)` shells to the current C++ behavior.

use protocol::{
    EMPTY_SIGNING_PUB_KEY_ERROR, INVALID_MULTISIGNER_ERROR, StTxMultiSignObject, StTxMultiSigner,
    run_sttx_check_batch_multi_sign, run_sttx_check_multi_sign,
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
fn protocol_sttx_check_multi_sign_rejects_primary_self_multisigning() {
    let result = run_sttx_check_multi_sign(
        &TestSignatureObject {
            signers_present: true,
            txn_signature_present: false,
            signers: vec![TestSigner {
                account_id: "alice",
            }],
        },
        true,
        &"alice",
        &"seed",
        |seed, account_id| format!("{seed}-{account_id}"),
        |_, _| Ok(()),
        |account_id| (*account_id).to_owned(),
    );

    assert_eq!(result, Err(INVALID_MULTISIGNER_ERROR.to_owned()));
}

#[test]
fn protocol_sttx_check_multi_sign_skips_self_multisigning_rejection_for_non_primary_objects() {
    let mut seen = Vec::new();

    let result = run_sttx_check_multi_sign(
        &TestSignatureObject {
            signers_present: true,
            txn_signature_present: false,
            signers: vec![TestSigner {
                account_id: "alice",
            }],
        },
        false,
        &"alice",
        &"seed",
        |seed, account_id| format!("{seed}-{account_id}"),
        |_, message| {
            seen.push(message);
            Ok(())
        },
        |account_id| (*account_id).to_owned(),
    );

    assert_eq!(result, Ok(()));
    assert_eq!(seen, vec!["seed-alice".to_owned()]);
}

#[test]
fn protocol_sttx_check_batch_multi_sign_passes_built_messages() {
    let mut seen = Vec::new();

    let result = run_sttx_check_batch_multi_sign(
        &TestSignatureObject {
            signers_present: true,
            txn_signature_present: false,
            signers: vec![TestSigner {
                account_id: "alice",
            }],
        },
        &"batch",
        |seed, account_id| format!("{seed}-{account_id}"),
        |_, message| {
            seen.push(message);
            Ok(())
        },
        |account_id| (*account_id).to_owned(),
    );

    assert_eq!(result, Ok(()));
    assert_eq!(seen, vec!["batch-alice".to_owned()]);
}

#[test]
fn protocol_sttx_check_multi_sign_returns_lower_helper_failure_unchanged() {
    let result = run_sttx_check_multi_sign(
        &TestSignatureObject {
            signers_present: false,
            txn_signature_present: false,
            signers: vec![],
        },
        true,
        &"alice",
        &"seed",
        |seed, account_id| format!("{seed}-{account_id}"),
        |_, _| Ok(()),
        |account_id| (*account_id).to_owned(),
    );

    assert_eq!(result, Err(EMPTY_SIGNING_PUB_KEY_ERROR.to_owned()));
}
