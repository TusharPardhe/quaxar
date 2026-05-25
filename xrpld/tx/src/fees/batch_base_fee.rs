//! Aggregate the reference implementation wrapper.
//!
//! This ports the deterministic outer behavior around:
//!
//! - adding one extra ledger base fee on top of the lower batch
//!   `Transactor::calculateBaseFee(...)` result,
//! - rejecting inner batch transactions,
//! - rejecting raw inner-transaction and batch-signer arrays that exceed the
//!   current batch limit,
//! - summing inner transaction fees in order,
//! - counting one signer fee for each `sfTxnSignature` batch signer and one
//!   signer fee per multisigner entry in `sfSigners`,
//! - and returning the invalid-fee sentinel when any guarded add
//!   or multiply overflows.

use protocol::TxType;

use crate::MAX_BATCH_TX_COUNT;

pub trait BatchBaseFeeInnerTransaction {
    fn txn_type(&self) -> TxType;
}

pub trait BatchBaseFeeSignerEntry {
    fn has_txn_signature(&self) -> bool;
    fn multisigner_count(&self) -> usize;
}

#[allow(clippy::too_many_arguments)]
pub fn run_batch_calculate_base_fee<Fee, InnerTx, Signer>(
    invalid_fee: Fee,
    transactor_base_fee: Fee,
    ledger_base_fee: Fee,
    raw_transactions: Option<impl IntoIterator<Item = InnerTx>>,
    batch_signers: Option<impl IntoIterator<Item = Signer>>,
    mut calculate_inner_base_fee: impl FnMut(InnerTx) -> Fee,
    mut checked_add: impl FnMut(Fee, Fee) -> Option<Fee>,
    mut checked_mul_fee_by_usize: impl FnMut(Fee, usize) -> Option<Fee>,
) -> Fee
where
    Fee: Copy,
    InnerTx: BatchBaseFeeInnerTransaction,
    Signer: BatchBaseFeeSignerEntry,
{
    let Some(batch_base) = checked_add(ledger_base_fee, transactor_base_fee) else {
        return invalid_fee;
    };

    let raw_transactions: Vec<_> = raw_transactions
        .map(IntoIterator::into_iter)
        .into_iter()
        .flatten()
        .collect();
    if raw_transactions.len() > MAX_BATCH_TX_COUNT {
        return invalid_fee;
    }

    let mut txn_fees = match checked_mul_fee_by_usize(ledger_base_fee, 0) {
        Some(zero) => zero,
        None => return invalid_fee,
    };

    for inner_tx in raw_transactions {
        if inner_tx.txn_type() == TxType::BATCH {
            return invalid_fee;
        }

        let inner_fee = calculate_inner_base_fee(inner_tx);
        let Some(next_txn_fees) = checked_add(txn_fees, inner_fee) else {
            return invalid_fee;
        };
        txn_fees = next_txn_fees;
    }

    let batch_signers: Vec<_> = batch_signers
        .map(IntoIterator::into_iter)
        .into_iter()
        .flatten()
        .collect();
    if batch_signers.len() > MAX_BATCH_TX_COUNT {
        return invalid_fee;
    }

    let signer_count = batch_signers.into_iter().fold(0usize, |count, signer| {
        if signer.has_txn_signature() {
            count.saturating_add(1)
        } else {
            count.saturating_add(signer.multisigner_count())
        }
    });

    let Some(signer_fees) = checked_mul_fee_by_usize(ledger_base_fee, signer_count) else {
        return invalid_fee;
    };
    let Some(inner_plus_signers) = checked_add(txn_fees, signer_fees) else {
        return invalid_fee;
    };
    let Some(total_fee) = checked_add(inner_plus_signers, batch_base) else {
        return invalid_fee;
    };

    total_fee
}

#[cfg(test)]
mod tests {
    use super::{
        BatchBaseFeeInnerTransaction, BatchBaseFeeSignerEntry, run_batch_calculate_base_fee,
    };
    use crate::MAX_BATCH_TX_COUNT;
    use protocol::TxType;

    #[derive(Clone, Copy)]
    struct TestInnerTx {
        txn_type: TxType,
        fee: u64,
    }

    impl BatchBaseFeeInnerTransaction for TestInnerTx {
        fn txn_type(&self) -> TxType {
            self.txn_type
        }
    }

    #[derive(Clone, Copy)]
    struct TestSigner {
        has_txn_signature: bool,
        multisigner_count: usize,
    }

    impl BatchBaseFeeSignerEntry for TestSigner {
        fn has_txn_signature(&self) -> bool {
            self.has_txn_signature
        }

        fn multisigner_count(&self) -> usize {
            self.multisigner_count
        }
    }

    fn checked_add(left: u64, right: u64) -> Option<u64> {
        left.checked_add(right)
    }

    fn checked_mul_fee_by_usize(fee: u64, count: usize) -> Option<u64> {
        fee.checked_mul(u64::try_from(count).ok()?)
    }

    #[test]
    fn batch_calculate_base_fee_rejects_nested_batch() {
        let fee = run_batch_calculate_base_fee(
            100_000_000_000_u64,
            10,
            10,
            Some([
                TestInnerTx {
                    txn_type: TxType::BATCH,
                    fee: 10,
                },
                TestInnerTx {
                    txn_type: TxType::PAYMENT,
                    fee: 10,
                },
            ]),
            None::<[TestSigner; 0]>,
            |inner| inner.fee,
            checked_add,
            checked_mul_fee_by_usize,
        );

        assert_eq!(fee, 100_000_000_000);
    }

    #[test]
    fn batch_calculate_base_fee_rejects_too_many_inner_transactions() {
        let fee = run_batch_calculate_base_fee(
            100_000_000_000_u64,
            10,
            10,
            Some((0..=MAX_BATCH_TX_COUNT).map(|_| TestInnerTx {
                txn_type: TxType::PAYMENT,
                fee: 10,
            })),
            None::<[TestSigner; 0]>,
            |inner| inner.fee,
            checked_add,
            checked_mul_fee_by_usize,
        );

        assert_eq!(fee, 100_000_000_000);
    }

    #[test]
    fn batch_calculate_base_fee_rejects_too_many_batch_signers() {
        let fee = run_batch_calculate_base_fee(
            100_000_000_000_u64,
            10,
            10,
            Some([
                TestInnerTx {
                    txn_type: TxType::PAYMENT,
                    fee: 10,
                },
                TestInnerTx {
                    txn_type: TxType::PAYMENT,
                    fee: 10,
                },
            ]),
            Some((0..=MAX_BATCH_TX_COUNT).map(|_| TestSigner {
                has_txn_signature: true,
                multisigner_count: 0,
            })),
            |inner| inner.fee,
            checked_add,
            checked_mul_fee_by_usize,
        );

        assert_eq!(fee, 100_000_000_000);
    }

    #[test]
    fn batch_calculate_base_fee_sums_batch_base_inner_fees_and_signers() {
        let fee = run_batch_calculate_base_fee(
            100_000_000_000_u64,
            10,
            10,
            Some([
                TestInnerTx {
                    txn_type: TxType::PAYMENT,
                    fee: 10,
                },
                TestInnerTx {
                    txn_type: TxType::PAYMENT,
                    fee: 10,
                },
            ]),
            Some([
                TestSigner {
                    has_txn_signature: true,
                    multisigner_count: 0,
                },
                TestSigner {
                    has_txn_signature: false,
                    multisigner_count: 2,
                },
            ]),
            |inner| inner.fee,
            checked_add,
            checked_mul_fee_by_usize,
        );

        assert_eq!(fee, 70);
    }
}
