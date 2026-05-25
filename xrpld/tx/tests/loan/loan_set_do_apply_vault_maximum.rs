//! Integration tests that pin the narrowed Rust `LoanSet.cpp::doApply()`
//! vault-maximum guard to the current C++ behavior.

use protocol::{Ter, trans_token};
use tx::{
    LOAN_SET_DO_APPLY_VAULT_MAXIMUM_EXCEEDED_WARNING, LoanSetDoApplyVaultMaximumFailure,
    check_loan_set_do_apply_vault_maximum,
};

#[test]
fn tx_loan_set_do_apply_vault_maximum_treats_zero_as_unlimited() {
    let result = check_loan_set_do_apply_vault_maximum(&0_u32, &100_u32, &1_000_u32, &0_u32);

    assert_eq!(result, Ok(()));
}

#[test]
fn tx_loan_set_do_apply_vault_maximum_allows_interest_equal_to_remaining_capacity() {
    let result = check_loan_set_do_apply_vault_maximum(&150_u32, &100_u32, &50_u32, &0_u32);

    assert_eq!(result, Ok(()));
}

#[test]
fn tx_loan_set_do_apply_vault_maximum_returns_limit_exceeded() {
    let result = check_loan_set_do_apply_vault_maximum(&150_u32, &100_u32, &51_u32, &0_u32);

    assert_eq!(
        result,
        Err(LoanSetDoApplyVaultMaximumFailure::VaultMaximumExceeded)
    );
    let err = result.expect_err("vault maximum overflow should fail");
    assert_eq!(err.ter(), Ter::TEC_LIMIT_EXCEEDED);
    assert_eq!(trans_token(err.ter()), "tecLIMIT_EXCEEDED");
    assert_eq!(
        err.warning_message(),
        LOAN_SET_DO_APPLY_VAULT_MAXIMUM_EXCEEDED_WARNING
    );
}
