//! Integration tests that pin the shared `applySteps.cpp` `invoke_preclaim(...)`
//! composition shell to the current C++ behavior.

use protocol::{Ter, trans_token};
use tx::run_transactor_invoke_preclaim;

#[test]
fn tx_transactor_invoke_preclaim_skips_presig_and_fee_when_account_is_zero() {
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
fn tx_transactor_invoke_preclaim_returns_first_presig_failure_unchanged() {
    let result = run_transactor_invoke_preclaim(
        false,
        || Ter::TES_SUCCESS,
        || Ter::TES_SUCCESS,
        || Ter::TER_NO_DELEGATE_PERMISSION,
        || panic!("permission failure should skip sign"),
        || panic!("permission failure should skip base-fee"),
        |_| panic!("permission failure should skip fee"),
        || panic!("permission failure should skip preclaim"),
    );

    assert_eq!(result, Ter::TER_NO_DELEGATE_PERMISSION);
    assert_eq!(trans_token(result), "terNO_DELEGATE_PERMISSION");
}

#[test]
fn tx_transactor_invoke_preclaim_returns_fee_failure_unchanged() {
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
        || panic!("fee failure should skip preclaim"),
    );

    assert_eq!(result, Ter::TEC_INSUFF_FEE);
    assert_eq!(trans_token(result), "tecINSUFF_FEE");
}

#[test]
fn tx_transactor_invoke_preclaim_returns_preclaim_result_after_all_guards() {
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
