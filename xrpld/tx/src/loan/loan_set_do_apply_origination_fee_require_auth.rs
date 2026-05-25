//! Broker-owner `requireAuth(...)` step after origination-fee holding setup for
//! the LoanSet transactor.
//!
//! This module preserves the deterministic behavior around:
//!
//! - still skipping the owner-side holding setup when `originationFee == 0`,
//! - still short-circuiting on the first non-success holding result other than
//!   `tecDUPLICATE`,
//! - always invoking the broker-owner auth check after a success
//!   or duplicate-tolerant holding outcome, and
//! - returning that auth check's `TER` unchanged.

use protocol::Ter;

use crate::{
    run_loan_set_do_apply_origination_fee_add_empty_holding, run_loan_set_do_apply_require_auth,
};

pub fn run_loan_set_do_apply_origination_fee_require_auth<
    Amount,
    AddEmptyHolding,
    CheckRequireAuth,
>(
    origination_fee: &Amount,
    zero: &Amount,
    add_empty_holding: AddEmptyHolding,
    check_require_auth: CheckRequireAuth,
) -> Ter
where
    Amount: PartialEq,
    AddEmptyHolding: FnOnce() -> Ter,
    CheckRequireAuth: FnOnce() -> Ter,
{
    let holding_result = run_loan_set_do_apply_origination_fee_add_empty_holding(
        origination_fee,
        zero,
        add_empty_holding,
    );
    if holding_result != Ter::TES_SUCCESS {
        return holding_result;
    }

    run_loan_set_do_apply_require_auth(check_require_auth)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use protocol::{Ter, trans_token};

    use super::run_loan_set_do_apply_origination_fee_require_auth;

    #[test]
    fn loan_set_do_apply_origination_fee_require_auth_checks_auth_for_zero_fee() {
        let auth_calls = Cell::new(0_u32);

        let result = run_loan_set_do_apply_origination_fee_require_auth(
            &0_u32,
            &0_u32,
            || Ter::TEC_INSUFFICIENT_RESERVE,
            || {
                auth_calls.set(auth_calls.get() + 1);
                Ter::TEC_NO_AUTH
            },
        );

        assert_eq!(result, Ter::TEC_NO_AUTH);
        assert_eq!(auth_calls.get(), 1);
    }

    #[test]
    fn loan_set_do_apply_origination_fee_require_auth_short_circuits_on_holding_failure() {
        let auth_calls = Cell::new(0_u32);

        let result = run_loan_set_do_apply_origination_fee_require_auth(
            &1_u32,
            &0_u32,
            || Ter::TEC_INSUFFICIENT_RESERVE,
            || {
                auth_calls.set(auth_calls.get() + 1);
                Ter::TEC_NO_AUTH
            },
        );

        assert_eq!(result, Ter::TEC_INSUFFICIENT_RESERVE);
        assert_eq!(auth_calls.get(), 0);
    }

    #[test]
    fn loan_set_do_apply_origination_fee_require_auth_continues_after_duplicate() {
        let auth_calls = Cell::new(0_u32);

        let result = run_loan_set_do_apply_origination_fee_require_auth(
            &1_u32,
            &0_u32,
            || Ter::TEC_DUPLICATE,
            || {
                auth_calls.set(auth_calls.get() + 1);
                Ter::TEC_NO_AUTH
            },
        );

        assert_eq!(result, Ter::TEC_NO_AUTH);
        assert_eq!(auth_calls.get(), 1);
    }

    #[test]
    fn loan_set_do_apply_origination_fee_require_auth_returns_success() {
        let result = run_loan_set_do_apply_origination_fee_require_auth(
            &1_u32,
            &0_u32,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
        );

        assert_eq!(result, Ter::TES_SUCCESS);
    }

    #[test]
    fn loan_set_do_apply_origination_fee_require_auth_returns_no_auth_unchanged() {
        let result = run_loan_set_do_apply_origination_fee_require_auth(
            &1_u32,
            &0_u32,
            || Ter::TES_SUCCESS,
            || Ter::TEC_NO_AUTH,
        );

        assert_eq!(result, Ter::TEC_NO_AUTH);
        assert_eq!(trans_token(result), "tecNO_AUTH");
    }

    #[test]
    fn loan_set_do_apply_origination_fee_require_auth_returns_owner_line_reserve_failure_unchanged()
    {
        let result = run_loan_set_do_apply_origination_fee_require_auth(
            &1_u32,
            &0_u32,
            || Ter::TEC_NO_LINE_INSUF_RESERVE,
            || Ter::TEC_NO_AUTH,
        );

        assert_eq!(result, Ter::TEC_NO_LINE_INSUF_RESERVE);
        assert_eq!(trans_token(result), "tecNO_LINE_INSUF_RESERVE");
    }
}
