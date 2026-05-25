//! Integration tests that pin the narrowed Rust `LoanSet.cpp::doApply()`
//! cover-rate minimum guard to the current C++ behavior.

use protocol::{Ter, trans_token};
use tx::{
    LOAN_SET_DO_APPLY_INSUFFICIENT_FIRST_LOSS_CAPITAL_WARNING,
    LoanSetDoApplyCoverRateMinimumFailure, check_loan_set_do_apply_cover_rate_minimum,
};

#[test]
fn tx_loan_set_do_apply_cover_rate_minimum_allows_equal_required_cover() {
    let result =
        check_loan_set_do_apply_cover_rate_minimum(&100_u32, &1_000_u32, 10_000_u32, |_, _| {
            100_u32
        });

    assert_eq!(result, Ok(()));
}

#[test]
fn tx_loan_set_do_apply_cover_rate_minimum_returns_insufficient_funds() {
    let result =
        check_loan_set_do_apply_cover_rate_minimum(&99_u32, &1_000_u32, 10_000_u32, |_, _| 100_u32);

    assert_eq!(
        result,
        Err(LoanSetDoApplyCoverRateMinimumFailure::InsufficientFirstLossCapital)
    );
    let err = result.expect_err("insufficient first-loss capital should fail");
    assert_eq!(err.ter(), Ter::TEC_INSUFFICIENT_FUNDS);
    assert_eq!(trans_token(err.ter()), "tecINSUFFICIENT_FUNDS");
    assert_eq!(
        err.warning_message(),
        LOAN_SET_DO_APPLY_INSUFFICIENT_FIRST_LOSS_CAPITAL_WARNING
    );
}

#[test]
fn tx_loan_set_do_apply_cover_rate_minimum_computes_required_cover_from_new_debt_total() {
    let mut seen = Vec::new();

    let result = check_loan_set_do_apply_cover_rate_minimum(
        &99_u32,
        &1_000_u32,
        10_000_u32,
        |new_debt_total, cover_rate_minimum| {
            seen.push(format!("{new_debt_total}:{cover_rate_minimum}"));
            100_u32
        },
    );

    assert_eq!(
        result,
        Err(LoanSetDoApplyCoverRateMinimumFailure::InsufficientFirstLossCapital)
    );
    assert_eq!(seen, vec!["1000:10000".to_string()]);
}
