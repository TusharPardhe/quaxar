//! the reference implementation parity — vault share/asset conversion math.

use basics::number::{NumberParts as RuntimeNumber, get_mantissa_scale};
use protocol::{
    IOUAmount, MPTAmount, MPTIssue, STAmount, STLedgerEntry, get_field_by_symbol, sf_generic,
};

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

fn stamount_as_number(amount: &STAmount) -> RuntimeNumber {
    if amount.native() {
        RuntimeNumber::from(amount.xrp())
    } else if amount.holds_mpt_issue() {
        RuntimeNumber::from(amount.mpt())
    } else {
        RuntimeNumber::from(amount.iou())
    }
}

fn number_to_mpt_stamount(issue: MPTIssue, number: RuntimeNumber) -> STAmount {
    let mpt = MPTAmount::try_from(number).expect("MPT amount should stay representable");
    STAmount::from_mpt_amount(sf_generic(), mpt, issue)
}

fn number_to_iou_stamount(issue: protocol::Issue, number: RuntimeNumber) -> STAmount {
    let iou = IOUAmount::try_from(number).expect("IOU amount should stay representable");
    STAmount::from_iou_amount(sf_generic(), iou, issue)
}

/// Vault share truncation control for withdraw calculations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TruncateShares {
    Yes,
    No,
}

/// Converts a deposit asset amount to the equivalent share amount.
///
pub fn assets_to_shares_deposit(
    vault: &STLedgerEntry,
    issuance: &STLedgerEntry,
    assets: &STAmount,
) -> Option<STAmount> {
    if assets.negative() {
        return None;
    }

    let asset_total = stamount_as_number(&vault.get_field_amount(sf("sfAssetsTotal")));
    let share_amount = vault.get_field_amount(sf("sfShareMPTID"));
    let share_issue = match share_amount.asset() {
        protocol::Asset::MPTIssue(issue) => issue,
        _ => return None,
    };

    let scale = get_mantissa_scale();
    let zero = RuntimeNumber::try_from_external_parts(0, 0, scale).unwrap();

    if asset_total == zero {
        let vault_scale = vault.get_field_u8(sf("sfScale")) as i32;
        let assets_num = stamount_as_number(assets);
        // Scale: shift exponent by vault scale, then truncate
        let scaled = RuntimeNumber::try_from_external_parts(
            assets_num.mantissa as i64,
            assets_num.exponent + vault_scale,
            scale,
        )
        .ok()?;
        let truncated = scaled.truncate(scale);
        return Some(number_to_mpt_stamount(share_issue, truncated));
    }

    let share_total = stamount_as_number(&issuance.get_field_amount(sf("sfOutstandingAmount")));
    let assets_num = stamount_as_number(assets);
    let result = ((share_total * assets_num) / asset_total).truncate(scale);
    Some(number_to_mpt_stamount(share_issue, result))
}

/// Converts a deposit share amount to the equivalent asset amount.
///
pub fn shares_to_assets_deposit(
    vault: &STLedgerEntry,
    issuance: &STLedgerEntry,
    shares: &STAmount,
) -> Option<STAmount> {
    if shares.negative() {
        return None;
    }

    let asset_total = stamount_as_number(&vault.get_field_amount(sf("sfAssetsTotal")));
    let vault_asset = vault.get_field_amount(sf("sfAsset"));
    let asset_issue = vault_asset.issue();

    let scale = get_mantissa_scale();
    let zero = RuntimeNumber::try_from_external_parts(0, 0, scale).unwrap();

    if asset_total == zero {
        let vault_scale = vault.get_field_u8(sf("sfScale")) as i32;
        let shares_num = stamount_as_number(shares);
        let result = RuntimeNumber::try_from_external_parts(
            shares_num.mantissa as i64,
            shares_num.exponent - vault_scale,
            scale,
        )
        .ok()?;
        return Some(number_to_iou_stamount(asset_issue, result));
    }

    let share_total = stamount_as_number(&issuance.get_field_amount(sf("sfOutstandingAmount")));
    let shares_num = stamount_as_number(shares);
    let result = (asset_total * shares_num) / share_total;
    Some(number_to_iou_stamount(asset_issue, result))
}

/// Converts a withdrawal asset amount to the equivalent share amount.
///
pub fn assets_to_shares_withdraw(
    vault: &STLedgerEntry,
    issuance: &STLedgerEntry,
    assets: &STAmount,
    truncate: TruncateShares,
) -> Option<STAmount> {
    if assets.negative() {
        return None;
    }

    let asset_total = stamount_as_number(&vault.get_field_amount(sf("sfAssetsTotal")));
    let loss = stamount_as_number(&vault.get_field_amount(sf("sfLossUnrealized")));
    let effective_total = asset_total - loss;

    let share_amount = vault.get_field_amount(sf("sfShareMPTID"));
    let share_issue = match share_amount.asset() {
        protocol::Asset::MPTIssue(issue) => issue,
        _ => return None,
    };

    let scale = get_mantissa_scale();
    let zero = RuntimeNumber::try_from_external_parts(0, 0, scale).unwrap();

    if effective_total == zero {
        return Some(number_to_mpt_stamount(share_issue, zero));
    }

    let share_total = stamount_as_number(&issuance.get_field_amount(sf("sfOutstandingAmount")));
    let assets_num = stamount_as_number(assets);
    let mut result = (share_total * assets_num) / effective_total;
    if truncate == TruncateShares::Yes {
        result = result.truncate(scale);
    }
    Some(number_to_mpt_stamount(share_issue, result))
}

/// Converts a withdrawal share amount to the equivalent asset amount.
///
pub fn shares_to_assets_withdraw(
    vault: &STLedgerEntry,
    issuance: &STLedgerEntry,
    shares: &STAmount,
) -> Option<STAmount> {
    if shares.negative() {
        return None;
    }

    let asset_total = stamount_as_number(&vault.get_field_amount(sf("sfAssetsTotal")));
    let loss = stamount_as_number(&vault.get_field_amount(sf("sfLossUnrealized")));
    let effective_total = asset_total - loss;

    let vault_asset = vault.get_field_amount(sf("sfAsset"));
    let asset_issue = vault_asset.issue();

    let scale = get_mantissa_scale();
    let zero = RuntimeNumber::try_from_external_parts(0, 0, scale).unwrap();

    if effective_total == zero {
        return Some(number_to_iou_stamount(asset_issue, zero));
    }

    let share_total = stamount_as_number(&issuance.get_field_amount(sf("sfOutstandingAmount")));
    let shares_num = stamount_as_number(shares);
    let result = (effective_total * shares_num) / share_total;
    Some(number_to_iou_stamount(asset_issue, result))
}
