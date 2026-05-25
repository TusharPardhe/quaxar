//! Deterministic
//! the reference implementation shell.
//!
//! This ports the exact ordered branch behavior over pre-decoded facts for:
//!
//! - zero `sfVaultID`,
//! - optional `sfData` payload validation via the current `validDataLength`
//!   rule,
//! - optional negative `sfAssetsMaximum`,
//! - and the final "nothing is being updated" guard.

use protocol::{NotTec, Ter};

use crate::run_transactor_valid_data_length;

pub const VAULT_SET_MAX_DATA_PAYLOAD_LENGTH: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VaultSetPreflightFacts {
    pub vault_id_is_zero: bool,
    pub data_len: Option<usize>,
    pub assets_maximum_is_negative: bool,
    pub domain_id_present: bool,
    pub assets_maximum_present: bool,
    pub data_present: bool,
}

pub const fn run_vault_set_preflight(facts: VaultSetPreflightFacts) -> NotTec {
    if facts.vault_id_is_zero {
        return Ter::TEM_MALFORMED;
    }

    if !run_transactor_valid_data_length(facts.data_len, VAULT_SET_MAX_DATA_PAYLOAD_LENGTH) {
        return Ter::TEM_MALFORMED;
    }

    if facts.assets_maximum_is_negative {
        return Ter::TEM_MALFORMED;
    }

    if !facts.domain_id_present && !facts.assets_maximum_present && !facts.data_present {
        return Ter::TEM_MALFORMED;
    }

    Ter::TES_SUCCESS
}

#[cfg(test)]
mod tests {
    use protocol::{Ter, trans_token};

    use super::{
        VAULT_SET_MAX_DATA_PAYLOAD_LENGTH, VaultSetPreflightFacts, run_vault_set_preflight,
    };

    #[test]
    fn vault_set_preflight_rejects_zero_vault_id() {
        let result = run_vault_set_preflight(VaultSetPreflightFacts {
            vault_id_is_zero: true,
            domain_id_present: true,
            ..VaultSetPreflightFacts::default()
        });

        assert_eq!(result, Ter::TEM_MALFORMED);
        assert_eq!(trans_token(result), "temMALFORMED");
    }

    #[test]
    fn vault_set_preflight_rejects_empty_and_oversized_data() {
        let empty = run_vault_set_preflight(VaultSetPreflightFacts {
            data_len: Some(0),
            data_present: true,
            ..VaultSetPreflightFacts::default()
        });
        let oversized = run_vault_set_preflight(VaultSetPreflightFacts {
            data_len: Some(VAULT_SET_MAX_DATA_PAYLOAD_LENGTH + 1),
            data_present: true,
            ..VaultSetPreflightFacts::default()
        });

        assert_eq!(empty, Ter::TEM_MALFORMED);
        assert_eq!(oversized, Ter::TEM_MALFORMED);
    }

    #[test]
    fn vault_set_preflight_rejects_negative_assets_maximum() {
        let result = run_vault_set_preflight(VaultSetPreflightFacts {
            assets_maximum_is_negative: true,
            assets_maximum_present: true,
            ..VaultSetPreflightFacts::default()
        });

        assert_eq!(result, Ter::TEM_MALFORMED);
    }

    #[test]
    fn vault_set_preflight_rejects_noop_updates() {
        let result = run_vault_set_preflight(VaultSetPreflightFacts::default());

        assert_eq!(result, Ter::TEM_MALFORMED);
    }

    #[test]
    fn vault_set_preflight_accepts_any_single_valid_update() {
        let domain = run_vault_set_preflight(VaultSetPreflightFacts {
            domain_id_present: true,
            ..VaultSetPreflightFacts::default()
        });
        let assets_maximum = run_vault_set_preflight(VaultSetPreflightFacts {
            assets_maximum_present: true,
            ..VaultSetPreflightFacts::default()
        });
        let data = run_vault_set_preflight(VaultSetPreflightFacts {
            data_len: Some(VAULT_SET_MAX_DATA_PAYLOAD_LENGTH),
            data_present: true,
            ..VaultSetPreflightFacts::default()
        });

        assert_eq!(domain, Ter::TES_SUCCESS);
        assert_eq!(assets_maximum, Ter::TES_SUCCESS);
        assert_eq!(data, Ter::TES_SUCCESS);
    }
}
