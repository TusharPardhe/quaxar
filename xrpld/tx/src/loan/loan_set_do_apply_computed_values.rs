//! Computed-values invariant block for the LoanSet transactor.
//!
//! This module ports the deterministic behavior around:
//!
//! - rejecting negative `managementFeeDue`,
//! - rejecting non-positive `valueOutstanding` and `periodicPayment`, and
//! - mapping that invariant failure to `tecINTERNAL` with the current warning
//!   text.

use std::fmt::Display;

use protocol::Ter;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanSetDoApplyComputedValues<Amount> {
    pub management_fee_due: Amount,
    pub total_value_outstanding: Amount,
    pub periodic_payment: Amount,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoanSetDoApplyComputedValuesFailure {
    InvalidComputedValues {
        management_fee_due_display: String,
        total_value_outstanding_display: String,
        periodic_payment_display: String,
    },
}

impl LoanSetDoApplyComputedValuesFailure {
    pub const fn ter(&self) -> Ter {
        match self {
            Self::InvalidComputedValues { .. } => Ter::TEC_INTERNAL,
        }
    }

    pub fn warning_message(&self) -> String {
        match self {
            Self::InvalidComputedValues {
                management_fee_due_display,
                total_value_outstanding_display,
                periodic_payment_display,
            } => format!(
                "Computed loan properties are invalid. Does not compute. Management fee: {management_fee_due_display}. Total Value: {total_value_outstanding_display}. PeriodicPayment: {periodic_payment_display}"
            ),
        }
    }
}

pub fn check_loan_set_do_apply_computed_values<Amount>(
    values: &LoanSetDoApplyComputedValues<Amount>,
    zero: &Amount,
) -> Result<(), LoanSetDoApplyComputedValuesFailure>
where
    Amount: Display + PartialOrd,
{
    if values.management_fee_due < *zero
        || values.total_value_outstanding <= *zero
        || values.periodic_payment <= *zero
    {
        return Err(LoanSetDoApplyComputedValuesFailure::InvalidComputedValues {
            management_fee_due_display: values.management_fee_due.to_string(),
            total_value_outstanding_display: values.total_value_outstanding.to_string(),
            periodic_payment_display: values.periodic_payment.to_string(),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use protocol::trans_token;

    use super::{
        LoanSetDoApplyComputedValues, LoanSetDoApplyComputedValuesFailure,
        check_loan_set_do_apply_computed_values,
    };

    fn valid_values() -> LoanSetDoApplyComputedValues<i64> {
        LoanSetDoApplyComputedValues {
            management_fee_due: 0,
            total_value_outstanding: 100,
            periodic_payment: 10,
        }
    }

    #[test]
    fn loan_set_do_apply_computed_values_accepts_zero_management_fee() {
        let result = check_loan_set_do_apply_computed_values(&valid_values(), &0);

        assert_eq!(result, Ok(()));
    }

    #[test]
    fn loan_set_do_apply_computed_values_returns_internal_for_negative_management_fee() {
        let result = check_loan_set_do_apply_computed_values(
            &LoanSetDoApplyComputedValues {
                management_fee_due: -1,
                ..valid_values()
            },
            &0,
        );

        assert_eq!(
            result,
            Err(LoanSetDoApplyComputedValuesFailure::InvalidComputedValues {
                management_fee_due_display: "-1".to_string(),
                total_value_outstanding_display: "100".to_string(),
                periodic_payment_display: "10".to_string(),
            })
        );
        let err = result.expect_err("negative management fee should fail");
        assert_eq!(err.ter(), protocol::Ter::TEC_INTERNAL);
        assert_eq!(trans_token(err.ter()), "tecINTERNAL");
        assert_eq!(
            err.warning_message(),
            "Computed loan properties are invalid. Does not compute. Management fee: -1. Total Value: 100. PeriodicPayment: 10"
        );
    }

    #[test]
    fn loan_set_do_apply_computed_values_returns_internal_for_non_positive_total_value() {
        let result = check_loan_set_do_apply_computed_values(
            &LoanSetDoApplyComputedValues {
                total_value_outstanding: 0,
                ..valid_values()
            },
            &0,
        );

        let err = result.expect_err("non-positive total value should fail");
        assert_eq!(err.ter(), protocol::Ter::TEC_INTERNAL);
        assert_eq!(trans_token(err.ter()), "tecINTERNAL");
        assert_eq!(
            err.warning_message(),
            "Computed loan properties are invalid. Does not compute. Management fee: 0. Total Value: 0. PeriodicPayment: 10"
        );
    }

    #[test]
    fn loan_set_do_apply_computed_values_returns_internal_for_non_positive_periodic_payment() {
        let result = check_loan_set_do_apply_computed_values(
            &LoanSetDoApplyComputedValues {
                periodic_payment: 0,
                ..valid_values()
            },
            &0,
        );

        let err = result.expect_err("non-positive periodic payment should fail");
        assert_eq!(err.ter(), protocol::Ter::TEC_INTERNAL);
        assert_eq!(trans_token(err.ter()), "tecINTERNAL");
        assert_eq!(
            err.warning_message(),
            "Computed loan properties are invalid. Does not compute. Management fee: 0. Total Value: 100. PeriodicPayment: 0"
        );
    }
}
