//! Static `VaultCreate` transactor metadata helpers in the reference implementation.
//!
//! This ports the exact deterministic behavior around:
//!
//! - requiring `featureMPTokensV1`,
//! - consulting `featurePermissionedDomains` only when `sfDomainID` is present,
//! - and returning the literal current `tfVaultCreateMask` value from
//!   `TxFlags.h`.

pub const FULLY_CANONICAL_SIGNATURE_FLAG: u32 = 0x8000_0000;
pub const INNER_BATCH_TRANSACTION_FLAG: u32 = 0x4000_0000;
pub const VAULT_PRIVATE_FLAG: u32 = 0x0001_0000;
pub const VAULT_SHARE_NON_TRANSFERABLE_FLAG: u32 = 0x0002_0000;
pub const VAULT_CREATE_FLAGS_MASK: u32 = !(FULLY_CANONICAL_SIGNATURE_FLAG
    | INNER_BATCH_TRANSACTION_FLAG
    | VAULT_PRIVATE_FLAG
    | VAULT_SHARE_NON_TRANSFERABLE_FLAG);

pub fn run_vault_create_check_extra_features(
    mptokens_v1_enabled: bool,
    domain_id_present: bool,
    permissioned_domains_enabled: impl FnOnce() -> bool,
) -> bool {
    if !mptokens_v1_enabled {
        return false;
    }

    if domain_id_present && !permissioned_domains_enabled() {
        return false;
    }

    true
}

pub const fn get_vault_create_flags_mask() -> u32 {
    VAULT_CREATE_FLAGS_MASK
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::{
        FULLY_CANONICAL_SIGNATURE_FLAG, INNER_BATCH_TRANSACTION_FLAG, VAULT_CREATE_FLAGS_MASK,
        VAULT_PRIVATE_FLAG, VAULT_SHARE_NON_TRANSFERABLE_FLAG, get_vault_create_flags_mask,
        run_vault_create_check_extra_features,
    };

    #[test]
    fn vault_create_check_extra_features_requires_mptokens_before_any_domain_logic() {
        let permissioned_domains_checked = Cell::new(false);

        let result = run_vault_create_check_extra_features(false, true, || {
            permissioned_domains_checked.set(true);
            true
        });

        assert!(!result);
        assert!(!permissioned_domains_checked.get());
    }

    #[test]
    fn vault_create_check_extra_features_skips_permissioned_domain_gate_without_domain() {
        let permissioned_domains_checked = Cell::new(false);

        let result = run_vault_create_check_extra_features(true, false, || {
            permissioned_domains_checked.set(true);
            false
        });

        assert!(result);
        assert!(!permissioned_domains_checked.get());
    }

    #[test]
    fn vault_create_check_extra_features_rejects_domain_without_permissioned_domains() {
        assert!(!run_vault_create_check_extra_features(true, true, || false));
        assert!(run_vault_create_check_extra_features(true, true, || true));
    }

    #[test]
    fn vault_create_flags_mask_txflags() {
        assert_eq!(VAULT_PRIVATE_FLAG, 0x0001_0000);
        assert_eq!(VAULT_SHARE_NON_TRANSFERABLE_FLAG, 0x0002_0000);
        assert_eq!(FULLY_CANONICAL_SIGNATURE_FLAG, 0x8000_0000);
        assert_eq!(INNER_BATCH_TRANSACTION_FLAG, 0x4000_0000);
        assert_eq!(VAULT_CREATE_FLAGS_MASK, 0x3ffc_ffff);
        assert_eq!(get_vault_create_flags_mask(), 0x3ffc_ffff);
    }

    #[test]
    fn vault_create_flags_mask_rejects_universal_and_vault_specific_bits() {
        assert_eq!(
            get_vault_create_flags_mask() & FULLY_CANONICAL_SIGNATURE_FLAG,
            0
        );
        assert_eq!(
            get_vault_create_flags_mask() & INNER_BATCH_TRANSACTION_FLAG,
            0
        );
        assert_eq!(get_vault_create_flags_mask() & VAULT_PRIVATE_FLAG, 0);
        assert_eq!(
            get_vault_create_flags_mask() & VAULT_SHARE_NON_TRANSFERABLE_FLAG,
            0
        );
        assert_eq!(get_vault_create_flags_mask() & 0x0004_0000, 0x0004_0000);
    }
}
