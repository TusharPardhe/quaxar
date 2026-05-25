//! Integration tests that pin the narrowed Rust `LoanSet.cpp::doApply()`
//! `requireAuth(...)` wrapper to the current C++ behavior.

use protocol::{Ter, trans_token};
use tx::run_loan_set_do_apply_require_auth;

#[test]
fn tx_loan_set_do_apply_require_auth_returns_success_unchanged() {
    let result = run_loan_set_do_apply_require_auth(|| Ter::TES_SUCCESS);

    assert_eq!(result, Ter::TES_SUCCESS);
}

#[test]
fn tx_loan_set_do_apply_require_auth_returns_no_ripple_unchanged() {
    let result = run_loan_set_do_apply_require_auth(|| Ter::TER_NO_RIPPLE);

    assert_eq!(result, Ter::TER_NO_RIPPLE);
    assert_eq!(trans_token(result), "terNO_RIPPLE");
}

#[test]
fn tx_loan_set_do_apply_require_auth_returns_no_line_insuf_reserve_unchanged() {
    let result = run_loan_set_do_apply_require_auth(|| Ter::TEC_NO_LINE_INSUF_RESERVE);

    assert_eq!(result, Ter::TEC_NO_LINE_INSUF_RESERVE);
    assert_eq!(trans_token(result), "tecNO_LINE_INSUF_RESERVE");
}
