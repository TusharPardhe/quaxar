//! Current Rust helper mirroring the shared the transaction dispatch layer
//! `invoke_preclaim(...)` composition shell above transaction-specific
//! `preclaim(...)`.
//!
//! This module preserves the deterministic outer behavior around:
//!
//! - skipping all transactor prechecks when the source account is zero,
//! - running `checkSeqProxy(...)`, `checkPriorTxAndLastLedger(...)`,
//!   `checkPermission(...)`, and `checkSign(...)` in order with first-failure
//!   return semantics,
//! - calculating the base fee only after all pre-sign checks succeed,
//! - running `checkFee(...)` only after base-fee calculation succeeds, and
//! - falling through to the transaction-specific `preclaim(...)` tail only
//!   when all earlier checks succeed.

use protocol::{NotTec, Ter, is_tes_success};

#[allow(clippy::too_many_arguments)]
pub fn run_transactor_invoke_preclaim<Fee>(
    account_is_zero: bool,
    check_seq_proxy: impl FnOnce() -> NotTec,
    check_prior_tx_and_last_ledger: impl FnOnce() -> NotTec,
    check_permission: impl FnOnce() -> NotTec,
    check_sign: impl FnOnce() -> NotTec,
    calculate_base_fee: impl FnOnce() -> Fee,
    check_fee: impl FnOnce(Fee) -> Ter,
    preclaim: impl FnOnce() -> Ter,
) -> Ter {
    if !account_is_zero {
        let ret = check_seq_proxy();
        if !is_tes_success(ret) {
            return ret;
        }

        let ret = check_prior_tx_and_last_ledger();
        if !is_tes_success(ret) {
            return ret;
        }

        let ret = check_permission();
        if !is_tes_success(ret) {
            return ret;
        }

        let ret = check_sign();
        if !is_tes_success(ret) {
            return ret;
        }

        let base_fee = calculate_base_fee();
        let ret = check_fee(base_fee);
        if !is_tes_success(ret) {
            return ret;
        }
    }

    preclaim()
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use protocol::{Ter, trans_token};

    use super::run_transactor_invoke_preclaim;

    #[test]
    fn transactor_invoke_preclaim_skips_presig_and_fee_when_account_is_zero() {
        let result = run_transactor_invoke_preclaim(
            true,
            || panic!("zero account should skip seq-proxy"),
            || panic!("zero account should skip prior-tx"),
            || panic!("zero account should skip permission"),
            || panic!("zero account should skip sign"),
            || panic!("zero account should skip base-fee"),
            |_| panic!("zero account should skip fee"),
            || Ter::TEC_NO_ENTRY,
        );

        assert_eq!(result, Ter::TEC_NO_ENTRY);
        assert_eq!(trans_token(result), "tecNO_ENTRY");
    }

    #[test]
    fn transactor_invoke_preclaim_preserves_current_cpp_presig_order() {
        let trace = RefCell::new(Vec::new());

        let result = run_transactor_invoke_preclaim(
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
    fn transactor_invoke_preclaim_returns_first_presig_failure_unchanged() {
        let permission_called = Cell::new(false);
        let fee_called = Cell::new(false);

        let result = run_transactor_invoke_preclaim(
            false,
            || Ter::TES_SUCCESS,
            || Ter::TEF_WRONG_PRIOR,
            || {
                permission_called.set(true);
                Ter::TES_SUCCESS
            },
            || Ter::TES_SUCCESS,
            || 20_u64,
            |_| {
                fee_called.set(true);
                Ter::TES_SUCCESS
            },
            || Ter::TES_SUCCESS,
        );

        assert_eq!(result, Ter::TEF_WRONG_PRIOR);
        assert_eq!(trans_token(result), "tefWRONG_PRIOR");
        assert!(!permission_called.get());
        assert!(!fee_called.get());
    }

    #[test]
    fn transactor_invoke_preclaim_returns_fee_failure_unchanged() {
        let preclaim_called = Cell::new(false);

        let result = run_transactor_invoke_preclaim(
            false,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || 20_u64,
            |fee| {
                assert_eq!(fee, 20);
                Ter::TEC_INSUFF_FEE
            },
            || {
                preclaim_called.set(true);
                Ter::TES_SUCCESS
            },
        );

        assert_eq!(result, Ter::TEC_INSUFF_FEE);
        assert_eq!(trans_token(result), "tecINSUFF_FEE");
        assert!(!preclaim_called.get());
    }

    #[test]
    fn transactor_invoke_preclaim_returns_preclaim_result_after_all_guards() {
        let result = run_transactor_invoke_preclaim(
            false,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || 20_u64,
            |_| Ter::TES_SUCCESS,
            || Ter::TEC_CLAIM,
        );

        assert_eq!(result, Ter::TEC_CLAIM);
        assert_eq!(trans_token(result), "tecCLAIM");
    }
}
