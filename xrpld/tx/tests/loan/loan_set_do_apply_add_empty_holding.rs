//! Integration tests that pin the narrowed Rust `LoanSet.cpp::doApply()`
//! duplicate-tolerant `addEmptyHolding(...)` wrapper to the current C++
//! behavior.

use protocol::{Ter, trans_token};
use tx::run_loan_set_do_apply_add_empty_holding;

#[test]
fn tx_loan_set_do_apply_add_empty_holding_returns_success() {
    let result = run_loan_set_do_apply_add_empty_holding(|| Ter::TES_SUCCESS);

    assert_eq!(result, Ter::TES_SUCCESS);
}

#[test]
fn tx_loan_set_do_apply_add_empty_holding_ignores_duplicate() {
    let result = run_loan_set_do_apply_add_empty_holding(|| Ter::TEC_DUPLICATE);

    assert_eq!(result, Ter::TES_SUCCESS);
}

#[test]
fn tx_loan_set_do_apply_add_empty_holding_returns_other_failure_unchanged() {
    let result = run_loan_set_do_apply_add_empty_holding(|| Ter::TER_NO_RIPPLE);

    assert_eq!(result, Ter::TER_NO_RIPPLE);
    assert_eq!(trans_token(result), "terNO_RIPPLE");
}
