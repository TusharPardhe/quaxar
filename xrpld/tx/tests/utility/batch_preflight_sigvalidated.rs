//! Integration tests that pin the narrowed Rust
//! `Batch::preflightSigValidated(...)` signer-validation seam to the current
//! C++ behavior.

use protocol::{Ter, trans_token};
use tx::{
    BatchInnerTransaction, BatchSignatureFacts, BatchSignerEntry,
    validate_batch_preflight_sig_validated,
};

#[derive(Clone)]
struct StubInnerTx {
    account: &'static str,
    counterparty: Option<&'static str>,
}

impl StubInnerTx {
    fn new(account: &'static str) -> Self {
        Self {
            account,
            counterparty: None,
        }
    }
}

impl BatchInnerTransaction for StubInnerTx {
    type TxId = &'static str;
    type Account = &'static str;

    fn transaction_id(&self) -> Self::TxId {
        "unused"
    }

    fn txn_type(&self) -> protocol::TxType {
        protocol::TxType::PAYMENT
    }

    fn flags(&self) -> u32 {
        0
    }

    fn signature_facts(&self) -> BatchSignatureFacts {
        BatchSignatureFacts::default()
    }

    fn counterparty_signature_facts(&self) -> Option<BatchSignatureFacts> {
        None
    }

    fn fee_is_native_zero(&self) -> bool {
        true
    }

    fn account(&self) -> Self::Account {
        self.account
    }

    fn counterparty(&self) -> Option<Self::Account> {
        self.counterparty
    }

    fn sequence(&self) -> u32 {
        0
    }

    fn ticket_sequence(&self) -> Option<u32> {
        None
    }
}

#[derive(Clone)]
struct StubBatchSigner {
    account: &'static str,
}

impl BatchSignerEntry for StubBatchSigner {
    type Account = &'static str;

    fn account(&self) -> Self::Account {
        self.account
    }
}

#[test]
fn tx_batch_preflight_sig_validated_rejects_signers_array_above_cpp_limit() {
    let signers = vec![
        StubBatchSigner { account: "a1" },
        StubBatchSigner { account: "a2" },
        StubBatchSigner { account: "a3" },
        StubBatchSigner { account: "a4" },
        StubBatchSigner { account: "a5" },
        StubBatchSigner { account: "a6" },
        StubBatchSigner { account: "a7" },
        StubBatchSigner { account: "a8" },
        StubBatchSigner { account: "a9" },
    ];

    let result = validate_batch_preflight_sig_validated(
        "outer",
        [StubInnerTx::new("alice"), StubInnerTx::new("outer")],
        Some(signers),
        || true,
    );

    assert_eq!(result, Ter::TEM_ARRAY_TOO_LARGE);
}

#[test]
fn tx_batch_preflight_sig_validated_rejects_outer_account_as_batch_signer() {
    let result = validate_batch_preflight_sig_validated(
        "outer",
        [StubInnerTx::new("alice"), StubInnerTx::new("outer")],
        Some([StubBatchSigner { account: "outer" }]),
        || true,
    );

    assert_eq!(result, Ter::TEM_BAD_SIGNER);
}

#[test]
fn tx_batch_preflight_sig_validated_rejects_duplicate_batch_signers() {
    let result = validate_batch_preflight_sig_validated(
        "outer",
        [StubInnerTx::new("alice"), StubInnerTx::new("outer")],
        Some([
            StubBatchSigner { account: "alice" },
            StubBatchSigner { account: "alice" },
        ]),
        || true,
    );

    assert_eq!(result, Ter::TEM_REDUNDANT);
}

#[test]
fn tx_batch_preflight_sig_validated_rejects_unrequired_batch_signer() {
    let result = validate_batch_preflight_sig_validated(
        "outer",
        [StubInnerTx::new("alice"), StubInnerTx::new("outer")],
        Some([StubBatchSigner { account: "bob" }]),
        || true,
    );

    assert_eq!(result, Ter::TEM_BAD_SIGNER);
}

#[test]
fn tx_batch_preflight_sig_validated_rejects_invalid_batch_signature() {
    let result = validate_batch_preflight_sig_validated(
        "outer",
        [StubInnerTx::new("alice"), StubInnerTx::new("outer")],
        Some([StubBatchSigner { account: "alice" }]),
        || false,
    );

    assert_eq!(result, Ter::TEM_BAD_SIGNATURE);
    assert_eq!(trans_token(result), "temBAD_SIGNATURE");
}

#[test]
fn tx_batch_preflight_sig_validated_rejects_missing_required_signer() {
    let result = validate_batch_preflight_sig_validated(
        "outer",
        [StubInnerTx::new("alice"), StubInnerTx::new("outer")],
        None::<Vec<StubBatchSigner>>,
        || true,
    );

    assert_eq!(result, Ter::TEM_BAD_SIGNER);
}

#[test]
fn tx_batch_preflight_sig_validated_accepts_inner_and_counterparty_signers() {
    let mut with_counterparty = StubInnerTx::new("alice");
    with_counterparty.counterparty = Some("carol");

    let result = validate_batch_preflight_sig_validated(
        "outer",
        [with_counterparty, StubInnerTx::new("outer")],
        Some([
            StubBatchSigner { account: "alice" },
            StubBatchSigner { account: "carol" },
        ]),
        || true,
    );

    assert_eq!(result, Ter::TES_SUCCESS);
}
