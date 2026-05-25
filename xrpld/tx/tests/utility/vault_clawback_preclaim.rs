//! Integration tests that pin the narrowed Rust `VaultClawback.cpp::preclaim(...)`
//! wrapper to the current C++ behavior.

use protocol::{Ter, trans_token};
use tx::{
    VaultClawbackPreclaimFacts, VaultClawbackSelectedAmountAssetKind, VaultClawbackVaultAssetKind,
    run_vault_clawback_preclaim,
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
