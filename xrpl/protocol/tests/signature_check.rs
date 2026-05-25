//! Integration tests that pin the narrowed Rust `STTx::checkSign(...)`
//! helpers to the current C++ behavior.

use std::cell::Cell;

use protocol::{
    COUNTERPARTY_SIGNATURE_ERROR_PREFIX, INTERNAL_SIGNATURE_CHECK_FAILURE, SignatureCheckObject,
    StTxMultiSignObject, StTxMultiSigner, check_signature, check_signature_with_counterparty,
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
    signing_pub_key_is_empty: bool,
    signers_present: bool,
    txn_signature_present: bool,
    signers: Vec<TestSigner>,
    panic_on_pub_key_lookup: bool,
}

impl protocol::StTxSingleSignObject for TestSignatureObject {
    fn signers_present(&self) -> bool {
        self.signers_present
    }
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

impl SignatureCheckObject<&'static str, TestSigner> for TestSignatureObject {
    fn signing_pub_key_is_empty(&self) -> bool {
        assert!(
            !self.panic_on_pub_key_lookup,
            "simulated SigningPubKey access failure"
        );
        self.signing_pub_key_is_empty
    }
}

#[test]
fn protocol_signature_check_uses_multi_sign_for_empty_signing_pub_key() {
    let multi_called = Cell::new(false);
    let single_called = Cell::new(false);

    let result = check_signature(
        &TestSignatureObject {
            signing_pub_key_is_empty: true,
            signers_present: true,
            txn_signature_present: false,
            signers: vec![TestSigner {
                account_id: "alice",
            }],
            panic_on_pub_key_lookup: false,
        },
        Some(&"carol"),
        |_| {
            single_called.set(true);
            true
        },
        |_| {
            multi_called.set(true);
            Ok(())
        },
        |account_id| format!("r{account_id}"),
    );

    assert_eq!(result, Ok(()));
    assert!(multi_called.get());
    assert!(!single_called.get());
}

#[test]
fn protocol_signature_check_uses_single_sign_for_present_signing_pub_key() {
    let multi_called = Cell::new(false);
    let single_called = Cell::new(false);

    let result = check_signature(
        &TestSignatureObject {
            signing_pub_key_is_empty: false,
            signers_present: false,
            txn_signature_present: false,
            signers: vec![],
            panic_on_pub_key_lookup: false,
        },
        Some(&"carol"),
        |_| {
            single_called.set(true);
            true
        },
        |_| {
            multi_called.set(true);
            Ok(())
        },
        |account_id| format!("r{account_id}"),
    );

    assert_eq!(result, Ok(()));
    assert!(!multi_called.get());
    assert!(single_called.get());
}

#[test]
fn protocol_signature_check_returns_selected_failure_unchanged() {
    let result = check_signature(
        &TestSignatureObject {
            signing_pub_key_is_empty: false,
            signers_present: true,
            txn_signature_present: false,
            signers: vec![],
            panic_on_pub_key_lookup: false,
        },
        None,
        |_| true,
        |_| Ok(()),
        |account_id| format!("r{account_id}"),
    );

    assert_eq!(
        result,
        Err(protocol::CANNOT_BOTH_SINGLE_AND_MULTI_SIGN_ERROR.to_owned())
    );
}

#[test]
fn protocol_signature_check_maps_panics_to_internal_failure() {
    let result = check_signature(
        &TestSignatureObject {
            signing_pub_key_is_empty: false,
            signers_present: false,
            txn_signature_present: false,
            signers: vec![],
            panic_on_pub_key_lookup: true,
        },
        None,
        |_| true,
        |_| Ok(()),
        |account_id| format!("r{account_id}"),
    );

    assert_eq!(result, Err(INTERNAL_SIGNATURE_CHECK_FAILURE.to_owned()));
}

#[test]
fn protocol_signature_check_with_counterparty_prefixes_counterparty_errors() {
    let result = check_signature_with_counterparty(
        &TestSignatureObject {
            signing_pub_key_is_empty: false,
            signers_present: false,
            txn_signature_present: false,
            signers: vec![],
            panic_on_pub_key_lookup: false,
        },
        Some(&TestSignatureObject {
            signing_pub_key_is_empty: false,
            signers_present: false,
            txn_signature_present: false,
            signers: vec![],
            panic_on_pub_key_lookup: false,
        }),
        |signature_object| {
            check_signature(
                signature_object,
                None,
                |_| true,
                |_| Ok(()),
                |account_id| format!("r{account_id}"),
            )
        },
        |_| Err("Invalid signature.".to_owned()),
    );

    assert_eq!(
        result,
        Err(format!(
            "{COUNTERPARTY_SIGNATURE_ERROR_PREFIX}Invalid signature."
        ))
    );
}

#[test]
fn protocol_signature_check_with_counterparty_prefixes_counterparty_panics() {
    let result = check_signature_with_counterparty(
        &TestSignatureObject {
            signing_pub_key_is_empty: false,
            signers_present: false,
            txn_signature_present: false,
            signers: vec![],
            panic_on_pub_key_lookup: false,
        },
        Some(&TestSignatureObject {
            signing_pub_key_is_empty: false,
            signers_present: false,
            txn_signature_present: false,
            signers: vec![],
            panic_on_pub_key_lookup: false,
        }),
        |signature_object| {
            check_signature(
                signature_object,
                None,
                |_| true,
                |_| Ok(()),
                |account_id| format!("r{account_id}"),
            )
        },
        |_| panic!("simulated counterparty verification failure"),
    );

    assert_eq!(
        result,
        Err(format!(
            "{COUNTERPARTY_SIGNATURE_ERROR_PREFIX}{INTERNAL_SIGNATURE_CHECK_FAILURE}"
        ))
    );
}
