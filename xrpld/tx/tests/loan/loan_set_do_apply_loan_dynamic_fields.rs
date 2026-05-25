//! Integration tests that pin the narrowed Rust
//! `LoanSet.cpp::doApply()` dynamic-field loan-population shell to the current
//! C++ behavior.

use tx::{
    LoanSetDoApplyLoanDynamicFieldSink, LoanSetDoApplyLoanDynamicFields,
    run_loan_set_do_apply_loan_dynamic_fields,
};

#[derive(Default)]
struct RecordingSink {
    steps: Vec<String>,
}

impl LoanSetDoApplyLoanDynamicFieldSink for RecordingSink {
    type Amount = i64;
    type Date = u32;
    type PaymentCount = u32;

    fn set_principal_outstanding(&mut self, value: Self::Amount) {
        self.steps.push(format!("principal_outstanding={value}"));
    }

    fn set_periodic_payment(&mut self, value: Self::Amount) {
        self.steps.push(format!("periodic_payment={value}"));
    }

    fn set_total_value_outstanding(&mut self, value: Self::Amount) {
        self.steps.push(format!("total_value_outstanding={value}"));
    }

    fn set_management_fee_outstanding(&mut self, value: Self::Amount) {
        self.steps
            .push(format!("management_fee_outstanding={value}"));
    }

    fn set_previous_payment_due_date(&mut self, value: Self::Date) {
        self.steps
            .push(format!("previous_payment_due_date={value}"));
    }

    fn set_next_payment_due_date(&mut self, value: Self::Date) {
        self.steps.push(format!("next_payment_due_date={value}"));
    }

    fn set_payment_remaining(&mut self, value: Self::PaymentCount) {
        self.steps.push(format!("payment_remaining={value}"));
    }

    fn insert_loan(&mut self) {
        self.steps.push("insert_loan".to_string());
    }
}

fn sample_fields() -> LoanSetDoApplyLoanDynamicFields<i64, u32, u32> {
    LoanSetDoApplyLoanDynamicFields {
        principal_outstanding: 1_000_000,
        periodic_payment: 25_000,
        total_value_outstanding: 1_050_000,
        management_fee_outstanding: 2_500,
        start_date: 1_000,
        payment_interval: 60,
        payment_remaining: 24,
    }
}

#[test]
fn tx_loan_set_do_apply_loan_dynamic_fields_uses_current_cpp_write_order() {
    let mut sink = RecordingSink::default();

    run_loan_set_do_apply_loan_dynamic_fields(&mut sink, sample_fields());

    assert_eq!(
        sink.steps,
        vec![
            "principal_outstanding=1000000",
            "periodic_payment=25000",
            "total_value_outstanding=1050000",
            "management_fee_outstanding=2500",
            "previous_payment_due_date=0",
            "next_payment_due_date=1060",
            "payment_remaining=24",
            "insert_loan",
        ]
    );
}

#[test]
fn tx_loan_set_do_apply_loan_dynamic_fields_computes_next_due_date() {
    let mut sink = RecordingSink::default();

    run_loan_set_do_apply_loan_dynamic_fields(
        &mut sink,
        LoanSetDoApplyLoanDynamicFields {
            start_date: 55,
            payment_interval: 600,
            ..sample_fields()
        },
    );

    assert!(
        sink.steps
            .contains(&"next_payment_due_date=655".to_string())
    );
}

#[test]
fn tx_loan_set_do_apply_loan_dynamic_fields_inserts_after_field_writes() {
    let mut sink = RecordingSink::default();

    run_loan_set_do_apply_loan_dynamic_fields(&mut sink, sample_fields());

    assert_eq!(sink.steps.last(), Some(&"insert_loan".to_string()));
}
