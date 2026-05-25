//! Integration tests that pin the narrowed Rust `Batch.cpp` transactor shells
//! to the current C++ behavior.

use protocol::{BatchTransactionFlags, INNER_BATCH_TRANSACTION_FLAG, Ter, TxType, trans_token};
use tx::batch_preflight::BatchSignatureFacts;
use tx::batch_transactor::{BatchPreflightCarrier, BatchSigValidatedCarrier};
use tx::{BatchInnerTransaction, BatchSignerEntry, run_batch_check_sign, run_batch_do_apply};

#[derive(Clone)]
struct StubInnerTx {
    tx_id: &'static str,
    txn_type: TxType,
    flags: u32,
    signature_facts: BatchSignatureFacts,
    counterparty_signature_facts: Option<BatchSignatureFacts>,
    fee_is_native_zero: bool,
    account: &'static str,
    counterparty: Option<&'static str>,
    sequence: u32,
    ticket_sequence: Option<u32>,
}

impl StubInnerTx {
    fn new(tx_id: &'static str, account: &'static str) -> Self {
        Self {
            tx_id,
            txn_type: TxType::PAYMENT,
            flags: INNER_BATCH_TRANSACTION_FLAG,
            signature_facts: BatchSignatureFacts {
                signing_pub_key_is_empty: true,
                ..BatchSignatureFacts::default()
            },
            counterparty_signature_facts: None,
            fee_is_native_zero: true,
            account,
            counterparty: None,
            sequence: 1,
            ticket_sequence: None,
        }
    }
}

impl BatchInnerTransaction for StubInnerTx {
    type TxId = &'static str;
    type Account = &'static str;

    fn transaction_id(&self) -> Self::TxId {
        self.tx_id
    }

    fn txn_type(&self) -> TxType {
        self.txn_type
    }

    fn flags(&self) -> u32 {
        self.flags
    }

    fn signature_facts(&self) -> BatchSignatureFacts {
        self.signature_facts
    }

    fn counterparty_signature_facts(&self) -> Option<BatchSignatureFacts> {
        self.counterparty_signature_facts
    }

    fn fee_is_native_zero(&self) -> bool {
        self.fee_is_native_zero
    }

    fn account(&self) -> Self::Account {
        self.account
    }

    fn counterparty(&self) -> Option<Self::Account> {
        self.counterparty
    }

    fn sequence(&self) -> u32 {
        self.sequence
    }

    fn ticket_sequence(&self) -> Option<u32> {
        self.ticket_sequence
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
fn tx_batch_check_sign_returns_standard_sign_failure_without_running_batch_check() {
    let result = run_batch_check_sign(
        || Ter::TEM_BAD_SIGNATURE,
        || panic!("batch check should not run after a standard sign failure"),
    );

    assert_eq!(result, Ter::TEM_BAD_SIGNATURE);
    assert_eq!(trans_token(result), "temBAD_SIGNATURE");
}

#[test]
fn tx_batch_check_sign_returns_batch_check_failure_after_standard_success() {
    let result = run_batch_check_sign(|| Ter::TES_SUCCESS, || Ter::TEM_BAD_SIGNER);

    assert_eq!(result, Ter::TEM_BAD_SIGNER);
    assert_eq!(trans_token(result), "temBAD_SIGNER");
}

#[test]
fn tx_batch_check_sign_accepts_when_both_checks_succeed() {
    let result = run_batch_check_sign(|| Ter::TES_SUCCESS, || Ter::TES_SUCCESS);

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(trans_token(result), "tesSUCCESS");
}

#[test]
fn tx_batch_do_apply_returns_success_for_outer_batch() {
    let result = run_batch_do_apply();

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(trans_token(result), "tesSUCCESS");
}

#[test]
fn tx_batch_preflight_carrier_rejects_invalid_batch_mode() {
    let result = BatchPreflightCarrier::new(
        BatchTransactionFlags::ALL_OR_NOTHING.bits() | BatchTransactionFlags::ONLY_ONE.bits(),
        vec![
            StubInnerTx::new("tx-1", "alice"),
            StubInnerTx::new("tx-2", "bob"),
        ],
    )
    .validate(|_| Ter::TES_SUCCESS);

    assert_eq!(result, Ter::TEM_INVALID_FLAG);
}

#[test]
fn tx_batch_preflight_carrier_rejects_invalid_inner_signature_and_fee() {
    let mut bad_signature = StubInnerTx::new("tx-1", "alice");
    bad_signature.signature_facts = BatchSignatureFacts {
        has_txn_signature: true,
        signing_pub_key_is_empty: true,
        ..BatchSignatureFacts::default()
    };
    let result = BatchPreflightCarrier::new(
        BatchTransactionFlags::ALL_OR_NOTHING.bits(),
        vec![bad_signature, StubInnerTx::new("tx-2", "bob")],
    )
    .validate(|_| Ter::TES_SUCCESS);
    assert_eq!(result, Ter::TEM_BAD_SIGNATURE);

    let mut bad_fee = StubInnerTx::new("tx-3", "alice");
    bad_fee.fee_is_native_zero = false;
    let result = BatchPreflightCarrier::new(
        BatchTransactionFlags::ALL_OR_NOTHING.bits(),
        vec![bad_fee, StubInnerTx::new("tx-4", "bob")],
    )
    .validate(|_| Ter::TES_SUCCESS);
    assert_eq!(result, Ter::TEM_BAD_FEE);
}

#[test]
fn tx_batch_sig_validated_carrier_rejects_bad_signers() {
    let carrier = BatchSigValidatedCarrier::new(
        "outer",
        vec![
            StubInnerTx::new("tx-1", "alice"),
            StubInnerTx::new("tx-2", "outer"),
        ],
        Some(vec![StubBatchSigner { account: "alice" }]),
    );

    assert_eq!(carrier.clone().validate(|| false), Ter::TEM_BAD_SIGNATURE);
    assert_eq!(
        BatchSigValidatedCarrier::new(
            "outer",
            vec![
                StubInnerTx::new("tx-1", "alice"),
                StubInnerTx::new("tx-2", "outer")
            ],
            Some(vec![StubBatchSigner { account: "outer" }]),
        )
        .validate(|| true),
        Ter::TEM_BAD_SIGNER
    );
    assert_eq!(
        BatchSigValidatedCarrier::new(
            "outer",
            vec![
                StubInnerTx::new("tx-1", "alice"),
                StubInnerTx::new("tx-2", "outer")
            ],
            None::<Vec<StubBatchSigner>>,
        )
        .validate(|| true),
        Ter::TEM_BAD_SIGNER
    );
}

#[test]
fn tx_batch_preflight_carrier_accepts_valid_independent_batch() {
    let first = StubInnerTx::new("tx-1", "alice");
    let mut second = StubInnerTx::new("tx-2", "alice");
    second.sequence = 1;

    let result = BatchPreflightCarrier::new(
        BatchTransactionFlags::INDEPENDENT.bits(),
        vec![first, second],
    )
    .validate(|_| Ter::TES_SUCCESS);

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(trans_token(result), "tesSUCCESS");
}
