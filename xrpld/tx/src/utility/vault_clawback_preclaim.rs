//! Deterministic
//! the reference implementation shell.
//!
//! This ports the exact current ordered guard chain for:
//!
//! - missing vault lookup,
//! - missing share issuance,
//! - ambiguous missing-amount rejection when issuer is owner,
//! - share clawback permission and empty-vault requirements,
//! - non-zero share clawback all-shares requirement,
//! - vault-asset clawback issuer and holder rules,
//! - MPT clawback permission,
//! - IOU clawback issuer-account and flag rules,
//! - and invalid selected-asset rejection.

use protocol::Ter;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VaultClawbackSelectedAmountAssetKind {
    Share,
    VaultAsset,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VaultClawbackVaultAssetKind {
    Mpt,
    Issue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VaultClawbackPreclaimFacts {
    pub vault_exists: bool,
    pub share_issuance_exists: bool,
    pub amount_present: bool,
    pub vault_asset_is_native: bool,
    pub vault_asset_issuer_is_owner: bool,
    pub selected_amount_asset_kind: VaultClawbackSelectedAmountAssetKind,
    pub submitter_is_owner: bool,
    pub vault_shares_total_is_zero: bool,
    pub vault_assets_total_is_zero: bool,
    pub vault_assets_available_is_zero: bool,
    pub selected_amount_is_zero: bool,
    pub selected_amount_matches_shares_held: bool,
    pub submitter_is_vault_asset_issuer: bool,
    pub submitter_is_holder: bool,
    pub vault_asset_kind: VaultClawbackVaultAssetKind,
    pub mpt_vault_asset_issuance_exists: bool,
    pub mpt_vault_asset_can_clawback: bool,
    pub issuer_account_exists: bool,
    pub issuer_allows_trustline_clawback: bool,
    pub issuer_has_no_freeze: bool,
}

impl Default for VaultClawbackPreclaimFacts {
    fn default() -> Self {
        Self {
            vault_exists: false,
            share_issuance_exists: false,
            amount_present: false,
            vault_asset_is_native: false,
            vault_asset_issuer_is_owner: false,
            selected_amount_asset_kind: VaultClawbackSelectedAmountAssetKind::Other,
            submitter_is_owner: false,
            vault_shares_total_is_zero: false,
            vault_assets_total_is_zero: false,
            vault_assets_available_is_zero: false,
            selected_amount_is_zero: true,
            selected_amount_matches_shares_held: false,
            submitter_is_vault_asset_issuer: false,
            submitter_is_holder: false,
            vault_asset_kind: VaultClawbackVaultAssetKind::Issue,
            mpt_vault_asset_issuance_exists: false,
            mpt_vault_asset_can_clawback: false,
            issuer_account_exists: false,
            issuer_allows_trustline_clawback: false,
            issuer_has_no_freeze: false,
        }
    }
}

pub const fn run_vault_clawback_preclaim(facts: VaultClawbackPreclaimFacts) -> Ter {
    if !facts.vault_exists {
        return Ter::TEC_NO_ENTRY;
    }

    if !facts.share_issuance_exists {
        return Ter::TEF_INTERNAL;
    }

    if !facts.amount_present && !facts.vault_asset_is_native && facts.vault_asset_issuer_is_owner {
        return Ter::TEC_WRONG_ASSET;
    }

    match facts.selected_amount_asset_kind {
        VaultClawbackSelectedAmountAssetKind::Share => {
            if !facts.submitter_is_owner {
                return Ter::TEC_NO_PERMISSION;
            }

            if facts.vault_shares_total_is_zero
                || !facts.vault_assets_total_is_zero
                || !facts.vault_assets_available_is_zero
            {
                return Ter::TEC_NO_PERMISSION;
            }

            if !facts.selected_amount_is_zero && !facts.selected_amount_matches_shares_held {
                return Ter::TEC_LIMIT_EXCEEDED;
            }

            Ter::TES_SUCCESS
        }
        VaultClawbackSelectedAmountAssetKind::VaultAsset => {
            if facts.vault_asset_is_native {
                return Ter::TEC_NO_PERMISSION;
            }

            if !facts.submitter_is_vault_asset_issuer {
                return Ter::TEC_NO_PERMISSION;
            }

            if facts.submitter_is_holder {
                return Ter::TEC_NO_PERMISSION;
            }

            match facts.vault_asset_kind {
                VaultClawbackVaultAssetKind::Mpt => {
                    if !facts.mpt_vault_asset_issuance_exists {
                        return Ter::TEC_OBJECT_NOT_FOUND;
                    }

                    if !facts.mpt_vault_asset_can_clawback {
                        return Ter::TEC_NO_PERMISSION;
                    }

                    Ter::TES_SUCCESS
                }
                VaultClawbackVaultAssetKind::Issue => {
                    if !facts.issuer_account_exists {
                        return Ter::TEF_INTERNAL;
                    }

                    if !facts.issuer_allows_trustline_clawback || facts.issuer_has_no_freeze {
                        return Ter::TEC_NO_PERMISSION;
                    }

                    Ter::TES_SUCCESS
                }
            }
        }
        VaultClawbackSelectedAmountAssetKind::Other => Ter::TEC_WRONG_ASSET,
    }
}

#[cfg(test)]
mod tests {
    use protocol::{Ter, trans_token};

    use super::{
        VaultClawbackPreclaimFacts, VaultClawbackSelectedAmountAssetKind,
        VaultClawbackVaultAssetKind, run_vault_clawback_preclaim,
    };

    #[test]
    fn vault_clawback_preclaim_rejects_missing_vault() {
        let result = run_vault_clawback_preclaim(VaultClawbackPreclaimFacts::default());

        assert_eq!(result, Ter::TEC_NO_ENTRY);
        assert_eq!(trans_token(result), "tecNO_ENTRY");
    }

    #[test]
    fn vault_clawback_preclaim_rejects_missing_share_issuance() {
        let result = run_vault_clawback_preclaim(VaultClawbackPreclaimFacts {
            vault_exists: true,
            ..VaultClawbackPreclaimFacts::default()
        });

        assert_eq!(result, Ter::TEF_INTERNAL);
        assert_eq!(trans_token(result), "tefINTERNAL");
    }

    #[test]
    fn vault_clawback_preclaim_rejects_ambiguous_missing_amount() {
        let result = run_vault_clawback_preclaim(VaultClawbackPreclaimFacts {
            vault_exists: true,
            share_issuance_exists: true,
            vault_asset_issuer_is_owner: true,
            ..VaultClawbackPreclaimFacts::default()
        });

        assert_eq!(result, Ter::TEC_WRONG_ASSET);
        assert_eq!(trans_token(result), "tecWRONG_ASSET");
    }

    #[test]
    fn vault_clawback_preclaim_enforces_share_clawback_permissions() {
        let non_owner = run_vault_clawback_preclaim(VaultClawbackPreclaimFacts {
            vault_exists: true,
            share_issuance_exists: true,
            amount_present: true,
            selected_amount_asset_kind: VaultClawbackSelectedAmountAssetKind::Share,
            ..VaultClawbackPreclaimFacts::default()
        });
        let vault_not_empty = run_vault_clawback_preclaim(VaultClawbackPreclaimFacts {
            vault_exists: true,
            share_issuance_exists: true,
            amount_present: true,
            selected_amount_asset_kind: VaultClawbackSelectedAmountAssetKind::Share,
            submitter_is_owner: true,
            vault_assets_total_is_zero: false,
            vault_assets_available_is_zero: true,
            vault_shares_total_is_zero: false,
            ..VaultClawbackPreclaimFacts::default()
        });

        assert_eq!(non_owner, Ter::TEC_NO_PERMISSION);
        assert_eq!(vault_not_empty, Ter::TEC_NO_PERMISSION);
    }

    #[test]
    fn vault_clawback_preclaim_requires_all_shares_for_non_zero_share_clawback() {
        let result = run_vault_clawback_preclaim(VaultClawbackPreclaimFacts {
            vault_exists: true,
            share_issuance_exists: true,
            amount_present: true,
            selected_amount_asset_kind: VaultClawbackSelectedAmountAssetKind::Share,
            submitter_is_owner: true,
            vault_assets_total_is_zero: true,
            vault_assets_available_is_zero: true,
            vault_shares_total_is_zero: false,
            selected_amount_is_zero: false,
            ..VaultClawbackPreclaimFacts::default()
        });

        assert_eq!(result, Ter::TEC_LIMIT_EXCEEDED);
        assert_eq!(trans_token(result), "tecLIMIT_EXCEEDED");
    }

    #[test]
    fn vault_clawback_preclaim_accepts_valid_share_clawback() {
        let zero = run_vault_clawback_preclaim(VaultClawbackPreclaimFacts {
            vault_exists: true,
            share_issuance_exists: true,
            amount_present: true,
            selected_amount_asset_kind: VaultClawbackSelectedAmountAssetKind::Share,
            submitter_is_owner: true,
            vault_assets_total_is_zero: true,
            vault_assets_available_is_zero: true,
            vault_shares_total_is_zero: false,
            ..VaultClawbackPreclaimFacts::default()
        });
        let full = run_vault_clawback_preclaim(VaultClawbackPreclaimFacts {
            vault_exists: true,
            share_issuance_exists: true,
            amount_present: true,
            selected_amount_asset_kind: VaultClawbackSelectedAmountAssetKind::Share,
            submitter_is_owner: true,
            vault_assets_total_is_zero: true,
            vault_assets_available_is_zero: true,
            vault_shares_total_is_zero: false,
            selected_amount_is_zero: false,
            selected_amount_matches_shares_held: true,
            ..VaultClawbackPreclaimFacts::default()
        });

        assert_eq!(zero, Ter::TES_SUCCESS);
        assert_eq!(full, Ter::TES_SUCCESS);
    }

    #[test]
    fn vault_clawback_preclaim_enforces_vault_asset_rules() {
        let native = run_vault_clawback_preclaim(VaultClawbackPreclaimFacts {
            vault_exists: true,
            share_issuance_exists: true,
            amount_present: true,
            selected_amount_asset_kind: VaultClawbackSelectedAmountAssetKind::VaultAsset,
            vault_asset_is_native: true,
            ..VaultClawbackPreclaimFacts::default()
        });
        let not_issuer = run_vault_clawback_preclaim(VaultClawbackPreclaimFacts {
            vault_exists: true,
            share_issuance_exists: true,
            amount_present: true,
            selected_amount_asset_kind: VaultClawbackSelectedAmountAssetKind::VaultAsset,
            ..VaultClawbackPreclaimFacts::default()
        });
        let issuer_is_holder = run_vault_clawback_preclaim(VaultClawbackPreclaimFacts {
            vault_exists: true,
            share_issuance_exists: true,
            amount_present: true,
            selected_amount_asset_kind: VaultClawbackSelectedAmountAssetKind::VaultAsset,
            submitter_is_vault_asset_issuer: true,
            submitter_is_holder: true,
            ..VaultClawbackPreclaimFacts::default()
        });

        assert_eq!(native, Ter::TEC_NO_PERMISSION);
        assert_eq!(not_issuer, Ter::TEC_NO_PERMISSION);
        assert_eq!(issuer_is_holder, Ter::TEC_NO_PERMISSION);
    }

    #[test]
    fn vault_clawback_preclaim_handles_mpt_vault_asset_rules() {
        let missing_mpt = run_vault_clawback_preclaim(VaultClawbackPreclaimFacts {
            vault_exists: true,
            share_issuance_exists: true,
            amount_present: true,
            selected_amount_asset_kind: VaultClawbackSelectedAmountAssetKind::VaultAsset,
            submitter_is_vault_asset_issuer: true,
            vault_asset_kind: VaultClawbackVaultAssetKind::Mpt,
            ..VaultClawbackPreclaimFacts::default()
        });
        let no_clawback = run_vault_clawback_preclaim(VaultClawbackPreclaimFacts {
            vault_exists: true,
            share_issuance_exists: true,
            amount_present: true,
            selected_amount_asset_kind: VaultClawbackSelectedAmountAssetKind::VaultAsset,
            submitter_is_vault_asset_issuer: true,
            vault_asset_kind: VaultClawbackVaultAssetKind::Mpt,
            mpt_vault_asset_issuance_exists: true,
            ..VaultClawbackPreclaimFacts::default()
        });
        let success = run_vault_clawback_preclaim(VaultClawbackPreclaimFacts {
            vault_exists: true,
            share_issuance_exists: true,
            amount_present: true,
            selected_amount_asset_kind: VaultClawbackSelectedAmountAssetKind::VaultAsset,
            submitter_is_vault_asset_issuer: true,
            vault_asset_kind: VaultClawbackVaultAssetKind::Mpt,
            mpt_vault_asset_issuance_exists: true,
            mpt_vault_asset_can_clawback: true,
            ..VaultClawbackPreclaimFacts::default()
        });

        assert_eq!(missing_mpt, Ter::TEC_OBJECT_NOT_FOUND);
        assert_eq!(no_clawback, Ter::TEC_NO_PERMISSION);
        assert_eq!(success, Ter::TES_SUCCESS);
    }

    #[test]
    fn vault_clawback_preclaim_handles_iou_vault_asset_rules() {
        let missing_issuer = run_vault_clawback_preclaim(VaultClawbackPreclaimFacts {
            vault_exists: true,
            share_issuance_exists: true,
            amount_present: true,
            selected_amount_asset_kind: VaultClawbackSelectedAmountAssetKind::VaultAsset,
            submitter_is_vault_asset_issuer: true,
            vault_asset_kind: VaultClawbackVaultAssetKind::Issue,
            ..VaultClawbackPreclaimFacts::default()
        });
        let no_clawback = run_vault_clawback_preclaim(VaultClawbackPreclaimFacts {
            vault_exists: true,
            share_issuance_exists: true,
            amount_present: true,
            selected_amount_asset_kind: VaultClawbackSelectedAmountAssetKind::VaultAsset,
            submitter_is_vault_asset_issuer: true,
            issuer_account_exists: true,
            vault_asset_kind: VaultClawbackVaultAssetKind::Issue,
            ..VaultClawbackPreclaimFacts::default()
        });
        let no_freeze = run_vault_clawback_preclaim(VaultClawbackPreclaimFacts {
            vault_exists: true,
            share_issuance_exists: true,
            amount_present: true,
            selected_amount_asset_kind: VaultClawbackSelectedAmountAssetKind::VaultAsset,
            submitter_is_vault_asset_issuer: true,
            issuer_account_exists: true,
            issuer_allows_trustline_clawback: true,
            issuer_has_no_freeze: true,
            vault_asset_kind: VaultClawbackVaultAssetKind::Issue,
            ..VaultClawbackPreclaimFacts::default()
        });
        let success = run_vault_clawback_preclaim(VaultClawbackPreclaimFacts {
            vault_exists: true,
            share_issuance_exists: true,
            amount_present: true,
            selected_amount_asset_kind: VaultClawbackSelectedAmountAssetKind::VaultAsset,
            submitter_is_vault_asset_issuer: true,
            issuer_account_exists: true,
            issuer_allows_trustline_clawback: true,
            vault_asset_kind: VaultClawbackVaultAssetKind::Issue,
            ..VaultClawbackPreclaimFacts::default()
        });

        assert_eq!(missing_issuer, Ter::TEF_INTERNAL);
        assert_eq!(no_clawback, Ter::TEC_NO_PERMISSION);
        assert_eq!(no_freeze, Ter::TEC_NO_PERMISSION);
        assert_eq!(success, Ter::TES_SUCCESS);
    }

    #[test]
    fn vault_clawback_preclaim_rejects_invalid_selected_asset() {
        let result = run_vault_clawback_preclaim(VaultClawbackPreclaimFacts {
            vault_exists: true,
            share_issuance_exists: true,
            amount_present: true,
            ..VaultClawbackPreclaimFacts::default()
        });

        assert_eq!(result, Ter::TEC_WRONG_ASSET);
        assert_eq!(trans_token(result), "tecWRONG_ASSET");
    }
}
