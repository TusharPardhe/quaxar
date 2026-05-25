//! Integration tests that pin the narrowed Rust
//! `LoanSet.cpp::doApply()` `setLoanField(...)` transaction-copy shell to the
//! current C++ behavior.

use tx::{LoanSetDoApplyLoanTransactionField, run_loan_set_do_apply_loan_transaction_fields};

#[test]
fn tx_loan_set_do_apply_loan_transaction_fields_uses_current_cpp_copy_order() {
    let mut seen = Vec::new();

    run_loan_set_do_apply_loan_transaction_fields(30, |field, default_value| {
        seen.push((field, default_value));
    });

    assert_eq!(
        seen,
        vec![
            (LoanSetDoApplyLoanTransactionField::LoanOriginationFee, None),
            (LoanSetDoApplyLoanTransactionField::LoanServiceFee, None),
            (LoanSetDoApplyLoanTransactionField::LatePaymentFee, None),
            (LoanSetDoApplyLoanTransactionField::ClosePaymentFee, None),
            (LoanSetDoApplyLoanTransactionField::OverpaymentFee, None),
            (LoanSetDoApplyLoanTransactionField::InterestRate, None),
            (LoanSetDoApplyLoanTransactionField::LateInterestRate, None),
            (LoanSetDoApplyLoanTransactionField::CloseInterestRate, None),
            (
                LoanSetDoApplyLoanTransactionField::OverpaymentInterestRate,
                None
            ),
            (LoanSetDoApplyLoanTransactionField::GracePeriod, Some(30)),
        ]
    );
}

#[test]
fn tx_loan_set_do_apply_loan_transaction_fields_only_defaults_grace_period() {
    let mut seen_non_none = Vec::new();

    run_loan_set_do_apply_loan_transaction_fields(45, |field, default_value| {
        if default_value.is_some() {
            seen_non_none.push((field, default_value));
        }
    });

    assert_eq!(
        seen_non_none,
        vec![(LoanSetDoApplyLoanTransactionField::GracePeriod, Some(45))]
    );
}

#[test]
fn tx_loan_set_do_apply_loan_transaction_fields_keeps_zero_default_grace_period() {
    let mut grace_default = None;

    run_loan_set_do_apply_loan_transaction_fields(0, |field, default_value| {
        if field == LoanSetDoApplyLoanTransactionField::GracePeriod {
            grace_default = default_value;
        }
    });

    assert_eq!(grace_default, Some(0));
}
