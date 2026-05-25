//! Higher transfer plus post-transfer helper for the LoanSet transactor
//! after the debt and cover guards succeed.
//!
//! This module preserves the exact deterministic behavior around:
//!
//! - incrementing the borrower owner count first,
//! - checking the borrower reserve against that updated owner count,
//! - running the borrower holding setup and borrower auth in the reference implementation
//!   order,
//! - running the landed origination-fee transfer shell next, and
//! - only then entering the landed post-transfer shell, returning the first
//!   failing `TER` unchanged.

use protocol::Ter;

use crate::{
    check_loan_set_do_apply_borrower_reserve, run_loan_set_do_apply_add_empty_holding,
    run_loan_set_do_apply_origination_fee_account_send_multi, run_loan_set_do_apply_require_auth,
};

pub fn run_loan_set_do_apply_transfer_and_post_transfer<
    Balance,
    OwnerCount,
    Amount,
    IncrementBorrowerOwnerCount,
    ComputeAccountReserve,
    AddBorrowerHolding,
    CheckBorrowerAuth,
    AddOwnerHolding,
    CheckOwnerAuth,
    AccountSendMulti,
    RunPostTransfer,
>(
    account_is_borrower: bool,
    pre_fee_balance: &Balance,
    borrower_balance: &Balance,
    origination_fee: &Amount,
    zero: &Amount,
    increment_borrower_owner_count: IncrementBorrowerOwnerCount,
    compute_account_reserve: ComputeAccountReserve,
    add_borrower_holding: AddBorrowerHolding,
    check_borrower_auth: CheckBorrowerAuth,
    add_owner_holding: AddOwnerHolding,
    check_owner_auth: CheckOwnerAuth,
    account_send_multi: AccountSendMulti,
    run_post_transfer: RunPostTransfer,
) -> Ter
where
    Balance: PartialOrd,
    OwnerCount: Copy,
    Amount: PartialEq,
    IncrementBorrowerOwnerCount: FnOnce() -> OwnerCount,
    ComputeAccountReserve: FnOnce(OwnerCount) -> Balance,
    AddBorrowerHolding: FnOnce() -> Ter,
    CheckBorrowerAuth: FnOnce() -> Ter,
    AddOwnerHolding: FnOnce() -> Ter,
    CheckOwnerAuth: FnOnce() -> Ter,
    AccountSendMulti: FnOnce() -> Ter,
    RunPostTransfer: FnOnce() -> Ter,
{
    let owner_count = increment_borrower_owner_count();

    if let Err(err) = check_loan_set_do_apply_borrower_reserve(
        account_is_borrower,
        pre_fee_balance,
        borrower_balance,
        owner_count,
        compute_account_reserve,
    ) {
        return err.ter();
    }

    let borrower_holding_result = run_loan_set_do_apply_add_empty_holding(add_borrower_holding);
    if borrower_holding_result != Ter::TES_SUCCESS {
        return borrower_holding_result;
    }

    let borrower_auth_result = run_loan_set_do_apply_require_auth(check_borrower_auth);
    if borrower_auth_result != Ter::TES_SUCCESS {
        return borrower_auth_result;
    }

    let transfer_result = run_loan_set_do_apply_origination_fee_account_send_multi(
        origination_fee,
        zero,
        add_owner_holding,
        check_owner_auth,
        account_send_multi,
    );
    if transfer_result != Ter::TES_SUCCESS {
        return transfer_result;
    }

    run_post_transfer()
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use protocol::{Ter, trans_token};

    use super::run_loan_set_do_apply_transfer_and_post_transfer;

    #[test]
    fn loan_set_do_apply_transfer_and_post_transfer_uses_current_on_success() {
        let steps = RefCell::new(Vec::new());

        let result = run_loan_set_do_apply_transfer_and_post_transfer(
            true,
            &30_u32,
            &1_u32,
            &5_u32,
            &0_u32,
            || {
                steps.borrow_mut().push("increment_owner_count".to_string());
                4_u32
            },
            |owner_count| {
                steps
                    .borrow_mut()
                    .push(format!("compute_reserve owner_count={owner_count}"));
                30_u32
            },
            || {
                steps
                    .borrow_mut()
                    .push("borrower_add_empty_holding".to_string());
                Ter::TEC_DUPLICATE
            },
            || {
                steps.borrow_mut().push("borrower_require_auth".to_string());
                Ter::TES_SUCCESS
            },
            || {
                steps
                    .borrow_mut()
                    .push("owner_add_empty_holding".to_string());
                Ter::TEC_DUPLICATE
            },
            || {
                steps.borrow_mut().push("owner_require_auth".to_string());
                Ter::TES_SUCCESS
            },
            || {
                steps.borrow_mut().push("account_send_multi".to_string());
                Ter::TES_SUCCESS
            },
            || {
                steps.borrow_mut().push("post_transfer".to_string());
                Ter::TES_SUCCESS
            },
        );

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(
            steps.into_inner(),
            vec![
                "increment_owner_count",
                "compute_reserve owner_count=4",
                "borrower_add_empty_holding",
                "borrower_require_auth",
                "owner_add_empty_holding",
                "owner_require_auth",
                "account_send_multi",
                "post_transfer",
            ]
        );
    }

    #[test]
    fn loan_set_do_apply_transfer_and_post_transfer_skips_owner_holding_for_zero_fee() {
        let steps = RefCell::new(Vec::new());

        let result = run_loan_set_do_apply_transfer_and_post_transfer(
            true,
            &30_u32,
            &1_u32,
            &0_u32,
            &0_u32,
            || {
                steps.borrow_mut().push("increment_owner_count".to_string());
                4_u32
            },
            |owner_count| {
                steps
                    .borrow_mut()
                    .push(format!("compute_reserve owner_count={owner_count}"));
                30_u32
            },
            || {
                steps
                    .borrow_mut()
                    .push("borrower_add_empty_holding".to_string());
                Ter::TES_SUCCESS
            },
            || {
                steps.borrow_mut().push("borrower_require_auth".to_string());
                Ter::TES_SUCCESS
            },
            || {
                steps
                    .borrow_mut()
                    .push("owner_add_empty_holding".to_string());
                Ter::TES_SUCCESS
            },
            || {
                steps.borrow_mut().push("owner_require_auth".to_string());
                Ter::TES_SUCCESS
            },
            || {
                steps.borrow_mut().push("account_send_multi".to_string());
                Ter::TES_SUCCESS
            },
            || {
                steps.borrow_mut().push("post_transfer".to_string());
                Ter::TES_SUCCESS
            },
        );

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(
            steps.into_inner(),
            vec![
                "increment_owner_count",
                "compute_reserve owner_count=4",
                "borrower_add_empty_holding",
                "borrower_require_auth",
                "owner_require_auth",
                "account_send_multi",
                "post_transfer",
            ]
        );
    }

    #[test]
    fn loan_set_do_apply_transfer_and_post_transfer_returns_reserve_failure_before_transfers() {
        let steps = RefCell::new(Vec::new());

        let result = run_loan_set_do_apply_transfer_and_post_transfer(
            true,
            &29_u32,
            &100_u32,
            &5_u32,
            &0_u32,
            || {
                steps.borrow_mut().push("increment_owner_count".to_string());
                4_u32
            },
            |owner_count| {
                steps
                    .borrow_mut()
                    .push(format!("compute_reserve owner_count={owner_count}"));
                30_u32
            },
            || {
                steps
                    .borrow_mut()
                    .push("borrower_add_empty_holding".to_string());
                Ter::TES_SUCCESS
            },
            || {
                steps.borrow_mut().push("borrower_require_auth".to_string());
                Ter::TES_SUCCESS
            },
            || {
                steps
                    .borrow_mut()
                    .push("owner_add_empty_holding".to_string());
                Ter::TES_SUCCESS
            },
            || {
                steps.borrow_mut().push("owner_require_auth".to_string());
                Ter::TES_SUCCESS
            },
            || {
                steps.borrow_mut().push("account_send_multi".to_string());
                Ter::TES_SUCCESS
            },
            || {
                steps.borrow_mut().push("post_transfer".to_string());
                Ter::TES_SUCCESS
            },
        );

        assert_eq!(result, Ter::TEC_INSUFFICIENT_RESERVE);
        assert_eq!(trans_token(result), "tecINSUFFICIENT_RESERVE");
        assert_eq!(
            steps.into_inner(),
            vec!["increment_owner_count", "compute_reserve owner_count=4"]
        );
    }

    #[test]
    fn loan_set_do_apply_transfer_and_post_transfer_short_circuits_on_borrower_auth_failure() {
        let steps = RefCell::new(Vec::new());

        let result = run_loan_set_do_apply_transfer_and_post_transfer(
            true,
            &30_u32,
            &1_u32,
            &5_u32,
            &0_u32,
            || {
                steps.borrow_mut().push("increment_owner_count".to_string());
                4_u32
            },
            |owner_count| {
                steps
                    .borrow_mut()
                    .push(format!("compute_reserve owner_count={owner_count}"));
                30_u32
            },
            || {
                steps
                    .borrow_mut()
                    .push("borrower_add_empty_holding".to_string());
                Ter::TES_SUCCESS
            },
            || {
                steps.borrow_mut().push("borrower_require_auth".to_string());
                Ter::TER_NO_RIPPLE
            },
            || {
                steps
                    .borrow_mut()
                    .push("owner_add_empty_holding".to_string());
                Ter::TES_SUCCESS
            },
            || {
                steps.borrow_mut().push("owner_require_auth".to_string());
                Ter::TES_SUCCESS
            },
            || {
                steps.borrow_mut().push("account_send_multi".to_string());
                Ter::TES_SUCCESS
            },
            || {
                steps.borrow_mut().push("post_transfer".to_string());
                Ter::TES_SUCCESS
            },
        );

        assert_eq!(result, Ter::TER_NO_RIPPLE);
        assert_eq!(trans_token(result), "terNO_RIPPLE");
        assert_eq!(
            steps.into_inner(),
            vec![
                "increment_owner_count",
                "compute_reserve owner_count=4",
                "borrower_add_empty_holding",
                "borrower_require_auth",
            ]
        );
    }

    #[test]
    fn loan_set_do_apply_transfer_and_post_transfer_returns_post_transfer_failure_unchanged() {
        let steps = RefCell::new(Vec::new());

        let result = run_loan_set_do_apply_transfer_and_post_transfer(
            true,
            &30_u32,
            &1_u32,
            &5_u32,
            &0_u32,
            || {
                steps.borrow_mut().push("increment_owner_count".to_string());
                4_u32
            },
            |owner_count| {
                steps
                    .borrow_mut()
                    .push(format!("compute_reserve owner_count={owner_count}"));
                30_u32
            },
            || {
                steps
                    .borrow_mut()
                    .push("borrower_add_empty_holding".to_string());
                Ter::TES_SUCCESS
            },
            || {
                steps.borrow_mut().push("borrower_require_auth".to_string());
                Ter::TES_SUCCESS
            },
            || {
                steps
                    .borrow_mut()
                    .push("owner_add_empty_holding".to_string());
                Ter::TES_SUCCESS
            },
            || {
                steps.borrow_mut().push("owner_require_auth".to_string());
                Ter::TES_SUCCESS
            },
            || {
                steps.borrow_mut().push("account_send_multi".to_string());
                Ter::TES_SUCCESS
            },
            || {
                steps.borrow_mut().push("post_transfer".to_string());
                Ter::TEC_MAX_SEQUENCE_REACHED
            },
        );

        assert_eq!(result, Ter::TEC_MAX_SEQUENCE_REACHED);
        assert_eq!(trans_token(result), "tecMAX_SEQUENCE_REACHED");
        assert_eq!(
            steps.into_inner(),
            vec![
                "increment_owner_count",
                "compute_reserve owner_count=4",
                "borrower_add_empty_holding",
                "borrower_require_auth",
                "owner_add_empty_holding",
                "owner_require_auth",
                "account_send_multi",
                "post_transfer",
            ]
        );
    }
}
