//! Dynamic-field population helper for the LoanSet transactor.
//!
//! This module preserves the exact deterministic behavior around:
//!
//! - setting the initial dynamic loan fields in the reference implementation order,
//! - computing `NextPaymentDueDate` from `startDate + paymentInterval`, and
//! - inserting the populated loan immediately after those writes.

use std::ops::Add;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanSetDoApplyLoanDynamicFields<Amount, Date, PaymentCount> {
    pub principal_outstanding: Amount,
    pub periodic_payment: Amount,
    pub total_value_outstanding: Amount,
    pub management_fee_outstanding: Amount,
    pub start_date: Date,
    pub payment_interval: Date,
    pub payment_remaining: PaymentCount,
}

pub trait LoanSetDoApplyLoanDynamicFieldSink {
    type Amount;
    type Date;
    type PaymentCount;

    fn set_principal_outstanding(&mut self, value: Self::Amount);
    fn set_periodic_payment(&mut self, value: Self::Amount);
    fn set_total_value_outstanding(&mut self, value: Self::Amount);
    fn set_management_fee_outstanding(&mut self, value: Self::Amount);
    fn set_previous_payment_due_date(&mut self, value: Self::Date);
    fn set_next_payment_due_date(&mut self, value: Self::Date);
    fn set_payment_remaining(&mut self, value: Self::PaymentCount);
    fn insert_loan(&mut self);
}

pub fn run_loan_set_do_apply_loan_dynamic_fields<Sink>(
    sink: &mut Sink,
    fields: LoanSetDoApplyLoanDynamicFields<Sink::Amount, Sink::Date, Sink::PaymentCount>,
) where
    Sink: LoanSetDoApplyLoanDynamicFieldSink,
    Sink::Date: Add<Sink::Date, Output = Sink::Date> + Copy + From<u32>,
{
    sink.set_principal_outstanding(fields.principal_outstanding);
    sink.set_periodic_payment(fields.periodic_payment);
    sink.set_total_value_outstanding(fields.total_value_outstanding);
    sink.set_management_fee_outstanding(fields.management_fee_outstanding);
    sink.set_previous_payment_due_date(0_u32.into());
    sink.set_next_payment_due_date(fields.start_date + fields.payment_interval);
    sink.set_payment_remaining(fields.payment_remaining);
    sink.insert_loan();
}

#[cfg(test)]
mod tests {
    use super::{
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
    fn loan_set_do_apply_loan_dynamic_fields_uses_current_cpp_write_order() {
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
    fn loan_set_do_apply_loan_dynamic_fields_computes_next_due_date_from_start_plus_interval() {
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
    fn loan_set_do_apply_loan_dynamic_fields_inserts_after_all_field_writes() {
        let mut sink = RecordingSink::default();

        run_loan_set_do_apply_loan_dynamic_fields(&mut sink, sample_fields());

        assert_eq!(sink.steps.last(), Some(&"insert_loan".to_string()));
    }
}
