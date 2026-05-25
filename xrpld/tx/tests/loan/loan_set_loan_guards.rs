//! Integration tests that pin the narrowed Rust `LendingHelpers.cpp`
//! `checkLoanGuards(...)` helper to the current C++ behavior.

use protocol::{Ter, trans_token};
use tx::{LoanSetLoanGuardProperties, LoanSetLoanGuardsFailure, check_loan_set_loan_guards};

fn base_properties() -> LoanSetLoanGuardProperties<i64> {
    LoanSetLoanGuardProperties {
        periodic_payment: 10,
        total_value_outstanding: 110,
        loan_scale: 4,
        first_payment_principal: 1,
    }
}

#[test]
fn tx_loan_set_loan_guards_returns_precision_loss_when_interest_is_expected_but_missing() {
    let result = check_loan_set_loan_guards(
        &"USD",
        &100,
        true,
        12,
        &LoanSetLoanGuardProperties {
            total_value_outstanding: 100,
            ..base_properties()
        },
        &0,
        |_, _, _| unreachable!("rounding should not run"),
        |_, _| unreachable!("payment count should not run"),
    );

    assert_eq!(
        result,
        Err(LoanSetLoanGuardsFailure::InterestExpectedButNone {
            principal_requested_display: "100".to_string(),
        })
    );
}

#[test]
fn tx_loan_set_loan_guards_returns_internal_when_interest_is_present_but_not_expected() {
    let result = check_loan_set_loan_guards(
        &"USD",
        &100,
        false,
        12,
        &base_properties(),
        &0,
        |_, _, _| unreachable!("rounding should not run"),
        |_, _| unreachable!("payment count should not run"),
    );

    let err = result.expect_err("unexpected interest should fail");
    assert_eq!(err.ter(), Ter::TEC_INTERNAL);
    assert_eq!(trans_token(err.ter()), "tecINTERNAL");
    assert_eq!(
        err.warning_message(),
        "Loan for 100 with no interest has interest due"
    );
}

#[test]
fn tx_loan_set_loan_guards_returns_precision_loss_when_rounded_payment_is_zero() {
    let result = check_loan_set_loan_guards(
        &"USD",
        &100,
        false,
        12,
        &LoanSetLoanGuardProperties {
            total_value_outstanding: 100,
            ..base_properties()
        },
        &0,
        |_, _, _| 0,
        |_, _| unreachable!("payment count should not run"),
    );

    let err = result.expect_err("rounded payment zero should fail");
    assert_eq!(err.ter(), Ter::TEC_PRECISION_LOSS);
    assert_eq!(trans_token(err.ter()), "tecPRECISION_LOSS");
    assert_eq!(
        err.warning_message(),
        "Loan Periodic payment (10) rounds to 0. "
    );
}

#[test]
fn tx_loan_set_loan_guards_returns_precision_loss_when_payment_count_mismatches() {
    let result = check_loan_set_loan_guards(
        &"USD",
        &100,
        false,
        12,
        &LoanSetLoanGuardProperties {
            total_value_outstanding: 100,
            ..base_properties()
        },
        &0,
        |_, _, _| 11,
        |_, _| 11,
    );

    let err = result.expect_err("payment-count mismatch should fail");
    assert_eq!(err.ter(), Ter::TEC_PRECISION_LOSS);
    assert_eq!(
        err.warning_message(),
        "Loan Periodic payment (10) rounding (11) on a total value of 100 can not complete the loan in the specified number of payments (11 != 12)"
    );
}

#[test]
fn tx_loan_set_loan_guards_returns_success_when_all_guards_pass() {
    let result = check_loan_set_loan_guards(
        &"USD",
        &100,
        false,
        12,
        &LoanSetLoanGuardProperties {
            total_value_outstanding: 100,
            ..base_properties()
        },
        &0,
        |_, _, _| 11,
        |_, _| 12,
    );

    assert_eq!(result, Ok(()));
}
