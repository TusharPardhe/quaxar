//! Integration tests that pin the narrowed Rust `LoanSet.cpp::doApply()`
//! non-zero origination-fee owner-side holding step to the current C++
//! behavior.

use protocol::{Ter, trans_token};
use tx::run_loan_set_do_apply_origination_fee_add_empty_holding;

#[test]
fn tx_loan_set_do_apply_origination_fee_add_empty_holding_skips_setup_for_zero_fee() {
    let result = run_loan_set_do_apply_origination_fee_add_empty_holding(&0_u32, &0_u32, || {
        Ter::TEC_INSUFFICIENT_RESERVE
    });

    assert_eq!(result, Ter::TES_SUCCESS);
}

#[test]
fn tx_loan_set_do_apply_origination_fee_add_empty_holding_ignores_duplicate() {
    let result = run_loan_set_do_apply_origination_fee_add_empty_holding(&1_u32, &0_u32, || {
        Ter::TEC_DUPLICATE
    });

    assert_eq!(result, Ter::TES_SUCCESS);
}

#[test]
fn tx_loan_set_do_apply_origination_fee_add_empty_holding_returns_insufficient_reserve_unchanged() {
    let result = run_loan_set_do_apply_origination_fee_add_empty_holding(&1_u32, &0_u32, || {
        Ter::TEC_INSUFFICIENT_RESERVE
    });

    assert_eq!(result, Ter::TEC_INSUFFICIENT_RESERVE);
    assert_eq!(trans_token(result), "tecINSUFFICIENT_RESERVE");
}

#[test]
fn tx_loan_set_do_apply_origination_fee_add_empty_holding_returns_owner_line_reserve_failure_unchanged()
 {
    let result = run_loan_set_do_apply_origination_fee_add_empty_holding(&1_u32, &0_u32, || {
        Ter::TEC_NO_LINE_INSUF_RESERVE
    });

    assert_eq!(result, Ter::TEC_NO_LINE_INSUF_RESERVE);
    assert_eq!(trans_token(result), "tecNO_LINE_INSUF_RESERVE");
}
