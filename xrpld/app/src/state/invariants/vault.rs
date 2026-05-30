use super::common::*;
use super::mpt::{mpt_id_from_issuance, mpt_max_amount};
use basics::{
    base_uint::Uint256,
    number::{NumberParts as RuntimeNumber, RoundingMode, get_mantissa_scale},
};
use ledger::{ApplyView, FlowSandbox, ReadView};
use protocol::{
    AccountID, Asset, Issue, LedgerEntryType, MPTID, STAmount, STLedgerEntry, STNumber, Ter,
    XRPAmount,
};
use std::collections::BTreeMap;

pub(super) struct VaultSnapshot {
    pub(super) key: Uint256,
    pub(super) asset: Asset,
    pub(super) pseudo_id: AccountID,
    pub(super) share_mpt_id: MPTID,
    pub(super) scale: Option<i32>,
    pub(super) assets_total: RuntimeNumber,
    pub(super) assets_available: RuntimeNumber,
    pub(super) loss_unrealized: RuntimeNumber,
}

#[derive(Clone)]
pub(super) struct VaultSharesSnapshot {
    share_mpt_id: MPTID,
    issuer: AccountID,
    shares_total: u64,
    shares_maximum: u64,
}

#[derive(Clone, Copy)]
pub(super) struct VaultAssetDelta {
    pub(super) delta: RuntimeNumber,
    pub(super) scale: Option<i32>,
}

#[derive(Default)]
pub(super) struct VaultState {
    before_vaults: Vec<VaultSnapshot>,
    after_vaults: Vec<VaultSnapshot>,
    before_shares: Vec<VaultSharesSnapshot>,
    after_shares: Vec<VaultSharesSnapshot>,
    share_issuance_delta: BTreeMap<MPTID, i128>,
    share_holder_delta: BTreeMap<MPTID, BTreeMap<AccountID, i128>>,
    asset_delta: BTreeMap<(AccountID, Asset), VaultAssetDelta>,
}

pub(super) fn validate_vault_entry(sle: &STLedgerEntry) -> bool {
    let asset = sle.get_field_issue(sf("sfAsset")).asset();
    let assets_total = sle.get_field_number(sf("sfAssetsTotal"));
    let assets_available = sle.get_field_number(sf("sfAssetsAvailable"));
    let loss_unrealized = sle.get_field_number(sf("sfLossUnrealized"));
    let assets_maximum = sle
        .is_field_present(sf("sfAssetsMaximum"))
        .then(|| sle.get_field_number(sf("sfAssetsMaximum")));
    let unavailable = assets_total.value() - assets_available.value();
    let zero = basics::number::NumberParts::zero();

    assets_total.associated_asset() == Some(asset)
        && assets_available.associated_asset() == Some(asset)
        && loss_unrealized.associated_asset() == Some(asset)
        && assets_total.value() >= zero
        && assets_available.value() >= zero
        && loss_unrealized.value() >= zero
        && assets_maximum.is_none_or(|value| {
            value.associated_asset() == Some(asset)
                && value.value() >= zero
                && (value.value() == zero || assets_total.value() <= value.value())
        })
        && assets_available.value() <= assets_total.value()
        && loss_unrealized.value() <= unavailable
}

pub(super) fn vault_snapshot(sle: &STLedgerEntry) -> VaultSnapshot {
    let asset = sle.get_field_issue(sf("sfAsset")).asset();
    VaultSnapshot {
        key: *sle.key(),
        asset,
        pseudo_id: sle.get_account_id(sf("sfAccount")),
        share_mpt_id: sle.get_field_h192(sf("sfShareMPTID")),
        scale: sle
            .is_field_present(sf("sfScale"))
            .then(|| -(sle.get_field_u8(sf("sfScale")) as i32)),
        assets_total: sle.get_field_number(sf("sfAssetsTotal")).value(),
        assets_available: sle.get_field_number(sf("sfAssetsAvailable")).value(),
        loss_unrealized: sle.get_field_number(sf("sfLossUnrealized")).value(),
    }
}

pub(super) fn vault_shares_snapshot(sle: &STLedgerEntry) -> VaultSharesSnapshot {
    VaultSharesSnapshot {
        share_mpt_id: mpt_id_from_issuance(sle),
        issuer: sle.get_account_id(sf("sfIssuer")),
        shares_total: optional_u64(sle, sf("sfOutstandingAmount")),
        shares_maximum: mpt_max_amount(sle),
    }
}

pub(super) fn add_vault_asset_delta(
    state: &mut VaultState,
    account: AccountID,
    asset: Asset,
    delta: RuntimeNumber,
    scale: Option<i32>,
) {
    if delta == RuntimeNumber::zero() {
        return;
    }

    let entry = state
        .asset_delta
        .entry((account, asset))
        .or_insert(VaultAssetDelta {
            delta: RuntimeNumber::zero(),
            scale: None,
        });
    entry.delta += delta;
    if let Some(scale) = scale {
        entry.scale = Some(entry.scale.map_or(scale, |current| current.max(scale)));
    }
}

pub(super) fn signed_delta(value: RuntimeNumber, before: bool) -> RuntimeNumber {
    if before { -value } else { value }
}

pub(super) fn record_vault_asset_delta(state: &mut VaultState, sle: &STLedgerEntry, before: bool) {
    match sle.get_type() {
        LedgerEntryType::AccountRoot => {
            if !sle.is_field_present(sf("sfAccount")) || !sle.is_field_present(sf("sfBalance")) {
                return;
            }
            add_vault_asset_delta(
                state,
                sle.get_account_id(sf("sfAccount")),
                Asset::Issue(protocol::xrp_issue()),
                signed_delta(
                    amount_to_number(&sle.get_field_amount(sf("sfBalance"))),
                    before,
                ),
                None,
            );
        }
        LedgerEntryType::RippleState => {
            if !sle.is_field_present(sf("sfBalance"))
                || !sle.is_field_present(sf("sfLowLimit"))
                || !sle.is_field_present(sf("sfHighLimit"))
            {
                return;
            }

            let low = sle.get_field_amount(sf("sfLowLimit")).issue().account;
            let high = sle.get_field_amount(sf("sfHighLimit")).issue().account;
            let currency = sle.get_field_amount(sf("sfLowLimit")).issue().currency;
            let balance = sle.get_field_amount(sf("sfBalance"));
            let delta = signed_delta(amount_to_number(&balance), before);
            let scale = Some(balance.exponent());

            add_vault_asset_delta(
                state,
                low,
                Asset::Issue(Issue {
                    currency,
                    account: high,
                }),
                delta,
                scale,
            );
            add_vault_asset_delta(
                state,
                high,
                Asset::Issue(Issue {
                    currency,
                    account: low,
                }),
                -delta,
                scale,
            );
        }
        LedgerEntryType::MPToken => {
            if !sle.is_field_present(sf("sfAccount"))
                || !sle.is_field_present(sf("sfMPTokenIssuanceID"))
                || !sle.is_field_present(sf("sfMPTAmount"))
            {
                return;
            }
            add_vault_asset_delta(
                state,
                sle.get_account_id(sf("sfAccount")),
                Asset::MPTIssue(protocol::MPTIssue::new(
                    sle.get_field_h192(sf("sfMPTokenIssuanceID")),
                )),
                signed_delta(
                    RuntimeNumber::from_i64(sle.get_field_u64(sf("sfMPTAmount")) as i64),
                    before,
                ),
                None,
            );
        }
        _ => {}
    }
}

pub(super) fn record_vault_state(
    state: &mut VaultState,
    is_delete: bool,
    before: Option<&STLedgerEntry>,
    after: Option<&STLedgerEntry>,
) {
    if let Some(before) = before {
        record_vault_asset_delta(state, before, true);
        match before.get_type() {
            LedgerEntryType::Vault => state.before_vaults.push(vault_snapshot(before)),
            LedgerEntryType::MPTokenIssuance => {
                let shares = vault_shares_snapshot(before);
                *state
                    .share_issuance_delta
                    .entry(shares.share_mpt_id)
                    .or_default() += i128::from(shares.shares_total);
                state.before_shares.push(shares);
            }
            LedgerEntryType::MPToken => {
                let id = before.get_field_h192(sf("sfMPTokenIssuanceID"));
                let account = before.get_account_id(sf("sfAccount"));
                let amount = optional_u64(before, sf("sfMPTAmount"));
                *state
                    .share_holder_delta
                    .entry(id)
                    .or_default()
                    .entry(account)
                    .or_default() -= i128::from(amount);
            }
            _ => {}
        }
    }

    if is_delete {
        return;
    }

    if let Some(after) = after {
        record_vault_asset_delta(state, after, false);
        match after.get_type() {
            LedgerEntryType::Vault => state.after_vaults.push(vault_snapshot(after)),
            LedgerEntryType::MPTokenIssuance => {
                let shares = vault_shares_snapshot(after);
                *state
                    .share_issuance_delta
                    .entry(shares.share_mpt_id)
                    .or_default() -= i128::from(shares.shares_total);
                state.after_shares.push(shares);
            }
            LedgerEntryType::MPToken => {
                let id = after.get_field_h192(sf("sfMPTokenIssuanceID"));
                let account = after.get_account_id(sf("sfAccount"));
                let amount = optional_u64(after, sf("sfMPTAmount"));
                *state
                    .share_holder_delta
                    .entry(id)
                    .or_default()
                    .entry(account)
                    .or_default() += i128::from(amount);
            }
            _ => {}
        }
    }
}

pub(super) fn vault_must_modify(txn_type: protocol::TxType) -> bool {
    matches!(
        txn_type,
        protocol::TxType::VAULT_CREATE
            | protocol::TxType::VAULT_SET
            | protocol::TxType::VAULT_DEPOSIT
            | protocol::TxType::VAULT_WITHDRAW
            | protocol::TxType::VAULT_DELETE
            | protocol::TxType::VAULT_CLAWBACK
            | protocol::TxType::LOAN_SET
            | protocol::TxType::LOAN_PAY
    )
}

pub(super) fn vault_may_modify(txn_type: protocol::TxType) -> bool {
    txn_type == protocol::TxType::LOAN_MANAGE
}

pub(super) fn find_vault_share<'a>(
    shares: &'a [VaultSharesSnapshot],
    share_mpt_id: MPTID,
) -> Option<&'a VaultSharesSnapshot> {
    shares
        .iter()
        .find(|candidate| candidate.share_mpt_id == share_mpt_id)
}

pub(super) fn read_vault_share<V: ApplyView>(
    sandbox: &FlowSandbox<V>,
    share_mpt_id: MPTID,
) -> Option<VaultSharesSnapshot> {
    sandbox
        .read(protocol::mpt_issuance_keylet_from_mptid(share_mpt_id))
        .ok()
        .flatten()
        .map(|sle| vault_shares_snapshot(&sle))
}

pub(super) fn vault_share_issuance_delta(state: &VaultState, share_mpt_id: MPTID) -> i128 {
    state
        .share_issuance_delta
        .get(&share_mpt_id)
        .copied()
        .unwrap_or_default()
}

pub(super) fn vault_share_issuance_delta_if_updated(
    state: &VaultState,
    share_mpt_id: MPTID,
) -> Option<i128> {
    state.share_issuance_delta.get(&share_mpt_id).copied()
}

pub(super) fn vault_share_holder_delta(
    state: &VaultState,
    share_mpt_id: MPTID,
    account: AccountID,
) -> Option<i128> {
    state
        .share_holder_delta
        .get(&share_mpt_id)
        .and_then(|holders| holders.get(&account))
        .copied()
}

pub(super) fn valid_vault_share_delta(
    state: &VaultState,
    share_mpt_id: MPTID,
    account: AccountID,
    holder_delta_valid: impl FnOnce(i128) -> bool,
) -> bool {
    let Some(holder_delta) = vault_share_holder_delta(state, share_mpt_id, account) else {
        return false;
    };
    if !holder_delta_valid(holder_delta) {
        return false;
    }

    let Some(vault_delta) = vault_share_issuance_delta_if_updated(state, share_mpt_id) else {
        return false;
    };
    vault_delta != 0 && vault_delta.saturating_neg() == holder_delta
}

pub(super) fn asset_issuer(asset: Asset) -> AccountID {
    match asset {
        Asset::Issue(issue) => issue.account,
        Asset::MPTIssue(issue) => issue.issuer(),
    }
}

pub(super) fn round_runtime_to_scale(
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

    let first = removed.first().copied().unwrap_or(0);
    let has_more = removed.iter().skip(1).any(|digit| *digit != 0);
    let round_up = match rounding {
        RoundingMode::TowardsZero => false,
        RoundingMode::Downward => negative && (first != 0 || has_more),
        RoundingMode::Upward => !negative && (first != 0 || has_more),
        RoundingMode::ToNearest => {
            first > 5 || (first == 5 && (has_more || ((abs as u64) & 1) == 1))
        }
    };
    if round_up {
        abs += 1;
    }

    let signed = if negative { -(abs as i64) } else { abs as i64 };
    RuntimeNumber::try_from_external_parts(signed, exponent, get_mantissa_scale()).unwrap_or(value)
}

pub(super) fn round_number_to_asset(asset: Asset, value: RuntimeNumber) -> RuntimeNumber {
    let mut number = STNumber::from(value);
    number.associate_asset(asset);
    number.value()
}

pub(super) fn round_number_to_asset_with_scale(
    asset: Asset,
    value: RuntimeNumber,
    scale: i32,
    rounding: RoundingMode,
) -> RuntimeNumber {
    let rounded_to_asset = round_number_to_asset(asset, value);
    if asset.integral() {
        return rounded_to_asset;
    }
    round_runtime_to_scale(rounded_to_asset, scale, rounding)
}

pub(super) fn number_scale(asset: Asset, value: RuntimeNumber) -> i32 {
    if asset.integral() {
        0
    } else {
        asset
            .amount(value)
            .map(|amount| amount.exponent())
            .unwrap_or(0)
    }
}

pub(super) fn vault_delta_scale(before: RuntimeNumber, after: RuntimeNumber, asset: Asset) -> i32 {
    number_scale(asset, before).max(number_scale(asset, after))
}

pub(super) fn compute_vault_min_scale(
    before: &VaultSnapshot,
    after: &VaultSnapshot,
    vault_delta: VaultAssetDelta,
    fix_cleanup_3_2_0: bool,
) -> i32 {
    if fix_cleanup_3_2_0 {
        return after
            .scale
            .unwrap_or_else(|| number_scale(after.asset, after.assets_total));
    }

    let total_scale = vault_delta_scale(before.assets_total, after.assets_total, after.asset);
    let available_scale =
        vault_delta_scale(before.assets_available, after.assets_available, after.asset);
    vault_delta
        .scale
        .unwrap_or(0)
        .max(total_scale)
        .max(available_scale)
}

pub(super) fn rounded_vault_delta(
    asset: Asset,
    delta: VaultAssetDelta,
    scale: i32,
) -> RuntimeNumber {
    round_number_to_asset_with_scale(asset, delta.delta, scale, RoundingMode::ToNearest)
}

pub(super) fn vault_account_asset_delta(
    state: &VaultState,
    account: AccountID,
    asset: Asset,
    fee: XRPAmount,
) -> Option<VaultAssetDelta> {
    let mut delta = state.asset_delta.get(&(account, asset)).copied()?;
    if asset.native() {
        delta.delta += RuntimeNumber::from(fee);
    }
    if delta.delta == RuntimeNumber::zero() {
        None
    } else {
        Some(delta)
    }
}

pub(super) fn vault_asset_delta(
    state: &VaultState,
    account: AccountID,
    asset: Asset,
) -> Option<VaultAssetDelta> {
    state
        .asset_delta
        .get(&(account, asset))
        .copied()
        .filter(|delta| delta.delta != RuntimeNumber::zero())
}

pub(super) fn validates_vault_state<V: ApplyView>(
    sandbox: &FlowSandbox<V>,
    txn_type: protocol::TxType,
    tx_account: Option<AccountID>,
    tx_destination: Option<AccountID>,
    tx_holder: Option<AccountID>,
    tx_amount: Option<&STAmount>,
    fix_cleanup_3_2_0: bool,
    result: Ter,
    fee: XRPAmount,
    state: &VaultState,
) -> bool {
    if !protocol::is_tes_success(result) {
        return true;
    }

    if state.before_vaults.is_empty() && state.after_vaults.is_empty() {
        return !vault_must_modify(txn_type);
    }

    if !(vault_must_modify(txn_type) || vault_may_modify(txn_type)) {
        return false;
    }

    if state.before_vaults.len() > 1 || state.after_vaults.len() > 1 {
        return false;
    }

    let zero = RuntimeNumber::zero();
    let before_vault = state.before_vaults.first();

    if state.after_vaults.is_empty() {
        if txn_type != protocol::TxType::VAULT_DELETE {
            return false;
        }
        let Some(before_vault) = before_vault else {
            return false;
        };
        let Some(deleted_shares) =
            find_vault_share(&state.before_shares, before_vault.share_mpt_id)
        else {
            return false;
        };
        return deleted_shares.shares_total == 0
            && before_vault.assets_total == zero
            && before_vault.assets_available == zero;
    }

    if txn_type == protocol::TxType::VAULT_DELETE {
        return false;
    }

    let after_vault = &state.after_vaults[0];
    if before_vault.is_some_and(|before| before.key != after_vault.key) {
        return false;
    }

    let updated_shares = find_vault_share(&state.after_shares, after_vault.share_mpt_id)
        .cloned()
        .or_else(|| read_vault_share(sandbox, after_vault.share_mpt_id));
    let Some(updated_shares) = updated_shares else {
        return false;
    };

    if let Some(before) = before_vault {
        if after_vault.asset != before.asset
            || after_vault.pseudo_id != before.pseudo_id
            || after_vault.share_mpt_id != before.share_mpt_id
        {
            return false;
        }
    }

    if updated_shares.shares_total == 0 {
        if after_vault.assets_total != zero || after_vault.assets_available != zero {
            return false;
        }
    } else if updated_shares.shares_total > updated_shares.shares_maximum {
        return false;
    }

    if before_vault.is_none() && txn_type != protocol::TxType::VAULT_CREATE {
        return false;
    }

    if let Some(before) = before_vault
        && after_vault.loss_unrealized != before.loss_unrealized
        && !matches!(
            txn_type,
            protocol::TxType::LOAN_MANAGE | protocol::TxType::LOAN_PAY
        )
    {
        return false;
    }

    if matches!(
        txn_type,
        protocol::TxType::VAULT_DEPOSIT
            | protocol::TxType::VAULT_WITHDRAW
            | protocol::TxType::VAULT_CLAWBACK
    ) && before_vault
        .is_some_and(|before| find_vault_share(&state.before_shares, before.share_mpt_id).is_none())
    {
        return false;
    }

    match txn_type {
        protocol::TxType::VAULT_CREATE => {
            if before_vault.is_some()
                || after_vault.assets_available != zero
                || after_vault.assets_total != zero
                || after_vault.loss_unrealized != zero
                || updated_shares.shares_total != 0
                || after_vault.pseudo_id != updated_shares.issuer
            {
                return false;
            }

            let Ok(Some(pseudo_account)) = sandbox.read(protocol::account_keylet(raw_account_id(
                updated_shares.issuer,
            ))) else {
                return false;
            };
            pseudo_account.is_field_present(sf("sfVaultID"))
                && pseudo_account.get_field_h256(sf("sfVaultID")) == after_vault.key
        }
        protocol::TxType::VAULT_SET => before_vault.is_some_and(|before| {
            before.assets_total == after_vault.assets_total
                && before.assets_available == after_vault.assets_available
                && vault_share_issuance_delta(state, after_vault.share_mpt_id) == 0
        }),
        protocol::TxType::VAULT_DEPOSIT => before_vault.is_some_and(|before| {
            let Some(pseudo_delta_assets) =
                vault_asset_delta(state, after_vault.pseudo_id, after_vault.asset)
            else {
                return false;
            };
            let min_scale = compute_vault_min_scale(
                before,
                after_vault,
                pseudo_delta_assets,
                fix_cleanup_3_2_0,
            );
            let vault_delta_assets =
                rounded_vault_delta(after_vault.asset, pseudo_delta_assets, min_scale);
            let vault_delta_total = round_number_to_asset_with_scale(
                after_vault.asset,
                after_vault.assets_total - before.assets_total,
                min_scale,
                RoundingMode::ToNearest,
            );
            let vault_delta_available = round_number_to_asset_with_scale(
                after_vault.asset,
                after_vault.assets_available - before.assets_available,
                min_scale,
                RoundingMode::ToNearest,
            );
            let tx_amount_valid = tx_amount.is_none_or(|amount| {
                vault_delta_assets
                    <= round_number_to_asset_with_scale(
                        after_vault.asset,
                        amount_to_number(amount),
                        min_scale,
                        RoundingMode::ToNearest,
                    )
            });
            after_vault.assets_total >= before.assets_total
                && after_vault.assets_available >= before.assets_available
                && tx_amount_valid
                && vault_delta_assets > RuntimeNumber::zero()
                && vault_delta_total == vault_delta_assets
                && vault_delta_available == vault_delta_assets
                && tx_account.is_some_and(|account| {
                    let shares_valid = valid_vault_share_delta(
                        state,
                        after_vault.share_mpt_id,
                        account,
                        |delta| delta > 0,
                    );
                    if !shares_valid {
                        return false;
                    }
                    if account == asset_issuer(after_vault.asset) {
                        return true;
                    }
                    vault_account_asset_delta(state, account, after_vault.asset, fee).is_some_and(
                        |delta| {
                            let local_scale = min_scale.max(delta.scale.unwrap_or(0));
                            let account_delta =
                                rounded_vault_delta(after_vault.asset, delta, local_scale);
                            let local_vault_delta = round_number_to_asset_with_scale(
                                after_vault.asset,
                                vault_delta_assets,
                                local_scale,
                                RoundingMode::ToNearest,
                            );
                            account_delta < RuntimeNumber::zero()
                                && -account_delta == local_vault_delta
                        },
                    )
                })
        }),
        protocol::TxType::VAULT_WITHDRAW => before_vault.is_some_and(|before| {
            let Some(pseudo_delta_assets) =
                vault_asset_delta(state, after_vault.pseudo_id, after_vault.asset)
            else {
                return false;
            };
            let min_scale = compute_vault_min_scale(
                before,
                after_vault,
                pseudo_delta_assets,
                fix_cleanup_3_2_0,
            );
            let vault_delta_assets =
                rounded_vault_delta(after_vault.asset, pseudo_delta_assets, min_scale);
            let vault_delta_total = round_number_to_asset_with_scale(
                after_vault.asset,
                after_vault.assets_total - before.assets_total,
                min_scale,
                RoundingMode::ToNearest,
            );
            let vault_delta_available = round_number_to_asset_with_scale(
                after_vault.asset,
                after_vault.assets_available - before.assets_available,
                min_scale,
                RoundingMode::ToNearest,
            );
            let destination = tx_destination.or(tx_account);
            let issuer_withdrawal =
                !after_vault.asset.native() && destination == Some(asset_issuer(after_vault.asset));
            let destination_valid = if issuer_withdrawal {
                true
            } else if let Some(destination) = destination {
                vault_asset_delta(state, destination, after_vault.asset).is_some_and(|delta| {
                    let destination_scale = delta.scale.unwrap_or(0);
                    let local_scale = min_scale.max(destination_scale);
                    let rounded_destination =
                        rounded_vault_delta(after_vault.asset, delta, local_scale);
                    let tolerate_zero_delta = fix_cleanup_3_2_0 && !after_vault.asset.integral();
                    let valid_balance_change = if tolerate_zero_delta {
                        rounded_destination >= RuntimeNumber::zero()
                    } else {
                        rounded_destination > RuntimeNumber::zero()
                    };
                    let local_pseudo_delta = round_number_to_asset_with_scale(
                        after_vault.asset,
                        vault_delta_assets,
                        local_scale,
                        RoundingMode::ToNearest,
                    );
                    let destroyed_is_sub_ulp = tolerate_zero_delta
                        && round_number_to_asset_with_scale(
                            after_vault.asset,
                            -pseudo_delta_assets.delta - delta.delta,
                            destination_scale,
                            RoundingMode::Downward,
                        ) == RuntimeNumber::zero();
                    valid_balance_change
                        && (destroyed_is_sub_ulp || -local_pseudo_delta == rounded_destination)
                })
            } else {
                false
            };
            after_vault.assets_total <= before.assets_total
                && after_vault.assets_available <= before.assets_available
                && vault_delta_total == vault_delta_assets
                && vault_delta_available == vault_delta_assets
                && vault_delta_assets < RuntimeNumber::zero()
                && destination_valid
                && tx_account.is_some_and(|account| {
                    valid_vault_share_delta(state, after_vault.share_mpt_id, account, |delta| {
                        delta < 0
                    })
                })
        }),
        protocol::TxType::VAULT_CLAWBACK => before_vault.is_some_and(|before| {
            let pseudo_delta_valid = if let Some(pseudo_delta_assets) =
                vault_asset_delta(state, after_vault.pseudo_id, after_vault.asset)
            {
                let min_scale = compute_vault_min_scale(
                    before,
                    after_vault,
                    pseudo_delta_assets,
                    fix_cleanup_3_2_0,
                );
                let vault_delta_assets =
                    rounded_vault_delta(after_vault.asset, pseudo_delta_assets, min_scale);
                let vault_delta_total = round_number_to_asset_with_scale(
                    after_vault.asset,
                    after_vault.assets_total - before.assets_total,
                    min_scale,
                    RoundingMode::ToNearest,
                );
                let vault_delta_available = round_number_to_asset_with_scale(
                    after_vault.asset,
                    after_vault.assets_available - before.assets_available,
                    min_scale,
                    RoundingMode::ToNearest,
                );
                vault_delta_assets < RuntimeNumber::zero()
                    && vault_delta_assets == vault_delta_total
                    && vault_delta_assets == vault_delta_available
            } else {
                before.assets_total == RuntimeNumber::zero()
                    && before.assets_available == RuntimeNumber::zero()
            };
            after_vault.assets_total <= before.assets_total
                && after_vault.assets_available <= before.assets_available
                && pseudo_delta_valid
                && tx_holder.is_some_and(|holder| {
                    valid_vault_share_delta(state, after_vault.share_mpt_id, holder, |delta| {
                        delta < 0
                    })
                })
        }),
        _ => true,
    }
}
