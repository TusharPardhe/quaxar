//! Integration tests that pin the narrowed Rust
//! `LoanSet.cpp::doApply()` fixed-field loan-population shell to the current
//! C++ behavior.

use tx::{
    LoanSetDoApplyLoanFixedFieldSink, LoanSetDoApplyLoanFixedFields,
    run_loan_set_do_apply_loan_fixed_fields,
};

#[derive(Default)]
struct RecordingSink {
    steps: Vec<String>,
}

impl LoanSetDoApplyLoanFixedFieldSink for RecordingSink {
    type LoanScale = &'static str;
    type StartDate = u32;
    type PaymentInterval = u32;
    type BrokerId = &'static str;
    type AccountId = &'static str;

    fn set_loan_scale(&mut self, value: Self::LoanScale) {
        self.steps.push(format!("loan_scale={value}"));
    }

    fn set_start_date(&mut self, value: Self::StartDate) {
        self.steps.push(format!("start_date={value}"));
    }

    fn set_payment_interval(&mut self, value: Self::PaymentInterval) {
        self.steps.push(format!("payment_interval={value}"));
    }

    fn set_loan_sequence(&mut self, value: u32) {
        self.steps.push(format!("loan_sequence={value}"));
    }

    fn set_loan_broker_id(&mut self, value: Self::BrokerId) {
        self.steps.push(format!("loan_broker_id={value}"));
    }

    fn set_borrower(&mut self, value: Self::AccountId) {
        self.steps.push(format!("borrower={value}"));
    }

    fn set_overpayment_flag(&mut self) {
        self.steps.push("overpayment_flag".to_string());
    }
}

fn sample_fields(
    overpayment_enabled: bool,
) -> LoanSetDoApplyLoanFixedFields<&'static str, u32, u32, &'static str, &'static str> {
    LoanSetDoApplyLoanFixedFields {
        loan_scale: "scale",
        start_date: 55,
        payment_interval: 600,
        loan_sequence: 7,
        loan_broker_id: "broker",
        borrower: "borrower",
        overpayment_enabled,
    }
}

#[test]
fn tx_loan_set_do_apply_loan_fixed_fields_sets_fields_in() {
    let mut sink = RecordingSink::default();

    run_loan_set_do_apply_loan_fixed_fields(&mut sink, sample_fields(true));

    assert_eq!(
        sink.steps,
        vec![
            "loan_scale=scale",
            "start_date=55",
            "payment_interval=600",
            "loan_sequence=7",
            "loan_broker_id=broker",
            "borrower=borrower",
            "overpayment_flag",
        ]
    );
}

#[test]
fn tx_loan_set_do_apply_loan_fixed_fields_skips_overpayment_flag_when_disabled() {
    let mut sink = RecordingSink::default();

    run_loan_set_do_apply_loan_fixed_fields(&mut sink, sample_fields(false));

    assert_eq!(
        sink.steps,
        vec![
            "loan_scale=scale",
            "start_date=55",
            "payment_interval=600",
            "loan_sequence=7",
            "loan_broker_id=broker",
            "borrower=borrower",
        ]
    );
}

#[test]
fn tx_loan_set_do_apply_loan_fixed_fields_keeps_zero_sequence_path() {
    let mut sink = RecordingSink::default();

    run_loan_set_do_apply_loan_fixed_fields(
        &mut sink,
        LoanSetDoApplyLoanFixedFields {
            loan_scale: "zero-scale",
            start_date: 99,
            payment_interval: 1,
            loan_sequence: 0,
            loan_broker_id: "broker-zero",
            borrower: "borrower-zero",
            overpayment_enabled: false,
        },
    );

    assert_eq!(
        sink.steps,
        vec![
            "loan_scale=zero-scale",
            "start_date=99",
            "payment_interval=1",
            "loan_sequence=0",
            "loan_broker_id=broker-zero",
            "borrower=borrower-zero",
        ]
    );
}
