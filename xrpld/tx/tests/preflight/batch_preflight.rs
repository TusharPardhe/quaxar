//! Integration tests that pin the narrowed Rust `Batch::preflight(...)`
//! structure-validation seam to the current C++ behavior.

use protocol::{BatchTransactionFlags, INNER_BATCH_TRANSACTION_FLAG, Ter, TxType, trans_token};
use tx::{BatchInnerTransaction, BatchSignatureFacts, validate_batch_preflight_structure};

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

#[test]
fn tx_batch_preflight_requires_exactly_one_batch_mode() {
    let result = validate_batch_preflight_structure(
        BatchTransactionFlags::ALL_OR_NOTHING.bits() | BatchTransactionFlags::ONLY_ONE.bits(),
        [
            StubInnerTx::new("tx-1", "alice"),
            StubInnerTx::new("tx-2", "bob"),
        ],
        |_| Ter::TES_SUCCESS,
    );

    assert_eq!(result, Ter::TEM_INVALID_FLAG);
    assert_eq!(trans_token(result), "temINVALID_FLAG");
}

#[test]
fn tx_batch_preflight_requires_at_least_two_inner_transactions() {
    let result = validate_batch_preflight_structure(
        BatchTransactionFlags::ALL_OR_NOTHING.bits(),
        [StubInnerTx::new("tx-1", "alice")],
        |_| Ter::TES_SUCCESS,
    );

    assert_eq!(result, Ter::TEM_ARRAY_EMPTY);
}

#[test]
fn tx_batch_preflight_rejects_duplicate_inner_hashes() {
    let result = validate_batch_preflight_structure(
        BatchTransactionFlags::ALL_OR_NOTHING.bits(),
        [
            StubInnerTx::new("dup", "alice"),
            StubInnerTx::new("dup", "bob"),
        ],
        |_| Ter::TES_SUCCESS,
    );

    assert_eq!(result, Ter::TEM_REDUNDANT);
}

#[test]
fn tx_batch_preflight_rejects_nested_batch_transactions() {
    let mut nested = StubInnerTx::new("tx-1", "alice");
    nested.txn_type = TxType::BATCH;

    let result = validate_batch_preflight_structure(
        BatchTransactionFlags::ALL_OR_NOTHING.bits(),
        [nested, StubInnerTx::new("tx-2", "bob")],
        |_| Ter::TES_SUCCESS,
    );

    assert_eq!(result, Ter::TEM_INVALID);
}

#[test]
fn tx_batch_preflight_rejects_disabled_inner_transaction_types() {
    let mut disabled = StubInnerTx::new("tx-1", "alice");
    disabled.txn_type = TxType::VAULT_CREATE;

    let result = validate_batch_preflight_structure(
        BatchTransactionFlags::ALL_OR_NOTHING.bits(),
        [disabled, StubInnerTx::new("tx-2", "bob")],
        |_| Ter::TES_SUCCESS,
    );

    assert_eq!(result, Ter::TEM_INVALID_INNER_BATCH);
}

#[test]
fn tx_batch_preflight_rejects_missing_inner_batch_flag() {
    let mut missing_flag = StubInnerTx::new("tx-1", "alice");
    missing_flag.flags = 0;

    let result = validate_batch_preflight_structure(
        BatchTransactionFlags::ALL_OR_NOTHING.bits(),
        [missing_flag, StubInnerTx::new("tx-2", "bob")],
        |_| Ter::TES_SUCCESS,
    );

    assert_eq!(result, Ter::TEM_INVALID_FLAG);
}

#[test]
fn tx_batch_preflight_rejects_signature_fields_and_non_empty_signing_pub_key() {
    let mut with_signature = StubInnerTx::new("tx-1", "alice");
    with_signature.signature_facts = BatchSignatureFacts {
        has_txn_signature: true,
        signing_pub_key_is_empty: true,
        ..BatchSignatureFacts::default()
    };
    let result = validate_batch_preflight_structure(
        BatchTransactionFlags::ALL_OR_NOTHING.bits(),
        [with_signature, StubInnerTx::new("tx-2", "bob")],
        |_| Ter::TES_SUCCESS,
    );
    assert_eq!(result, Ter::TEM_BAD_SIGNATURE);

    let mut with_regkey = StubInnerTx::new("tx-3", "alice");
    with_regkey.signature_facts = BatchSignatureFacts {
        signing_pub_key_is_empty: false,
        ..BatchSignatureFacts::default()
    };
    let result = validate_batch_preflight_structure(
        BatchTransactionFlags::ALL_OR_NOTHING.bits(),
        [with_regkey, StubInnerTx::new("tx-4", "bob")],
        |_| Ter::TES_SUCCESS,
    );
    assert_eq!(result, Ter::TEM_BAD_REGKEY);
}

#[test]
fn tx_batch_preflight_rejects_non_zero_or_non_native_fee() {
    let mut bad_fee = StubInnerTx::new("tx-1", "alice");
    bad_fee.fee_is_native_zero = false;

    let result = validate_batch_preflight_structure(
        BatchTransactionFlags::ALL_OR_NOTHING.bits(),
        [bad_fee, StubInnerTx::new("tx-2", "bob")],
        |_| Ter::TES_SUCCESS,
    );

    assert_eq!(result, Ter::TEM_BAD_FEE);
}

#[test]
fn tx_batch_preflight_maps_failed_inner_preflight_to_invalid_inner_batch() {
    let result = validate_batch_preflight_structure(
        BatchTransactionFlags::ALL_OR_NOTHING.bits(),
        [
            StubInnerTx::new("tx-1", "alice"),
            StubInnerTx::new("tx-2", "bob"),
        ],
        |_| Ter::TER_PRE_SEQ,
    );

    assert_eq!(result, Ter::TEM_INVALID_INNER_BATCH);
}

#[test]
fn tx_batch_preflight_requires_exactly_one_of_sequence_and_ticket() {
    let mut both = StubInnerTx::new("tx-1", "alice");
    both.ticket_sequence = Some(7);

    let result = validate_batch_preflight_structure(
        BatchTransactionFlags::ALL_OR_NOTHING.bits(),
        [both, StubInnerTx::new("tx-2", "bob")],
        |_| Ter::TES_SUCCESS,
    );

    assert_eq!(result, Ter::TEM_SEQ_AND_TICKET);
}

#[test]
fn tx_batch_preflight_rejects_duplicate_sequence_and_ticket_for_ordered_modes() {
    let first = StubInnerTx::new("tx-1", "alice");
    let mut second = StubInnerTx::new("tx-2", "alice");
    second.sequence = 1;

    let result = validate_batch_preflight_structure(
        BatchTransactionFlags::UNTIL_FAILURE.bits(),
        [first, second],
        |_| Ter::TES_SUCCESS,
    );

    assert_eq!(result, Ter::TEM_REDUNDANT);
}

#[test]
fn tx_batch_preflight_accepts_valid_independent_batch_structure() {
    let first = StubInnerTx::new("tx-1", "alice");
    let mut second = StubInnerTx::new("tx-2", "alice");
    second.sequence = 1;

    let result = validate_batch_preflight_structure(
        BatchTransactionFlags::INDEPENDENT.bits(),
        [first, second],
        |_| Ter::TES_SUCCESS,
    );

    assert_eq!(result, Ter::TES_SUCCESS);
}
