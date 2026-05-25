//! Deterministic
//! the reference implementation shell.
//!
//! This ports the exact ordered branch behavior over pre-decoded facts for:
//!
//! - missing vault lookup,
//! - non-owner submitter rejection,
//! - non-zero assets-available and assets-total obligation checks,
//! - missing share issuance,
//! - mismatched issuance issuer,
//! - and non-zero outstanding shares.

use protocol::Ter;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VaultDeletePreclaimFacts {
    pub vault_exists: bool,
    pub submitter_is_owner: bool,
    pub assets_available_is_zero: bool,
    pub assets_total_is_zero: bool,
    pub issuance_exists: bool,
    pub issuance_issuer_matches_pseudo: bool,
    pub outstanding_amount_is_zero: bool,
}

pub const fn run_vault_delete_preclaim(facts: VaultDeletePreclaimFacts) -> Ter {
    if !facts.vault_exists {
        return Ter::TEC_NO_ENTRY;
    }

    if !facts.submitter_is_owner {
        return Ter::TEC_NO_PERMISSION;
    }

    if !facts.assets_available_is_zero {
        return Ter::TEC_HAS_OBLIGATIONS;
    }

    if !facts.assets_total_is_zero {
        return Ter::TEC_HAS_OBLIGATIONS;
    }

    if !facts.issuance_exists {
        return Ter::TEC_OBJECT_NOT_FOUND;
    }

    if !facts.issuance_issuer_matches_pseudo {
        return Ter::TEC_NO_PERMISSION;
    }

    if !facts.outstanding_amount_is_zero {
        return Ter::TEC_HAS_OBLIGATIONS;
    }

    Ter::TES_SUCCESS
}

#[cfg(test)]
mod tests {
    use protocol::{Ter, trans_token};

    use super::{VaultDeletePreclaimFacts, run_vault_delete_preclaim};

    fn base() -> VaultDeletePreclaimFacts {
        VaultDeletePreclaimFacts {
            vault_exists: true,
            submitter_is_owner: true,
            assets_available_is_zero: true,
            assets_total_is_zero: true,
            issuance_exists: true,
            issuance_issuer_matches_pseudo: true,
            outstanding_amount_is_zero: true,
        }
    }

    #[test]
    fn vault_delete_preclaim_rejects_missing_vault() {
        let result = run_vault_delete_preclaim(VaultDeletePreclaimFacts::default());

        assert_eq!(result, Ter::TEC_NO_ENTRY);
    }

    #[test]
    fn vault_delete_preclaim_rejects_non_owner_submitter() {
        let result = run_vault_delete_preclaim(VaultDeletePreclaimFacts {
            vault_exists: true,
            ..VaultDeletePreclaimFacts::default()
        });

        assert_eq!(result, Ter::TEC_NO_PERMISSION);
    }

    #[test]
    fn vault_delete_preclaim_rejects_nonzero_assets_available() {
        let result = run_vault_delete_preclaim(VaultDeletePreclaimFacts {
            assets_available_is_zero: false,
            ..base()
        });

        assert_eq!(result, Ter::TEC_HAS_OBLIGATIONS);
        assert_eq!(trans_token(result), "tecHAS_OBLIGATIONS");
    }

    #[test]
    fn vault_delete_preclaim_rejects_nonzero_assets_total() {
        let result = run_vault_delete_preclaim(VaultDeletePreclaimFacts {
            assets_total_is_zero: false,
            ..base()
        });

        assert_eq!(result, Ter::TEC_HAS_OBLIGATIONS);
    }

    #[test]
    fn vault_delete_preclaim_rejects_missing_share_issuance() {
        let result = run_vault_delete_preclaim(VaultDeletePreclaimFacts {
            issuance_exists: false,
            ..base()
        });

        assert_eq!(result, Ter::TEC_OBJECT_NOT_FOUND);
    }

    #[test]
    fn vault_delete_preclaim_rejects_mismatched_issuance_owner() {
        let result = run_vault_delete_preclaim(VaultDeletePreclaimFacts {
            issuance_issuer_matches_pseudo: false,
            ..base()
        });

        assert_eq!(result, Ter::TEC_NO_PERMISSION);
    }

    #[test]
    fn vault_delete_preclaim_rejects_nonzero_outstanding_shares() {
        let result = run_vault_delete_preclaim(VaultDeletePreclaimFacts {
            outstanding_amount_is_zero: false,
            ..base()
        });

        assert_eq!(result, Ter::TEC_HAS_OBLIGATIONS);
    }

    #[test]
    fn vault_delete_preclaim_accepts_empty_owner_vault() {
        let result = run_vault_delete_preclaim(base());

        assert_eq!(result, Ter::TES_SUCCESS);
    }
}
