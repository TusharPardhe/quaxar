//! Integration tests that pin the narrowed Rust `LoanSet.cpp::doApply()`
//! computed-values invariant block to the current C++ behavior.

use protocol::{Ter, trans_token};
use tx::{
    LoanSetDoApplyComputedValues, LoanSetDoApplyComputedValuesFailure,
    check_loan_set_do_apply_computed_values,
};

fn valid_values() -> LoanSetDoApplyComputedValues<i64> {
    LoanSetDoApplyComputedValues {
        management_fee_due: 0,
        total_value_outstanding: 100,
        periodic_payment: 10,
    }
}

#[test]
fn tx_loan_set_do_apply_computed_values_accepts_valid_values() {
    let result = check_loan_set_do_apply_computed_values(&valid_values(), &0);

    assert_eq!(result, Ok(()));
}

#[test]
fn tx_loan_set_do_apply_computed_values_returns_internal_for_negative_management_fee() {
    let result = check_loan_set_do_apply_computed_values(
        &LoanSetDoApplyComputedValues {
            management_fee_due: -1,
            ..valid_values()
        },
        &0,
    );

    assert_eq!(
        result,
        Err(LoanSetDoApplyComputedValuesFailure::InvalidComputedValues {
            management_fee_due_display: "-1".to_string(),
            total_value_outstanding_display: "100".to_string(),
            periodic_payment_display: "10".to_string(),
        })
    );
}

#[test]
fn tx_loan_set_do_apply_computed_values_returns_internal_for_zero_total_value() {
    let result = check_loan_set_do_apply_computed_values(
        &LoanSetDoApplyComputedValues {
            total_value_outstanding: 0,
            ..valid_values()
        },
        &0,
    );

    let err = result.expect_err("zero total value should fail");
    assert_eq!(err.ter(), Ter::TEC_INTERNAL);
    assert_eq!(trans_token(err.ter()), "tecINTERNAL");
    assert_eq!(
        err.warning_message(),
        "Computed loan properties are invalid. Does not compute. Management fee: 0. Total Value: 0. PeriodicPayment: 10"
    );
}

#[test]
fn tx_loan_set_do_apply_computed_values_returns_internal_for_zero_periodic_payment() {
    let result = check_loan_set_do_apply_computed_values(
        &LoanSetDoApplyComputedValues {
            periodic_payment: 0,
            ..valid_values()
        },
        &0,
    );

    let err = result.expect_err("zero periodic payment should fail");
    assert_eq!(err.ter(), Ter::TEC_INTERNAL);
    assert_eq!(trans_token(err.ter()), "tecINTERNAL");
    assert_eq!(
        err.warning_message(),
        "Computed loan properties are invalid. Does not compute. Management fee: 0. Total Value: 100. PeriodicPayment: 0"
    );
}
