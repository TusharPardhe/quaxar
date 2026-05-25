//! Integration tests that pin the narrowed Rust `STTx::checkBatchSign(...)`
//! seam to the current C++ behavior.

use std::cell::Cell;

use protocol::{
    BatchSigner, INTERNAL_BATCH_SIGNATURE_CHECK_FAILURE, NOT_A_BATCH_TRANSACTION_ERROR,
    StTxMultiSignObject, StTxMultiSigner, TxType, check_batch_sign,
};

#[derive(Clone, Copy)]
struct TestMultiSigner {
    account_id: &'static str,
}

impl StTxMultiSigner<&'static str> for TestMultiSigner {
    fn account_id(&self) -> &'static str {
        self.account_id
    }
}

#[derive(Clone)]
struct TestSigner {
    signing_pub_key_is_empty: bool,
    signers_present: bool,
    txn_signature_present: bool,
    signers: Vec<TestMultiSigner>,
    panic_on_pub_key_lookup: bool,
}

impl protocol::StTxSingleSignObject for TestSigner {
    fn signers_present(&self) -> bool {
        self.signers_present
    }
}

impl StTxMultiSignObject<&'static str, TestMultiSigner> for TestSigner {
    type Signers = Vec<TestMultiSigner>;

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

impl BatchSigner<&'static str, TestMultiSigner> for TestSigner {
    fn signing_pub_key_is_empty(&self) -> bool {
        assert!(
            !self.panic_on_pub_key_lookup,
            "simulated signer SigningPubKey access failure"
        );
        self.signing_pub_key_is_empty
    }
}

#[test]
fn protocol_batch_sign_rejects_non_batch_transactions() {
    let result = check_batch_sign(
        TxType::PAYMENT,
        [TestSigner {
            signing_pub_key_is_empty: false,
            signers_present: false,
            txn_signature_present: false,
            signers: vec![],
            panic_on_pub_key_lookup: false,
        }],
        |_| true,
        |_| Ok(()),
        |account_id| format!("r{account_id}"),
    );

    assert_eq!(result, Err(NOT_A_BATCH_TRANSACTION_ERROR.to_owned()));
}

#[test]
fn protocol_batch_sign_uses_multi_sign_callback_for_empty_signing_pub_key() {
    let multi_called = Cell::new(false);
    let single_called = Cell::new(false);

    let result = check_batch_sign(
        TxType::BATCH,
        [TestSigner {
            signing_pub_key_is_empty: true,
            signers_present: true,
            txn_signature_present: false,
            signers: vec![TestMultiSigner {
                account_id: "alice",
            }],
            panic_on_pub_key_lookup: false,
        }],
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
fn protocol_batch_sign_uses_single_sign_callback_for_present_signing_pub_key() {
    let multi_called = Cell::new(false);
    let single_called = Cell::new(false);

    let result = check_batch_sign(
        TxType::BATCH,
        [TestSigner {
            signing_pub_key_is_empty: false,
            signers_present: false,
            txn_signature_present: false,
            signers: vec![],
            panic_on_pub_key_lookup: false,
        }],
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
fn protocol_batch_sign_returns_first_callback_failure_unchanged() {
    let later_callback_ran = Cell::new(false);

    let result = check_batch_sign(
        TxType::BATCH,
        [
            TestSigner {
                signing_pub_key_is_empty: false,
                signers_present: true,
                txn_signature_present: false,
                signers: vec![],
                panic_on_pub_key_lookup: false,
            },
            TestSigner {
                signing_pub_key_is_empty: true,
                signers_present: true,
                txn_signature_present: false,
                signers: vec![TestMultiSigner { account_id: "bob" }],
                panic_on_pub_key_lookup: false,
            },
        ],
        |_| true,
        |_| {
            later_callback_ran.set(true);
            Ok(())
        },
        |account_id| format!("r{account_id}"),
    );

    assert_eq!(
        result,
        Err(protocol::CANNOT_BOTH_SINGLE_AND_MULTI_SIGN_ERROR.to_owned())
    );
    assert!(!later_callback_ran.get());
}

#[test]
fn protocol_batch_sign_maps_signer_access_panics_to_internal_failure() {
    let result = check_batch_sign(
        TxType::BATCH,
        [TestSigner {
            signing_pub_key_is_empty: false,
            signers_present: false,
            txn_signature_present: false,
            signers: vec![],
            panic_on_pub_key_lookup: true,
        }],
        |_| true,
        |_| Ok(()),
        |account_id| format!("r{account_id}"),
    );

    assert_eq!(
        result,
        Err(INTERNAL_BATCH_SIGNATURE_CHECK_FAILURE.to_owned())
    );
}
