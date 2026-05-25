//! Current Rust helper mirroring the early the LoanPay transactor
//! payment-parts validity bundle.
//!
//! This module preserves the current checks around:
//!
//! - principal, interest, and fee parts staying non-negative,
//! - `principalPaid + interestPaid > 0`, and
//! - the component sum matching a caller-provided total.

use core::ops::Add;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanPayPaymentValidityFacts<Amount> {
    pub principal_paid: Amount,
    pub interest_paid: Amount,
    pub fee_paid: Amount,
    pub total_paid: Amount,
    pub parts_sum: Amount,
    pub principal_paid_non_negative: bool,
    pub interest_paid_non_negative: bool,
    pub fee_paid_non_negative: bool,
    pub principal_and_interest_positive: bool,
    pub aggregate_relation_holds: bool,
    pub all_assertions_hold: bool,
}

pub fn compute_loan_pay_payment_validity<Amount>(
    principal_paid: Amount,
    interest_paid: Amount,
    fee_paid: Amount,
    total_paid: Amount,
    zero_amount: Amount,
) -> LoanPayPaymentValidityFacts<Amount>
where
    Amount: Clone + PartialEq + PartialOrd + Add<Output = Amount>,
{
    let principal_paid_non_negative = principal_paid.clone() >= zero_amount.clone();
    let interest_paid_non_negative = interest_paid.clone() >= zero_amount.clone();
    let fee_paid_non_negative = fee_paid.clone() >= zero_amount.clone();

    let principal_and_interest = principal_paid.clone() + interest_paid.clone();
    let principal_and_interest_positive = principal_and_interest.clone() > zero_amount.clone();
    let parts_sum = principal_and_interest.clone() + fee_paid.clone();
    let aggregate_relation_holds = parts_sum == total_paid.clone();

    let all_assertions_hold = principal_paid_non_negative
        && interest_paid_non_negative
        && fee_paid_non_negative
        && principal_and_interest_positive
        && aggregate_relation_holds;

    LoanPayPaymentValidityFacts {
        principal_paid,
        interest_paid,
        fee_paid,
        total_paid,
        parts_sum,
        principal_paid_non_negative,
        interest_paid_non_negative,
        fee_paid_non_negative,
        principal_and_interest_positive,
        aggregate_relation_holds,
        all_assertions_hold,
    }
}

#[cfg(test)]
mod tests {
    use super::compute_loan_pay_payment_validity;

    #[test]
    fn compute_loan_pay_payment_validity_valid_case() {
        let facts = compute_loan_pay_payment_validity(25_i64, 12, 3, 40, 0);

        assert_eq!(facts.principal_paid, 25);
        assert_eq!(facts.interest_paid, 12);
        assert_eq!(facts.fee_paid, 3);
        assert_eq!(facts.total_paid, 40);
        assert_eq!(facts.parts_sum, 40);
        assert!(facts.principal_paid_non_negative);
        assert!(facts.interest_paid_non_negative);
        assert!(facts.fee_paid_non_negative);
        assert!(facts.principal_and_interest_positive);
        assert!(facts.aggregate_relation_holds);
        assert!(facts.all_assertions_hold);
    }

    #[test]
    fn compute_loan_pay_payment_validity_rejects_fee_only_payment() {
        let facts = compute_loan_pay_payment_validity(0_i64, 0, 3, 3, 0);

        assert!(facts.principal_paid_non_negative);
        assert!(facts.interest_paid_non_negative);
        assert!(facts.fee_paid_non_negative);
        assert!(!facts.principal_and_interest_positive);
        assert!(facts.aggregate_relation_holds);
        assert!(!facts.all_assertions_hold);
    }

    #[test]
    fn compute_loan_pay_payment_validity_rejects_negative_part() {
        let facts = compute_loan_pay_payment_validity(-1_i64, 1, 0, 0, 0);

        assert!(!facts.principal_paid_non_negative);
        assert!(facts.interest_paid_non_negative);
        assert!(facts.fee_paid_non_negative);
        assert!(!facts.principal_and_interest_positive);
        assert!(facts.aggregate_relation_holds);
        assert!(!facts.all_assertions_hold);
    }

    #[test]
    fn compute_loan_pay_payment_validity_rejects_aggregate_mismatch() {
        let facts = compute_loan_pay_payment_validity(10_i64, 3, 1, 20, 0);

        assert!(facts.principal_paid_non_negative);
        assert!(facts.interest_paid_non_negative);
        assert!(facts.fee_paid_non_negative);
        assert!(facts.principal_and_interest_positive);
        assert!(!facts.aggregate_relation_holds);
        assert!(!facts.all_assertions_hold);
    }
}
