//! Integration tests that pin the narrowed Rust higher post-transfer
//! `LoanSet.cpp::doApply()` tail to the current C++ behavior.

use std::{cell::RefCell, rc::Rc};

use basics::base_uint::Uint256;
use protocol::{Ter, loan_key, trans_token};
use tx::{
    LoanSetDoApplyAssociateAssetsSink, LoanSetDoApplyBrokerUpdateSink,
    LoanSetDoApplyLoanDynamicFieldSink, LoanSetDoApplyLoanFixedFieldSink,
    LoanSetDoApplyPostTransfer, LoanSetDoApplyVaultUpdate, LoanSetDoApplyVaultUpdateSink,
    run_loan_set_do_apply_post_transfer,
};

#[derive(Clone)]
struct RecordingSink {
    steps: Rc<RefCell<Vec<String>>>,
    next_broker_sequence: u32,
}

impl RecordingSink {
    fn new(steps: Rc<RefCell<Vec<String>>>, next_broker_sequence: u32) -> Self {
        Self {
            steps,
            next_broker_sequence,
        }
    }
}

impl LoanSetDoApplyLoanFixedFieldSink for RecordingSink {
    type LoanScale = &'static str;
    type StartDate = u32;
    type PaymentInterval = u32;
    type BrokerId = Uint256;
    type AccountId = &'static str;

    fn set_loan_scale(&mut self, value: Self::LoanScale) {
        self.steps.borrow_mut().push(format!("loan_scale={value}"));
    }

    fn set_start_date(&mut self, value: Self::StartDate) {
        self.steps.borrow_mut().push(format!("start_date={value}"));
    }

    fn set_payment_interval(&mut self, value: Self::PaymentInterval) {
        self.steps
            .borrow_mut()
            .push(format!("payment_interval={value}"));
    }

    fn set_loan_sequence(&mut self, value: u32) {
        self.steps
            .borrow_mut()
            .push(format!("loan_sequence={value}"));
    }

    fn set_loan_broker_id(&mut self, value: Self::BrokerId) {
        self.steps
            .borrow_mut()
            .push(format!("loan_broker_id={value}"));
    }

    fn set_borrower(&mut self, value: Self::AccountId) {
        self.steps.borrow_mut().push(format!("borrower={value}"));
    }

    fn set_overpayment_flag(&mut self) {
        self.steps.borrow_mut().push("overpayment_flag".to_string());
    }
}

impl LoanSetDoApplyLoanDynamicFieldSink for RecordingSink {
    type Amount = i64;
    type Date = u32;
    type PaymentCount = u32;

    fn set_principal_outstanding(&mut self, value: Self::Amount) {
        self.steps
            .borrow_mut()
            .push(format!("principal_outstanding={value}"));
    }

    fn set_periodic_payment(&mut self, value: Self::Amount) {
        self.steps
            .borrow_mut()
            .push(format!("periodic_payment={value}"));
    }

    fn set_total_value_outstanding(&mut self, value: Self::Amount) {
        self.steps
            .borrow_mut()
            .push(format!("total_value_outstanding={value}"));
    }

    fn set_management_fee_outstanding(&mut self, value: Self::Amount) {
        self.steps
            .borrow_mut()
            .push(format!("management_fee_outstanding={value}"));
    }

    fn set_previous_payment_due_date(&mut self, value: Self::Date) {
        self.steps
            .borrow_mut()
            .push(format!("previous_payment_due_date={value}"));
    }

    fn set_next_payment_due_date(&mut self, value: Self::Date) {
        self.steps
            .borrow_mut()
            .push(format!("next_payment_due_date={value}"));
    }

    fn set_payment_remaining(&mut self, value: Self::PaymentCount) {
        self.steps
            .borrow_mut()
            .push(format!("payment_remaining={value}"));
    }

    fn insert_loan(&mut self) {
        self.steps.borrow_mut().push("insert_loan".to_string());
    }
}

impl LoanSetDoApplyVaultUpdateSink for RecordingSink {
    type Amount = i64;

    fn subtract_assets_available(&mut self, value: Self::Amount) {
        self.steps
            .borrow_mut()
            .push(format!("assets_available-={value}"));
    }

    fn add_assets_total(&mut self, value: Self::Amount) {
        self.steps
            .borrow_mut()
            .push(format!("assets_total+={value}"));
    }

    fn assets_available(&self) -> &Self::Amount {
        static AVAILABLE: i64 = 10;
        &AVAILABLE
    }

    fn assets_total(&self) -> &Self::Amount {
        static TOTAL: i64 = 20;
        &TOTAL
    }

    fn update_vault(&mut self) {
        self.steps.borrow_mut().push("update_vault".to_string());
    }
}

impl LoanSetDoApplyBrokerUpdateSink for RecordingSink {
    type DebtDelta = i64;
    type Asset = &'static str;
    type Scale = u32;

    fn adjust_debt_total(
        &mut self,
        delta: Self::DebtDelta,
        asset: Self::Asset,
        scale: Self::Scale,
    ) {
        self.steps.borrow_mut().push(format!(
            "adjust_debt_total delta={delta} asset={asset} scale={scale}"
        ));
    }

    fn increment_owner_count(&mut self) {
        self.steps
            .borrow_mut()
            .push("increment_owner_count".to_string());
    }

    fn increment_loan_sequence(&mut self) -> u32 {
        self.steps.borrow_mut().push(format!(
            "increment_loan_sequence={}",
            self.next_broker_sequence
        ));
        self.next_broker_sequence
    }

    fn update_broker(&mut self) {
        self.steps.borrow_mut().push("update_broker".to_string());
    }
}

impl LoanSetDoApplyAssociateAssetsSink for RecordingSink {
    type Asset = &'static str;

    fn associate_vault_asset(&mut self, asset: &Self::Asset) {
        self.steps.borrow_mut().push(format!("vault={asset}"));
    }

    fn associate_broker_asset(&mut self, asset: &Self::Asset) {
        self.steps.borrow_mut().push(format!("broker={asset}"));
    }

    fn associate_loan_asset(&mut self, asset: &Self::Asset) {
        self.steps.borrow_mut().push(format!("loan={asset}"));
    }
}

fn sample_broker_id() -> Uint256 {
    Uint256::from_hex("0123456789ABCDEFFEDCBA98765432100123456789ABCDEFFEDCBA9876543210")
        .expect("expected loan broker id should parse")
}

fn sample_post_transfer()
-> LoanSetDoApplyPostTransfer<&'static str, u32, &'static str, i64, u32, i64, &'static str, u32> {
    LoanSetDoApplyPostTransfer {
        loan_broker_id: sample_broker_id(),
        loan_scale: "scale",
        payment_interval: 60,
        borrower: "borrower",
        overpayment_enabled: true,
        default_grace_period: 30,
        principal_outstanding: 1_000,
        periodic_payment: 125,
        total_value_outstanding: 1_250,
        management_fee_outstanding: 25,
        payment_remaining: 24,
        vault_update: LoanSetDoApplyVaultUpdate {
            principal_requested: 1_000,
            interest_due: 250,
        },
        new_debt_delta: 1_250,
        vault_asset: "USD",
        vault_scale: 6,
    }
}

#[test]
fn tx_loan_set_do_apply_post_transfer_uses_current_on_success() {
    let steps = Rc::new(RefCell::new(Vec::new()));
    let mut sink = RecordingSink::new(steps.clone(), 8);
    let allocated_loan_key = RefCell::new(None);

    let result = run_loan_set_do_apply_post_transfer(
        &mut sink,
        sample_post_transfer(),
        || {
            steps.borrow_mut().push("setup.start_date".to_string());
            55
        },
        || {
            steps.borrow_mut().push("setup.loan_sequence".to_string());
            7
        },
        |loan_id| {
            steps.borrow_mut().push("allocate_loan".to_string());
            *allocated_loan_key.borrow_mut() = Some(loan_id);
            "loan"
        },
        |field, default_value| {
            steps
                .borrow_mut()
                .push(format!("copy_field={field:?}:{default_value:?}"));
        },
        || {
            steps.borrow_mut().push("broker_dir_link".to_string());
            Ter::TES_SUCCESS
        },
        || {
            steps.borrow_mut().push("borrower_dir_link".to_string());
            Ter::TES_SUCCESS
        },
    );

    assert_eq!(result, Ter::TES_SUCCESS);
    assert_eq!(
        *allocated_loan_key.borrow(),
        Some(loan_key(sample_broker_id(), 7))
    );
    assert_eq!(steps.borrow().last(), Some(&"loan=USD".to_string()));
}

#[test]
fn tx_loan_set_do_apply_post_transfer_returns_broker_update_failure_unchanged() {
    let steps = Rc::new(RefCell::new(Vec::new()));
    let mut sink = RecordingSink::new(steps.clone(), 0);

    let result = run_loan_set_do_apply_post_transfer(
        &mut sink,
        sample_post_transfer(),
        || 55,
        || 7,
        |_| "loan",
        |_, _| {},
        || {
            steps.borrow_mut().push("broker_dir_link".to_string());
            Ter::TES_SUCCESS
        },
        || {
            steps.borrow_mut().push("borrower_dir_link".to_string());
            Ter::TES_SUCCESS
        },
    );

    assert_eq!(result, Ter::TEC_MAX_SEQUENCE_REACHED);
    assert_eq!(trans_token(result), "tecMAX_SEQUENCE_REACHED");
    assert!(!steps.borrow().iter().any(|step| step == "broker_dir_link"));
    assert!(!steps.borrow().iter().any(|step| step == "vault=USD"));
}

#[test]
fn tx_loan_set_do_apply_post_transfer_short_circuits_on_broker_dir_link_failure() {
    let steps = Rc::new(RefCell::new(Vec::new()));
    let mut sink = RecordingSink::new(steps.clone(), 8);

    let result = run_loan_set_do_apply_post_transfer(
        &mut sink,
        sample_post_transfer(),
        || 55,
        || 7,
        |_| "loan",
        |_, _| {},
        || {
            steps.borrow_mut().push("broker_dir_link".to_string());
            Ter::TEC_DIR_FULL
        },
        || {
            steps.borrow_mut().push("borrower_dir_link".to_string());
            Ter::TES_SUCCESS
        },
    );

    assert_eq!(result, Ter::TEC_DIR_FULL);
    assert_eq!(trans_token(result), "tecDIR_FULL");
    assert!(
        !steps
            .borrow()
            .iter()
            .any(|step| step == "borrower_dir_link")
    );
    assert!(!steps.borrow().iter().any(|step| step == "vault=USD"));
}

#[test]
fn tx_loan_set_do_apply_post_transfer_short_circuits_on_borrower_dir_link_failure() {
    let steps = Rc::new(RefCell::new(Vec::new()));
    let mut sink = RecordingSink::new(steps.clone(), 8);

    let result = run_loan_set_do_apply_post_transfer(
        &mut sink,
        sample_post_transfer(),
        || 55,
        || 7,
        |_| "loan",
        |_, _| {},
        || {
            steps.borrow_mut().push("broker_dir_link".to_string());
            Ter::TES_SUCCESS
        },
        || {
            steps.borrow_mut().push("borrower_dir_link".to_string());
            Ter::TEC_DIR_FULL
        },
    );

    assert_eq!(result, Ter::TEC_DIR_FULL);
    assert_eq!(trans_token(result), "tecDIR_FULL");
    assert!(steps.borrow().iter().any(|step| step == "broker_dir_link"));
    assert!(
        steps
            .borrow()
            .iter()
            .any(|step| step == "borrower_dir_link")
    );
    assert!(!steps.borrow().iter().any(|step| step == "vault=USD"));
}
