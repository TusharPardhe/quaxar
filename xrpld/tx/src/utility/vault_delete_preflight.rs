//! Deterministic
//! the reference implementation shell.
//!
//! This ports the exact current malformed guard for a zero or empty
//! `sfVaultID`.

use protocol::{NotTec, Ter};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VaultDeletePreflightFacts {
    pub vault_id_is_zero: bool,
}

pub const fn run_vault_delete_preflight(facts: VaultDeletePreflightFacts) -> NotTec {
    if facts.vault_id_is_zero {
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
        });

        assert_eq!(result, Ter::TEM_MALFORMED);
    }

    #[test]
    fn vault_delete_preflight_accepts_nonzero_vault_id() {
        let result = run_vault_delete_preflight(VaultDeletePreflightFacts::default());

        assert_eq!(result, Ter::TES_SUCCESS);
    }
}
