//! Integration tests that pin the narrowed Rust transfer-plus-post-transfer
//! `LoanSet.cpp::doApply()` shell to the current C++ behavior.

use std::cell::RefCell;

use protocol::{Ter, trans_token};
use tx::run_loan_set_do_apply_transfer_and_post_transfer;

#[test]
fn tx_loan_set_do_apply_transfer_and_post_transfer_uses_current_on_success() {
    let steps = RefCell::new(Vec::new());

    let result = run_loan_set_do_apply_transfer_and_post_transfer(
        true,
        &30_u32,
        &1_u32,
        &5_u32,
        &0_u32,
        || {
            steps.borrow_mut().push("increment_owner_count".to_string());
            4_u32
        },
        |owner_count| {
            steps
                .borrow_mut()
                .push(format!("compute_reserve owner_count={owner_count}"));
            30_u32
        },
        || {
            steps
                .borrow_mut()
                .push("borrower_add_empty_holding".to_string());
            Ter::TEC_DUPLICATE
        },
        || {
            steps.borrow_mut().push("borrower_require_auth".to_string());
            Ter::TES_SUCCESS
        },
        || {
            steps
                .borrow_mut()
                .push("owner_add_empty_holding".to_string());
            Ter::TEC_DUPLICATE
        },
        || {
            steps.borrow_mut().push("owner_require_auth".to_string());
            Ter::TES_SUCCESS
        },
        || {
            steps.borrow_mut().push("account_send_multi".to_string());
            Ter::TES_SUCCESS
        },
        || {
            steps.borrow_mut().push("post_transfer".to_string());
            Ter::TES_SUCCESS
        },
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        steps.into_inner(),
        vec![
            "increment_owner_count",
            "compute_reserve owner_count=4",
            "borrower_add_empty_holding",
            "borrower_require_auth",
            "owner_add_empty_holding",
            "owner_require_auth",
            "account_send_multi",
            "post_transfer",
        ]
    );
}

#[test]
fn tx_loan_set_do_apply_transfer_and_post_transfer_skips_owner_holding_for_zero_fee() {
    let steps = RefCell::new(Vec::new());

    let result = run_loan_set_do_apply_transfer_and_post_transfer(
        true,
        &30_u32,
        &1_u32,
        &0_u32,
        &0_u32,
        || {
            steps.borrow_mut().push("increment_owner_count".to_string());
            4_u32
        },
        |owner_count| {
            steps
                .borrow_mut()
                .push(format!("compute_reserve owner_count={owner_count}"));
            30_u32
        },
        || {
            steps
                .borrow_mut()
                .push("borrower_add_empty_holding".to_string());
            Ter::TES_SUCCESS
        },
        || {
            steps.borrow_mut().push("borrower_require_auth".to_string());
            Ter::TES_SUCCESS
        },
        || {
            steps
                .borrow_mut()
                .push("owner_add_empty_holding".to_string());
            Ter::TES_SUCCESS
        },
        || {
            steps.borrow_mut().push("owner_require_auth".to_string());
            Ter::TES_SUCCESS
        },
        || {
            steps.borrow_mut().push("account_send_multi".to_string());
            Ter::TES_SUCCESS
        },
        || {
            steps.borrow_mut().push("post_transfer".to_string());
            Ter::TES_SUCCESS
        },
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        steps.into_inner(),
        vec![
            "increment_owner_count",
            "compute_reserve owner_count=4",
            "borrower_add_empty_holding",
            "borrower_require_auth",
            "owner_require_auth",
            "account_send_multi",
            "post_transfer",
        ]
    );
}

#[test]
fn tx_loan_set_do_apply_transfer_and_post_transfer_returns_reserve_failure_before_transfers() {
    let steps = RefCell::new(Vec::new());

    let result = run_loan_set_do_apply_transfer_and_post_transfer(
        true,
        &29_u32,
        &100_u32,
        &5_u32,
        &0_u32,
        || {
            steps.borrow_mut().push("increment_owner_count".to_string());
            4_u32
        },
        |owner_count| {
            steps
                .borrow_mut()
                .push(format!("compute_reserve owner_count={owner_count}"));
            30_u32
        },
        || {
            steps
                .borrow_mut()
                .push("borrower_add_empty_holding".to_string());
            Ter::TES_SUCCESS
        },
        || {
            steps.borrow_mut().push("borrower_require_auth".to_string());
            Ter::TES_SUCCESS
        },
        || {
            steps
                .borrow_mut()
                .push("owner_add_empty_holding".to_string());
            Ter::TES_SUCCESS
        },
        || {
            steps.borrow_mut().push("owner_require_auth".to_string());
            Ter::TES_SUCCESS
        },
        || {
            steps.borrow_mut().push("account_send_multi".to_string());
            Ter::TES_SUCCESS
        },
        || {
            steps.borrow_mut().push("post_transfer".to_string());
            Ter::TES_SUCCESS
        },
    );

    assert_eq!(result, Ter::TEC_INSUFFICIENT_RESERVE);
    assert_eq!(trans_token(result), "tecINSUFFICIENT_RESERVE");
    assert_eq!(
        steps.into_inner(),
        vec!["increment_owner_count", "compute_reserve owner_count=4"]
    );
}

#[test]
fn tx_loan_set_do_apply_transfer_and_post_transfer_short_circuits_on_borrower_auth_failure() {
    let steps = RefCell::new(Vec::new());

    let result = run_loan_set_do_apply_transfer_and_post_transfer(
        true,
        &30_u32,
        &1_u32,
        &5_u32,
        &0_u32,
        || {
            steps.borrow_mut().push("increment_owner_count".to_string());
            4_u32
        },
        |owner_count| {
            steps
                .borrow_mut()
                .push(format!("compute_reserve owner_count={owner_count}"));
            30_u32
        },
        || {
            steps
                .borrow_mut()
                .push("borrower_add_empty_holding".to_string());
            Ter::TES_SUCCESS
        },
        || {
            steps.borrow_mut().push("borrower_require_auth".to_string());
            Ter::TER_NO_RIPPLE
        },
        || {
            steps
                .borrow_mut()
                .push("owner_add_empty_holding".to_string());
            Ter::TES_SUCCESS
        },
        || {
            steps.borrow_mut().push("owner_require_auth".to_string());
            Ter::TES_SUCCESS
        },
        || {
            steps.borrow_mut().push("account_send_multi".to_string());
            Ter::TES_SUCCESS
        },
        || {
            steps.borrow_mut().push("post_transfer".to_string());
            Ter::TES_SUCCESS
        },
    );

    assert_eq!(result, Ter::TER_NO_RIPPLE);
    assert_eq!(trans_token(result), "terNO_RIPPLE");
    assert_eq!(
        steps.into_inner(),
        vec![
            "increment_owner_count",
            "compute_reserve owner_count=4",
            "borrower_add_empty_holding",
            "borrower_require_auth",
        ]
    );
}

#[test]
fn tx_loan_set_do_apply_transfer_and_post_transfer_returns_post_transfer_failure_unchanged() {
    let steps = RefCell::new(Vec::new());

    let result = run_loan_set_do_apply_transfer_and_post_transfer(
        true,
        &30_u32,
        &1_u32,
        &5_u32,
        &0_u32,
        || {
            steps.borrow_mut().push("increment_owner_count".to_string());
            4_u32
        },
        |owner_count| {
            steps
                .borrow_mut()
                .push(format!("compute_reserve owner_count={owner_count}"));
            30_u32
        },
        || {
            steps
                .borrow_mut()
                .push("borrower_add_empty_holding".to_string());
            Ter::TES_SUCCESS
        },
        || {
            steps.borrow_mut().push("borrower_require_auth".to_string());
            Ter::TES_SUCCESS
        },
        || {
            steps
                .borrow_mut()
                .push("owner_add_empty_holding".to_string());
            Ter::TES_SUCCESS
        },
        || {
            steps.borrow_mut().push("owner_require_auth".to_string());
            Ter::TES_SUCCESS
        },
        || {
            steps.borrow_mut().push("account_send_multi".to_string());
            Ter::TES_SUCCESS
        },
        || {
            steps.borrow_mut().push("post_transfer".to_string());
            Ter::TEC_MAX_SEQUENCE_REACHED
        },
    );

    assert_eq!(result, Ter::TEC_MAX_SEQUENCE_REACHED);
    assert_eq!(trans_token(result), "tecMAX_SEQUENCE_REACHED");
    assert_eq!(
        steps.into_inner(),
        vec![
            "increment_owner_count",
            "compute_reserve owner_count=4",
            "borrower_add_empty_holding",
            "borrower_require_auth",
            "owner_add_empty_holding",
            "owner_require_auth",
            "account_send_multi",
            "post_transfer",
        ]
    );
}
