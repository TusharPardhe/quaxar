//! Pure parts of `LoanManage::defaultLoan(...)`.
//!
//! This helper ports the current formula and guard ordering around:
//!
//! - `owedToVault(...)`-driven total default amount,
//! - minimum-cover then liquidation-cover ordering,
//! - the `min(..., coverAvailable)` cap,
//! - vault default amount derivation,
//! - the vault-total shortfall guard,
//! - the non-integral vault dust reconciliation branch,
//! - `adjustImpreciseNumber(...)` for broker debt and realized loss,
//! - the impaired-loan unrealized-loss guard,
//! - and the post-rounding vault-assets-vs-total consistency check.

use std::{
    cmp::min,
    ops::{Add, Sub},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoanManageDefaultRoundingMode {
    Upward,
    Downward,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanManageDefaultFacts<Amount, Asset> {
    pub asset: Asset,
    pub loan_scale: i32,
    pub vault_scale: i32,
    pub total_value_outstanding: Amount,
    pub management_fee_outstanding: Amount,
    pub broker_debt_total: Amount,
    pub cover_rate_minimum: u32,
    pub cover_rate_liquidation: u32,
    pub cover_available: Amount,
    pub vault_total_assets: Amount,
    pub vault_available_assets: Amount,
    pub vault_loss_unrealized: Amount,
    pub loan_is_impaired: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoanManageDefaultPlan<Amount> {
    pub total_default_amount: Amount,
    pub minimum_cover: Amount,
    pub liquidation_cover: Amount,
    pub liquidation_cover_capped: Amount,
    pub covered_before_cover_available: Amount,
    pub default_covered: Amount,
    pub vault_default_amount: Amount,
    pub vault_default_rounded: Amount,
    pub cover_available_after: Amount,
    pub broker_debt_after: Amount,
    pub vault_total_after: Amount,
    pub vault_available_after: Amount,
    pub dust_reconciled: bool,
    pub vault_loss_unrealized_after: Option<Amount>,
}

#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoanManageDefaultError {
    VaultTotalShortfall,
    VaultAvailableExceedsTotal,
    VaultUnrealizedLossShortfall,
}

pub trait LoanManageDefaultMath {
    type Amount: Copy + Ord + Add<Output = Self::Amount> + Sub<Output = Self::Amount>;
    type Asset;

    fn tenth_bips_of_value(&mut self, value: Self::Amount, rate: u32) -> Self::Amount;

    fn round_to_asset(
        &mut self,
        asset: &Self::Asset,
        value: Self::Amount,
        scale: i32,
        mode: LoanManageDefaultRoundingMode,
    ) -> Self::Amount;

    fn asset_is_integral(&mut self, asset: &Self::Asset) -> bool;

    fn exponent(&mut self, value: Self::Amount) -> i32;

    fn adjust_imprecise_subtract(
        &mut self,
        asset: &Self::Asset,
        value: Self::Amount,
        decrement: Self::Amount,
        scale: i32,
    ) -> Self::Amount;
}

pub fn run_loan_manage_default<Math>(
    facts: LoanManageDefaultFacts<Math::Amount, Math::Asset>,
    math: &mut Math,
) -> Result<LoanManageDefaultPlan<Math::Amount>, LoanManageDefaultError>
where
    Math: LoanManageDefaultMath,
{
    let total_default_amount = facts.total_value_outstanding - facts.management_fee_outstanding;

    let minimum_cover = math.tenth_bips_of_value(facts.broker_debt_total, facts.cover_rate_minimum);
    let liquidation_cover = math.tenth_bips_of_value(minimum_cover, facts.cover_rate_liquidation);
    let liquidation_cover_capped = min(liquidation_cover, total_default_amount);
    let covered_before_cover_available = math.round_to_asset(
        &facts.asset,
        liquidation_cover_capped,
        facts.loan_scale,
        LoanManageDefaultRoundingMode::Upward,
    );
    let default_covered = min(covered_before_cover_available, facts.cover_available);
    let vault_default_amount = total_default_amount - default_covered;

    if facts.vault_total_assets < vault_default_amount {
        return Err(LoanManageDefaultError::VaultTotalShortfall);
    }

    let vault_default_rounded = math.round_to_asset(
        &facts.asset,
        vault_default_amount,
        facts.vault_scale,
        LoanManageDefaultRoundingMode::Downward,
    );
    let mut vault_total_after = facts.vault_total_assets - vault_default_rounded;
    let vault_available_after = facts.vault_available_assets + default_covered;
    let mut dust_reconciled = false;

    if vault_available_after > vault_total_after && !math.asset_is_integral(&facts.asset) {
        let difference = vault_available_after - vault_total_after;
        if math.exponent(vault_available_after) - math.exponent(difference) > 13 {
            vault_total_after = vault_available_after;
            dust_reconciled = true;
        }
    }

    if vault_available_after > vault_total_after {
        return Err(LoanManageDefaultError::VaultAvailableExceedsTotal);
    }

    let vault_loss_unrealized_after = if facts.loan_is_impaired {
        if facts.vault_loss_unrealized < total_default_amount {
            return Err(LoanManageDefaultError::VaultUnrealizedLossShortfall);
        }
        Some(math.adjust_imprecise_subtract(
            &facts.asset,
            facts.vault_loss_unrealized,
            total_default_amount,
            facts.vault_scale,
        ))
    } else {
        None
    };

    let broker_debt_after = math.adjust_imprecise_subtract(
        &facts.asset,
        facts.broker_debt_total,
        total_default_amount,
        facts.vault_scale,
    );

    Ok(LoanManageDefaultPlan {
        total_default_amount,
        minimum_cover,
        liquidation_cover,
        liquidation_cover_capped,
        covered_before_cover_available,
        default_covered,
        vault_default_amount,
        vault_default_rounded,
        cover_available_after: facts.cover_available - default_covered,
        broker_debt_after,
        vault_total_after,
        vault_available_after,
        dust_reconciled,
        vault_loss_unrealized_after,
    })
}
