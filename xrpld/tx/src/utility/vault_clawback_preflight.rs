//! Deterministic
//! the reference implementation shell.
//!
//! This ports the exact current ordered malformed and amount checks for:
//!
//! - zero `sfVaultID`,
//! - optional negative `sfAmount`,
//! - and optional XRP-asset rejection.

use protocol::{NotTec, Ter};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VaultClawbackPreflightFacts {
    pub vault_id_is_zero: bool,
    pub amount_present: bool,
    pub amount_is_negative: bool,
    pub amount_asset_is_xrp: bool,
}

pub const fn run_vault_clawback_preflight(facts: VaultClawbackPreflightFacts) -> NotTec {
    if facts.vault_id_is_zero {
        return Ter::TEM_MALFORMED;
    }

    if facts.amount_present {
        if facts.amount_is_negative {
            return Ter::TEM_BAD_AMOUNT;
        }
        if facts.amount_asset_is_xrp {
            return Ter::TEM_MALFORMED;
        }
    }

    Ter::TES_SUCCESS
}

#[cfg(test)]
mod tests {
    use protocol::{Ter, trans_token};

    use super::{VaultClawbackPreflightFacts, run_vault_clawback_preflight};

    #[test]
    fn vault_clawback_preflight_rejects_zero_vault_id() {
        let result = run_vault_clawback_preflight(VaultClawbackPreflightFacts {
            vault_id_is_zero: true,
            ..VaultClawbackPreflightFacts::default()
        });

        assert_eq!(result, Ter::TEM_MALFORMED);
        assert_eq!(trans_token(result), "temMALFORMED");
    }

    #[test]
    fn vault_clawback_preflight_rejects_negative_amount() {
        let result = run_vault_clawback_preflight(VaultClawbackPreflightFacts {
            amount_present: true,
            amount_is_negative: true,
            ..VaultClawbackPreflightFacts::default()
        });

        assert_eq!(result, Ter::TEM_BAD_AMOUNT);
        assert_eq!(trans_token(result), "temBAD_AMOUNT");
    }

    #[test]
    fn vault_clawback_preflight_rejects_xrp_amount_asset() {
        let result = run_vault_clawback_preflight(VaultClawbackPreflightFacts {
            amount_present: true,
            amount_asset_is_xrp: true,
            ..VaultClawbackPreflightFacts::default()
        });

        assert_eq!(result, Ter::TEM_MALFORMED);
    }

    #[test]
    fn vault_clawback_preflight_accepts_missing_or_zero_amount() {
        let missing = run_vault_clawback_preflight(VaultClawbackPreflightFacts::default());
        let zero = run_vault_clawback_preflight(VaultClawbackPreflightFacts {
            amount_present: true,
            ..VaultClawbackPreflightFacts::default()
        });

        assert_eq!(missing, Ter::TES_SUCCESS);
        assert_eq!(zero, Ter::TES_SUCCESS);
    }
}
