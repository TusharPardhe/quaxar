//! Current lending-only `calculateBaseFee(...)` transaction-type selection.
//!
//! This ports the deterministic subset behavior around:
//!
//! - selecting `LoanSet::calculateBaseFee(...)` for `ttLOAN_SET`,
//! - selecting `LoanPay::calculateBaseFee(...)` for `ttLOAN_PAY`, and
//! - preserving the current "unknown within this lending subset"
//!   behavior for all other transaction types.

use protocol::TxType;

use crate::{HasTxnType, UnknownTransactionType};

pub fn run_lending_calculate_base_fee_for_txn_type<Fee>(
    txn_type: TxType,
    run_loan_set_calculate_base_fee: impl FnOnce() -> Fee,
    run_loan_pay_calculate_base_fee: impl FnOnce() -> Fee,
) -> Result<Fee, UnknownTransactionType<TxType>> {
    match txn_type {
        TxType::LOAN_SET => Ok(run_loan_set_calculate_base_fee()),
        TxType::LOAN_PAY => Ok(run_loan_pay_calculate_base_fee()),
        other => Err(UnknownTransactionType::new(other)),
    }
}

pub fn run_lending_calculate_base_fee_for_txn_source<Tx: HasTxnType + ?Sized, Fee>(
    tx: &Tx,
    run_loan_set_calculate_base_fee: impl FnOnce() -> Fee,
    run_loan_pay_calculate_base_fee: impl FnOnce() -> Fee,
) -> Result<Fee, UnknownTransactionType<TxType>> {
    run_lending_calculate_base_fee_for_txn_type(
        tx.txn_type(),
        run_loan_set_calculate_base_fee,
        run_loan_pay_calculate_base_fee,
    )
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use protocol::TxType;

    use super::{
        run_lending_calculate_base_fee_for_txn_source, run_lending_calculate_base_fee_for_txn_type,
    };
    use crate::{HasTxnType, UnknownTransactionType};

    struct TestTx {
        txn_type: TxType,
    }

    impl HasTxnType for TestTx {
        fn txn_type(&self) -> TxType {
            self.txn_type
        }
    }

    #[test]
    fn lending_calculate_base_fee_selects_loan_set_path() {
        let loan_pay_called = Cell::new(false);

        let fee = run_lending_calculate_base_fee_for_txn_type(
            TxType::LOAN_SET,
            || 22_u64,
            || {
                loan_pay_called.set(true);
                99_u64
            },
        );

        assert_eq!(fee, Ok(22));
        assert!(!loan_pay_called.get());
    }

    #[test]
    fn lending_calculate_base_fee_selects_loan_pay_path() {
        let loan_set_called = Cell::new(false);

        let fee = run_lending_calculate_base_fee_for_txn_type(
            TxType::LOAN_PAY,
            || {
                loan_set_called.set(true);
                22_u64
            },
            || 33_u64,
        );

        assert_eq!(fee, Ok(33));
        assert!(!loan_set_called.get());
    }

    #[test]
    fn lending_calculate_base_fee_preserves_unknowns_for_non_lending_types_subset() {
        let fee =
            run_lending_calculate_base_fee_for_txn_type(TxType::PAYMENT, || 22_u64, || 33_u64);

        assert_eq!(fee, Err(UnknownTransactionType::new(TxType::PAYMENT)));
    }

    #[test]
    fn lending_calculate_base_fee_source_wrapper_uses_txn_type() {
        let tx = TestTx {
            txn_type: TxType::LOAN_PAY,
        };

        let fee = run_lending_calculate_base_fee_for_txn_source(&tx, || 22_u64, || 33_u64);

        assert_eq!(fee, Ok(33));
    }
}
