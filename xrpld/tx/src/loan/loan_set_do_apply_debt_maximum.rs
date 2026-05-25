//! Debt-maximum guard for the LoanSet transactor.
//!
//! This module ports the deterministic behavior around:
//!
//! - treating `debtMaximum == 0` as unlimited,
//! - rejecting only when `debtMaximum < newDebtTotal`, and
//! - mapping that failure to `tecLIMIT_EXCEEDED` with the current warning
//!   string.

use protocol::Ter;

pub const LOAN_SET_DO_APPLY_DEBT_MAXIMUM_EXCEEDED_WARNING: &str =
    "Loan would exceed the maximum debt limit of the LoanBroker.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoanSetDoApplyDebtMaximumFailure {
    DebtMaximumExceeded,
}

impl LoanSetDoApplyDebtMaximumFailure {
    pub const fn ter(self) -> Ter {
        match self {
            Self::DebtMaximumExceeded => Ter::TEC_LIMIT_EXCEEDED,
        }
    }

    pub const fn warning_message(self) -> &'static str {
        match self {
            Self::DebtMaximumExceeded => LOAN_SET_DO_APPLY_DEBT_MAXIMUM_EXCEEDED_WARNING,
        }
    }
}

pub fn check_loan_set_do_apply_debt_maximum<Amount>(
    debt_maximum: &Amount,
    new_debt_total: &Amount,
    zero: &Amount,
) -> Result<(), LoanSetDoApplyDebtMaximumFailure>
where
    Amount: PartialEq + PartialOrd,
{
    if debt_maximum != zero && debt_maximum < new_debt_total {
        return Err(LoanSetDoApplyDebtMaximumFailure::DebtMaximumExceeded);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use protocol::trans_token;

    use super::{
        LOAN_SET_DO_APPLY_DEBT_MAXIMUM_EXCEEDED_WARNING, LoanSetDoApplyDebtMaximumFailure,
        check_loan_set_do_apply_debt_maximum,
    };

    #[test]
    fn loan_set_do_apply_debt_maximum_treats_zero_as_unlimited() {
        let result = check_loan_set_do_apply_debt_maximum(&0_u32, &1_000_u32, &0_u32);

        assert_eq!(result, Ok(()));
    }

    #[test]
    fn loan_set_do_apply_debt_maximum_allows_equal_total() {
        let result = check_loan_set_do_apply_debt_maximum(&100_u32, &100_u32, &0_u32);

        assert_eq!(result, Ok(()));
    }

    #[test]
    fn loan_set_do_apply_debt_maximum_allows_total_below_limit() {
        let result = check_loan_set_do_apply_debt_maximum(&101_u32, &100_u32, &0_u32);

        assert_eq!(result, Ok(()));
    }

    #[test]
    fn loan_set_do_apply_debt_maximum_returns_limit_exceeded() {
        let result = check_loan_set_do_apply_debt_maximum(&99_u32, &100_u32, &0_u32);

        assert_eq!(
            result,
            Err(LoanSetDoApplyDebtMaximumFailure::DebtMaximumExceeded)
        );
        let err = result.expect_err("debt maximum should fail");
        assert_eq!(err.ter(), protocol::Ter::TEC_LIMIT_EXCEEDED);
        assert_eq!(trans_token(err.ter()), "tecLIMIT_EXCEEDED");
        assert_eq!(
            err.warning_message(),
            LOAN_SET_DO_APPLY_DEBT_MAXIMUM_EXCEEDED_WARNING
        );
    }
}
