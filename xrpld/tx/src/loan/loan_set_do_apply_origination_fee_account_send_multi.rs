//! `accountSendMulti(...)` origination-fee transfer step for
//! the LoanSet transactor.
//!
//! This module preserves the deterministic behavior around:
//!
//! - still skipping the holding setup when `originationFee == 0`,
//! - still short-circuiting on the first non-success holding result other than
//!   `tecDUPLICATE`,
//! - still requiring broker-owner auth before any transfer attempt, and
//! - returning the transfer helper's `TER` unchanged.

use protocol::Ter;

use crate::run_loan_set_do_apply_origination_fee_require_auth;

pub fn run_loan_set_do_apply_origination_fee_account_send_multi<
    Amount,
    AddEmptyHolding,
    CheckRequireAuth,
    AccountSendMulti,
>(
    origination_fee: &Amount,
    zero: &Amount,
    add_empty_holding: AddEmptyHolding,
    check_require_auth: CheckRequireAuth,
    account_send_multi: AccountSendMulti,
) -> Ter
where
    Amount: PartialEq,
    AddEmptyHolding: FnOnce() -> Ter,
    CheckRequireAuth: FnOnce() -> Ter,
    AccountSendMulti: FnOnce() -> Ter,
{
    let auth_result = run_loan_set_do_apply_origination_fee_require_auth(
        origination_fee,
        zero,
        add_empty_holding,
        check_require_auth,
    );
    if auth_result != Ter::TES_SUCCESS {
        return auth_result;
    }

    account_send_multi()
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use protocol::{Ter, trans_token};

    use super::run_loan_set_do_apply_origination_fee_account_send_multi;

    #[test]
    fn loan_set_do_apply_origination_fee_account_send_multi_runs_transfer_for_zero_fee() {
        let holding_calls = Cell::new(0_u32);
        let transfer_calls = Cell::new(0_u32);

        let result = run_loan_set_do_apply_origination_fee_account_send_multi(
            &0_u32,
            &0_u32,
            || {
                holding_calls.set(holding_calls.get() + 1);
                Ter::TEC_INSUFFICIENT_RESERVE
            },
            || Ter::TES_SUCCESS,
            || {
                transfer_calls.set(transfer_calls.get() + 1);
                Ter::TEC_NO_AUTH
            },
        );

        assert_eq!(result, Ter::TEC_NO_AUTH);
        assert_eq!(holding_calls.get(), 0);
        assert_eq!(transfer_calls.get(), 1);
    }

    #[test]
    fn loan_set_do_apply_origination_fee_account_send_multi_short_circuits_on_holding_failure() {
        let auth_calls = Cell::new(0_u32);
        let transfer_calls = Cell::new(0_u32);

        let result = run_loan_set_do_apply_origination_fee_account_send_multi(
            &1_u32,
            &0_u32,
            || Ter::TEC_INSUFFICIENT_RESERVE,
            || {
                auth_calls.set(auth_calls.get() + 1);
                Ter::TES_SUCCESS
            },
            || {
                transfer_calls.set(transfer_calls.get() + 1);
                Ter::TEC_NO_AUTH
            },
        );

        assert_eq!(result, Ter::TEC_INSUFFICIENT_RESERVE);
        assert_eq!(auth_calls.get(), 0);
        assert_eq!(transfer_calls.get(), 0);
    }

    #[test]
    fn loan_set_do_apply_origination_fee_account_send_multi_short_circuits_on_auth_failure() {
        let transfer_calls = Cell::new(0_u32);

        let result = run_loan_set_do_apply_origination_fee_account_send_multi(
            &1_u32,
            &0_u32,
            || Ter::TES_SUCCESS,
            || Ter::TEC_NO_AUTH,
            || {
                transfer_calls.set(transfer_calls.get() + 1);
                Ter::TES_SUCCESS
            },
        );

        assert_eq!(result, Ter::TEC_NO_AUTH);
        assert_eq!(transfer_calls.get(), 0);
    }

    #[test]
    fn loan_set_do_apply_origination_fee_account_send_multi_continues_after_duplicate() {
        let transfer_calls = Cell::new(0_u32);

        let result = run_loan_set_do_apply_origination_fee_account_send_multi(
            &1_u32,
            &0_u32,
            || Ter::TEC_DUPLICATE,
            || Ter::TES_SUCCESS,
            || {
                transfer_calls.set(transfer_calls.get() + 1);
                Ter::TEC_NO_AUTH
            },
        );

        assert_eq!(result, Ter::TEC_NO_AUTH);
        assert_eq!(transfer_calls.get(), 1);
    }

    #[test]
    fn loan_set_do_apply_origination_fee_account_send_multi_returns_success() {
        let result = run_loan_set_do_apply_origination_fee_account_send_multi(
            &1_u32,
            &0_u32,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
        );

        assert_eq!(result, Ter::TES_SUCCESS);
    }

    #[test]
    fn loan_set_do_apply_origination_fee_account_send_multi_returns_no_auth_unchanged() {
        let result = run_loan_set_do_apply_origination_fee_account_send_multi(
            &1_u32,
            &0_u32,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TEC_NO_AUTH,
        );

        assert_eq!(result, Ter::TEC_NO_AUTH);
        assert_eq!(trans_token(result), "tecNO_AUTH");
    }
}
