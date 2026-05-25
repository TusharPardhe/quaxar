//! Integration tests that pin the narrowed Rust higher
//! `LoanSet.cpp::doApply()` pre-guarded-transfer shell to the current C++
//! behavior.

use std::{cell::RefCell, collections::BTreeMap};

use protocol::{Ter, trans_token};
use tx::{
    LoanSetDoApplyPreGuardedTransferProperties, LoanSetDoApplyPreGuardedTransferState,
    LoanSetDoApplyPreGuardedTransferTx, LoanSetDoApplyRepresentabilityTx,
    LoanSetRepresentabilityField, run_loan_set_do_apply_pre_guarded_transfer,
};

struct TestTx {
    principal_requested: i64,
    interest_rate: Option<u32>,
    payment_interval: Option<u32>,
    payment_total: Option<u32>,
    values: BTreeMap<LoanSetRepresentabilityField, &'static str>,
}

impl LoanSetDoApplyRepresentabilityTx for TestTx {
    type Value = &'static str;

    fn value(&self, field: LoanSetRepresentabilityField) -> Option<&Self::Value> {
        self.values.get(&field)
    }
}

impl LoanSetDoApplyPreGuardedTransferTx for TestTx {
    type Amount = i64;
    type InterestRate = u32;

    fn principal_requested(&self) -> &Self::Amount {
        &self.principal_requested
    }

    fn interest_rate(&self) -> Option<Self::InterestRate> {
        self.interest_rate
    }

    fn payment_interval(&self) -> Option<u32> {
        self.payment_interval
    }

    fn payment_total(&self) -> Option<u32> {
        self.payment_total
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestProperties {
    loan_scale: i32,
    total_value_outstanding: i64,
    management_fee_due: i64,
    periodic_payment: i64,
}

impl LoanSetDoApplyPreGuardedTransferProperties for TestProperties {
    type Amount = i64;

    fn loan_scale(&self) -> i32 {
        self.loan_scale
    }

    fn total_value_outstanding(&self) -> &Self::Amount {
        &self.total_value_outstanding
    }

    fn management_fee_due(&self) -> &Self::Amount {
        &self.management_fee_due
    }

    fn periodic_payment(&self) -> &Self::Amount {
        &self.periodic_payment
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestState {
    interest_due: i64,
}

impl LoanSetDoApplyPreGuardedTransferState for TestState {
    type Amount = i64;

    fn interest_due(&self) -> &Self::Amount {
        &self.interest_due
    }
}

fn valid_tx() -> TestTx {
    TestTx {
        principal_requested: 100,
        interest_rate: Some(250),
        payment_interval: Some(45),
        payment_total: Some(9),
        values: BTreeMap::from([
            (LoanSetRepresentabilityField::PrincipalRequested, "100"),
            (LoanSetRepresentabilityField::LoanOriginationFee, "5"),
        ]),
    }
}

fn valid_properties() -> TestProperties {
    TestProperties {
        loan_scale: 4,
        total_value_outstanding: 1_250,
        management_fee_due: 25,
        periodic_payment: 150,
    }
}

#[test]
fn tx_loan_set_do_apply_pre_guarded_transfer_uses_current_on_success() {
    let steps = RefCell::new(Vec::new());

    let result = run_loan_set_do_apply_pre_guarded_transfer(
        &valid_tx(),
        &"USD",
        &200_i64,
        &500_i64,
        &100_i64,
        4,
        12_u16,
        0_u32,
        30,
        12,
        &0_i64,
        &2_000_i64,
        &1_250_i64,
        &500_i64,
        10_000_u32,
        |asset, principal, interest, interval, total, management_fee_rate, scale| {
            steps.borrow_mut().push(format!(
                "compute_properties={asset}:{principal}:{interest}:{interval}:{total}:{management_fee_rate}:{scale}"
            ));
            valid_properties()
        },
        |value_outstanding, principal, management_fee_due| {
            steps.borrow_mut().push(format!(
                "construct_state={value_outstanding}:{principal}:{management_fee_due}"
            ));
            TestState { interest_due: 25 }
        },
        |field, _| {
            steps
                .borrow_mut()
                .push(format!("representability={field:?}"));
            true
        },
        |asset, principal, expect_interest, payment_total, properties| {
            steps.borrow_mut().push(format!(
                "loan_guards={asset}:{principal}:{expect_interest}:{payment_total}:{}",
                properties.total_value_outstanding
            ));
            Ter::TES_SUCCESS
        },
        |new_debt_total, cover_rate_minimum| {
            steps.borrow_mut().push(format!(
                "compute_required_cover={new_debt_total}:{cover_rate_minimum}"
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
            "compute_properties=USD:100:250:45:9:12:4",
            "construct_state=1250:100:25",
            "representability=PrincipalRequested",
            "representability=LoanOriginationFee",
            "loan_guards=USD:100:true:9:1250",
            "compute_required_cover=1250:10000",
            "transfer_shell",
        ]
    );
}

#[test]
fn tx_loan_set_do_apply_pre_guarded_transfer_uses_cpp_defaults_before_loan_guards() {
    let steps = RefCell::new(Vec::new());
    let tx = TestTx {
        principal_requested: 100,
        interest_rate: None,
        payment_interval: None,
        payment_total: None,
        values: BTreeMap::from([(LoanSetRepresentabilityField::PrincipalRequested, "100")]),
    };

    let result = run_loan_set_do_apply_pre_guarded_transfer(
        &tx,
        &"USD",
        &200_i64,
        &500_i64,
        &100_i64,
        4,
        12_u16,
        0_u32,
        30,
        12,
        &0_i64,
        &2_000_i64,
        &1_250_i64,
        &500_i64,
        10_000_u32,
        |asset, principal, interest, interval, total, management_fee_rate, scale| {
            steps.borrow_mut().push(format!(
                "compute_properties={asset}:{principal}:{interest}:{interval}:{total}:{management_fee_rate}:{scale}"
            ));
            valid_properties()
        },
        |_, _, _| {
            steps.borrow_mut().push("construct_state".to_string());
            TestState { interest_due: 25 }
        },
        |field, _| {
            steps
                .borrow_mut()
                .push(format!("representability={field:?}"));
            true
        },
        |_, _, expect_interest, payment_total, _| {
            steps
                .borrow_mut()
                .push(format!("loan_guards={expect_interest}:{payment_total}"));
            Ter::TEC_INTERNAL
        },
        |_, _| {
            steps
                .borrow_mut()
                .push("compute_required_cover".to_string());
            400_i64
        },
        || {
            steps.borrow_mut().push("transfer_shell".to_string());
            Ter::TES_SUCCESS
        },
    );

    assert_eq!(result, Ter::TEC_INTERNAL);
    assert_eq!(trans_token(result), "tecINTERNAL");
    assert_eq!(
        steps.into_inner(),
        vec![
            "compute_properties=USD:100:0:30:12:12:4",
            "construct_state",
            "representability=PrincipalRequested",
            "loan_guards=false:12",
        ]
    );
}

#[test]
fn tx_loan_set_do_apply_pre_guarded_transfer_returns_precision_loss_before_loan_guards() {
    let steps = RefCell::new(Vec::new());

    let result = run_loan_set_do_apply_pre_guarded_transfer(
        &valid_tx(),
        &"USD",
        &200_i64,
        &500_i64,
        &100_i64,
        4,
        12_u16,
        0_u32,
        30,
        12,
        &0_i64,
        &2_000_i64,
        &1_250_i64,
        &500_i64,
        10_000_u32,
        |_, _, _, _, _, _, _| {
            steps.borrow_mut().push("compute_properties".to_string());
            valid_properties()
        },
        |_, _, _| {
            steps.borrow_mut().push("construct_state".to_string());
            TestState { interest_due: 25 }
        },
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
        |_, _| {
            steps
                .borrow_mut()
                .push("compute_required_cover".to_string());
            400_i64
        },
        || {
            steps.borrow_mut().push("transfer_shell".to_string());
            Ter::TES_SUCCESS
        },
    );

    assert_eq!(result, Ter::TEC_PRECISION_LOSS);
    assert_eq!(trans_token(result), "tecPRECISION_LOSS");
    assert_eq!(
        steps.into_inner(),
        vec![
            "compute_properties",
            "construct_state",
            "representability=PrincipalRequested",
        ]
    );
}

#[test]
fn tx_loan_set_do_apply_pre_guarded_transfer_maps_property_values_into_computed_value_guards() {
    let steps = RefCell::new(Vec::new());

    let result = run_loan_set_do_apply_pre_guarded_transfer(
        &valid_tx(),
        &"USD",
        &200_i64,
        &500_i64,
        &100_i64,
        4,
        12_u16,
        0_u32,
        30,
        12,
        &0_i64,
        &2_000_i64,
        &1_250_i64,
        &500_i64,
        10_000_u32,
        |_, _, _, _, _, _, _| {
            steps.borrow_mut().push("compute_properties".to_string());
            TestProperties {
                periodic_payment: 0,
                ..valid_properties()
            }
        },
        |_, _, _| {
            steps.borrow_mut().push("construct_state".to_string());
            TestState { interest_due: 25 }
        },
        |field, _| {
            steps
                .borrow_mut()
                .push(format!("representability={field:?}"));
            true
        },
        |_, _, _, _, _| {
            steps.borrow_mut().push("loan_guards".to_string());
            Ter::TES_SUCCESS
        },
        |_, _| {
            steps
                .borrow_mut()
                .push("compute_required_cover".to_string());
            400_i64
        },
        || {
            steps.borrow_mut().push("transfer_shell".to_string());
            Ter::TES_SUCCESS
        },
    );

    assert_eq!(result, Ter::TEC_INTERNAL);
    assert_eq!(trans_token(result), "tecINTERNAL");
    assert_eq!(
        steps.into_inner(),
        vec![
            "compute_properties",
            "construct_state",
            "representability=PrincipalRequested",
            "representability=LoanOriginationFee",
            "loan_guards",
        ]
    );
}

#[test]
fn tx_loan_set_do_apply_pre_guarded_transfer_returns_vault_limit_before_representability() {
    let steps = RefCell::new(Vec::new());

    let result = run_loan_set_do_apply_pre_guarded_transfer(
        &valid_tx(),
        &"USD",
        &200_i64,
        &120_i64,
        &100_i64,
        4,
        12_u16,
        0_u32,
        30,
        12,
        &0_i64,
        &2_000_i64,
        &1_250_i64,
        &500_i64,
        10_000_u32,
        |_, _, _, _, _, _, _| {
            steps.borrow_mut().push("compute_properties".to_string());
            valid_properties()
        },
        |_, _, _| {
            steps.borrow_mut().push("construct_state".to_string());
            TestState { interest_due: 21 }
        },
        |field, _| {
            steps
                .borrow_mut()
                .push(format!("representability={field:?}"));
            true
        },
        |_, _, _, _, _| {
            steps.borrow_mut().push("loan_guards".to_string());
            Ter::TES_SUCCESS
        },
        |_, _| {
            steps
                .borrow_mut()
                .push("compute_required_cover".to_string());
            400_i64
        },
        || {
            steps.borrow_mut().push("transfer_shell".to_string());
            Ter::TES_SUCCESS
        },
    );

    assert_eq!(result, Ter::TEC_LIMIT_EXCEEDED);
    assert_eq!(trans_token(result), "tecLIMIT_EXCEEDED");
    assert_eq!(
        steps.into_inner(),
        vec!["compute_properties", "construct_state"]
    );
}
