//! Integration tests that pin the narrowed Rust post-computed-values
//! `LoanSet.cpp::doApply()` shell to the current C++ behavior.

use std::cell::RefCell;

use protocol::{Ter, trans_token};
use tx::{LoanSetDoApplyComputedValues, run_loan_set_do_apply_post_computed_transfer};

fn valid_values() -> LoanSetDoApplyComputedValues<i64> {
    LoanSetDoApplyComputedValues {
        management_fee_due: 0,
        total_value_outstanding: 1_250,
        periodic_payment: 125,
    }
}

#[test]
fn tx_loan_set_do_apply_post_computed_transfer_uses_current_on_success() {
    let steps = RefCell::new(Vec::new());

    let result = run_loan_set_do_apply_post_computed_transfer(
        &valid_values(),
        &0_i64,
        &2_000_i64,
        &1_250_i64,
        &500_i64,
        10_000_u32,
        |new_debt_total, cover_rate_minimum| {
            steps.borrow_mut().push(format!(
                "compute_required_cover {new_debt_total}:{cover_rate_minimum}"
            ));
            400_i64
        },
        || {
            steps.borrow_mut().push("transfer_shell".to_string());
            Ter::TES_SUCCESS
        },
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        steps.into_inner(),
        vec!["compute_required_cover 1250:10000", "transfer_shell",]
    );
}

#[test]
fn tx_loan_set_do_apply_post_computed_transfer_returns_internal_before_later_guards() {
    let steps = RefCell::new(Vec::new());

    let result = run_loan_set_do_apply_post_computed_transfer(
        &LoanSetDoApplyComputedValues {
            periodic_payment: 0,
            ..valid_values()
        },
        &0_i64,
        &2_000_i64,
        &1_250_i64,
        &500_i64,
        10_000_u32,
        |new_debt_total, cover_rate_minimum| {
            steps.borrow_mut().push(format!(
                "compute_required_cover {new_debt_total}:{cover_rate_minimum}"
            ));
            400_i64
        },
        || {
            steps.borrow_mut().push("transfer_shell".to_string());
            Ter::TES_SUCCESS
        },
    );

    assert_eq!(result, Ter::TEC_INTERNAL);
    assert_eq!(trans_token(result), "tecINTERNAL");
    assert!(steps.borrow().is_empty());
}

#[test]
fn tx_loan_set_do_apply_post_computed_transfer_returns_limit_exceeded_before_cover() {
    let steps = RefCell::new(Vec::new());

    let result = run_loan_set_do_apply_post_computed_transfer(
        &valid_values(),
        &0_i64,
        &1_000_i64,
        &1_250_i64,
        &500_i64,
        10_000_u32,
        |new_debt_total, cover_rate_minimum| {
            steps.borrow_mut().push(format!(
                "compute_required_cover {new_debt_total}:{cover_rate_minimum}"
            ));
            400_i64
        },
        || {
            steps.borrow_mut().push("transfer_shell".to_string());
            Ter::TES_SUCCESS
        },
    );

    assert_eq!(result, Ter::TEC_LIMIT_EXCEEDED);
    assert_eq!(trans_token(result), "tecLIMIT_EXCEEDED");
    assert!(steps.borrow().is_empty());
}

#[test]
fn tx_loan_set_do_apply_post_computed_transfer_returns_insufficient_funds_before_transfer() {
    let steps = RefCell::new(Vec::new());

    let result = run_loan_set_do_apply_post_computed_transfer(
        &valid_values(),
        &0_i64,
        &2_000_i64,
        &1_250_i64,
        &399_i64,
        10_000_u32,
        |new_debt_total, cover_rate_minimum| {
            steps.borrow_mut().push(format!(
                "compute_required_cover {new_debt_total}:{cover_rate_minimum}"
            ));
            400_i64
        },
        || {
            steps.borrow_mut().push("transfer_shell".to_string());
            Ter::TES_SUCCESS
        },
    );

    assert_eq!(result, Ter::TEC_INSUFFICIENT_FUNDS);
    assert_eq!(trans_token(result), "tecINSUFFICIENT_FUNDS");
    assert_eq!(
        steps.into_inner(),
        vec!["compute_required_cover 1250:10000"]
    );
}

#[test]
fn tx_loan_set_do_apply_post_computed_transfer_returns_transfer_failure_unchanged() {
    let result = run_loan_set_do_apply_post_computed_transfer(
        &valid_values(),
        &0_i64,
        &2_000_i64,
        &1_250_i64,
        &500_i64,
        10_000_u32,
        |_, _| 400_i64,
        || Ter::TEC_INSUFFICIENT_RESERVE,
    );

    assert_eq!(result, Ter::TEC_INSUFFICIENT_RESERVE);
    assert_eq!(trans_token(result), "tecINSUFFICIENT_RESERVE");
}
