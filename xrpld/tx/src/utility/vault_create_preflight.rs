//! Deterministic
//! the reference implementation shell.
//!
//! This ports the exact ordered branch behavior over pre-decoded facts for:
//!
//! - `sfData` length validation via the current `Transactor::validDataLength`
//!   rule,
//! - `sfWithdrawalPolicy`,
//! - `sfDomainID` plus `tfVaultPrivate`,
//! - `sfAssetsMaximum`,
//! - `sfMPTokenMetadata`,
//! - and `sfScale` against native or MPT assets and the IOU scale cap.

use protocol::{NotTec, Ter};

use crate::run_transactor_valid_data_length;

pub const MAX_DATA_PAYLOAD_LENGTH: usize = 256;
pub const MAX_MPTOKEN_METADATA_LENGTH: usize = 1024;
pub const VAULT_STRATEGY_FIRST_COME_FIRST_SERVE: u8 = 1;
pub const VAULT_MAXIMUM_IOU_SCALE: u8 = 18;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VaultCreatePreflightFacts {
    pub data_len: Option<usize>,
    pub withdrawal_policy: Option<u8>,
    pub domain_id_present: bool,
    pub domain_id_is_zero: bool,
    pub is_private: bool,
    pub assets_maximum_is_negative: bool,
    pub mptoken_metadata_len: Option<usize>,
    pub scale: Option<u8>,
    pub asset_is_mpt: bool,
    pub asset_is_native: bool,
}

pub const fn run_vault_create_preflight(facts: VaultCreatePreflightFacts) -> NotTec {
    if !run_transactor_valid_data_length(facts.data_len, MAX_DATA_PAYLOAD_LENGTH) {
        return Ter::TEM_MALFORMED;
    }

    if let Some(withdrawal_policy) = facts.withdrawal_policy
        && withdrawal_policy != VAULT_STRATEGY_FIRST_COME_FIRST_SERVE
    {
        return Ter::TEM_MALFORMED;
    }

    if facts.domain_id_present {
        if facts.domain_id_is_zero {
            return Ter::TEM_MALFORMED;
        }
        if !facts.is_private {
            return Ter::TEM_MALFORMED;
        }
    }

    if facts.assets_maximum_is_negative {
        return Ter::TEM_MALFORMED;
    }

    if let Some(metadata_len) = facts.mptoken_metadata_len
        && (metadata_len == 0 || metadata_len > MAX_MPTOKEN_METADATA_LENGTH)
    {
        return Ter::TEM_MALFORMED;
    }

    if let Some(scale) = facts.scale {
        if facts.asset_is_mpt || facts.asset_is_native {
            return Ter::TEM_MALFORMED;
        }
        if scale > VAULT_MAXIMUM_IOU_SCALE {
            return Ter::TEM_MALFORMED;
        }
    }

    Ter::TES_SUCCESS
}

#[cfg(test)]
mod tests {
    use protocol::{Ter, trans_token};

    use super::{
        MAX_DATA_PAYLOAD_LENGTH, MAX_MPTOKEN_METADATA_LENGTH, VAULT_MAXIMUM_IOU_SCALE,
        VAULT_STRATEGY_FIRST_COME_FIRST_SERVE, VaultCreatePreflightFacts,
        run_vault_create_preflight,
    };

    #[test]
    fn vault_create_preflight_rejects_empty_and_oversized_data() {
        let empty = run_vault_create_preflight(VaultCreatePreflightFacts {
            data_len: Some(0),
            ..VaultCreatePreflightFacts::default()
        });
        let oversized = run_vault_create_preflight(VaultCreatePreflightFacts {
            data_len: Some(MAX_DATA_PAYLOAD_LENGTH + 1),
            ..VaultCreatePreflightFacts::default()
        });

        assert_eq!(empty, Ter::TEM_MALFORMED);
        assert_eq!(oversized, Ter::TEM_MALFORMED);
        assert_eq!(trans_token(empty), "temMALFORMED");
    }

    #[test]
    fn vault_create_preflight_rejects_non_fcfs_withdrawal_policy() {
        let result = run_vault_create_preflight(VaultCreatePreflightFacts {
            withdrawal_policy: Some(VAULT_STRATEGY_FIRST_COME_FIRST_SERVE - 1),
            ..VaultCreatePreflightFacts::default()
        });

        assert_eq!(result, Ter::TEM_MALFORMED);
    }

    #[test]
    fn vault_create_preflight_rejects_zero_or_public_domain() {
        let zero_domain = run_vault_create_preflight(VaultCreatePreflightFacts {
            domain_id_present: true,
            domain_id_is_zero: true,
            is_private: true,
            ..VaultCreatePreflightFacts::default()
        });
        let public_domain = run_vault_create_preflight(VaultCreatePreflightFacts {
            domain_id_present: true,
            domain_id_is_zero: false,
            is_private: false,
            ..VaultCreatePreflightFacts::default()
        });

        assert_eq!(zero_domain, Ter::TEM_MALFORMED);
        assert_eq!(public_domain, Ter::TEM_MALFORMED);
    }

    #[test]
    fn vault_create_preflight_rejects_negative_assets_maximum() {
        let result = run_vault_create_preflight(VaultCreatePreflightFacts {
            assets_maximum_is_negative: true,
            ..VaultCreatePreflightFacts::default()
        });

        assert_eq!(result, Ter::TEM_MALFORMED);
    }

    #[test]
    fn vault_create_preflight_rejects_empty_and_oversized_metadata() {
        let empty = run_vault_create_preflight(VaultCreatePreflightFacts {
            mptoken_metadata_len: Some(0),
            ..VaultCreatePreflightFacts::default()
        });
        let oversized = run_vault_create_preflight(VaultCreatePreflightFacts {
            mptoken_metadata_len: Some(MAX_MPTOKEN_METADATA_LENGTH + 1),
            ..VaultCreatePreflightFacts::default()
        });

        assert_eq!(empty, Ter::TEM_MALFORMED);
        assert_eq!(oversized, Ter::TEM_MALFORMED);
    }

    #[test]
    fn vault_create_preflight_rejects_scale_for_mpt_and_native_assets() {
        let mpt = run_vault_create_preflight(VaultCreatePreflightFacts {
            scale: Some(0),
            asset_is_mpt: true,
            ..VaultCreatePreflightFacts::default()
        });
        let native = run_vault_create_preflight(VaultCreatePreflightFacts {
            scale: Some(0),
            asset_is_native: true,
            ..VaultCreatePreflightFacts::default()
        });

        assert_eq!(mpt, Ter::TEM_MALFORMED);
        assert_eq!(native, Ter::TEM_MALFORMED);
    }

    #[test]
    fn vault_create_preflight_rejects_scale_above_iou_cap() {
        let result = run_vault_create_preflight(VaultCreatePreflightFacts {
            scale: Some(VAULT_MAXIMUM_IOU_SCALE + 1),
            ..VaultCreatePreflightFacts::default()
        });

        assert_eq!(result, Ter::TEM_MALFORMED);
    }

    #[test]
    fn vault_create_preflight_accepts_valid_private_iou_boundary_values() {
        let result = run_vault_create_preflight(VaultCreatePreflightFacts {
            data_len: Some(MAX_DATA_PAYLOAD_LENGTH),
            withdrawal_policy: Some(VAULT_STRATEGY_FIRST_COME_FIRST_SERVE),
            domain_id_present: true,
            domain_id_is_zero: false,
            is_private: true,
            mptoken_metadata_len: Some(MAX_MPTOKEN_METADATA_LENGTH),
            scale: Some(VAULT_MAXIMUM_IOU_SCALE),
            ..VaultCreatePreflightFacts::default()
        });

        assert_eq!(result, Ter::TES_SUCCESS);
    }
}
