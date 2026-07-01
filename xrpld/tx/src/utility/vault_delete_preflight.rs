//! Deterministic
//! the reference implementation shell.
//!
//! This ports the exact current malformed guard for a zero or empty
//! `sfVaultID`, and the Lending 1.1 `sfMemoData` length check.

use protocol::{NotTec, Ter};

pub const VAULT_DELETE_MAX_DATA_PAYLOAD_LENGTH: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VaultDeletePreflightFacts {
    pub vault_id_is_zero: bool,
    pub has_memo_data: bool,
    pub lending_protocol_v1_1_enabled: bool,
    pub memo_data_length_valid: bool,
}

pub const fn run_vault_delete_preflight(facts: VaultDeletePreflightFacts) -> NotTec {
    if facts.vault_id_is_zero {
        return Ter::TEM_MALFORMED;
    }

    if facts.has_memo_data && !facts.lending_protocol_v1_1_enabled {
        return Ter::TEM_DISABLED;
    }

    if facts.has_memo_data && !facts.memo_data_length_valid {
        return Ter::TEM_MALFORMED;
    }

    Ter::TES_SUCCESS
}

#[cfg(test)]
mod tests {
    use protocol::Ter;

    use super::{VaultDeletePreflightFacts, run_vault_delete_preflight};

    #[test]
    fn vault_delete_preflight_rejects_zero_vault_id() {
        let result = run_vault_delete_preflight(VaultDeletePreflightFacts {
            vault_id_is_zero: true,
            ..Default::default()
        });

        assert_eq!(result, Ter::TEM_MALFORMED);
    }

    #[test]
    fn vault_delete_preflight_accepts_nonzero_vault_id() {
        let result = run_vault_delete_preflight(VaultDeletePreflightFacts::default());

        assert_eq!(result, Ter::TES_SUCCESS);
    }

    #[test]
    fn vault_delete_preflight_rejects_memo_data_without_lending_v1_1() {
        let result = run_vault_delete_preflight(VaultDeletePreflightFacts {
            has_memo_data: true,
            lending_protocol_v1_1_enabled: false,
            memo_data_length_valid: true,
            ..Default::default()
        });

        assert_eq!(result, Ter::TEM_DISABLED);
    }

    #[test]
    fn vault_delete_preflight_rejects_memo_data_too_long() {
        let result = run_vault_delete_preflight(VaultDeletePreflightFacts {
            has_memo_data: true,
            lending_protocol_v1_1_enabled: true,
            memo_data_length_valid: false,
            ..Default::default()
        });

        assert_eq!(result, Ter::TEM_MALFORMED);
    }

    #[test]
    fn vault_delete_preflight_accepts_valid_memo_data_with_lending_v1_1() {
        let result = run_vault_delete_preflight(VaultDeletePreflightFacts {
            has_memo_data: true,
            lending_protocol_v1_1_enabled: true,
            memo_data_length_valid: true,
            ..Default::default()
        });

        assert_eq!(result, Ter::TES_SUCCESS);
    }
}
