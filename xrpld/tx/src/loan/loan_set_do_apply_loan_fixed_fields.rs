//! Fixed-field population helper for the LoanSet transactor.
//!
//! This module preserves the exact deterministic behavior around:
//!
//! - setting the fixed loan fields in the reference implementation order, and
//! - conditionally setting the overpayment flag after those fixed fields.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanSetDoApplyLoanFixedFields<LoanScale, StartDate, PaymentInterval, BrokerId, AccountId>
{
    pub loan_scale: LoanScale,
    pub start_date: StartDate,
    pub payment_interval: PaymentInterval,
    pub loan_sequence: u32,
    pub loan_broker_id: BrokerId,
    pub borrower: AccountId,
    pub overpayment_enabled: bool,
}

pub trait LoanSetDoApplyLoanFixedFieldSink {
    type LoanScale;
    type StartDate;
    type PaymentInterval;
    type BrokerId;
    type AccountId;

    fn set_loan_scale(&mut self, value: Self::LoanScale);
    fn set_start_date(&mut self, value: Self::StartDate);
    fn set_payment_interval(&mut self, value: Self::PaymentInterval);
    fn set_loan_sequence(&mut self, value: u32);
    fn set_loan_broker_id(&mut self, value: Self::BrokerId);
    fn set_borrower(&mut self, value: Self::AccountId);
    fn set_overpayment_flag(&mut self);
}

pub fn run_loan_set_do_apply_loan_fixed_fields<Sink>(
    sink: &mut Sink,
    fields: LoanSetDoApplyLoanFixedFields<
        Sink::LoanScale,
        Sink::StartDate,
        Sink::PaymentInterval,
        Sink::BrokerId,
        Sink::AccountId,
    >,
) where
    Sink: LoanSetDoApplyLoanFixedFieldSink,
{
    sink.set_loan_scale(fields.loan_scale);
    sink.set_start_date(fields.start_date);
    sink.set_payment_interval(fields.payment_interval);
    sink.set_loan_sequence(fields.loan_sequence);
    sink.set_loan_broker_id(fields.loan_broker_id);
    sink.set_borrower(fields.borrower);

    if fields.overpayment_enabled {
        sink.set_overpayment_flag();
    }
}

#[cfg(test)]
mod tests {
    use super::{
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
    fn loan_set_do_apply_loan_fixed_fields_sets_fields_in() {
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
    fn loan_set_do_apply_loan_fixed_fields_skips_overpayment_flag_when_disabled() {
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
    fn loan_set_do_apply_loan_fixed_fields_keeps_sequence_and_borrower_values_unchanged() {
        let mut sink = RecordingSink::default();

        run_loan_set_do_apply_loan_fixed_fields(
            &mut sink,
            LoanSetDoApplyLoanFixedFields {
                loan_scale: "other-scale",
                start_date: 99,
                payment_interval: 1,
                loan_sequence: 0,
                loan_broker_id: "other-broker",
                borrower: "other-borrower",
                overpayment_enabled: false,
            },
        );

        assert_eq!(
            sink.steps,
            vec![
                "loan_scale=other-scale",
                "start_date=99",
                "payment_interval=1",
                "loan_sequence=0",
                "loan_broker_id=other-broker",
                "borrower=other-borrower",
            ]
        );
    }
}
