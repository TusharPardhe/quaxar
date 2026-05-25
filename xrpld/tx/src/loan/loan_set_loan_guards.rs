//! the reference implementation helper mirrored into Rust.
//!
//! This module preserves the deterministic control flow around:
//!
//! - the total-interest presence guards,
//! - the first-payment-principal guard,
//! - the rounded-periodic-payment zero guard, and
//! - the rounded-payment amortization-count guard.

use std::{fmt::Display, ops::Sub};

use protocol::Ter;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanSetLoanGuardProperties<Amount> {
    pub periodic_payment: Amount,
    pub total_value_outstanding: Amount,
    pub loan_scale: i32,
    pub first_payment_principal: Amount,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoanSetLoanGuardsFailure {
    InterestExpectedButNone {
        principal_requested_display: String,
    },
    NoInterestExpectedButInterestDue {
        principal_requested_display: String,
    },
    UnableToPayPrincipal,
    RoundedPeriodicPaymentIsZero {
        periodic_payment_display: String,
    },
    RoundedPeriodicPaymentCannotCompleteLoan {
        periodic_payment_display: String,
        rounded_payment_display: String,
        total_value_outstanding_display: String,
        computed_payments: i64,
        payment_total: u32,
    },
}

impl LoanSetLoanGuardsFailure {
    pub const fn ter(&self) -> Ter {
        match self {
            Self::InterestExpectedButNone { .. } => Ter::TEC_PRECISION_LOSS,
            Self::NoInterestExpectedButInterestDue { .. } => Ter::TEC_INTERNAL,
            Self::UnableToPayPrincipal => Ter::TEC_PRECISION_LOSS,
            Self::RoundedPeriodicPaymentIsZero { .. } => Ter::TEC_PRECISION_LOSS,
            Self::RoundedPeriodicPaymentCannotCompleteLoan { .. } => Ter::TEC_PRECISION_LOSS,
        }
    }

    pub fn warning_message(&self) -> String {
        match self {
            Self::InterestExpectedButNone {
                principal_requested_display,
            } => {
                format!("Loan for {principal_requested_display} with interest has no interest due")
            }
            Self::NoInterestExpectedButInterestDue {
                principal_requested_display,
            } => {
                format!("Loan for {principal_requested_display} with no interest has interest due")
            }
            Self::UnableToPayPrincipal => "Loan is unable to pay principal.".to_string(),
            Self::RoundedPeriodicPaymentIsZero {
                periodic_payment_display,
            } => format!("Loan Periodic payment ({periodic_payment_display}) rounds to 0. "),
            Self::RoundedPeriodicPaymentCannotCompleteLoan {
                periodic_payment_display,
                rounded_payment_display,
                total_value_outstanding_display,
                computed_payments,
                payment_total,
            } => format!(
                "Loan Periodic payment ({periodic_payment_display}) rounding ({rounded_payment_display}) on a total value of {total_value_outstanding_display} can not complete the loan in the specified number of payments ({computed_payments} != {payment_total})"
            ),
        }
    }
}

pub fn check_loan_set_loan_guards<Asset, Amount, RoundPeriodicPayment, ComputePayments>(
    vault_asset: &Asset,
    principal_requested: &Amount,
    expect_interest: bool,
    payment_total: u32,
    properties: &LoanSetLoanGuardProperties<Amount>,
    zero: &Amount,
    round_periodic_payment: RoundPeriodicPayment,
    compute_payments: ComputePayments,
) -> Result<(), LoanSetLoanGuardsFailure>
where
    Amount: Clone + Display + PartialOrd + Sub<Output = Amount>,
    RoundPeriodicPayment: FnOnce(&Asset, &Amount, i32) -> Amount,
    ComputePayments: FnOnce(&Amount, &Amount) -> i64,
{
    let total_interest_outstanding =
        properties.total_value_outstanding.clone() - principal_requested.clone();

    if expect_interest && total_interest_outstanding <= zero.clone() {
        return Err(LoanSetLoanGuardsFailure::InterestExpectedButNone {
            principal_requested_display: principal_requested.to_string(),
        });
    }

    if !expect_interest && total_interest_outstanding > zero.clone() {
        return Err(LoanSetLoanGuardsFailure::NoInterestExpectedButInterestDue {
            principal_requested_display: principal_requested.to_string(),
        });
    }

    if properties.first_payment_principal <= zero.clone() {
        return Err(LoanSetLoanGuardsFailure::UnableToPayPrincipal);
    }

    let rounded_payment = round_periodic_payment(
        vault_asset,
        &properties.periodic_payment,
        properties.loan_scale,
    );
    if rounded_payment == zero.clone() {
        return Err(LoanSetLoanGuardsFailure::RoundedPeriodicPaymentIsZero {
            periodic_payment_display: properties.periodic_payment.to_string(),
        });
    }

    let computed_payments = compute_payments(&properties.total_value_outstanding, &rounded_payment);
    if computed_payments != i64::from(payment_total) {
        return Err(
            LoanSetLoanGuardsFailure::RoundedPeriodicPaymentCannotCompleteLoan {
                periodic_payment_display: properties.periodic_payment.to_string(),
                rounded_payment_display: rounded_payment.to_string(),
                total_value_outstanding_display: properties.total_value_outstanding.to_string(),
                computed_payments,
                payment_total,
            },
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use protocol::trans_token;

    use super::{LoanSetLoanGuardProperties, LoanSetLoanGuardsFailure, check_loan_set_loan_guards};

    fn base_properties() -> LoanSetLoanGuardProperties<i64> {
        LoanSetLoanGuardProperties {
            periodic_payment: 10,
            total_value_outstanding: 110,
            loan_scale: 4,
            first_payment_principal: 1,
        }
    }

    #[test]
    fn loan_set_loan_guards_returns_precision_loss_when_interest_is_expected_but_missing() {
        let properties = LoanSetLoanGuardProperties {
            total_value_outstanding: 100,
            ..base_properties()
        };

        let result = check_loan_set_loan_guards(
            &"USD",
            &100,
            true,
            12,
            &properties,
            &0,
            |_, _, _| unreachable!("rounding should not run"),
            |_, _| unreachable!("payment count should not run"),
        );

        assert_eq!(
            result,
            Err(LoanSetLoanGuardsFailure::InterestExpectedButNone {
                principal_requested_display: "100".to_string(),
            })
        );
        let err = result.expect_err("missing interest should fail");
        assert_eq!(err.ter(), protocol::Ter::TEC_PRECISION_LOSS);
        assert_eq!(trans_token(err.ter()), "tecPRECISION_LOSS");
        assert_eq!(
            err.warning_message(),
            "Loan for 100 with interest has no interest due"
        );
    }

    #[test]
    fn loan_set_loan_guards_returns_internal_when_interest_is_present_but_not_expected() {
        let result = check_loan_set_loan_guards(
            &"USD",
            &100,
            false,
            12,
            &base_properties(),
            &0,
            |_, _, _| unreachable!("rounding should not run"),
            |_, _| unreachable!("payment count should not run"),
        );

        assert_eq!(
            result,
            Err(LoanSetLoanGuardsFailure::NoInterestExpectedButInterestDue {
                principal_requested_display: "100".to_string(),
            })
        );
        let err = result.expect_err("unexpected interest should fail");
        assert_eq!(err.ter(), protocol::Ter::TEC_INTERNAL);
        assert_eq!(trans_token(err.ter()), "tecINTERNAL");
        assert_eq!(
            err.warning_message(),
            "Loan for 100 with no interest has interest due"
        );
    }

    #[test]
    fn loan_set_loan_guards_returns_precision_loss_when_first_payment_principal_is_not_positive() {
        let properties = LoanSetLoanGuardProperties {
            total_value_outstanding: 100,
            first_payment_principal: 0,
            ..base_properties()
        };

        let result = check_loan_set_loan_guards(
            &"USD",
            &100,
            false,
            12,
            &properties,
            &0,
            |_, _, _| unreachable!("rounding should not run"),
            |_, _| unreachable!("payment count should not run"),
        );

        assert_eq!(result, Err(LoanSetLoanGuardsFailure::UnableToPayPrincipal));
        let err = result.expect_err("non-positive first payment principal should fail");
        assert_eq!(err.ter(), protocol::Ter::TEC_PRECISION_LOSS);
        assert_eq!(err.warning_message(), "Loan is unable to pay principal.");
    }

    #[test]
    fn loan_set_loan_guards_returns_precision_loss_when_rounded_payment_is_zero() {
        let seen = RefCell::new(Vec::new());

        let result = check_loan_set_loan_guards(
            &"USD",
            &100,
            false,
            12,
            &LoanSetLoanGuardProperties {
                total_value_outstanding: 100,
                ..base_properties()
            },
            &0,
            |asset, payment, scale| {
                seen.borrow_mut().push(format!("{asset}:{payment}:{scale}"));
                0
            },
            |_, _| unreachable!("payment count should not run"),
        );

        assert_eq!(
            result,
            Err(LoanSetLoanGuardsFailure::RoundedPeriodicPaymentIsZero {
                periodic_payment_display: "10".to_string(),
            })
        );
        let err = result.expect_err("rounded payment zero should fail");
        assert_eq!(err.ter(), protocol::Ter::TEC_PRECISION_LOSS);
        assert_eq!(
            err.warning_message(),
            "Loan Periodic payment (10) rounds to 0. "
        );
        assert_eq!(*seen.borrow(), vec!["USD:10:4".to_string()]);
    }

    #[test]
    fn loan_set_loan_guards_returns_precision_loss_when_payment_count_mismatches() {
        let seen = RefCell::new(Vec::new());

        let result = check_loan_set_loan_guards(
            &"USD",
            &100,
            false,
            12,
            &LoanSetLoanGuardProperties {
                total_value_outstanding: 100,
                ..base_properties()
            },
            &0,
            |_, _, _| 11,
            |total_value, rounded_payment| {
                seen.borrow_mut()
                    .push(format!("{total_value}:{rounded_payment}"));
                11
            },
        );

        assert_eq!(
            result,
            Err(
                LoanSetLoanGuardsFailure::RoundedPeriodicPaymentCannotCompleteLoan {
                    periodic_payment_display: "10".to_string(),
                    rounded_payment_display: "11".to_string(),
                    total_value_outstanding_display: "100".to_string(),
                    computed_payments: 11,
                    payment_total: 12,
                }
            )
        );
        let err = result.expect_err("payment-count mismatch should fail");
        assert_eq!(err.ter(), protocol::Ter::TEC_PRECISION_LOSS);
        assert_eq!(
            err.warning_message(),
            "Loan Periodic payment (10) rounding (11) on a total value of 100 can not complete the loan in the specified number of payments (11 != 12)"
        );
        assert_eq!(*seen.borrow(), vec!["100:11".to_string()]);
    }

    #[test]
    fn loan_set_loan_guards_returns_success_when_all_guards_pass() {
        let result = check_loan_set_loan_guards(
            &"USD",
            &100,
            false,
            12,
            &LoanSetLoanGuardProperties {
                total_value_outstanding: 100,
                ..base_properties()
            },
            &0,
            |_, _, _| 11,
            |_, _| 12,
        );

        assert_eq!(result, Ok(()));
    }
}
