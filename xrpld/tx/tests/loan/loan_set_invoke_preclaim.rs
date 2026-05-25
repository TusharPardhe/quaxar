//! Integration tests that pin the higher `LoanSet` invoke-preclaim shell to
//! the current C++ ordering.

use protocol::{Ter, trans_token};
use tx::run_loan_set_invoke_preclaim;

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
fn loan_set_invoke_preclaim_returns_first_shared_failure_unchanged() {
    let result = run_loan_set_invoke_preclaim(
        false,
        || Ter::TES_SUCCESS,
        || Ter::TEF_WRONG_PRIOR,
        || panic!("prior failure should skip permission"),
        || panic!("prior failure should skip sign"),
        || panic!("prior failure should skip base-fee"),
        |_| panic!("prior failure should skip fee"),
        || panic!("prior failure should skip loan-set preclaim"),
    );

    assert_eq!(result, Ter::TEF_WRONG_PRIOR);
    assert_eq!(trans_token(result), "tefWRONG_PRIOR");
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
        |fee| {
            assert_eq!(fee, 20);
            Ter::TEC_INSUFF_FEE
        },
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
