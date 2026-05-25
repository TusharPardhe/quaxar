//! Integration tests that pin the first post-transfer
//! `LoanSet.cpp::doApply()` loan-creation setup facts to the current C++
//! behavior.

use std::cell::RefCell;

use tx::{LoanSetDoApplyLoanSetup, load_loan_set_do_apply_loan_setup};

#[test]
fn tx_loan_set_do_apply_loan_setup_reads_start_date_before_loan_sequence() {
    let steps = RefCell::new(Vec::new());

    let result = load_loan_set_do_apply_loan_setup(
        || {
            steps.borrow_mut().push("start_date");
            123_u32
        },
        || {
            steps.borrow_mut().push("loan_sequence");
            1_u32
        },
    );

    assert_eq!(
        result,
        LoanSetDoApplyLoanSetup {
            start_date: 123_u32,
            loan_sequence: 1_u32,
        }
    );
    assert_eq!(steps.into_inner(), vec!["start_date", "loan_sequence"]);
}

#[test]
fn tx_loan_set_do_apply_loan_setup_preserves_parent_close_time_and_sequence() {
    let result = load_loan_set_do_apply_loan_setup(|| 5_432_u32, || 17_u32);

    assert_eq!(
        result,
        LoanSetDoApplyLoanSetup {
            start_date: 5_432_u32,
            loan_sequence: 17_u32,
        }
    );
}

#[test]
fn tx_loan_set_do_apply_loan_setup_keeps_zero_sequence_unchanged() {
    let result = load_loan_set_do_apply_loan_setup(|| 99_u32, || 0_u32);

    assert_eq!(
        result,
        LoanSetDoApplyLoanSetup {
            start_date: 99_u32,
            loan_sequence: 0_u32,
        }
    );
}
