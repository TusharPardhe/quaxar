//! Integration tests that pin the narrowed lending `calculateBaseFee(...)`
//! selector to the current C++ transaction-type subset behavior.

use protocol::TxType;
use tx::{
    HasTxnType, UnknownTransactionType, run_lending_calculate_base_fee_for_txn_source,
    run_lending_calculate_base_fee_for_txn_type,
};

struct TestTx {
    txn_type: TxType,
}

impl HasTxnType for TestTx {
    fn txn_type(&self) -> TxType {
        self.txn_type
    }
}

#[test]
fn lending_calculate_base_fee_selects_loan_set_fee() {
    let fee = run_lending_calculate_base_fee_for_txn_type(
        TxType::LOAN_SET,
        || 22_u64,
        || panic!("loan-set selection should skip loan-pay fee"),
    );

    assert_eq!(fee, Ok(22));
}

#[test]
fn lending_calculate_base_fee_selects_loan_pay_fee() {
    let fee = run_lending_calculate_base_fee_for_txn_type(
        TxType::LOAN_PAY,
        || panic!("loan-pay selection should skip loan-set fee"),
        || 33_u64,
    );

    assert_eq!(fee, Ok(33));
}

#[test]
fn lending_calculate_base_fee_source_wrapper_preserves_unknowns_subset() {
    let tx = TestTx {
        txn_type: TxType::PAYMENT,
    };

    let fee = run_lending_calculate_base_fee_for_txn_source(
        &tx,
        || panic!("unknown type should skip loan-set fee"),
        || panic!("unknown type should skip loan-pay fee"),
    );

    assert_eq!(fee, Err(UnknownTransactionType::new(TxType::PAYMENT)));
}
