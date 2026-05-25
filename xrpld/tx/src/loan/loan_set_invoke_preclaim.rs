//! Higher `LoanSet` preclaim composition order mirrored from
//! `LoanSet::invoke_preclaim(...)`.
//!
//! This module ports the exact current ordering around:
//!
//! - skipping all transactor prechecks when `sfAccount` is zero,
//! - otherwise running the shared `checkSeqProxy(...)`,
//!   `checkPriorTxAndLastLedger(...)`, `checkPermission(...)`, and
//!   `LoanSet::checkSign(...)` sequence,
//! - calculating the `LoanSet` base fee only after those checks succeed,
//! - running `checkFee(...)` only after that base-fee calculation,
//! - and falling through to the landed `LoanSet::preclaim(...)` tail only when
//!   all earlier steps succeed.

use protocol::{NotTec, Ter};

use crate::run_transactor_invoke_preclaim;

#[allow(clippy::too_many_arguments)]
pub fn run_loan_set_invoke_preclaim<Fee>(
    account_is_zero: bool,
    check_seq_proxy: impl FnOnce() -> NotTec,
    check_prior_tx_and_last_ledger: impl FnOnce() -> NotTec,
    check_permission: impl FnOnce() -> NotTec,
    check_sign: impl FnOnce() -> NotTec,
    calculate_base_fee: impl FnOnce() -> Fee,
    check_fee: impl FnOnce(Fee) -> Ter,
    preclaim: impl FnOnce() -> Ter,
) -> Ter {
    run_transactor_invoke_preclaim(
        account_is_zero,
        check_seq_proxy,
        check_prior_tx_and_last_ledger,
        check_permission,
        check_sign,
        calculate_base_fee,
        check_fee,
        preclaim,
    )
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use protocol::{Ter, trans_token};

    use super::run_loan_set_invoke_preclaim;

    #[test]
    fn loan_set_invoke_preclaim_skips_shared_checks_when_account_is_zero() {
        let result = run_loan_set_invoke_preclaim(
            true,
            || panic!("zero account should skip seq-proxy"),
            || panic!("zero account should skip prior-tx"),
            || panic!("zero account should skip permission"),
            || panic!("zero account should skip loan-set sign"),
            || panic!("zero account should skip base-fee"),
            |_| panic!("zero account should skip fee"),
            || Ter::TEC_NO_ENTRY,
        );

        assert_eq!(result, Ter::TEC_NO_ENTRY);
        assert_eq!(trans_token(result), "tecNO_ENTRY");
    }

    #[test]
    fn loan_set_invoke_preclaim_preserves_current() {
        let trace = RefCell::new(Vec::new());

        let result = run_loan_set_invoke_preclaim(
            false,
            || {
                trace.borrow_mut().push("seq");
                Ter::TES_SUCCESS
            },
            || {
                trace.borrow_mut().push("prior");
                Ter::TES_SUCCESS
            },
            || {
                trace.borrow_mut().push("permission");
                Ter::TES_SUCCESS
            },
            || {
                trace.borrow_mut().push("sign");
                Ter::TES_SUCCESS
            },
            || {
                trace.borrow_mut().push("base-fee");
                20_u64
            },
            |fee| {
                trace.borrow_mut().push("fee");
                assert_eq!(fee, 20);
                Ter::TES_SUCCESS
            },
            || {
                trace.borrow_mut().push("preclaim");
                Ter::TES_SUCCESS
            },
        );

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(
            trace.into_inner(),
            vec![
                "seq",
                "prior",
                "permission",
                "sign",
                "base-fee",
                "fee",
                "preclaim"
            ]
        );
    }

    #[test]
    fn loan_set_invoke_preclaim_returns_first_shared_failure_unchanged() {
        let sign_called = Cell::new(false);
        let preclaim_called = Cell::new(false);

        let result = run_loan_set_invoke_preclaim(
            false,
            || Ter::TES_SUCCESS,
            || Ter::TEF_WRONG_PRIOR,
            || Ter::TES_SUCCESS,
            || {
                sign_called.set(true);
                Ter::TES_SUCCESS
            },
            || 20_u64,
            |_| Ter::TES_SUCCESS,
            || {
                preclaim_called.set(true);
                Ter::TES_SUCCESS
            },
        );

        assert_eq!(result, Ter::TEF_WRONG_PRIOR);
        assert_eq!(trans_token(result), "tefWRONG_PRIOR");
        assert!(!sign_called.get());
        assert!(!preclaim_called.get());
    }

    #[test]
    fn loan_set_invoke_preclaim_returns_fee_failure_unchanged() {
        let result = run_loan_set_invoke_preclaim(
            false,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || 20_u64,
            |_| Ter::TEC_INSUFF_FEE,
            || panic!("fee failure should skip loan-set preclaim"),
        );

        assert_eq!(result, Ter::TEC_INSUFF_FEE);
        assert_eq!(trans_token(result), "tecINSUFF_FEE");
    }

    #[test]
    fn loan_set_invoke_preclaim_returns_loan_set_preclaim_result() {
        let result = run_loan_set_invoke_preclaim(
            false,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || 20_u64,
            |_| Ter::TES_SUCCESS,
            || Ter::TEC_NO_PERMISSION,
        );

        assert_eq!(result, Ter::TEC_NO_PERMISSION);
        assert_eq!(trans_token(result), "tecNO_PERMISSION");
    }
}
