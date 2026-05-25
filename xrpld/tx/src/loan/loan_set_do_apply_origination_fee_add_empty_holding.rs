//! Non-zero origination-fee owner-side holding step for
//! the LoanSet transactor.
//!
//! This module preserves the deterministic behavior around:
//!
//! - skipping the owner-side holding setup when `originationFee == 0`,
//! - invoking the holding-creation attempt exactly once when
//!   `originationFee != 0`,
//! - treating `tesSUCCESS` and `tecDUPLICATE` as continue cases, and
//! - returning any other `TER` unchanged.

use protocol::Ter;

use crate::{
    run_loan_set_do_apply_add_empty_holding, run_loan_set_do_apply_origination_fee_branch,
};

pub fn run_loan_set_do_apply_origination_fee_add_empty_holding<Amount, AddEmptyHolding>(
    origination_fee: &Amount,
    zero: &Amount,
    add_empty_holding: AddEmptyHolding,
) -> Ter
where
    Amount: PartialEq,
    AddEmptyHolding: FnOnce() -> Ter,
{
    run_loan_set_do_apply_origination_fee_branch(origination_fee, zero, || {
        run_loan_set_do_apply_add_empty_holding(add_empty_holding)
    })
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use protocol::{Ter, trans_token};

    use super::run_loan_set_do_apply_origination_fee_add_empty_holding;

    #[test]
    fn loan_set_do_apply_origination_fee_add_empty_holding_skips_setup_for_zero_fee() {
        let calls = Cell::new(0_u32);

        let result =
            run_loan_set_do_apply_origination_fee_add_empty_holding(&0_u32, &0_u32, || {
                calls.set(calls.get() + 1);
                Ter::TEC_INSUFFICIENT_RESERVE
            });

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(calls.get(), 0);
    }

    #[test]
    fn loan_set_do_apply_origination_fee_add_empty_holding_runs_setup_once() {
        let calls = Cell::new(0_u32);

        let result =
            run_loan_set_do_apply_origination_fee_add_empty_holding(&1_u32, &0_u32, || {
                calls.set(calls.get() + 1);
                Ter::TEC_DUPLICATE
            });

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn loan_set_do_apply_origination_fee_add_empty_holding_returns_success() {
        let result =
            run_loan_set_do_apply_origination_fee_add_empty_holding(&1_u32, &0_u32, || {
                Ter::TES_SUCCESS
            });

        assert_eq!(result, Ter::TES_SUCCESS);
    }

    #[test]
    fn loan_set_do_apply_origination_fee_add_empty_holding_ignores_duplicate() {
        let result =
            run_loan_set_do_apply_origination_fee_add_empty_holding(&1_u32, &0_u32, || {
                Ter::TEC_DUPLICATE
            });

        assert_eq!(result, Ter::TES_SUCCESS);
    }

    #[test]
    fn loan_set_do_apply_origination_fee_add_empty_holding_returns_insufficient_reserve_unchanged()
    {
        let result =
            run_loan_set_do_apply_origination_fee_add_empty_holding(&1_u32, &0_u32, || {
                Ter::TEC_INSUFFICIENT_RESERVE
            });

        assert_eq!(result, Ter::TEC_INSUFFICIENT_RESERVE);
        assert_eq!(trans_token(result), "tecINSUFFICIENT_RESERVE");
    }

    #[test]
    fn loan_set_do_apply_origination_fee_add_empty_holding_returns_owner_line_reserve_failure_unchanged()
     {
        let result =
            run_loan_set_do_apply_origination_fee_add_empty_holding(&1_u32, &0_u32, || {
                Ter::TEC_NO_LINE_INSUF_RESERVE
            });

        assert_eq!(result, Ter::TEC_NO_LINE_INSUF_RESERVE);
        assert_eq!(trans_token(result), "tecNO_LINE_INSUF_RESERVE");
    }
}
