//! Integration tests that pin the narrowed Rust `LoanSet.cpp::preclaim(...)`
//! borrower-account existence branch to the current C++ behavior.

use protocol::{Ter, trans_token};
use tx::{
    LOAN_SET_BORROWER_DOES_NOT_EXIST_WARNING, LoanSetPreclaimBorrowerFailure,
    check_loan_set_preclaim_borrower,
};

#[test]
fn tx_loan_set_preclaim_borrower_returns_loaded_account_unchanged() {
    let result = check_loan_set_preclaim_borrower(&"borrower", |borrower| {
        assert_eq!(*borrower, "borrower");
        Some("borrower-sle")
    });

    assert_eq!(result, Ok("borrower-sle"));
}

#[test]
fn tx_loan_set_preclaim_borrower_returns_no_account() {
    let result = check_loan_set_preclaim_borrower(&"missing-borrower", |_| None::<&'static str>);

    assert_eq!(
        result,
        Err(LoanSetPreclaimBorrowerFailure::BorrowerDoesNotExist)
    );
    let err = result.unwrap_err();
    assert_eq!(err.ter(), Ter::TER_NO_ACCOUNT);
    assert_eq!(trans_token(err.ter()), "terNO_ACCOUNT");
    assert_eq!(
        err.warning_message(),
        LOAN_SET_BORROWER_DOES_NOT_EXIST_WARNING
    );
}

#[test]
fn tx_loan_set_preclaim_borrower_reads_account_exactly_once() {
    let mut seen = Vec::new();

    let result = check_loan_set_preclaim_borrower(&"borrower", |borrower| {
        seen.push(*borrower);
        Some("borrower-sle")
    });

    assert_eq!(result, Ok("borrower-sle"));
    assert_eq!(seen, vec!["borrower"]);
}
