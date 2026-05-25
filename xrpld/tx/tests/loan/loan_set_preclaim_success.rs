//! Integration tests that pin the narrowed Rust outer
//! `LoanSet.cpp::preclaim(...)` success shell to the current C++ behavior.

use protocol::trans_token;
use tx::run_loan_set_preclaim_success;

#[test]
fn tx_loan_set_preclaim_success_returns_tes_success() {
    let result = run_loan_set_preclaim_success();

    assert_eq!(result, protocol::Ter::TES_SUCCESS);
    assert_eq!(trans_token(result), "tesSUCCESS");
}
