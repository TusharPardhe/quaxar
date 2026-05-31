//! the reference implementation parity — vault share/asset conversion math.

use basics::base_uint::Uint160;
use basics::number::{NumberParts as RuntimeNumber, RoundingMode, get_mantissa_scale};
use protocol::{
    AccountID, Asset, MPTIssue, STAmount, STLedgerEntry, get_field_by_symbol, make_mpt_id,
    mptoken_keylet_from_mptid, to_amount_from_number,
};

use crate::views::read_view::{ReadView, ViewError};

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
    to_amount_from_number(Asset::MPTIssue(issue), number, RoundingMode::TowardsZero)
        .expect("MPT amount should stay representable")
}

fn number_to_asset_stamount(asset: Asset, number: RuntimeNumber) -> STAmount {
    to_amount_from_number(asset, number, RoundingMode::TowardsZero)
        .expect("asset amount should stay representable")
}

fn vault_asset(vault: &STLedgerEntry) -> Asset {
    vault.get_field_issue(sf("sfAsset")).asset()
}

fn vault_share_issue(vault: &STLedgerEntry) -> MPTIssue {
    MPTIssue::new(vault.get_field_h192(sf("sfShareMPTID")))
}

fn vault_number(vault: &STLedgerEntry, field: &str) -> RuntimeNumber {
    vault.get_field_number(sf(field)).value()
}

fn outstanding_amount(issuance: &STLedgerEntry) -> RuntimeNumber {
    RuntimeNumber::from_i64(issuance.get_field_u64(sf("sfOutstandingAmount")) as i64)
}

fn effective_withdraw_total(
    vault: &STLedgerEntry,
    waive_unrealized_loss: WaiveUnrealizedLoss,
) -> RuntimeNumber {
    let asset_total = vault_number(vault, "sfAssetsTotal");
    if waive_unrealized_loss.enabled() {
        asset_total
    } else {
        asset_total - vault_number(vault, "sfLossUnrealized")
    }
}

/// Vault share truncation control for withdraw calculations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TruncateShares {
    Yes,
    No,
}

/// Whether withdraw math should ignore unrealized loss.
///
/// This matches the C++ `WaiveUnrealizedLoss` path used for post-`fixCleanup3_2_0`
/// final withdrawals by the sole vault shareholder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaiveUnrealizedLoss {
    Yes,
    No,
}

impl WaiveUnrealizedLoss {
    fn enabled(self) -> bool {
        matches!(self, Self::Yes)
    }
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

    let asset_total = vault_number(vault, "sfAssetsTotal");
    let share_issue = vault_share_issue(vault);

    let scale = get_mantissa_scale();
    let zero = RuntimeNumber::try_from_external_parts(0, 0, scale).unwrap();

    if asset_total == zero {
        let vault_scale = vault.get_field_u8(sf("sfScale")) as i32;
        // Scale: shift the STAmount exponent by the vault scale, then truncate.
        let scaled = RuntimeNumber::try_from_external_parts(
            assets.mantissa() as i64,
            assets.exponent() + vault_scale,
            scale,
        )
        .ok()?;
        let truncated = scaled.truncate(scale);
        return Some(number_to_mpt_stamount(share_issue, truncated));
    }

    let share_total = outstanding_amount(issuance);
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

    let asset_total = vault_number(vault, "sfAssetsTotal");
    let asset = vault_asset(vault);

    let scale = get_mantissa_scale();
    let zero = RuntimeNumber::try_from_external_parts(0, 0, scale).unwrap();

    if asset_total == zero {
        let vault_scale = vault.get_field_u8(sf("sfScale")) as i32;
        let result = RuntimeNumber::try_from_external_parts(
            shares.mantissa() as i64,
            shares.exponent() - vault_scale,
            scale,
        )
        .ok()?;
        return Some(number_to_asset_stamount(asset, result));
    }

    let share_total = outstanding_amount(issuance);
    let shares_num = stamount_as_number(shares);
    let result = (asset_total * shares_num) / share_total;
    Some(number_to_asset_stamount(asset, result))
}

/// Converts a withdrawal asset amount to the equivalent share amount.
///
pub fn assets_to_shares_withdraw(
    vault: &STLedgerEntry,
    issuance: &STLedgerEntry,
    assets: &STAmount,
    truncate: TruncateShares,
    waive_unrealized_loss: WaiveUnrealizedLoss,
) -> Option<STAmount> {
    if assets.negative() {
        return None;
    }

    let effective_total = effective_withdraw_total(vault, waive_unrealized_loss);
    let share_issue = vault_share_issue(vault);

    let scale = get_mantissa_scale();
    let zero = RuntimeNumber::try_from_external_parts(0, 0, scale).unwrap();

    if effective_total == zero {
        return Some(number_to_mpt_stamount(share_issue, zero));
    }

    let share_total = outstanding_amount(issuance);
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
    waive_unrealized_loss: WaiveUnrealizedLoss,
) -> Option<STAmount> {
    if shares.negative() {
        return None;
    }

    let effective_total = effective_withdraw_total(vault, waive_unrealized_loss);
    let asset = vault_asset(vault);

    let scale = get_mantissa_scale();
    let zero = RuntimeNumber::try_from_external_parts(0, 0, scale).unwrap();

    if effective_total == zero {
        return Some(number_to_asset_stamount(asset, zero));
    }

    let share_total = outstanding_amount(issuance);
    let shares_num = stamount_as_number(shares);
    let result = (effective_total * shares_num) / share_total;
    Some(number_to_asset_stamount(asset, result))
}

/// Returns true when `account` owns the entire vault share issuance.
pub fn is_sole_shareholder(
    view: &dyn ReadView,
    account: &AccountID,
    issuance: &STLedgerEntry,
) -> Result<bool, ViewError> {
    let outstanding = issuance.get_field_u64(sf("sfOutstandingAmount"));
    if outstanding == 0 {
        return Ok(false);
    }

    let issuer = issuance.get_account_id(sf("sfIssuer"));
    let sequence = issuance.get_field_u32(sf("sfSequence"));
    let mpt_id = make_mpt_id(sequence, issuer);
    let account_id = Uint160::from_slice(account.data())
        .ok_or_else(|| ViewError::Conversion("account id must be 160 bits".to_string()))?;
    let Some(token) = view.read(mptoken_keylet_from_mptid(mpt_id, account_id))? else {
        return Ok(false);
    };

    Ok(token.get_field_u64(sf("sfMPTAmount")) == outstanding)
}
