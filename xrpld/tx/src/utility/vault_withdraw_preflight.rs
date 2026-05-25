//! Deterministic
//! the reference implementation shell.
//!
//! This ports the exact current ordered malformed and amount checks for:
//!
//! - zero `sfVaultID`,
//! - non-positive `sfAmount`,
//! - and optional zero `sfDestination`.

use protocol::{NotTec, Ter};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VaultWithdrawPreflightFacts {
    pub vault_id_is_zero: bool,
    pub amount_is_positive: bool,
    pub destination_present: bool,
    pub destination_is_zero: bool,
}

pub const fn run_vault_withdraw_preflight(facts: VaultWithdrawPreflightFacts) -> NotTec {
    if facts.vault_id_is_zero {
        return Ter::TEM_MALFORMED;
    }

    if !facts.amount_is_positive {
        return Ter::TEM_BAD_AMOUNT;
    }

    if facts.destination_present && facts.destination_is_zero {
        return Ter::TEM_MALFORMED;
    }

    Ter::TES_SUCCESS
}

#[cfg(test)]
mod tests {
    use protocol::{Ter, trans_token};

    use super::{VaultWithdrawPreflightFacts, run_vault_withdraw_preflight};

    #[test]
    fn vault_withdraw_preflight_rejects_zero_vault_id() {
        let result = run_vault_withdraw_preflight(VaultWithdrawPreflightFacts {
            vault_id_is_zero: true,
            amount_is_positive: true,
            ..VaultWithdrawPreflightFacts::default()
        });

        assert_eq!(result, Ter::TEM_MALFORMED);
        assert_eq!(trans_token(result), "temMALFORMED");
    }

    #[test]
    fn vault_withdraw_preflight_rejects_non_positive_amount() {
        let result = run_vault_withdraw_preflight(VaultWithdrawPreflightFacts::default());

        assert_eq!(result, Ter::TEM_BAD_AMOUNT);
        assert_eq!(trans_token(result), "temBAD_AMOUNT");
    }

    #[test]
    fn vault_withdraw_preflight_rejects_zero_destination() {
        let result = run_vault_withdraw_preflight(VaultWithdrawPreflightFacts {
            amount_is_positive: true,
            destination_present: true,
            destination_is_zero: true,
            ..VaultWithdrawPreflightFacts::default()
        });

        assert_eq!(result, Ter::TEM_MALFORMED);
    }

    #[test]
    fn vault_withdraw_preflight_accepts_missing_or_nonzero_destination() {
        let missing = run_vault_withdraw_preflight(VaultWithdrawPreflightFacts {
            amount_is_positive: true,
            ..VaultWithdrawPreflightFacts::default()
        });
        let present = run_vault_withdraw_preflight(VaultWithdrawPreflightFacts {
            amount_is_positive: true,
            destination_present: true,
            ..VaultWithdrawPreflightFacts::default()
        });

        assert_eq!(missing, Ter::TES_SUCCESS);
        assert_eq!(present, Ter::TES_SUCCESS);
    }
}
