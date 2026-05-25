//! Cover-rate minimum guard for the LoanSet transactor.
//!
//! This module ports the deterministic behavior around:
//!
//! - computing the minimum required cover from `newDebtTotal` and
//!   `coverRateMinimum`,
//! - rejecting only when `coverAvailable` is strictly less than that computed
//!   minimum, and
//! - mapping that failure to `tecINSUFFICIENT_FUNDS` with the current warning
//!   string.

use protocol::Ter;

pub const LOAN_SET_DO_APPLY_INSUFFICIENT_FIRST_LOSS_CAPITAL_WARNING: &str =
    "Insufficient first-loss capital to cover the loan.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoanSetDoApplyCoverRateMinimumFailure {
    InsufficientFirstLossCapital,
}

impl LoanSetDoApplyCoverRateMinimumFailure {
    pub const fn ter(self) -> Ter {
        match self {
            Self::InsufficientFirstLossCapital => Ter::TEC_INSUFFICIENT_FUNDS,
        }
    }

    pub const fn warning_message(self) -> &'static str {
        match self {
            Self::InsufficientFirstLossCapital => {
                LOAN_SET_DO_APPLY_INSUFFICIENT_FIRST_LOSS_CAPITAL_WARNING
            }
        }
    }
}

pub fn check_loan_set_do_apply_cover_rate_minimum<Amount, Rate, ComputeRequiredCover>(
    cover_available: &Amount,
    new_debt_total: &Amount,
    cover_rate_minimum: Rate,
    compute_required_cover: ComputeRequiredCover,
) -> Result<(), LoanSetDoApplyCoverRateMinimumFailure>
where
    Amount: PartialOrd,
    Rate: Copy,
    ComputeRequiredCover: FnOnce(&Amount, Rate) -> Amount,
{
    let required_cover = compute_required_cover(new_debt_total, cover_rate_minimum);
    if cover_available < &required_cover {
        return Err(LoanSetDoApplyCoverRateMinimumFailure::InsufficientFirstLossCapital);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use protocol::trans_token;

    use super::{
        LOAN_SET_DO_APPLY_INSUFFICIENT_FIRST_LOSS_CAPITAL_WARNING,
        LoanSetDoApplyCoverRateMinimumFailure, check_loan_set_do_apply_cover_rate_minimum,
    };

    #[test]
    fn loan_set_do_apply_cover_rate_minimum_allows_equal_required_cover() {
        let result =
            check_loan_set_do_apply_cover_rate_minimum(&100_u32, &1_000_u32, 10_000_u32, |_, _| {
                100_u32
            });

        assert_eq!(result, Ok(()));
    }

    #[test]
    fn loan_set_do_apply_cover_rate_minimum_allows_more_than_required_cover() {
        let result =
            check_loan_set_do_apply_cover_rate_minimum(&101_u32, &1_000_u32, 10_000_u32, |_, _| {
                100_u32
            });

        assert_eq!(result, Ok(()));
    }

    #[test]
    fn loan_set_do_apply_cover_rate_minimum_returns_insufficient_funds() {
        let result =
            check_loan_set_do_apply_cover_rate_minimum(&99_u32, &1_000_u32, 10_000_u32, |_, _| {
                100_u32
            });

        assert_eq!(
            result,
            Err(LoanSetDoApplyCoverRateMinimumFailure::InsufficientFirstLossCapital)
        );
        let err = result.expect_err("insufficient first-loss capital should fail");
        assert_eq!(err.ter(), protocol::Ter::TEC_INSUFFICIENT_FUNDS);
        assert_eq!(trans_token(err.ter()), "tecINSUFFICIENT_FUNDS");
        assert_eq!(
            err.warning_message(),
            LOAN_SET_DO_APPLY_INSUFFICIENT_FIRST_LOSS_CAPITAL_WARNING
        );
    }

    #[test]
    fn loan_set_do_apply_cover_rate_minimum_computes_required_cover_once_from_current_inputs() {
        let seen = RefCell::new(Vec::new());

        let result = check_loan_set_do_apply_cover_rate_minimum(
            &99_u32,
            &1_000_u32,
            10_000_u32,
            |new_debt_total, cover_rate_minimum| {
                seen.borrow_mut()
                    .push(format!("{new_debt_total}:{cover_rate_minimum}"));
                100_u32
            },
        );

        assert_eq!(
            result,
            Err(LoanSetDoApplyCoverRateMinimumFailure::InsufficientFirstLossCapital)
        );
        assert_eq!(*seen.borrow(), vec!["1000:10000".to_string()]);
    }
}
