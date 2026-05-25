//! Deterministic
//! the reference implementation shell.
//!
//! This ports the exact current malformed and amount checks for:
//!
//! - zero `sfVaultID`,
//! - and non-positive `sfAmount`.

use protocol::{NotTec, Ter};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VaultDepositPreflightFacts {
    pub vault_id_is_zero: bool,
    pub amount_is_positive: bool,
}

pub const fn run_vault_deposit_preflight(facts: VaultDepositPreflightFacts) -> NotTec {
    if facts.vault_id_is_zero {
        return Ter::TEM_MALFORMED;
    }

    if !facts.amount_is_positive {
        return Ter::TEM_BAD_AMOUNT;
    }

    Ter::TES_SUCCESS
}

#[cfg(test)]
mod tests {
    use protocol::{Ter, trans_token};

    use super::{VaultDepositPreflightFacts, run_vault_deposit_preflight};

    #[test]
    fn vault_deposit_preflight_rejects_zero_vault_id() {
        let result = run_vault_deposit_preflight(VaultDepositPreflightFacts {
            vault_id_is_zero: true,
            amount_is_positive: true,
        });

        assert_eq!(result, Ter::TEM_MALFORMED);
        assert_eq!(trans_token(result), "temMALFORMED");
    }

    #[test]
    fn vault_deposit_preflight_rejects_non_positive_amount() {
        let result = run_vault_deposit_preflight(VaultDepositPreflightFacts::default());

        assert_eq!(result, Ter::TEM_BAD_AMOUNT);
        assert_eq!(trans_token(result), "temBAD_AMOUNT");
    }

    #[test]
    fn vault_deposit_preflight_accepts_positive_amount() {
        let result = run_vault_deposit_preflight(VaultDepositPreflightFacts {
            amount_is_positive: true,
            ..VaultDepositPreflightFacts::default()
        });

        assert_eq!(result, Ter::TES_SUCCESS);
    }
}
