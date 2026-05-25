//! Integration tests that pin the narrowed Rust `LoanSet.cpp::doApply()`
//! `accountSendMulti(...)` transfer step after the origination-fee setup to
//! the current C++ behavior.

use protocol::{Ter, trans_token};
use tx::run_loan_set_do_apply_origination_fee_account_send_multi;

#[test]
fn tx_loan_set_do_apply_origination_fee_account_send_multi_runs_transfer_for_zero_fee() {
    let result = run_loan_set_do_apply_origination_fee_account_send_multi(
        &0_u32,
        &0_u32,
        || Ter::TEC_INSUFFICIENT_RESERVE,
        || Ter::TES_SUCCESS,
        || Ter::TEC_NO_AUTH,
    );

    assert_eq!(result, Ter::TEC_NO_AUTH);
    assert_eq!(trans_token(result), "tecNO_AUTH");
}

#[test]
fn tx_loan_set_do_apply_origination_fee_account_send_multi_returns_no_auth_unchanged() {
    let result = run_loan_set_do_apply_origination_fee_account_send_multi(
        &1_u32,
        &0_u32,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TEC_NO_AUTH,
    );

    assert_eq!(result, Ter::TEC_NO_AUTH);
    assert_eq!(trans_token(result), "tecNO_AUTH");
}

#[test]
fn tx_loan_set_do_apply_origination_fee_account_send_multi_returns_insufficient_reserve_unchanged()
{
    let result = run_loan_set_do_apply_origination_fee_account_send_multi(
        &1_u32,
        &0_u32,
        || Ter::TEC_INSUFFICIENT_RESERVE,
        || Ter::TES_SUCCESS,
        || Ter::TEC_NO_AUTH,
    );

    assert_eq!(result, Ter::TEC_INSUFFICIENT_RESERVE);
    assert_eq!(trans_token(result), "tecINSUFFICIENT_RESERVE");
}

#[test]
fn tx_loan_set_do_apply_origination_fee_account_send_multi_short_circuits_on_auth_failure() {
    let result = run_loan_set_do_apply_origination_fee_account_send_multi(
        &1_u32,
        &0_u32,
        || Ter::TES_SUCCESS,
        || Ter::TEC_NO_AUTH,
        || Ter::TES_SUCCESS,
    );

    assert_eq!(result, Ter::TEC_NO_AUTH);
    assert_eq!(trans_token(result), "tecNO_AUTH");
}
