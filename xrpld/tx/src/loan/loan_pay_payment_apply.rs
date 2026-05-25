//! Current Rust helper mirroring the the LoanPay transactor
//! payment-invocation and immediate post-payment shell.
//!
//! This module preserves the current deterministic behavior around:
//!
//! - invoking `loanMakePayment(...)` with the selected payment type,
//! - returning the payment error unchanged,
//! - updating the loan immediately after a successful payment,
//! - validating non-negative principal, interest, and fee parts, and
//! - mapping an invalid negative part to `tecLIMIT_EXCEEDED` before the later
//!   broker update.

use core::ops::Add;

use protocol::Ter;

use crate::loan_pay::{LoanPayPaymentParts, LoanPayPaymentType};
use crate::loan_pay_payment_validity::{
    LoanPayPaymentValidityFacts, compute_loan_pay_payment_validity,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanPayPaymentApplyFacts<Amount> {
    pub amount: Amount,
    pub payment_type: LoanPayPaymentType,
    pub zero_amount: Amount,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanPayPaymentApplyResult<Amount> {
    pub payment_parts: LoanPayPaymentParts<Amount>,
    pub payment_validity: LoanPayPaymentValidityFacts<Amount>,
}

pub trait LoanPayPaymentApplySink {
    type Loan;
    type Broker;
    type Asset;
    type Amount;

    fn make_payment(
        &mut self,
        asset: &Self::Asset,
        loan: &mut Self::Loan,
        broker: &mut Self::Broker,
        amount: &Self::Amount,
        payment_type: LoanPayPaymentType,
    ) -> Result<LoanPayPaymentParts<Self::Amount>, Ter>;

    fn update_loan(&mut self, loan: &Self::Loan);
}

pub fn run_loan_pay_payment_apply<Sink>(
    sink: &mut Sink,
    asset: &Sink::Asset,
    loan: &mut Sink::Loan,
    broker: &mut Sink::Broker,
    facts: LoanPayPaymentApplyFacts<Sink::Amount>,
) -> Result<LoanPayPaymentApplyResult<Sink::Amount>, Ter>
where
    Sink: LoanPayPaymentApplySink,
    Sink::Amount: Clone + PartialEq + PartialOrd + Add<Output = Sink::Amount>,
{
    let payment_parts =
        sink.make_payment(asset, loan, broker, &facts.amount, facts.payment_type)?;

    sink.update_loan(loan);

    let payment_validity = compute_loan_pay_payment_validity(
        payment_parts.principal_paid.clone(),
        payment_parts.interest_paid.clone(),
        payment_parts.fee_paid.clone(),
        payment_parts.principal_paid.clone()
            + payment_parts.interest_paid.clone()
            + payment_parts.fee_paid.clone(),
        facts.zero_amount,
    );

    if !payment_validity.principal_paid_non_negative
        || !payment_validity.interest_paid_non_negative
        || !payment_validity.fee_paid_non_negative
    {
        return Err(Ter::TEC_LIMIT_EXCEEDED);
    }

    Ok(LoanPayPaymentApplyResult {
        payment_parts,
        payment_validity,
    })
}

#[cfg(test)]
mod tests {
    use protocol::Ter;

    use super::{LoanPayPaymentApplyFacts, LoanPayPaymentApplySink, run_loan_pay_payment_apply};
    use crate::loan_pay::{LoanPayPaymentParts, LoanPayPaymentType};

    struct TestSink {
        steps: Vec<&'static str>,
        result: Result<LoanPayPaymentParts<i64>, Ter>,
    }

    impl LoanPayPaymentApplySink for TestSink {
        type Loan = i32;
        type Broker = i32;
        type Asset = &'static str;
        type Amount = i64;

        fn make_payment(
            &mut self,
            asset: &Self::Asset,
            _loan: &mut Self::Loan,
            _broker: &mut Self::Broker,
            amount: &Self::Amount,
            payment_type: LoanPayPaymentType,
        ) -> Result<LoanPayPaymentParts<Self::Amount>, Ter> {
            assert_eq!(*asset, "USD");
            assert_eq!(*amount, 40);
            assert_eq!(payment_type, LoanPayPaymentType::Full);
            self.steps.push("make_payment");
            self.result.clone()
        }

        fn update_loan(&mut self, _loan: &Self::Loan) {
            self.steps.push("update_loan");
        }
    }

    #[test]
    fn loan_pay_payment_apply_updates_loan_after_successful_payment() {
        let mut sink = TestSink {
            steps: Vec::new(),
            result: Ok(LoanPayPaymentParts {
                principal_paid: 25,
                interest_paid: 12,
                fee_paid: 3,
                value_change: 7,
            }),
        };

        let result = run_loan_pay_payment_apply(
            &mut sink,
            &"USD",
            &mut 1,
            &mut 2,
            LoanPayPaymentApplyFacts {
                amount: 40,
                payment_type: LoanPayPaymentType::Full,
                zero_amount: 0,
            },
        )
        .expect("success");

        assert_eq!(sink.steps, vec!["make_payment", "update_loan"]);
        assert_eq!(result.payment_parts.principal_paid, 25);
        assert!(result.payment_validity.all_assertions_hold);
    }

    #[test]
    fn loan_pay_payment_apply_passthroughs_payment_error_before_update() {
        let mut sink = TestSink {
            steps: Vec::new(),
            result: Err(Ter::TEC_PATH_DRY),
        };

        let result = run_loan_pay_payment_apply(
            &mut sink,
            &"USD",
            &mut 1,
            &mut 2,
            LoanPayPaymentApplyFacts {
                amount: 40,
                payment_type: LoanPayPaymentType::Full,
                zero_amount: 0,
            },
        );

        assert_eq!(result, Err(Ter::TEC_PATH_DRY));
        assert_eq!(sink.steps, vec!["make_payment"]);
    }

    #[test]
    fn loan_pay_payment_apply_maps_negative_payment_part_to_limit_exceeded() {
        let mut sink = TestSink {
            steps: Vec::new(),
            result: Ok(LoanPayPaymentParts {
                principal_paid: -1,
                interest_paid: 1,
                fee_paid: 0,
                value_change: 0,
            }),
        };

        let result = run_loan_pay_payment_apply(
            &mut sink,
            &"USD",
            &mut 1,
            &mut 2,
            LoanPayPaymentApplyFacts {
                amount: 40,
                payment_type: LoanPayPaymentType::Full,
                zero_amount: 0,
            },
        );

        assert_eq!(result, Err(Ter::TEC_LIMIT_EXCEEDED));
        assert_eq!(sink.steps, vec!["make_payment", "update_loan"]);
    }
}
