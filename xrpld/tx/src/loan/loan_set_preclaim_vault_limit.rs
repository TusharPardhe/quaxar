//! Vault maximum-assets branch for the reference implementation.
//!
//! This module ports the deterministic behavior around:
//!
//! - treating `AssetsMaximum == 0` as "no limit",
//! - rejecting the transaction when `AssetsTotal >= AssetsMaximum`, and
//! - mapping that failure to `tecLIMIT_EXCEEDED` with the current warning text.

use protocol::Ter;

pub const LOAN_SET_VAULT_AT_MAXIMUM_ASSETS_LIMIT_WARNING: &str =
    "Vault at maximum assets limit. Can't add another loan.";

pub trait LoanSetPreclaimVaultLimit {
    type Amount: Default + PartialEq + PartialOrd;

    fn assets_maximum(&self) -> &Self::Amount;
    fn assets_total(&self) -> &Self::Amount;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoanSetPreclaimVaultLimitFailure {
    VaultAtMaximumAssetsLimit,
}

impl LoanSetPreclaimVaultLimitFailure {
    pub const fn ter(self) -> Ter {
        match self {
            Self::VaultAtMaximumAssetsLimit => Ter::TEC_LIMIT_EXCEEDED,
        }
    }

    pub const fn warning_message(self) -> &'static str {
        match self {
            Self::VaultAtMaximumAssetsLimit => LOAN_SET_VAULT_AT_MAXIMUM_ASSETS_LIMIT_WARNING,
        }
    }
}

pub fn check_loan_set_preclaim_vault_limit<Vault>(
    vault: Vault,
) -> Result<Vault, LoanSetPreclaimVaultLimitFailure>
where
    Vault: LoanSetPreclaimVaultLimit,
{
    let assets_maximum = vault.assets_maximum();
    let assets_total = vault.assets_total();

    if assets_maximum != &Vault::Amount::default() && assets_total >= assets_maximum {
        return Err(LoanSetPreclaimVaultLimitFailure::VaultAtMaximumAssetsLimit);
    }

    Ok(vault)
}

#[cfg(test)]
mod tests {
    use protocol::trans_token;

    use super::{
        LOAN_SET_VAULT_AT_MAXIMUM_ASSETS_LIMIT_WARNING, LoanSetPreclaimVaultLimit,
        LoanSetPreclaimVaultLimitFailure, check_loan_set_preclaim_vault_limit,
    };

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct TestVault {
        assets_maximum: u32,
        assets_total: u32,
    }

    impl LoanSetPreclaimVaultLimit for TestVault {
        type Amount = u32;

        fn assets_maximum(&self) -> &Self::Amount {
            &self.assets_maximum
        }

        fn assets_total(&self) -> &Self::Amount {
            &self.assets_total
        }
    }

    #[test]
    fn loan_set_preclaim_vault_limit_treats_zero_maximum_as_unlimited() {
        let vault = TestVault {
            assets_maximum: 0,
            assets_total: 1_000_000,
        };

        assert_eq!(check_loan_set_preclaim_vault_limit(vault), Ok(vault));
    }

    #[test]
    fn loan_set_preclaim_vault_limit_returns_loaded_vault_when_below_maximum() {
        let vault = TestVault {
            assets_maximum: 1_000,
            assets_total: 999,
        };

        assert_eq!(check_loan_set_preclaim_vault_limit(vault), Ok(vault));
    }

    #[test]
    fn loan_set_preclaim_vault_limit_rejects_equal_maximum() {
        let result = check_loan_set_preclaim_vault_limit(TestVault {
            assets_maximum: 1_000,
            assets_total: 1_000,
        });

        assert_eq!(
            result,
            Err(LoanSetPreclaimVaultLimitFailure::VaultAtMaximumAssetsLimit)
        );
        let err = result.unwrap_err();
        assert_eq!(err.ter(), protocol::Ter::TEC_LIMIT_EXCEEDED);
        assert_eq!(trans_token(err.ter()), "tecLIMIT_EXCEEDED");
        assert_eq!(
            err.warning_message(),
            LOAN_SET_VAULT_AT_MAXIMUM_ASSETS_LIMIT_WARNING
        );
    }

    #[test]
    fn loan_set_preclaim_vault_limit_rejects_above_maximum() {
        let result = check_loan_set_preclaim_vault_limit(TestVault {
            assets_maximum: 1_000,
            assets_total: 1_001,
        });

        assert_eq!(
            result,
            Err(LoanSetPreclaimVaultLimitFailure::VaultAtMaximumAssetsLimit)
        );
    }
}
