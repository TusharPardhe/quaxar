//! Integration tests that pin the narrowed Rust `LoanSet.cpp::preclaim(...)`
//! vault maximum-assets branch to the current C++ behavior.

use protocol::{Ter, trans_token};
use tx::{
    LOAN_SET_VAULT_AT_MAXIMUM_ASSETS_LIMIT_WARNING, LoanSetPreclaimVaultLimit,
    LoanSetPreclaimVaultLimitFailure, check_loan_set_preclaim_vault_limit,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StubVault {
    assets_maximum: u32,
    assets_total: u32,
}

impl LoanSetPreclaimVaultLimit for StubVault {
    type Amount = u32;

    fn assets_maximum(&self) -> &Self::Amount {
        &self.assets_maximum
    }

    fn assets_total(&self) -> &Self::Amount {
        &self.assets_total
    }
}

#[test]
fn tx_loan_set_preclaim_vault_limit_treats_zero_maximum_as_unlimited() {
    let vault = StubVault {
        assets_maximum: 0,
        assets_total: 1_000_000,
    };

    assert_eq!(check_loan_set_preclaim_vault_limit(vault), Ok(vault));
}

#[test]
fn tx_loan_set_preclaim_vault_limit_returns_loaded_vault_below_maximum() {
    let vault = StubVault {
        assets_maximum: 1_000,
        assets_total: 999,
    };

    assert_eq!(check_loan_set_preclaim_vault_limit(vault), Ok(vault));
}

#[test]
fn tx_loan_set_preclaim_vault_limit_returns_limit_exceeded_at_maximum() {
    let result = check_loan_set_preclaim_vault_limit(StubVault {
        assets_maximum: 1_000,
        assets_total: 1_000,
    });

    assert_eq!(
        result,
        Err(LoanSetPreclaimVaultLimitFailure::VaultAtMaximumAssetsLimit)
    );
    let err = result.unwrap_err();
    assert_eq!(err.ter(), Ter::TEC_LIMIT_EXCEEDED);
    assert_eq!(trans_token(err.ter()), "tecLIMIT_EXCEEDED");
    assert_eq!(
        err.warning_message(),
        LOAN_SET_VAULT_AT_MAXIMUM_ASSETS_LIMIT_WARNING
    );
}

#[test]
fn tx_loan_set_preclaim_vault_limit_returns_limit_exceeded_above_maximum() {
    let result = check_loan_set_preclaim_vault_limit(StubVault {
        assets_maximum: 1_000,
        assets_total: 1_001,
    });

    assert_eq!(
        result,
        Err(LoanSetPreclaimVaultLimitFailure::VaultAtMaximumAssetsLimit)
    );
}
