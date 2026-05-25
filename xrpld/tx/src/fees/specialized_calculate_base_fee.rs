//! Current specialized `calculateBaseFee(...)` selector above the landed
//! custom fee wrappers.
//!
//! This ports the deterministic higher selection among the currently
//! specialized fee owners.

use protocol::TxType;

use crate::HasTxnType;

#[allow(clippy::too_many_arguments)]
pub fn run_specialized_calculate_base_fee_for_txn_type<Fee>(
    txn_type: TxType,
    run_change_calculate_base_fee: impl FnOnce() -> Fee,
    run_set_regular_key_calculate_base_fee: impl FnOnce() -> Fee,
    run_account_delete_calculate_base_fee: impl FnOnce() -> Fee,
    run_amm_create_calculate_base_fee: impl FnOnce() -> Fee,
    run_escrow_finish_calculate_base_fee: impl FnOnce() -> Fee,
    run_loan_set_calculate_base_fee: impl FnOnce() -> Fee,
    run_loan_pay_calculate_base_fee: impl FnOnce() -> Fee,
    run_batch_calculate_base_fee: impl FnOnce() -> Fee,
    run_ledger_state_fix_calculate_base_fee: impl FnOnce() -> Fee,
    run_default_calculate_base_fee: impl FnOnce() -> Fee,
) -> Fee {
    match txn_type {
        TxType::AMENDMENT | TxType::FEE | TxType::UNL_MODIFY => run_change_calculate_base_fee(),
        TxType::REGULAR_KEY_SET => run_set_regular_key_calculate_base_fee(),
        TxType::ACCOUNT_DELETE => run_account_delete_calculate_base_fee(),
        TxType::AMM_CREATE => run_amm_create_calculate_base_fee(),
        TxType::ESCROW_FINISH => run_escrow_finish_calculate_base_fee(),
        TxType::LOAN_SET => run_loan_set_calculate_base_fee(),
        TxType::LOAN_PAY => run_loan_pay_calculate_base_fee(),
        TxType::BATCH => run_batch_calculate_base_fee(),
        TxType::LEDGER_STATE_FIX => run_ledger_state_fix_calculate_base_fee(),
        _ => run_default_calculate_base_fee(),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn run_specialized_calculate_base_fee_for_txn_source<Tx: HasTxnType + ?Sized, Fee>(
    tx: &Tx,
    run_change_calculate_base_fee: impl FnOnce() -> Fee,
    run_set_regular_key_calculate_base_fee: impl FnOnce() -> Fee,
    run_account_delete_calculate_base_fee: impl FnOnce() -> Fee,
    run_amm_create_calculate_base_fee: impl FnOnce() -> Fee,
    run_escrow_finish_calculate_base_fee: impl FnOnce() -> Fee,
    run_loan_set_calculate_base_fee: impl FnOnce() -> Fee,
    run_loan_pay_calculate_base_fee: impl FnOnce() -> Fee,
    run_batch_calculate_base_fee: impl FnOnce() -> Fee,
    run_ledger_state_fix_calculate_base_fee: impl FnOnce() -> Fee,
    run_default_calculate_base_fee: impl FnOnce() -> Fee,
) -> Fee {
    run_specialized_calculate_base_fee_for_txn_type(
        tx.txn_type(),
        run_change_calculate_base_fee,
        run_set_regular_key_calculate_base_fee,
        run_account_delete_calculate_base_fee,
        run_amm_create_calculate_base_fee,
        run_escrow_finish_calculate_base_fee,
        run_loan_set_calculate_base_fee,
        run_loan_pay_calculate_base_fee,
        run_batch_calculate_base_fee,
        run_ledger_state_fix_calculate_base_fee,
        run_default_calculate_base_fee,
    )
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use protocol::TxType;

    use super::{
        run_specialized_calculate_base_fee_for_txn_source,
        run_specialized_calculate_base_fee_for_txn_type,
    };
    use crate::HasTxnType;

    struct TestTx {
        txn_type: TxType,
    }

    impl HasTxnType for TestTx {
        fn txn_type(&self) -> TxType {
            self.txn_type
        }
    }

    fn dispatch_for(txn_type: TxType) -> u64 {
        let default_called = Cell::new(false);

        let fee = run_specialized_calculate_base_fee_for_txn_type(
            txn_type,
            || 0_u64,
            || 1_u64,
            || 2_u64,
            || 3_u64,
            || 4_u64,
            || 5_u64,
            || 6_u64,
            || 7_u64,
            || 8_u64,
            || {
                default_called.set(true);
                9_u64
            },
        );

        if txn_type != TxType::PAYMENT {
            assert!(!default_called.get());
        }

        fee
    }

    #[test]
    fn specialized_calculate_base_fee_routes_change_family() {
        assert_eq!(dispatch_for(TxType::AMENDMENT), 0);
        assert_eq!(dispatch_for(TxType::FEE), 0);
        assert_eq!(dispatch_for(TxType::UNL_MODIFY), 0);
    }

    #[test]
    fn specialized_calculate_base_fee_routes_each_landed_custom_fee_owner() {
        assert_eq!(dispatch_for(TxType::REGULAR_KEY_SET), 1);
        assert_eq!(dispatch_for(TxType::ACCOUNT_DELETE), 2);
        assert_eq!(dispatch_for(TxType::AMM_CREATE), 3);
        assert_eq!(dispatch_for(TxType::ESCROW_FINISH), 4);
        assert_eq!(dispatch_for(TxType::LOAN_SET), 5);
        assert_eq!(dispatch_for(TxType::LOAN_PAY), 6);
        assert_eq!(dispatch_for(TxType::BATCH), 7);
        assert_eq!(dispatch_for(TxType::LEDGER_STATE_FIX), 8);
    }

    #[test]
    fn specialized_calculate_base_fee_falls_back_to_default_for_unspecialized_types() {
        assert_eq!(dispatch_for(TxType::PAYMENT), 9);
    }

    #[test]
    fn specialized_calculate_base_fee_source_wrapper_uses_txn_type() {
        let tx = TestTx {
            txn_type: TxType::ESCROW_FINISH,
        };

        let fee = run_specialized_calculate_base_fee_for_txn_source(
            &tx,
            || 0_u64,
            || 1_u64,
            || 2_u64,
            || 3_u64,
            || 4_u64,
            || 5_u64,
            || 6_u64,
            || 7_u64,
            || 8_u64,
            || 9_u64,
        );

        assert_eq!(fee, 4);
    }
}
