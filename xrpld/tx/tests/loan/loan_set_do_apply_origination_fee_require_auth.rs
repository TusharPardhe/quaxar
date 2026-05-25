//! Integration tests that pin the narrowed Rust `LoanSet.cpp::doApply()`
//! broker-owner auth step after the origination-fee holding setup to the
//! current C++ behavior.

use protocol::{Ter, trans_token};
use tx::run_loan_set_do_apply_origination_fee_require_auth;

#[test]
fn tx_loan_set_do_apply_origination_fee_require_auth_checks_auth_for_zero_fee() {
    let result = run_loan_set_do_apply_origination_fee_require_auth(
        &0_u32,
        &0_u32,
        || Ter::TEC_INSUFFICIENT_RESERVE,
        || Ter::TEC_NO_AUTH,
    );

    assert_eq!(result, Ter::TEC_NO_AUTH);
    assert_eq!(trans_token(result), "tecNO_AUTH");
}

#[test]
fn tx_loan_set_do_apply_origination_fee_require_auth_returns_no_auth_unchanged() {
    let result = run_loan_set_do_apply_origination_fee_require_auth(
        &1_u32,
        &0_u32,
        || Ter::TES_SUCCESS,
        || Ter::TEC_NO_AUTH,
    );

    assert_eq!(result, Ter::TEC_NO_AUTH);
    assert_eq!(trans_token(result), "tecNO_AUTH");
}

#[test]
fn tx_loan_set_do_apply_origination_fee_require_auth_returns_insufficient_reserve_unchanged() {
    let result = run_loan_set_do_apply_origination_fee_require_auth(
        &1_u32,
        &0_u32,
        || Ter::TEC_INSUFFICIENT_RESERVE,
        || Ter::TEC_NO_AUTH,
    );

    assert_eq!(result, Ter::TEC_INSUFFICIENT_RESERVE);
    assert_eq!(trans_token(result), "tecINSUFFICIENT_RESERVE");
}

#[test]
fn tx_loan_set_do_apply_origination_fee_require_auth_returns_owner_line_reserve_failure_unchanged()
{
    let result = run_loan_set_do_apply_origination_fee_require_auth(
        &1_u32,
        &0_u32,
        || Ter::TEC_NO_LINE_INSUF_RESERVE,
        || Ter::TEC_NO_AUTH,
    );

    assert_eq!(result, Ter::TEC_NO_LINE_INSUF_RESERVE);
    assert_eq!(trans_token(result), "tecNO_LINE_INSUF_RESERVE");
}
