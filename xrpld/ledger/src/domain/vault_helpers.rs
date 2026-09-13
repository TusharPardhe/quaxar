//! the reference implementation parity — vault share/asset conversion math.

use crate::amm_helpers::RelativeDistanceAmount;
use basics::base_uint::Uint160;
use basics::number::{
    NumberParts as RuntimeNumber, NumberRoundModeGuard, RoundingMode, get_mantissa_scale,
};
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

/// Return the canonical storage scale for an asset value.
///
/// This is rippled's `getAssetsTotalScale` rule: it is intentionally unrelated
/// to a vault's `sfScale`, which controls share conversion rather than asset
/// storage precision.
pub fn asset_scale_from_value(asset: Asset, value: RuntimeNumber) -> i32 {
    if asset.integral() {
        0
    } else {
        asset
            .amount(value)
            .map(|amount| amount.exponent())
            .unwrap_or(0)
    }
}

/// Round a runtime number to a decimal grid without depending on ambient
/// rounding state. Digits are removed least-significant first, so the final
/// removed digit is the rounding digit and earlier digits are sticky.
pub fn round_runtime_to_scale(
    value: RuntimeNumber,
    target_scale: i32,
    rounding: RoundingMode,
) -> RuntimeNumber {
    let Ok((mantissa, mut exponent)) = value.external_parts() else {
        return value;
    };
    if mantissa == 0 || exponent >= target_scale {
        return value;
    }

    let negative = mantissa < 0;
    let mut abs = mantissa.unsigned_abs() as u128;
    let mut removed = Vec::new();
    while exponent < target_scale {
        removed.push((abs % 10) as u8);
        abs /= 10;
        exponent += 1;
    }
    let rounding_digit = removed.last().copied().unwrap_or(0);
    let sticky = removed
        .get(..removed.len().saturating_sub(1))
        .is_some_and(|tail| tail.iter().any(|digit| *digit != 0));
    let round_up = match rounding {
        RoundingMode::TowardsZero => false,
        RoundingMode::Downward => negative && (rounding_digit != 0 || sticky),
        RoundingMode::Upward => !negative && (rounding_digit != 0 || sticky),
        RoundingMode::ToNearest => {
            rounding_digit > 5 || (rounding_digit == 5 && (sticky || ((abs as u64) & 1) == 1))
        }
    };
    if round_up {
        abs += 1;
    }

    let signed = if negative { -(abs as i64) } else { abs as i64 };
    RuntimeNumber::try_from_external_parts(signed, exponent, get_mantissa_scale()).unwrap_or(value)
}

/// Quantize an asset value at `scale`, matching rippled's `roundToAsset` /
/// `roundToScale` behavior while isolating callers from ambient rounding mode.
pub fn round_number_to_asset_with_scale(
    asset: Asset,
    value: RuntimeNumber,
    scale: i32,
    rounding: RoundingMode,
) -> RuntimeNumber {
    if asset.integral() {
        return round_runtime_to_scale(value, 0, rounding);
    }

    let _rounding = NumberRoundModeGuard::new(rounding);
    let Some(value_amount) = asset.amount(value).ok() else {
        return value;
    };
    if value_amount.signum() == 0 || value_amount.exponent() >= scale {
        return value_amount.as_number();
    }
    let reference_mantissa = if value < RuntimeNumber::zero() {
        -1_000_000_000_000_000_i64
    } else {
        1_000_000_000_000_000_i64
    };
    let Ok(reference_value) =
        RuntimeNumber::try_from_external_parts(reference_mantissa, scale, get_mantissa_scale())
    else {
        return value;
    };
    let Some(reference_amount) = asset.amount(reference_value).ok() else {
        return value;
    };
    (value_amount + reference_amount.clone() - reference_amount).as_number()
}

/// Clamp a signed Vault asset change to the decimal grid selected by the
/// posterior `sfAssetsTotal`. The returned amount is always positive and never
/// exceeds the requested magnitude. A sub-ULP result is `tecPRECISION_LOSS`.
pub fn clamp_to_assets_total_scale(
    vault: &STLedgerEntry,
    delta: &STAmount,
) -> Result<STAmount, protocol::Ter> {
    let asset = vault_asset(vault);
    if delta.asset() != asset {
        return Err(protocol::Ter::TEC_INTERNAL);
    }
    let mut magnitude = delta.clone();
    if magnitude.negative() {
        magnitude.negate();
    }
    if asset.integral() {
        return Ok(magnitude);
    }

    let total = vault_number(vault, "sfAssetsTotal");
    let delta_number = stamount_as_number(delta);
    let post_scale = {
        let _rounding = NumberRoundModeGuard::new(RoundingMode::ToNearest);
        asset_scale_from_value(asset, total + delta_number)
    };
    let actual = if delta.negative() {
        round_number_to_asset_with_scale(
            asset,
            stamount_as_number(&magnitude),
            post_scale,
            RoundingMode::Downward,
        )
    } else {
        let _rounding = NumberRoundModeGuard::new(RoundingMode::Downward);
        let posterior = total + stamount_as_number(&magnitude);
        let floored =
            round_number_to_asset_with_scale(asset, posterior, post_scale, RoundingMode::Downward);
        floored - total
    };
    if actual <= RuntimeNumber::zero() {
        return Err(protocol::Ter::TEC_PRECISION_LOSS);
    }
    Ok(number_to_asset_stamount(asset, actual))
}

/// Canonicalize a runtime number to the asset's normal storage representation.
pub fn round_number_to_asset(asset: Asset, value: RuntimeNumber) -> RuntimeNumber {
    asset
        .amount(value)
        .map(|amount| amount.as_number())
        .unwrap_or(value)
}
