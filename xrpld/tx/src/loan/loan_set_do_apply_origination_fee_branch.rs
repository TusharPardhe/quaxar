//! Origination-fee owner-side branch gate for the LoanSet transactor.
//!
//! This module preserves the deterministic behavior around:
//!
//! - skipping the owner-side setup entirely when `originationFee == 0`,
//! - invoking the owner-side setup exactly once when
//!   `originationFee != 0`, and
//! - returning that callback's `TER` unchanged.

use protocol::Ter;

pub fn run_loan_set_do_apply_origination_fee_branch<Amount, RunOwnerFeeBranch>(
    origination_fee: &Amount,
    zero: &Amount,
    run_owner_fee_branch: RunOwnerFeeBranch,
) -> Ter
where
    Amount: PartialEq,
    RunOwnerFeeBranch: FnOnce() -> Ter,
{
    if origination_fee != zero {
        return run_owner_fee_branch();
    }

    Ter::TES_SUCCESS
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use protocol::{Ter, trans_token};

    use super::run_loan_set_do_apply_origination_fee_branch;

    #[test]
    fn loan_set_do_apply_origination_fee_branch_skips_owner_setup_for_zero_fee() {
        let calls = Cell::new(0_u32);

        let result = run_loan_set_do_apply_origination_fee_branch(&0_u32, &0_u32, || {
            calls.set(calls.get() + 1);
            Ter::TEC_INSUFFICIENT_RESERVE
        });

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(calls.get(), 0);
    }

    #[test]
    fn loan_set_do_apply_origination_fee_branch_runs_owner_setup_for_non_zero_fee() {
        let calls = Cell::new(0_u32);

        let result = run_loan_set_do_apply_origination_fee_branch(&1_u32, &0_u32, || {
            calls.set(calls.get() + 1);
            Ter::TES_SUCCESS
        });

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn loan_set_do_apply_origination_fee_branch_returns_insufficient_reserve_unchanged() {
        let result = run_loan_set_do_apply_origination_fee_branch(&1_u32, &0_u32, || {
            Ter::TEC_INSUFFICIENT_RESERVE
        });

        assert_eq!(result, Ter::TEC_INSUFFICIENT_RESERVE);
        assert_eq!(trans_token(result), "tecINSUFFICIENT_RESERVE");
    }

    #[test]
    fn loan_set_do_apply_origination_fee_branch_returns_owner_line_reserve_failure_unchanged() {
        let result = run_loan_set_do_apply_origination_fee_branch(&1_u32, &0_u32, || {
            Ter::TEC_NO_LINE_INSUF_RESERVE
        });

        assert_eq!(result, Ter::TEC_NO_LINE_INSUF_RESERVE);
        assert_eq!(trans_token(result), "tecNO_LINE_INSUF_RESERVE");
    }
}
