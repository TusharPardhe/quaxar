//! Current the reference implementation transactor shells.
//!
//! This ports the deterministic outer `Batch` behavior around:
//!
//! - `Batch::preflight(...)` and `Batch::preflightSigValidated(...)` being
//!   driven from a typed owner-facing carrier over the landed batch validation
//!   helpers,
//! - `Batch::checkSign(...)` running the standard transaction-signature check
//!   first,
//! - only running the batch-specific signature check when the standard check
//!   succeeds,
//! - returning the first non-success code without remapping it,
//! - `Batch::doApply()` returning `tesSUCCESS` for the outer batch
//!   transaction.

use std::hash::Hash;

pub use crate::batch_preflight::{BatchInnerTransaction, BatchSignatureFacts, BatchSignerEntry};
use crate::batch_preflight::{
    validate_batch_preflight_sig_validated, validate_batch_preflight_structure,
};
use protocol::{NotTec, Ter, is_tes_success};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchPreflightCarrier<InnerTx>
where
    InnerTx: BatchInnerTransaction,
{
    batch_flags: u32,
    inner_transactions: Vec<InnerTx>,
}

impl<InnerTx> BatchPreflightCarrier<InnerTx>
where
    InnerTx: BatchInnerTransaction,
{
    pub fn new(batch_flags: u32, inner_transactions: Vec<InnerTx>) -> Self {
        Self {
            batch_flags,
            inner_transactions,
        }
    }

    pub fn validate<InnerPreflight>(self, preflight_inner_transaction: InnerPreflight) -> NotTec
    where
        InnerPreflight: FnMut(&InnerTx) -> NotTec,
    {
        validate_batch_preflight_structure(
            self.batch_flags,
            self.inner_transactions,
            preflight_inner_transaction,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchSigValidatedCarrier<InnerTx, Signer>
where
    InnerTx: BatchInnerTransaction,
    Signer: BatchSignerEntry<Account = InnerTx::Account>,
{
    outer_account: InnerTx::Account,
    inner_transactions: Vec<InnerTx>,
    batch_signers: Option<Vec<Signer>>,
}

impl<InnerTx, Signer> BatchSigValidatedCarrier<InnerTx, Signer>
where
    InnerTx: BatchInnerTransaction,
    InnerTx::Account: Clone + Eq + Hash,
    Signer: BatchSignerEntry<Account = InnerTx::Account>,
{
    pub fn new(
        outer_account: InnerTx::Account,
        inner_transactions: Vec<InnerTx>,
        batch_signers: Option<Vec<Signer>>,
    ) -> Self {
        Self {
            outer_account,
            inner_transactions,
            batch_signers,
        }
    }

    pub fn validate(self, check_batch_sign: impl FnOnce() -> bool) -> NotTec {
        validate_batch_preflight_sig_validated(
            self.outer_account,
            self.inner_transactions,
            self.batch_signers,
            check_batch_sign,
        )
    }
}

pub fn run_batch_check_sign<CheckSign, CheckBatchSign>(
    check_sign: CheckSign,
    check_batch_sign: CheckBatchSign,
) -> NotTec
where
    CheckSign: FnOnce() -> NotTec,
    CheckBatchSign: FnOnce() -> NotTec,
{
    let sign_result = check_sign();
    if !is_tes_success(sign_result) {
        return sign_result;
    }

    let batch_sign_result = check_batch_sign();
    if !is_tes_success(batch_sign_result) {
        return batch_sign_result;
    }

    Ter::TES_SUCCESS
}

pub fn run_batch_do_apply() -> Ter {
    Ter::TES_SUCCESS
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use protocol::{BatchTransactionFlags, INNER_BATCH_TRANSACTION_FLAG, trans_token};

    use crate::batch_preflight::BatchSignatureFacts;

    use super::{
        BatchInnerTransaction, BatchPreflightCarrier, BatchSigValidatedCarrier, BatchSignerEntry,
        run_batch_check_sign, run_batch_do_apply,
    };

    #[derive(Clone)]
    struct StubInnerTx {
        tx_id: &'static str,
        txn_type: protocol::TxType,
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
                txn_type: protocol::TxType::PAYMENT,
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

        fn txn_type(&self) -> protocol::TxType {
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
    fn batch_preflight_carrier_rejects_invalid_batch_mode() {
        let result = BatchPreflightCarrier::new(
            BatchTransactionFlags::ALL_OR_NOTHING.bits() | BatchTransactionFlags::ONLY_ONE.bits(),
            vec![
                StubInnerTx::new("tx-1", "alice"),
                StubInnerTx::new("tx-2", "bob"),
            ],
        )
        .validate(|_| protocol::Ter::TES_SUCCESS);

        assert_eq!(result, protocol::Ter::TEM_INVALID_FLAG);
    }

    #[test]
    fn batch_preflight_carrier_rejects_invalid_inner_signature_and_fee() {
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
        .validate(|_| protocol::Ter::TES_SUCCESS);
        assert_eq!(result, protocol::Ter::TEM_BAD_SIGNATURE);

        let mut bad_fee = StubInnerTx::new("tx-3", "alice");
        bad_fee.fee_is_native_zero = false;
        let result = BatchPreflightCarrier::new(
            BatchTransactionFlags::ALL_OR_NOTHING.bits(),
            vec![bad_fee, StubInnerTx::new("tx-4", "bob")],
        )
        .validate(|_| protocol::Ter::TES_SUCCESS);
        assert_eq!(result, protocol::Ter::TEM_BAD_FEE);
    }

    #[test]
    fn batch_preflight_carrier_accepts_valid_independent_batch() {
        let first = StubInnerTx::new("tx-1", "alice");
        let mut second = StubInnerTx::new("tx-2", "alice");
        second.sequence = 1;

        let result = BatchPreflightCarrier::new(
            BatchTransactionFlags::INDEPENDENT.bits(),
            vec![first, second],
        )
        .validate(|_| protocol::Ter::TES_SUCCESS);

        assert_eq!(result, protocol::Ter::TES_SUCCESS);
        assert_eq!(trans_token(result), "tesSUCCESS");
    }

    #[test]
    fn batch_sig_validated_carrier_rejects_bad_signers() {
        let carrier = BatchSigValidatedCarrier::new(
            "outer",
            vec![
                StubInnerTx::new("tx-1", "alice"),
                StubInnerTx::new("tx-2", "outer"),
            ],
            Some(vec![StubBatchSigner { account: "alice" }]),
        );

        assert_eq!(
            carrier.clone().validate(|| false),
            protocol::Ter::TEM_BAD_SIGNATURE
        );
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
            protocol::Ter::TEM_BAD_SIGNER
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
            protocol::Ter::TEM_BAD_SIGNER
        );
    }

    #[test]
    fn batch_check_sign_returns_standard_sign_failure_without_running_batch_check() {
        let batch_check_called = Cell::new(false);

        let result = run_batch_check_sign(
            || protocol::Ter::TEM_BAD_SIGNATURE,
            || {
                batch_check_called.set(true);
                protocol::Ter::TES_SUCCESS
            },
        );

        assert_eq!(result, protocol::Ter::TEM_BAD_SIGNATURE);
        assert!(!batch_check_called.get());
    }

    #[test]
    fn batch_check_sign_returns_batch_sign_failure_after_standard_success() {
        let result = run_batch_check_sign(
            || protocol::Ter::TES_SUCCESS,
            || protocol::Ter::TEM_BAD_SIGNER,
        );

        assert_eq!(result, protocol::Ter::TEM_BAD_SIGNER);
    }

    #[test]
    fn batch_check_sign_accepts_when_both_sign_checks_succeed() {
        let result =
            run_batch_check_sign(|| protocol::Ter::TES_SUCCESS, || protocol::Ter::TES_SUCCESS);

        assert_eq!(result, protocol::Ter::TES_SUCCESS);
    }

    #[test]
    fn batch_do_apply_returns_success_for_outer_batch() {
        assert_eq!(run_batch_do_apply(), protocol::Ter::TES_SUCCESS);
    }
}
