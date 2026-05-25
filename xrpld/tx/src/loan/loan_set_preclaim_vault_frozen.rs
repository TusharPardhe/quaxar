//! Vault-pseudo freeze guard for the reference implementation.
//!
//! This module ports the deterministic behavior around:
//!
//! - invoking `checkFrozen(ctx.view, vaultPseudo, asset)` exactly once,
//! - returning `tesSUCCESS` as success, and
//! - returning the helper's non-success `TER` unchanged with the current
//!   warning text.

use protocol::{Ter, is_tes_success};

pub const LOAN_SET_VAULT_PSEUDO_ACCOUNT_IS_FROZEN_WARNING: &str = "Vault pseudo-account is frozen.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoanSetPreclaimVaultFrozenFailure {
    ter: Ter,
}

impl LoanSetPreclaimVaultFrozenFailure {
    pub const fn ter(self) -> Ter {
        self.ter
    }

    pub const fn warning_message(self) -> &'static str {
        LOAN_SET_VAULT_PSEUDO_ACCOUNT_IS_FROZEN_WARNING
    }
}

pub fn check_loan_set_preclaim_vault_frozen<AccountId, Asset, CheckFrozen>(
    vault_pseudo: &AccountId,
    asset: &Asset,
    check_frozen: CheckFrozen,
) -> Result<(), LoanSetPreclaimVaultFrozenFailure>
where
    CheckFrozen: FnOnce(&AccountId, &Asset) -> Ter,
{
    let ter = check_frozen(vault_pseudo, asset);

    if is_tes_success(ter) {
        Ok(())
    } else {
        Err(LoanSetPreclaimVaultFrozenFailure { ter })
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use protocol::{Ter, trans_token};

    use super::{
        LOAN_SET_VAULT_PSEUDO_ACCOUNT_IS_FROZEN_WARNING, LoanSetPreclaimVaultFrozenFailure,
        check_loan_set_preclaim_vault_frozen,
    };

    #[test]
    fn loan_set_preclaim_vault_frozen_returns_success_when_not_frozen() {
        let result =
            check_loan_set_preclaim_vault_frozen(&"vault-pseudo", &"XRP", |account, asset| {
                assert_eq!(*account, "vault-pseudo");
                assert_eq!(*asset, "XRP");
                Ter::TES_SUCCESS
            });

        assert_eq!(result, Ok(()));
    }

    #[test]
    fn loan_set_preclaim_vault_frozen_returns_frozen_ter_unchanged() {
        let result =
            check_loan_set_preclaim_vault_frozen(&"vault-pseudo", &"USD", |_, _| Ter::TEC_FROZEN);

        assert_eq!(
            result,
            Err(LoanSetPreclaimVaultFrozenFailure {
                ter: Ter::TEC_FROZEN,
            })
        );
        let err = result.unwrap_err();
        assert_eq!(err.ter(), Ter::TEC_FROZEN);
        assert_eq!(trans_token(err.ter()), "tecFROZEN");
        assert_eq!(
            err.warning_message(),
            LOAN_SET_VAULT_PSEUDO_ACCOUNT_IS_FROZEN_WARNING
        );
    }

    #[test]
    fn loan_set_preclaim_vault_frozen_returns_locked_ter_unchanged() {
        let result =
            check_loan_set_preclaim_vault_frozen(&"vault-pseudo", &"MPT", |_, _| Ter::TEC_LOCKED);

        assert_eq!(
            result,
            Err(LoanSetPreclaimVaultFrozenFailure {
                ter: Ter::TEC_LOCKED,
            })
        );
        let err = result.unwrap_err();
        assert_eq!(err.ter(), Ter::TEC_LOCKED);
        assert_eq!(trans_token(err.ter()), "tecLOCKED");
    }

    #[test]
    fn loan_set_preclaim_vault_frozen_checks_account_and_asset_exactly_once() {
        let seen = RefCell::new(Vec::new());

        let result =
            check_loan_set_preclaim_vault_frozen(&"vault-pseudo", &"USD", |account, asset| {
                seen.borrow_mut().push((*account, *asset));
                Ter::TEC_FROZEN
            });

        assert_eq!(
            result,
            Err(LoanSetPreclaimVaultFrozenFailure {
                ter: Ter::TEC_FROZEN,
            })
        );
        assert_eq!(*seen.borrow(), vec![("vault-pseudo", "USD")]);
    }
}
