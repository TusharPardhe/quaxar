//! `the transaction dispatch layer::invoke_calculateBaseFee(...)`.
//!
//! This ports the current higher dispatch shape above the landed specialized
//! selector and default transactor fee path.

use protocol::{Rules, TxType};

use crate::{
    HasTxnType, run_calculate_base_fee_for_txn_source, run_calculate_base_fee_for_txn_type,
    run_specialized_calculate_base_fee_for_txn_type,
};

#[allow(clippy::too_many_arguments)]
pub fn run_invoke_calculate_base_fee_for_txn_type<Fee>(
    rules: &Rules,
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
    zero_fee: impl FnOnce() -> Fee,
) -> Fee {
    run_calculate_base_fee_for_txn_type(
        rules,
        txn_type,
        move |txn_type| {
            run_specialized_calculate_base_fee_for_txn_type(
                txn_type,
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
        },
        zero_fee,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_invoke_calculate_base_fee_for_txn_source<Tx: HasTxnType + ?Sized, Fee>(
    rules: &Rules,
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
    zero_fee: impl FnOnce() -> Fee,
) -> Fee {
    run_calculate_base_fee_for_txn_source(
        rules,
        tx,
        move |txn_type| {
            run_specialized_calculate_base_fee_for_txn_type(
                txn_type,
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
        },
        zero_fee,
    )
}

#[cfg(test)]
mod tests {
    use protocol::{Rules, TxType};

    use super::{
        run_invoke_calculate_base_fee_for_txn_source, run_invoke_calculate_base_fee_for_txn_type,
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
        run_invoke_calculate_base_fee_for_txn_type(
            &Rules::new(std::iter::empty()),
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
            || 9_u64,
            || 10_u64,
        )
    }

    #[test]
    fn invoke_calculate_base_fee_routes_specialized_change_family() {
        assert_eq!(dispatch_for(TxType::AMENDMENT), 0);
        assert_eq!(dispatch_for(TxType::FEE), 0);
        assert_eq!(dispatch_for(TxType::UNL_MODIFY), 0);
    }

    #[test]
    fn invoke_calculate_base_fee_routes_default_for_unspecialized_types() {
        assert_eq!(dispatch_for(TxType::PAYMENT), 9);
    }

    #[test]
    fn invoke_calculate_base_fee_maps_unknown_txn_type_to_zero() {
        assert_eq!(dispatch_for(TxType::HOOK_SET), 10);
    }

    #[test]
    fn invoke_calculate_base_fee_source_wrapper_uses_txn_type() {
        let tx = TestTx {
            txn_type: TxType::BATCH,
        };

        let fee = run_invoke_calculate_base_fee_for_txn_source(
            &Rules::new(std::iter::empty()),
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
            || 10_u64,
        );

        assert_eq!(fee, 7);
    }
}
