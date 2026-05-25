//! Integration tests that pin the narrowed Rust `LoanSet.cpp::doApply()`
//! debt-maximum guard to the current C++ behavior.

use protocol::{Ter, trans_token};
use tx::{
    LOAN_SET_DO_APPLY_DEBT_MAXIMUM_EXCEEDED_WARNING, LoanSetDoApplyDebtMaximumFailure,
    check_loan_set_do_apply_debt_maximum,
};

#[test]
fn tx_loan_set_do_apply_debt_maximum_treats_zero_as_unlimited() {
    let result = check_loan_set_do_apply_debt_maximum(&0_u32, &1_000_u32, &0_u32);

    assert_eq!(result, Ok(()));
}

#[test]
fn tx_loan_set_do_apply_debt_maximum_allows_equal_total() {
    let result = check_loan_set_do_apply_debt_maximum(&100_u32, &100_u32, &0_u32);

    assert_eq!(result, Ok(()));
}

#[test]
fn tx_loan_set_do_apply_debt_maximum_returns_limit_exceeded() {
    let result = check_loan_set_do_apply_debt_maximum(&99_u32, &100_u32, &0_u32);

    assert_eq!(
        result,
        Err(LoanSetDoApplyDebtMaximumFailure::DebtMaximumExceeded)
    );
    let err = result.expect_err("debt maximum should fail");
    assert_eq!(err.ter(), Ter::TEC_LIMIT_EXCEEDED);
    assert_eq!(trans_token(err.ter()), "tecLIMIT_EXCEEDED");
    assert_eq!(
        err.warning_message(),
        LOAN_SET_DO_APPLY_DEBT_MAXIMUM_EXCEEDED_WARNING
    );
}
