//! Integration tests that pin the narrowed Rust `LoanSet.cpp::doApply()`
//! vault-available guard to the current C++ behavior.

use protocol::{Ter, trans_token};
use tx::{
    LOAN_SET_DO_APPLY_INSUFFICIENT_VAULT_ASSETS_WARNING, LoanSetDoApplyVaultAvailableFailure,
    check_loan_set_do_apply_vault_available,
};

#[test]
fn tx_loan_set_do_apply_vault_available_allows_equal_available_balance() {
    let result = check_loan_set_do_apply_vault_available(&100_u32, &100_u32);

    assert_eq!(result, Ok(()));
}

#[test]
fn tx_loan_set_do_apply_vault_available_allows_more_than_requested() {
    let result = check_loan_set_do_apply_vault_available(&101_u32, &100_u32);

    assert_eq!(result, Ok(()));
}

#[test]
fn tx_loan_set_do_apply_vault_available_returns_insufficient_funds() {
    let result = check_loan_set_do_apply_vault_available(&99_u32, &100_u32);

    assert_eq!(
        result,
        Err(LoanSetDoApplyVaultAvailableFailure::InsufficientVaultAssetsAvailable)
    );
    let err = result.expect_err("insufficient vault assets should fail");
    assert_eq!(err.ter(), Ter::TEC_INSUFFICIENT_FUNDS);
    assert_eq!(trans_token(err.ter()), "tecINSUFFICIENT_FUNDS");
    assert_eq!(
        err.warning_message(),
        LOAN_SET_DO_APPLY_INSUFFICIENT_VAULT_ASSETS_WARNING
    );
}
