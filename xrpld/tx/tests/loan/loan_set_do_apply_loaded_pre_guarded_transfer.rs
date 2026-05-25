//! Integration tests that pin the narrowed Rust higher
//! `LoanSet.cpp::doApply()` loaded pre-guarded-transfer shell to the current
//! C++ behavior.

use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

use protocol::{Ter, trans_token};
use tx::{
    LoanSetDoApplyLoadedPreGuardedTransferBroker, LoanSetDoApplyLoadedPreGuardedTransferVault,
    LoanSetDoApplyPreGuardedTransferProperties, LoanSetDoApplyPreGuardedTransferState,
    LoanSetDoApplyPreGuardedTransferTx, LoanSetDoApplyRepresentabilityTx,
    LoanSetRepresentabilityField, run_loan_set_do_apply_loaded_pre_guarded_transfer,
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

struct TestBroker {
    management_fee_rate: u16,
    steps: Rc<RefCell<Vec<String>>>,
}

impl LoanSetDoApplyLoadedPreGuardedTransferBroker for TestBroker {
    type ManagementFeeRate = u16;

    fn management_fee_rate(&self) -> Self::ManagementFeeRate {
        self.steps
            .borrow_mut()
            .push("management_fee_rate".to_string());
        self.management_fee_rate
    }
}

struct TestVault {
    assets_available: i64,
    assets_total: i64,
    assets_maximum: i64,
    steps: Rc<RefCell<Vec<String>>>,
}

impl LoanSetDoApplyLoadedPreGuardedTransferVault for TestVault {
    type Amount = i64;

    fn assets_available(&self) -> &Self::Amount {
        self.steps.borrow_mut().push("assets_available".to_string());
        &self.assets_available
    }

    fn assets_total(&self) -> &Self::Amount {
        self.steps.borrow_mut().push("assets_total".to_string());
        &self.assets_total
    }

    fn assets_maximum(&self) -> &Self::Amount {
        self.steps.borrow_mut().push("assets_maximum".to_string());
        &self.assets_maximum
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
fn tx_loan_set_do_apply_loaded_pre_guarded_transfer_uses_current_on_success() {
    let steps = Rc::new(RefCell::new(Vec::new()));
    let broker = TestBroker {
        management_fee_rate: 12,
        steps: Rc::clone(&steps),
    };
    let vault = TestVault {
        assets_available: 200,
        assets_total: 100,
        assets_maximum: 500,
        steps: Rc::clone(&steps),
    };

    let result = run_loan_set_do_apply_loaded_pre_guarded_transfer(
        &valid_tx(),
        &broker,
        &vault,
        &"USD",
        0_u32,
        30,
        12,
        &0_i64,
        &2_000_i64,
        &1_250_i64,
        &500_i64,
        10_000_u32,
        |vault| {
            vault.steps.borrow_mut().push("vault_scale".to_string());
            4
        },
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
        steps.borrow().as_slice(),
        [
            "assets_available",
            "assets_total",
            "vault_scale",
            "management_fee_rate",
            "compute_properties=USD:100:250:45:9:12:4",
            "construct_state=1250:100:25",
            "assets_maximum",
            "representability=PrincipalRequested",
            "representability=LoanOriginationFee",
            "loan_guards=USD:100:true:9:1250",
            "compute_required_cover=1250:10000",
            "transfer_shell",
        ]
    );
}

#[test]
fn tx_loan_set_do_apply_loaded_pre_guarded_transfer_uses_cpp_defaults_before_loan_guards() {
    let steps = Rc::new(RefCell::new(Vec::new()));
    let broker = TestBroker {
        management_fee_rate: 12,
        steps: Rc::clone(&steps),
    };
    let vault = TestVault {
        assets_available: 200,
        assets_total: 100,
        assets_maximum: 500,
        steps: Rc::clone(&steps),
    };
    let tx = TestTx {
        principal_requested: 100,
        interest_rate: None,
        payment_interval: None,
        payment_total: None,
        values: BTreeMap::from([(LoanSetRepresentabilityField::PrincipalRequested, "100")]),
    };

    let result = run_loan_set_do_apply_loaded_pre_guarded_transfer(
        &tx,
        &broker,
        &vault,
        &"USD",
        0_u32,
        30,
        12,
        &0_i64,
        &2_000_i64,
        &1_250_i64,
        &500_i64,
        10_000_u32,
        |vault| {
            vault.steps.borrow_mut().push("vault_scale".to_string());
            4
        },
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
        steps.borrow().as_slice(),
        [
            "assets_available",
            "assets_total",
            "vault_scale",
            "management_fee_rate",
            "compute_properties=USD:100:0:30:12:12:4",
            "construct_state",
            "assets_maximum",
            "representability=PrincipalRequested",
            "loan_guards=false:12",
        ]
    );
}

#[test]
fn tx_loan_set_do_apply_loaded_pre_guarded_transfer_returns_vault_limit_before_representability() {
    let steps = Rc::new(RefCell::new(Vec::new()));
    let broker = TestBroker {
        management_fee_rate: 12,
        steps: Rc::clone(&steps),
    };
    let vault = TestVault {
        assets_available: 200,
        assets_total: 100,
        assets_maximum: 120,
        steps: Rc::clone(&steps),
    };

    let result = run_loan_set_do_apply_loaded_pre_guarded_transfer(
        &valid_tx(),
        &broker,
        &vault,
        &"USD",
        0_u32,
        30,
        12,
        &0_i64,
        &2_000_i64,
        &1_250_i64,
        &500_i64,
        10_000_u32,
        |vault| {
            vault.steps.borrow_mut().push("vault_scale".to_string());
            4
        },
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
        steps.borrow().as_slice(),
        [
            "assets_available",
            "assets_total",
            "vault_scale",
            "management_fee_rate",
            "compute_properties",
            "construct_state",
            "assets_maximum",
        ]
    );
}

#[test]
fn tx_loan_set_do_apply_loaded_pre_guarded_transfer_returns_precision_loss_after_assets_maximum() {
    let steps = Rc::new(RefCell::new(Vec::new()));
    let broker = TestBroker {
        management_fee_rate: 12,
        steps: Rc::clone(&steps),
    };
    let vault = TestVault {
        assets_available: 200,
        assets_total: 100,
        assets_maximum: 500,
        steps: Rc::clone(&steps),
    };

    let result = run_loan_set_do_apply_loaded_pre_guarded_transfer(
        &valid_tx(),
        &broker,
        &vault,
        &"USD",
        0_u32,
        30,
        12,
        &0_i64,
        &2_000_i64,
        &1_250_i64,
        &500_i64,
        10_000_u32,
        |vault| {
            vault.steps.borrow_mut().push("vault_scale".to_string());
            4
        },
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
        steps.borrow().as_slice(),
        [
            "assets_available",
            "assets_total",
            "vault_scale",
            "management_fee_rate",
            "compute_properties",
            "construct_state",
            "assets_maximum",
            "representability=PrincipalRequested",
        ]
    );
}

#[test]
fn tx_loan_set_do_apply_loaded_pre_guarded_transfer_maps_property_values_into_computed_value_guards()
 {
    let steps = Rc::new(RefCell::new(Vec::new()));
    let broker = TestBroker {
        management_fee_rate: 12,
        steps: Rc::clone(&steps),
    };
    let vault = TestVault {
        assets_available: 200,
        assets_total: 100,
        assets_maximum: 500,
        steps: Rc::clone(&steps),
    };

    let result = run_loan_set_do_apply_loaded_pre_guarded_transfer(
        &valid_tx(),
        &broker,
        &vault,
        &"USD",
        0_u32,
        30,
        12,
        &0_i64,
        &2_000_i64,
        &1_250_i64,
        &500_i64,
        10_000_u32,
        |vault| {
            vault.steps.borrow_mut().push("vault_scale".to_string());
            4
        },
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
        steps.borrow().as_slice(),
        [
            "assets_available",
            "assets_total",
            "vault_scale",
            "management_fee_rate",
            "compute_properties",
            "construct_state",
            "assets_maximum",
            "representability=PrincipalRequested",
            "representability=LoanOriginationFee",
            "loan_guards",
        ]
    );
}
