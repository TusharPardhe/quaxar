//! Higher `LoanSet` preflight composition order mirrored from
//! `Transactor::invokePreflight<T>`.
//!
//! This module ports the exact current ordering around:
//!
//! - the transaction-type `featureLendingProtocol` gate,
//! - `LoanSet::checkExtraFeatures(...)`,
//! - `preflight1(...)` with the current `LoanSet` flags mask,
//! - `LoanSet::preflight(...)`,
//! - `preflight2(...)`,
//! - and the base `preflightSigValidated(...)` success tail.

use protocol::{NotTec, Ter, is_tes_success};

use crate::{get_loan_set_flags_mask, run_loan_set_check_extra_features};

pub fn run_loan_set_invoke_preflight(
    lending_protocol_enabled: bool,
    single_asset_vault_enabled: bool,
    check_vault_create_extra_features: impl FnOnce() -> bool,
    run_preflight1: impl FnOnce(u32) -> NotTec,
    run_loan_set_preflight: impl FnOnce() -> NotTec,
    run_preflight2: impl FnOnce() -> NotTec,
) -> NotTec {
    if !lending_protocol_enabled {
        return Ter::TEM_DISABLED;
    }

    if !run_loan_set_check_extra_features(
        single_asset_vault_enabled,
        check_vault_create_extra_features,
    ) {
        return Ter::TEM_DISABLED;
    }

    let ret = run_preflight1(get_loan_set_flags_mask());
    if !is_tes_success(ret) {
        return ret;
    }

    let ret = run_loan_set_preflight();
    if !is_tes_success(ret) {
        return ret;
    }

    let ret = run_preflight2();
    if !is_tes_success(ret) {
        return ret;
    }

    Ter::TES_SUCCESS
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use protocol::Ter;

    use super::run_loan_set_invoke_preflight;

    #[test]
    fn loan_set_invoke_preflight_short_circuits_tx_feature_gate() {
        let result = run_loan_set_invoke_preflight(
            false,
            true,
            || panic!("tx feature gate should skip extra-features"),
            |_| panic!("tx feature gate should skip preflight1"),
            || panic!("tx feature gate should skip loan-set preflight"),
            || panic!("tx feature gate should skip preflight2"),
        );

        assert_eq!(result, Ter::TEM_DISABLED);
    }

    #[test]
    fn loan_set_invoke_preflight_short_circuits_dependency_gate() {
        let result = run_loan_set_invoke_preflight(
            true,
            false,
            || panic!("single-asset-vault gate should skip vault extra-features"),
            |_| panic!("dependency gate should skip preflight1"),
            || panic!("dependency gate should skip loan-set preflight"),
            || panic!("dependency gate should skip preflight2"),
        );

        assert_eq!(result, Ter::TEM_DISABLED);
    }

    #[test]
    fn loan_set_invoke_preflight_preserves_current_cpp_step_order() {
        let trace = RefCell::new(Vec::new());

        let result = run_loan_set_invoke_preflight(
            true,
            true,
            || {
                trace.borrow_mut().push("extra-features");
                true
            },
            |mask| {
                trace.borrow_mut().push("preflight1");
                assert_eq!(mask, 0x3ffe_ffff);
                Ter::TES_SUCCESS
            },
            || {
                trace.borrow_mut().push("loan-set-preflight");
                Ter::TES_SUCCESS
            },
            || {
                trace.borrow_mut().push("preflight2");
                Ter::TES_SUCCESS
            },
        );

        assert_eq!(result, Ter::TES_SUCCESS);
        assert_eq!(
            trace.into_inner(),
            vec![
                "extra-features",
                "preflight1",
                "loan-set-preflight",
                "preflight2"
            ]
        );
    }

    #[test]
    fn loan_set_invoke_preflight_returns_first_failure_unchanged() {
        let preflight1_failure = run_loan_set_invoke_preflight(
            true,
            true,
            || true,
            |_| Ter::TEM_INVALID_FLAG,
            || panic!("preflight1 failure should skip loan-set preflight"),
            || panic!("preflight1 failure should skip preflight2"),
        );
        let loan_set_preflight_failure = run_loan_set_invoke_preflight(
            true,
            true,
            || true,
            |_| Ter::TES_SUCCESS,
            || Ter::TEM_INVALID,
            || panic!("loan-set preflight failure should skip preflight2"),
        );
        let preflight2_failure = run_loan_set_invoke_preflight(
            true,
            true,
            || true,
            |_| Ter::TES_SUCCESS,
            || Ter::TES_SUCCESS,
            || Ter::TEM_BAD_SIGNATURE,
        );

        assert_eq!(preflight1_failure, Ter::TEM_INVALID_FLAG);
        assert_eq!(loan_set_preflight_failure, Ter::TEM_INVALID);
        assert_eq!(preflight2_failure, Ter::TEM_BAD_SIGNATURE);
    }
}
