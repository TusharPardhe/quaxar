//! Integration tests that pin the narrowed Rust `LoanSet.cpp::doApply()`
//! `checkLoanGuards(...)` wrapper to the current C++ behavior.

use protocol::{Ter, trans_token};
use tx::run_loan_set_do_apply_check_loan_guards;

#[test]
fn tx_loan_set_do_apply_check_loan_guards_returns_success_unchanged() {
    let result = run_loan_set_do_apply_check_loan_guards(
        &"USD",
        &"100",
        true,
        12,
        &"props",
        |_, _, _, _, _| Ter::TES_SUCCESS,
    );

    assert_eq!(result, Ter::TES_SUCCESS);
}

#[test]
fn tx_loan_set_do_apply_check_loan_guards_returns_precision_loss_unchanged() {
    let result = run_loan_set_do_apply_check_loan_guards(
        &"USD",
        &"100",
        true,
        12,
        &"props",
        |_, _, _, _, _| Ter::TEC_PRECISION_LOSS,
    );

    assert_eq!(result, Ter::TEC_PRECISION_LOSS);
    assert_eq!(trans_token(result), "tecPRECISION_LOSS");
}

#[test]
fn tx_loan_set_do_apply_check_loan_guards_returns_internal_unchanged() {
    let result = run_loan_set_do_apply_check_loan_guards(
        &"USD",
        &"100",
        false,
        12,
        &"props",
        |_, _, _, _, _| Ter::TEC_INTERNAL,
    );

    assert_eq!(result, Ter::TEC_INTERNAL);
    assert_eq!(trans_token(result), "tecINTERNAL");
}

#[test]
fn tx_loan_set_do_apply_check_loan_guards_invokes_helper_once_with_cpp_argument_order() {
    let mut seen = Vec::new();

    let result = run_loan_set_do_apply_check_loan_guards(
        &"XRP",
        &"250",
        false,
        24,
        &"loan-props",
        |asset, principal, expect_interest, payment_total, properties| {
            seen.push(format!(
                "{asset}:{principal}:{expect_interest}:{payment_total}:{properties}"
            ));
            Ter::TEC_PRECISION_LOSS
        },
    );

    assert_eq!(result, Ter::TEC_PRECISION_LOSS);
    assert_eq!(seen, vec!["XRP:250:false:24:loan-props".to_string()]);
}
