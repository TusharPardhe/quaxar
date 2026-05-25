//! Integration tests that pin the narrowed Rust
//! `LoanSet.cpp::doApply()` borrower-owner `dirLink(...)` wrapper to the
//! current C++ behavior.

use protocol::{Ter, trans_token};
use tx::run_loan_set_do_apply_borrower_owner_dir_link;

#[test]
fn tx_loan_set_do_apply_borrower_owner_dir_link_returns_success_unchanged() {
    let result = run_loan_set_do_apply_borrower_owner_dir_link(|| Ter::TES_SUCCESS);

    assert_eq!(result, Ter::TES_SUCCESS);
}

#[test]
fn tx_loan_set_do_apply_borrower_owner_dir_link_returns_dir_full_unchanged() {
    let result = run_loan_set_do_apply_borrower_owner_dir_link(|| Ter::TEC_DIR_FULL);

    assert_eq!(result, Ter::TEC_DIR_FULL);
    assert_eq!(trans_token(result), "tecDIR_FULL");
}

#[test]
fn tx_loan_set_do_apply_borrower_owner_dir_link_calls_the_helper_once() {
    let mut calls = 0_u32;

    let result = run_loan_set_do_apply_borrower_owner_dir_link(|| {
        calls += 1;
        Ter::TEC_DIR_FULL
    });

    assert_eq!(result, Ter::TEC_DIR_FULL);
    assert_eq!(calls, 1);
}
