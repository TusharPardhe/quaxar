//! Current Rust helper mirroring the top-level `loanMakePayment(...)`
//! control flow from the reference implementation.
//!
//! This ports the current branch ordering around:
//!
//! - rejecting already-paid loans,
//! - rejecting missing `NextPaymentDueDate`,
//! - rejecting overdue non-late payments,
//! - `Full` payment dispatch,
//! - `Late` payment dispatch,
//! - regular periodic-payment looping,
//! - optional overpayment handling after regular payments,
//! - and the the reference implementation split between impossible helper `Noop` results
//!   for full/late payments versus tolerated `Noop` results for
//!   overpayments.

use std::ops::{Add, Sub};

use protocol::Ter;

use crate::{LoanPayPaymentParts, LoanPayPaymentType};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanPayPeriodicComponents<Amount, Marker> {
    pub marker: Marker,
    pub total_due: Amount,
    pub is_final: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoanPayMakePaymentAttempt<T> {
    Applied(T),
    Noop,
    Failed(Ter),
}

pub trait LoanPayMakePaymentSink {
    type Amount;
    type PeriodicMarker;

    fn payment_remaining(&self) -> u32;
    fn principal_outstanding_is_zero(&self) -> bool;
    fn total_value_outstanding(&self) -> &Self::Amount;
    fn next_payment_due_date_is_zero(&self) -> bool;
    fn loan_scale(&self) -> i32;
    fn overpayment_enabled(&self) -> bool;

    fn has_expired(&mut self) -> bool;

    fn compute_full_payment(
        &mut self,
        amount: &Self::Amount,
    ) -> LoanPayMakePaymentAttempt<LoanPayPaymentParts<Self::Amount>>;

    fn compute_periodic_payment(
        &mut self,
    ) -> LoanPayPeriodicComponents<Self::Amount, Self::PeriodicMarker>;

    fn apply_periodic_payment(
        &mut self,
        periodic: LoanPayPeriodicComponents<Self::Amount, Self::PeriodicMarker>,
    ) -> LoanPayPaymentParts<Self::Amount>;

    fn compute_late_payment(
        &mut self,
        periodic: LoanPayPeriodicComponents<Self::Amount, Self::PeriodicMarker>,
        amount: &Self::Amount,
    ) -> LoanPayMakePaymentAttempt<LoanPayPaymentParts<Self::Amount>>;

    fn round_amount_for_overpayment(
        &mut self,
        amount: &Self::Amount,
        loan_scale: i32,
    ) -> Self::Amount;

    fn compute_overpayment(
        &mut self,
        overpayment: &Self::Amount,
    ) -> LoanPayMakePaymentAttempt<LoanPayPaymentParts<Self::Amount>>;
}

fn add_parts<Amount>(total: &mut LoanPayPaymentParts<Amount>, next: LoanPayPaymentParts<Amount>)
where
    Amount: Add<Output = Amount> + Clone,
{
    total.principal_paid = total.principal_paid.clone() + next.principal_paid;
    total.interest_paid = total.interest_paid.clone() + next.interest_paid;
    total.fee_paid = total.fee_paid.clone() + next.fee_paid;
    total.value_change = total.value_change.clone() + next.value_change;
}

pub fn run_loan_pay_make_payment<Sink>(
    sink: &mut Sink,
    amount: &Sink::Amount,
    zero_amount: &Sink::Amount,
    payment_type: LoanPayPaymentType,
    max_payments_per_transaction: usize,
) -> Result<LoanPayPaymentParts<Sink::Amount>, Ter>
where
    Sink: LoanPayMakePaymentSink,
    Sink::Amount: Clone + PartialOrd + Add<Output = Sink::Amount> + Sub<Output = Sink::Amount>,
{
    if sink.payment_remaining() == 0 || sink.principal_outstanding_is_zero() {
        return Err(Ter::TEC_KILLED);
    }

    if sink.next_payment_due_date_is_zero() {
        return Err(Ter::TEC_INTERNAL);
    }

    if payment_type != LoanPayPaymentType::Late && sink.has_expired() {
        return Err(Ter::TEC_EXPIRED);
    }

    if payment_type == LoanPayPaymentType::Full {
        return match sink.compute_full_payment(amount) {
            LoanPayMakePaymentAttempt::Applied(parts) => Ok(parts),
            LoanPayMakePaymentAttempt::Failed(ter) => Err(ter),
            LoanPayMakePaymentAttempt::Noop => Err(Ter::TEC_INTERNAL),
        };
    }

    let first_periodic = sink.compute_periodic_payment();
    if payment_type == LoanPayPaymentType::Late {
        return match sink.compute_late_payment(first_periodic, amount) {
            LoanPayMakePaymentAttempt::Applied(parts) => Ok(parts),
            LoanPayMakePaymentAttempt::Failed(ter) => Err(ter),
            LoanPayMakePaymentAttempt::Noop => Err(Ter::TEC_INTERNAL),
        };
    }

    let mut total_parts = LoanPayPaymentParts {
        principal_paid: zero_amount.clone(),
        interest_paid: zero_amount.clone(),
        fee_paid: zero_amount.clone(),
        value_change: zero_amount.clone(),
    };
    let mut total_paid = zero_amount.clone();
    let mut num_payments = 0_usize;
    let mut periodic = first_periodic;

    while amount.clone() >= total_paid.clone() + periodic.total_due.clone()
        && sink.payment_remaining() > 0
        && num_payments < max_payments_per_transaction
    {
        total_paid = total_paid + periodic.total_due.clone();
        let is_final = periodic.is_final;
        add_parts(&mut total_parts, sink.apply_periodic_payment(periodic));
        num_payments += 1;

        if is_final {
            break;
        }

        periodic = sink.compute_periodic_payment();
    }

    if num_payments == 0 {
        return Err(Ter::TEC_INSUFFICIENT_PAYMENT);
    }

    let rounded_amount = sink.round_amount_for_overpayment(amount, sink.loan_scale());
    if payment_type == LoanPayPaymentType::Overpayment
        && sink.overpayment_enabled()
        && sink.payment_remaining() > 0
        && total_paid < rounded_amount
        && num_payments < max_payments_per_transaction
    {
        let remaining = rounded_amount - total_paid;
        let overpayment = if remaining < *sink.total_value_outstanding() {
            remaining
        } else {
            sink.total_value_outstanding().clone()
        };
        match sink.compute_overpayment(&overpayment) {
            LoanPayMakePaymentAttempt::Applied(parts) => add_parts(&mut total_parts, parts),
            LoanPayMakePaymentAttempt::Failed(ter) => return Err(ter),
            LoanPayMakePaymentAttempt::Noop => {}
        }
    }

    Ok(total_parts)
}

#[cfg(test)]
mod tests {
    use super::{
        LoanPayMakePaymentAttempt, LoanPayMakePaymentSink, LoanPayPeriodicComponents,
        run_loan_pay_make_payment,
    };
    use crate::{LoanPayPaymentParts, LoanPayPaymentType};
    use protocol::Ter;

    #[derive(Clone)]
    struct TestPeriodic {
        total_due: i64,
        is_final: bool,
    }

    struct TestSink {
        payment_remaining: u32,
        principal_zero: bool,
        total_value_outstanding: i64,
        next_due_zero: bool,
        expired: bool,
        overpayment_enabled: bool,
        full_attempt: LoanPayMakePaymentAttempt<LoanPayPaymentParts<i64>>,
        late_attempt: LoanPayMakePaymentAttempt<LoanPayPaymentParts<i64>>,
        over_attempt: LoanPayMakePaymentAttempt<LoanPayPaymentParts<i64>>,
        periodic: Vec<TestPeriodic>,
        periodic_parts: Vec<LoanPayPaymentParts<i64>>,
        rounded_amount: i64,
        steps: Vec<&'static str>,
    }

    impl Default for TestSink {
        fn default() -> Self {
            Self {
                payment_remaining: 1,
                principal_zero: false,
                total_value_outstanding: 50,
                next_due_zero: false,
                expired: false,
                overpayment_enabled: false,
                full_attempt: LoanPayMakePaymentAttempt::Applied(LoanPayPaymentParts {
                    principal_paid: 10,
                    interest_paid: 1,
                    fee_paid: 0,
                    value_change: 0,
                }),
                late_attempt: LoanPayMakePaymentAttempt::Applied(LoanPayPaymentParts {
                    principal_paid: 7,
                    interest_paid: 2,
                    fee_paid: 1,
                    value_change: 3,
                }),
                over_attempt: LoanPayMakePaymentAttempt::Noop,
                periodic: vec![TestPeriodic {
                    total_due: 5,
                    is_final: true,
                }],
                periodic_parts: vec![LoanPayPaymentParts {
                    principal_paid: 4,
                    interest_paid: 1,
                    fee_paid: 0,
                    value_change: 0,
                }],
                rounded_amount: 0,
                steps: Vec::new(),
            }
        }
    }

    impl LoanPayMakePaymentSink for TestSink {
        type Amount = i64;
        type PeriodicMarker = usize;

        fn payment_remaining(&self) -> u32 {
            self.payment_remaining
        }

        fn principal_outstanding_is_zero(&self) -> bool {
            self.principal_zero
        }

        fn total_value_outstanding(&self) -> &Self::Amount {
            &self.total_value_outstanding
        }

        fn next_payment_due_date_is_zero(&self) -> bool {
            self.next_due_zero
        }

        fn loan_scale(&self) -> i32 {
            0
        }

        fn overpayment_enabled(&self) -> bool {
            self.overpayment_enabled
        }

        fn has_expired(&mut self) -> bool {
            self.steps.push("has_expired");
            self.expired
        }

        fn compute_full_payment(
            &mut self,
            _amount: &Self::Amount,
        ) -> LoanPayMakePaymentAttempt<LoanPayPaymentParts<Self::Amount>> {
            self.steps.push("full");
            self.full_attempt.clone()
        }

        fn compute_periodic_payment(
            &mut self,
        ) -> LoanPayPeriodicComponents<Self::Amount, Self::PeriodicMarker> {
            self.steps.push("compute_periodic");
            let periodic = self.periodic.remove(0);
            LoanPayPeriodicComponents {
                marker: self.periodic_parts.len(),
                total_due: periodic.total_due,
                is_final: periodic.is_final,
            }
        }

        fn apply_periodic_payment(
            &mut self,
            _periodic: LoanPayPeriodicComponents<Self::Amount, Self::PeriodicMarker>,
        ) -> LoanPayPaymentParts<Self::Amount> {
            self.steps.push("apply_periodic");
            self.payment_remaining = self.payment_remaining.saturating_sub(1);
            self.periodic_parts.remove(0)
        }

        fn compute_late_payment(
            &mut self,
            _periodic: LoanPayPeriodicComponents<Self::Amount, Self::PeriodicMarker>,
            _amount: &Self::Amount,
        ) -> LoanPayMakePaymentAttempt<LoanPayPaymentParts<Self::Amount>> {
            self.steps.push("late");
            self.late_attempt.clone()
        }

        fn round_amount_for_overpayment(
            &mut self,
            _amount: &Self::Amount,
            _loan_scale: i32,
        ) -> Self::Amount {
            self.steps.push("round_amount");
            self.rounded_amount
        }

        fn compute_overpayment(
            &mut self,
            _overpayment: &Self::Amount,
        ) -> LoanPayMakePaymentAttempt<LoanPayPaymentParts<Self::Amount>> {
            self.steps.push("overpayment");
            self.over_attempt.clone()
        }
    }

    #[test]
    fn loan_pay_make_payment_rejects_paid_off_loan() {
        let mut sink = TestSink {
            payment_remaining: 0,
            ..TestSink::default()
        };

        let result = run_loan_pay_make_payment(&mut sink, &10, &0, LoanPayPaymentType::Regular, 8);

        assert_eq!(result, Err(Ter::TEC_KILLED));
        assert!(sink.steps.is_empty());
    }

    #[test]
    fn loan_pay_make_payment_rejects_missing_due_date() {
        let mut sink = TestSink {
            next_due_zero: true,
            ..TestSink::default()
        };

        let result = run_loan_pay_make_payment(&mut sink, &10, &0, LoanPayPaymentType::Regular, 8);

        assert_eq!(result, Err(Ter::TEC_INTERNAL));
        assert!(sink.steps.is_empty());
    }

    #[test]
    fn loan_pay_make_payment_rejects_overdue_regular_payment_before_branch_dispatch() {
        let mut sink = TestSink {
            expired: true,
            ..TestSink::default()
        };

        let result = run_loan_pay_make_payment(&mut sink, &10, &0, LoanPayPaymentType::Regular, 8);

        assert_eq!(result, Err(Ter::TEC_EXPIRED));
        assert_eq!(sink.steps, vec!["has_expired"]);
    }

    #[test]
    fn loan_pay_make_payment_dispatches_full_branch() {
        let mut sink = TestSink::default();

        let result = run_loan_pay_make_payment(&mut sink, &20, &0, LoanPayPaymentType::Full, 8)
            .expect("full payment should succeed");

        assert_eq!(
            result,
            LoanPayPaymentParts {
                principal_paid: 10,
                interest_paid: 1,
                fee_paid: 0,
                value_change: 0,
            }
        );
        assert_eq!(sink.steps, vec!["has_expired", "full"]);
    }

    #[test]
    fn loan_pay_make_payment_maps_impossible_full_noop_to_internal() {
        let mut sink = TestSink {
            full_attempt: LoanPayMakePaymentAttempt::Noop,
            ..TestSink::default()
        };

        let result = run_loan_pay_make_payment(&mut sink, &20, &0, LoanPayPaymentType::Full, 8);

        assert_eq!(result, Err(Ter::TEC_INTERNAL));
        assert_eq!(sink.steps, vec!["has_expired", "full"]);
    }

    #[test]
    fn loan_pay_make_payment_dispatches_late_branch() {
        let mut sink = TestSink::default();

        let result = run_loan_pay_make_payment(&mut sink, &20, &0, LoanPayPaymentType::Late, 8)
            .expect("late payment should succeed");

        assert_eq!(
            result,
            LoanPayPaymentParts {
                principal_paid: 7,
                interest_paid: 2,
                fee_paid: 1,
                value_change: 3,
            }
        );
        assert_eq!(sink.steps, vec!["compute_periodic", "late"]);
    }

    #[test]
    fn loan_pay_make_payment_returns_insufficient_payment_when_regular_loop_never_runs() {
        let mut sink = TestSink {
            periodic: vec![TestPeriodic {
                total_due: 15,
                is_final: true,
            }],
            ..TestSink::default()
        };

        let result = run_loan_pay_make_payment(&mut sink, &10, &0, LoanPayPaymentType::Regular, 8);

        assert_eq!(result, Err(Ter::TEC_INSUFFICIENT_PAYMENT));
        assert_eq!(sink.steps, vec!["has_expired", "compute_periodic"]);
    }

    #[test]
    fn loan_pay_make_payment_accumulates_regular_periodic_payments() {
        let mut sink = TestSink {
            payment_remaining: 2,
            periodic: vec![
                TestPeriodic {
                    total_due: 5,
                    is_final: false,
                },
                TestPeriodic {
                    total_due: 5,
                    is_final: true,
                },
            ],
            periodic_parts: vec![
                LoanPayPaymentParts {
                    principal_paid: 4,
                    interest_paid: 1,
                    fee_paid: 0,
                    value_change: 0,
                },
                LoanPayPaymentParts {
                    principal_paid: 3,
                    interest_paid: 2,
                    fee_paid: 0,
                    value_change: 0,
                },
            ],
            ..TestSink::default()
        };

        let result = run_loan_pay_make_payment(&mut sink, &10, &0, LoanPayPaymentType::Regular, 8)
            .expect("regular payment should succeed");

        assert_eq!(
            result,
            LoanPayPaymentParts {
                principal_paid: 7,
                interest_paid: 3,
                fee_paid: 0,
                value_change: 0,
            }
        );
        assert_eq!(
            sink.steps,
            vec![
                "has_expired",
                "compute_periodic",
                "apply_periodic",
                "compute_periodic",
                "apply_periodic",
                "round_amount",
            ]
        );
    }

    #[test]
    fn loan_pay_make_payment_tolerates_noop_overpayment() {
        let mut sink = TestSink {
            overpayment_enabled: true,
            payment_remaining: 2,
            rounded_amount: 7,
            periodic: vec![
                TestPeriodic {
                    total_due: 5,
                    is_final: false,
                },
                TestPeriodic {
                    total_due: 5,
                    is_final: false,
                },
            ],
            periodic_parts: vec![LoanPayPaymentParts {
                principal_paid: 4,
                interest_paid: 1,
                fee_paid: 0,
                value_change: 0,
            }],
            ..TestSink::default()
        };

        let result =
            run_loan_pay_make_payment(&mut sink, &7, &0, LoanPayPaymentType::Overpayment, 8)
                .expect("overpayment noop should still succeed");

        assert_eq!(
            result,
            LoanPayPaymentParts {
                principal_paid: 4,
                interest_paid: 1,
                fee_paid: 0,
                value_change: 0,
            }
        );
        assert_eq!(
            sink.steps,
            vec![
                "has_expired",
                "compute_periodic",
                "apply_periodic",
                "compute_periodic",
                "round_amount",
                "overpayment",
            ]
        );
    }
}
