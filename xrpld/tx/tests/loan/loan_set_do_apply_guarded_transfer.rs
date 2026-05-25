//! Integration tests that pin the narrowed Rust guarded-transfer
//! `LoanSet.cpp::doApply()` shell to the current C++ behavior.

use std::{cell::RefCell, collections::BTreeMap};

use protocol::{Ter, trans_token};
use tx::{
    LoanSetDoApplyComputedValues, LoanSetDoApplyRepresentabilityTx, LoanSetRepresentabilityField,
    run_loan_set_do_apply_guarded_transfer,
};

struct TestTx {
    values: BTreeMap<LoanSetRepresentabilityField, &'static str>,
}

impl LoanSetDoApplyRepresentabilityTx for TestTx {
    type Value = &'static str;

    fn value(&self, field: LoanSetRepresentabilityField) -> Option<&Self::Value> {
        self.values.get(&field)
    }
}

fn valid_tx() -> TestTx {
    TestTx {
        values: BTreeMap::from([
            (LoanSetRepresentabilityField::PrincipalRequested, "100"),
            (LoanSetRepresentabilityField::LoanOriginationFee, "5"),
        ]),
    }
}

fn valid_computed_values() -> LoanSetDoApplyComputedValues<i64> {
    LoanSetDoApplyComputedValues {
        management_fee_due: 0,
        total_value_outstanding: 1_250,
        periodic_payment: 125,
    }
}

#[test]
fn tx_loan_set_do_apply_guarded_transfer_uses_current_on_success() {
    let steps = RefCell::new(Vec::new());

    let result = run_loan_set_do_apply_guarded_transfer(
        &valid_tx(),
        &"USD",
        &100_i64,
        &200_i64,
        &500_i64,
        &100_i64,
        &25_i64,
        &1_250_i64,
        4,
        true,
        12,
        &"props",
        &valid_computed_values(),
        &0_i64,
        &2_000_i64,
        &125_i64,
        &500_i64,
        10_000_u32,
        |field, _| {
            steps
                .borrow_mut()
                .push(format!("representability={field:?}"));
            true
        },
        |asset, principal, expect_interest, payment_total, properties| {
            steps.borrow_mut().push(format!(
                "loan_guards={asset}:{principal}:{expect_interest}:{payment_total}:{properties}"
            ));
            Ter::TES_SUCCESS
        },
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
        vec![
            "representability=PrincipalRequested",
            "representability=LoanOriginationFee",
            "loan_guards=USD:100:true:12:props",
            "compute_required_cover 125:10000",
            "transfer_shell",
        ]
    );
}

#[test]
fn tx_loan_set_do_apply_guarded_transfer_returns_vault_funds_failure_first() {
    let steps = RefCell::new(Vec::new());

    let result = run_loan_set_do_apply_guarded_transfer(
        &valid_tx(),
        &"USD",
        &100_i64,
        &99_i64,
        &500_i64,
        &100_i64,
        &25_i64,
        &1_250_i64,
        4,
        true,
        12,
        &"props",
        &valid_computed_values(),
        &0_i64,
        &2_000_i64,
        &125_i64,
        &500_i64,
        10_000_u32,
        |field, _| {
            steps
                .borrow_mut()
                .push(format!("representability={field:?}"));
            true
        },
        |_, _, _, _, _| Ter::TES_SUCCESS,
        |_, _| 400_i64,
        || Ter::TES_SUCCESS,
    );

    assert_eq!(result, Ter::TEC_INSUFFICIENT_FUNDS);
    assert_eq!(trans_token(result), "tecINSUFFICIENT_FUNDS");
    assert!(steps.borrow().is_empty());
}

#[test]
fn tx_loan_set_do_apply_guarded_transfer_returns_precision_loss_before_loan_guards() {
    let steps = RefCell::new(Vec::new());

    let result = run_loan_set_do_apply_guarded_transfer(
        &valid_tx(),
        &"USD",
        &100_i64,
        &200_i64,
        &500_i64,
        &100_i64,
        &25_i64,
        &1_250_i64,
        4,
        true,
        12,
        &"props",
        &valid_computed_values(),
        &0_i64,
        &2_000_i64,
        &125_i64,
        &500_i64,
        10_000_u32,
        |field, _| {
            steps
                .borrow_mut()
                .push(format!("representability={field:?}"));
            field != LoanSetRepresentabilityField::PrincipalRequested
        },
        |_, _, _, _, _| {
            steps.borrow_mut().push("loan_guards".to_string());
            Ter::TES_SUCCESS
        },
        |_, _| 400_i64,
        || Ter::TES_SUCCESS,
    );

    assert_eq!(result, Ter::TEC_PRECISION_LOSS);
    assert_eq!(trans_token(result), "tecPRECISION_LOSS");
    assert_eq!(
        steps.into_inner(),
        vec!["representability=PrincipalRequested"]
    );
}

#[test]
fn tx_loan_set_do_apply_guarded_transfer_returns_loan_guard_failure_before_post_computed() {
    let steps = RefCell::new(Vec::new());

    let result = run_loan_set_do_apply_guarded_transfer(
        &valid_tx(),
        &"USD",
        &100_i64,
        &200_i64,
        &500_i64,
        &100_i64,
        &25_i64,
        &1_250_i64,
        4,
        true,
        12,
        &"props",
        &valid_computed_values(),
        &0_i64,
        &2_000_i64,
        &125_i64,
        &500_i64,
        10_000_u32,
        |field, _| {
            steps
                .borrow_mut()
                .push(format!("representability={field:?}"));
            true
        },
        |_, _, _, _, _| {
            steps.borrow_mut().push("loan_guards".to_string());
            Ter::TEC_INTERNAL
        },
        |_, _| 400_i64,
        || Ter::TES_SUCCESS,
    );

    assert_eq!(result, Ter::TEC_INTERNAL);
    assert_eq!(trans_token(result), "tecINTERNAL");
    assert_eq!(
        steps.into_inner(),
        vec![
            "representability=PrincipalRequested",
            "representability=LoanOriginationFee",
            "loan_guards",
        ]
    );
}

#[test]
fn tx_loan_set_do_apply_guarded_transfer_returns_post_computed_failure_unchanged() {
    let result = run_loan_set_do_apply_guarded_transfer(
        &valid_tx(),
        &"USD",
        &100_i64,
        &200_i64,
        &500_i64,
        &100_i64,
        &25_i64,
        &1_250_i64,
        4,
        true,
        12,
        &"props",
        &valid_computed_values(),
        &0_i64,
        &2_000_i64,
        &125_i64,
        &399_i64,
        10_000_u32,
        |_, _| true,
        |_, _, _, _, _| Ter::TES_SUCCESS,
        |_, _| 400_i64,
        || Ter::TES_SUCCESS,
    );

    assert_eq!(result, Ter::TEC_INSUFFICIENT_FUNDS);
    assert_eq!(trans_token(result), "tecINSUFFICIENT_FUNDS");
}
