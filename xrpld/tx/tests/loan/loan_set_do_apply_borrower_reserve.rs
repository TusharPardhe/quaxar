//! Integration tests that pin the narrowed Rust `LoanSet.cpp::doApply()`
//! borrower-reserve guard to the current C++ behavior.

use protocol::{Ter, trans_token};
use tx::{LoanSetDoApplyBorrowerReserveFailure, check_loan_set_do_apply_borrower_reserve};

#[test]
fn tx_loan_set_do_apply_borrower_reserve_uses_pre_fee_balance_for_borrower_signed_tx() {
    let result = check_loan_set_do_apply_borrower_reserve(true, &30_u32, &1_u32, 4_u32, |_| 30_u32);

    assert_eq!(result, Ok(()));
}

#[test]
fn tx_loan_set_do_apply_borrower_reserve_uses_borrower_balance_for_counterparty_signed_tx() {
    let result =
        check_loan_set_do_apply_borrower_reserve(false, &100_u32, &29_u32, 4_u32, |_| 30_u32);

    assert_eq!(
        result,
        Err(LoanSetDoApplyBorrowerReserveFailure::InsufficientReserve)
    );
}

#[test]
fn tx_loan_set_do_apply_borrower_reserve_allows_exact_reserve() {
    let result =
        check_loan_set_do_apply_borrower_reserve(false, &1_u32, &30_u32, 4_u32, |_| 30_u32);

    assert_eq!(result, Ok(()));
}

#[test]
fn tx_loan_set_do_apply_borrower_reserve_returns_insufficient_reserve() {
    let result =
        check_loan_set_do_apply_borrower_reserve(true, &29_u32, &100_u32, 4_u32, |_| 30_u32);

    assert_eq!(
        result,
        Err(LoanSetDoApplyBorrowerReserveFailure::InsufficientReserve)
    );
    let err = result.expect_err("borrower reserve shortage should fail");
    assert_eq!(err.ter(), Ter::TEC_INSUFFICIENT_RESERVE);
    assert_eq!(trans_token(err.ter()), "tecINSUFFICIENT_RESERVE");
}
