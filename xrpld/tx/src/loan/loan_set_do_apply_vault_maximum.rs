//! Vault-maximum guard for the LoanSet transactor.
//!
//! This module ports the deterministic behavior around:
//!
//! - treating `vaultMaximum == 0` as unlimited,
//! - comparing `state.interestDue` against `vaultMaximum - vaultTotalProxy`,
//! - allowing equality to pass, and
//! - mapping an exceeded maximum to `tecLIMIT_EXCEEDED` with the current
//!   warning string.

use std::ops::Sub;

use protocol::Ter;

pub const LOAN_SET_DO_APPLY_VAULT_MAXIMUM_EXCEEDED_WARNING: &str =
    "Loan would exceed the maximum assets of the vault";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoanSetDoApplyVaultMaximumFailure {
    VaultMaximumExceeded,
}

impl LoanSetDoApplyVaultMaximumFailure {
    pub const fn ter(self) -> Ter {
        match self {
            Self::VaultMaximumExceeded => Ter::TEC_LIMIT_EXCEEDED,
        }
    }

    pub const fn warning_message(self) -> &'static str {
        match self {
            Self::VaultMaximumExceeded => LOAN_SET_DO_APPLY_VAULT_MAXIMUM_EXCEEDED_WARNING,
        }
    }
}

pub fn check_loan_set_do_apply_vault_maximum<Amount>(
    vault_maximum: &Amount,
    vault_total: &Amount,
    interest_due: &Amount,
    zero: &Amount,
) -> Result<(), LoanSetDoApplyVaultMaximumFailure>
where
    Amount: Clone + PartialEq + PartialOrd + Sub<Output = Amount>,
{
    if vault_maximum == zero {
        return Ok(());
    }

    let remaining_capacity = vault_maximum.clone() - vault_total.clone();
    if interest_due > &remaining_capacity {
        return Err(LoanSetDoApplyVaultMaximumFailure::VaultMaximumExceeded);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use protocol::trans_token;

    use super::{
        LOAN_SET_DO_APPLY_VAULT_MAXIMUM_EXCEEDED_WARNING, LoanSetDoApplyVaultMaximumFailure,
        check_loan_set_do_apply_vault_maximum,
    };

    #[test]
    fn loan_set_do_apply_vault_maximum_treats_zero_as_unlimited() {
        let result = check_loan_set_do_apply_vault_maximum(&0_u32, &100_u32, &1_000_u32, &0_u32);

        assert_eq!(result, Ok(()));
    }

    #[test]
    fn loan_set_do_apply_vault_maximum_allows_interest_equal_to_remaining_capacity() {
        let result = check_loan_set_do_apply_vault_maximum(&150_u32, &100_u32, &50_u32, &0_u32);

        assert_eq!(result, Ok(()));
    }

    #[test]
    fn loan_set_do_apply_vault_maximum_allows_interest_below_remaining_capacity() {
        let result = check_loan_set_do_apply_vault_maximum(&150_u32, &100_u32, &49_u32, &0_u32);

        assert_eq!(result, Ok(()));
    }

    #[test]
    fn loan_set_do_apply_vault_maximum_returns_limit_exceeded() {
        let result = check_loan_set_do_apply_vault_maximum(&150_u32, &100_u32, &51_u32, &0_u32);

        assert_eq!(
            result,
            Err(LoanSetDoApplyVaultMaximumFailure::VaultMaximumExceeded)
        );
        let err = result.expect_err("vault maximum overflow should fail");
        assert_eq!(err.ter(), protocol::Ter::TEC_LIMIT_EXCEEDED);
        assert_eq!(trans_token(err.ter()), "tecLIMIT_EXCEEDED");
        assert_eq!(
            err.warning_message(),
            LOAN_SET_DO_APPLY_VAULT_MAXIMUM_EXCEEDED_WARNING
        );
    }
}
