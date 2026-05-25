//! Vault-available guard for the LoanSet transactor.
//!
//! This module ports the deterministic behavior around:
//!
//! - comparing `vaultAvailableProxy` against `principalRequested`,
//! - using a strict `<` check so equality still succeeds, and
//! - mapping insufficient available assets to `tecINSUFFICIENT_FUNDS` with the
//!   current warning string.

use protocol::Ter;

pub const LOAN_SET_DO_APPLY_INSUFFICIENT_VAULT_ASSETS_WARNING: &str =
    "Insufficient assets available in the Vault to fund the loan.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoanSetDoApplyVaultAvailableFailure {
    InsufficientVaultAssetsAvailable,
}

impl LoanSetDoApplyVaultAvailableFailure {
    pub const fn ter(self) -> Ter {
        match self {
            Self::InsufficientVaultAssetsAvailable => Ter::TEC_INSUFFICIENT_FUNDS,
        }
    }

    pub const fn warning_message(self) -> &'static str {
        match self {
            Self::InsufficientVaultAssetsAvailable => {
                LOAN_SET_DO_APPLY_INSUFFICIENT_VAULT_ASSETS_WARNING
            }
        }
    }
}

pub fn check_loan_set_do_apply_vault_available<Amount>(
    vault_available: &Amount,
    principal_requested: &Amount,
) -> Result<(), LoanSetDoApplyVaultAvailableFailure>
where
    Amount: PartialOrd,
{
    if vault_available < principal_requested {
        return Err(LoanSetDoApplyVaultAvailableFailure::InsufficientVaultAssetsAvailable);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use protocol::trans_token;

    use super::{
        LOAN_SET_DO_APPLY_INSUFFICIENT_VAULT_ASSETS_WARNING, LoanSetDoApplyVaultAvailableFailure,
        check_loan_set_do_apply_vault_available,
    };

    #[test]
    fn loan_set_do_apply_vault_available_allows_exact_available_balance() {
        let result = check_loan_set_do_apply_vault_available(&100_u32, &100_u32);

        assert_eq!(result, Ok(()));
    }

    #[test]
    fn loan_set_do_apply_vault_available_allows_more_than_requested() {
        let result = check_loan_set_do_apply_vault_available(&101_u32, &100_u32);

        assert_eq!(result, Ok(()));
    }

    #[test]
    fn loan_set_do_apply_vault_available_returns_insufficient_funds() {
        let result = check_loan_set_do_apply_vault_available(&99_u32, &100_u32);

        assert_eq!(
            result,
            Err(LoanSetDoApplyVaultAvailableFailure::InsufficientVaultAssetsAvailable)
        );
        let err = result.expect_err("insufficient vault assets should fail");
        assert_eq!(err.ter(), protocol::Ter::TEC_INSUFFICIENT_FUNDS);
        assert_eq!(trans_token(err.ter()), "tecINSUFFICIENT_FUNDS");
        assert_eq!(
            err.warning_message(),
            LOAN_SET_DO_APPLY_INSUFFICIENT_VAULT_ASSETS_WARNING
        );
    }
}
