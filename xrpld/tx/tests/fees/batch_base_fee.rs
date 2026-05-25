//! Integration tests that pin the narrowed `Batch::calculateBaseFee(...)`
//! wrapper to the current C++ aggregate-fee behavior.

use protocol::TxType;
use tx::{
    BatchBaseFeeInnerTransaction, BatchBaseFeeSignerEntry, MAX_BATCH_TX_COUNT,
    run_batch_calculate_base_fee,
};

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
fn batch_calculate_base_fee_rejects_nested_inner_batch() {
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
fn batch_calculate_base_fee_adds_batch_base_inner_fees_and_signer_fees() {
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
