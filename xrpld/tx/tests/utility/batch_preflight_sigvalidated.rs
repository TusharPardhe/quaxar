//! Integration tests that pin the narrowed Rust
//! `Batch::preflightSigValidated(...)` signer-validation seam to the current
//! C++ behavior.

use protocol::{Ter, trans_token};
use tx::{
    BatchInnerTransaction, BatchSignatureFacts, BatchSignerEntry, MAX_BATCH_SIGNER_COUNT,
    validate_batch_preflight_sig_validated,
};

#[derive(Clone)]
struct StubInnerTx {
    account: &'static str,
    initiator: Option<&'static str>,
    counterparty: Option<&'static str>,
    sponsor: Option<&'static str>,
    sponsor_signature_facts: Option<BatchSignatureFacts>,
}

impl StubInnerTx {
    fn new(account: &'static str) -> Self {
        Self {
            account,
            initiator: None,
            counterparty: None,
            sponsor: None,
            sponsor_signature_facts: None,
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

    fn initiator(&self) -> Self::Account {
        self.initiator.unwrap_or(self.account)
    }

    fn sponsor(&self) -> Option<Self::Account> {
        self.sponsor
    }

    fn sponsor_signature_facts(&self) -> Option<BatchSignatureFacts> {
        self.sponsor_signature_facts
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
fn tx_batch_preflight_sig_validated_uses_delegate_as_required_authorizer() {
    let mut delegated = StubInnerTx::new("account");
    delegated.initiator = Some("delegate");

    let result = validate_batch_preflight_sig_validated(
        "outer",
        [delegated.clone(), StubInnerTx::new("outer")],
        Some([StubBatchSigner { account: "account" }]),
        || true,
    );
    assert_eq!(result, Ter::TEM_BAD_SIGNER);

    let result = validate_batch_preflight_sig_validated(
        "outer",
        [delegated, StubInnerTx::new("outer")],
        Some([StubBatchSigner {
            account: "delegate",
        }]),
        || true,
    );
    assert_eq!(result, Ter::TES_SUCCESS);
}

#[test]
fn tx_batch_preflight_sig_validated_requires_sponsor_only_with_sponsor_signature() {
    let mut sponsored = StubInnerTx::new("alice");
    sponsored.sponsor = Some("sponsor");

    let result = validate_batch_preflight_sig_validated(
        "outer",
        [sponsored.clone(), StubInnerTx::new("outer")],
        Some([StubBatchSigner { account: "alice" }]),
        || true,
    );
    assert_eq!(result, Ter::TES_SUCCESS);

    sponsored.sponsor_signature_facts = Some(BatchSignatureFacts {
        signing_pub_key_is_empty: true,
        ..BatchSignatureFacts::default()
    });
    let result = validate_batch_preflight_sig_validated(
        "outer",
        [sponsored.clone(), StubInnerTx::new("outer")],
        Some([StubBatchSigner { account: "alice" }]),
        || true,
    );
    assert_eq!(result, Ter::TEM_BAD_SIGNER);

    let result = validate_batch_preflight_sig_validated(
        "outer",
        [sponsored, StubInnerTx::new("outer")],
        Some([
            StubBatchSigner { account: "alice" },
            StubBatchSigner { account: "sponsor" },
        ]),
        || true,
    );
    assert_eq!(result, Ter::TES_SUCCESS);
}

#[test]
fn tx_batch_preflight_sig_validated_rejects_signers_array_above_cpp_limit() {
    let signers = vec![
        StubBatchSigner {
            account: "oversized",
        };
        MAX_BATCH_SIGNER_COUNT + 1
    ];
    assert_eq!(signers.len(), 25);

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

    assert_eq!(result, Ter::TEM_BAD_SIGNER);
}

#[test]
fn tx_batch_preflight_sig_validated_rejects_descending_batch_signers() {
    let mut with_counterparty = StubInnerTx::new("alice");
    with_counterparty.counterparty = Some("carol");

    let result = validate_batch_preflight_sig_validated(
        "outer",
        [with_counterparty, StubInnerTx::new("outer")],
        Some([
            StubBatchSigner { account: "carol" },
            StubBatchSigner { account: "alice" },
        ]),
        || true,
    );

    assert_eq!(result, Ter::TEM_BAD_SIGNER);
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
