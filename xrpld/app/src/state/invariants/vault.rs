use super::common::*;
use super::mpt::{mpt_id_from_issuance, mpt_max_amount};
use basics::{
    base_uint::Uint256,
    number::{NumberParts as RuntimeNumber, RoundingMode, get_mantissa_scale},
};
use ledger::{ApplyView, FlowSandbox, ReadView};
use protocol::{AccountID, Asset, Issue, LedgerEntryType, MPTID, STAmount, STLedgerEntry, Ter};
use std::collections::BTreeMap;

pub(super) struct VaultSnapshot {
    pub(super) key: Uint256,
    pub(super) asset: Asset,
    pub(super) pseudo_id: AccountID,
    pub(super) share_mpt_id: MPTID,
    pub(super) assets_total: RuntimeNumber,
    pub(super) assets_available: RuntimeNumber,
    pub(super) loss_unrealized: RuntimeNumber,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum V1_1VaultPhase {
    OpenEnded,
    Subscription,
    Investment,
    Redemption,
    Invalid,
}

fn v1_1_vault_phase(vault: &STLedgerEntry, now: u32) -> V1_1VaultPhase {
    let kind = vault
        .is_field_present(sf("sfVaultKind"))
        .then(|| vault.get_field_u8(sf("sfVaultKind")))
        .unwrap_or(0);
    match kind {
        0 => V1_1VaultPhase::OpenEnded,
        1 if !vault.is_field_present(sf("sfSubscriptionDate"))
            || !vault.is_field_present(sf("sfRedemptionDate")) =>
        {
            V1_1VaultPhase::Invalid
        }
        1 => {
            let subscription = vault.get_field_u32(sf("sfSubscriptionDate"));
            let redemption = vault.get_field_u32(sf("sfRedemptionDate"));
            let gap = i64::from(redemption) - i64::from(subscription);
            if !(180..946_708_560).contains(&gap) {
                return V1_1VaultPhase::Invalid;
            }
            if now <= subscription {
                V1_1VaultPhase::Subscription
            } else if now < redemption {
                V1_1VaultPhase::Investment
            } else {
                V1_1VaultPhase::Redemption
            }
        }
        _ => V1_1VaultPhase::Invalid,
    }
}

/// ValidVault's V1.1 phase rules. The helper is intentionally independent of
/// transaction preclaim so an invalid raw ApplyView mutation is rejected too.
fn valid_v1_1_vault_lifecycle(txn_type: protocol::TxType, vault: &STLedgerEntry, now: u32) -> bool {
    let phase = v1_1_vault_phase(vault, now);
    if phase == V1_1VaultPhase::Invalid {
        return false;
    }
    match txn_type {
        protocol::TxType::VAULT_DEPOSIT => {
            matches!(
                phase,
                V1_1VaultPhase::OpenEnded | V1_1VaultPhase::Subscription
            )
        }
        protocol::TxType::VAULT_WITHDRAW => phase != V1_1VaultPhase::Investment,
        protocol::TxType::LOAN_SET => {
            phase == V1_1VaultPhase::OpenEnded || phase == V1_1VaultPhase::Investment
        }
        _ => true,
    }
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

pub(super) fn vault_snapshot(sle: &STLedgerEntry) -> VaultSnapshot {
    let asset = sle.get_field_issue(sf("sfAsset")).asset();
    VaultSnapshot {
        key: *sle.key(),
        asset,
        pseudo_id: sle.get_account_id(sf("sfAccount")),
        share_mpt_id: sle.get_field_h192(sf("sfShareMPTID")),
        assets_total: sle.get_field_number(sf("sfAssetsTotal")).value(),
        assets_available: sle.get_field_number(sf("sfAssetsAvailable")).value(),
        loss_unrealized: sle.get_field_number(sf("sfLossUnrealized")).value(),
    }
}

pub(super) fn valid_vault_loss_unrealized(loss: RuntimeNumber, fix_cleanup_3_4_0: bool) -> bool {
    !fix_cleanup_3_4_0 || loss >= RuntimeNumber::zero()
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

pub(super) fn find_vault_share(
    shares: &[VaultSharesSnapshot],
    share_mpt_id: MPTID,
) -> Option<&VaultSharesSnapshot> {
    shares
        .iter()
        .find(|candidate| candidate.share_mpt_id == share_mpt_id)
}

#[derive(Default)]
pub(super) struct VaultInvariantReads {
    updated_shares: Option<VaultSharesSnapshot>,
    current_vault: Option<std::sync::Arc<STLedgerEntry>>,
    pseudo_account: Option<std::sync::Arc<STLedgerEntry>>,
}

/// Resolve every ledger object that `validates_vault_state` consults before
/// entering its boolean protocol checks. This keeps canonical absence as an
/// invariant failure while preserving a storage fault as `tefBAD_LEDGER` at
/// the shared invariant boundary.
pub(super) fn validate_vault_read_channel<V: ApplyView + ?Sized>(
    sandbox: &FlowSandbox<V>,
    txn_type: protocol::TxType,
    state: &VaultState,
) -> Result<VaultInvariantReads, ledger::ViewError> {
    let Some(after_vault) = state.after_vaults.first() else {
        return Ok(VaultInvariantReads::default());
    };
    let updated_shares =
        if let Some(shares) = find_vault_share(&state.after_shares, after_vault.share_mpt_id) {
            Some(shares.clone())
        } else {
            sandbox
                .read(protocol::mpt_issuance_keylet_from_mptid(
                    after_vault.share_mpt_id,
                ))?
                .map(|sle| vault_shares_snapshot(&sle))
        };
    let current_vault = sandbox.read(protocol::vault_keylet_from_key(after_vault.key))?;
    let pseudo_account = if txn_type == protocol::TxType::VAULT_CREATE
        && let Some(shares) = updated_shares.as_ref()
    {
        sandbox.read(protocol::account_keylet(raw_account_id(shares.issuer)))?
    } else {
        None
    };
    Ok(VaultInvariantReads {
        updated_shares,
        current_vault,
        pseudo_account,
    })
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

pub(super) fn round_number_to_asset_with_scale(
    asset: Asset,
    value: RuntimeNumber,
    scale: i32,
    rounding: RoundingMode,
) -> RuntimeNumber {
    ledger::vault_helpers::round_number_to_asset_with_scale(asset, value, scale, rounding)
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
    fix_cleanup_3_4_0: bool,
) -> i32 {
    if fix_cleanup_3_4_0 {
        // The fixCleanup3_4_0 clamp uses the updated AssetsTotal grid. Vault
        // sfScale governs share conversion and must not participate here.
        return number_scale(after.asset, after.assets_total);
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

fn one_unit_at_scale(scale: i32) -> RuntimeNumber {
    RuntimeNumber::try_from_external_parts(1, scale, get_mantissa_scale())
        .unwrap_or_else(|_| RuntimeNumber::zero())
}

/// fixCleanup3_4_0 tolerates exactly one IOU storage unit of independent
/// quantization noise. XRP and MPT remain exact, and legacy ledgers preserve
/// strict equality.
pub(super) fn agrees_within_one_unit(
    lhs: RuntimeNumber,
    rhs: RuntimeNumber,
    asset: Asset,
    scale: i32,
    fix_cleanup_3_4_0: bool,
) -> bool {
    if !fix_cleanup_3_4_0 || asset.integral() {
        return lhs == rhs;
    }
    let difference = lhs - rhs;
    let magnitude = if difference < RuntimeNumber::zero() {
        -difference
    } else {
        difference
    };
    magnitude <= one_unit_at_scale(scale)
}

pub(super) fn less_or_equal_plus_one_unit(
    lhs: RuntimeNumber,
    rhs: RuntimeNumber,
    asset: Asset,
    scale: i32,
    fix_cleanup_3_4_0: bool,
) -> bool {
    if !fix_cleanup_3_4_0 || asset.integral() {
        lhs <= rhs
    } else {
        lhs <= rhs + one_unit_at_scale(scale)
    }
}
pub(super) fn rounded_vault_delta(
    asset: Asset,
    delta: VaultAssetDelta,
    scale: i32,
) -> RuntimeNumber {
    round_number_to_asset_with_scale(asset, delta.delta, scale, RoundingMode::ToNearest)
}

pub(super) fn vault_transaction_account_asset_delta(
    state: &VaultState,
    account: AccountID,
    asset: Asset,
    account_paid_fee: bool,
    fee: protocol::XRPAmount,
) -> Option<VaultAssetDelta> {
    let mut delta = vault_asset_delta(state, account, asset)?;
    // Pinned ValidVault::deltaAssetsTxAccount adds the fee back only when
    // sfAccount is the effective fee payer. The invariant sees the complete
    // outer XRP delta, while delegated/sponsored fees do not touch sfAccount.
    if asset.native() && account_paid_fee {
        delta.delta += RuntimeNumber::from_i64(fee.drops());
        if delta.delta == RuntimeNumber::zero() {
            return None;
        }
    }
    Some(delta)
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

pub(super) fn validates_vault_state<V: ApplyView + ?Sized>(
    sandbox: &FlowSandbox<V>,
    txn_type: protocol::TxType,
    tx_account: Option<AccountID>,
    tx_destination: Option<AccountID>,
    tx_holder: Option<AccountID>,
    tx_amount: Option<&STAmount>,
    tx_account_paid_fee: bool,
    fee: protocol::XRPAmount,
    fix_cleanup_3_4_0: bool,
    result: Ter,
    state: &VaultState,
    reads: &VaultInvariantReads,
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
        .or_else(|| reads.updated_shares.clone());
    let Some(updated_shares) = updated_shares else {
        return false;
    };

    if sandbox
        .rules()
        .enabled(&protocol::feature_id("LendingProtocolV1_1"))
    {
        let Some(current_vault) = reads.current_vault.as_deref() else {
            return false;
        };
        if !valid_v1_1_vault_lifecycle(txn_type, current_vault, sandbox.header().parent_close_time)
        {
            return false;
        }
    }

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

    // Mirrors rippled ValidVault::finalize's universal persisted-value checks.
    // STNumber's asset association is transient serialization metadata and is
    // therefore deliberately not part of invariant validation.
    let unavailable = after_vault.assets_total - after_vault.assets_available;
    let assets_maximum = reads
        .current_vault
        .as_ref()
        .map(|vault| vault.get_field_number(sf("sfAssetsMaximum")).value())
        .unwrap_or(zero);
    if after_vault.assets_available < zero
        || after_vault.assets_available > after_vault.assets_total
        || !less_or_equal_plus_one_unit(
            after_vault.loss_unrealized,
            unavailable,
            after_vault.asset,
            number_scale(after_vault.asset, after_vault.assets_total),
            fix_cleanup_3_4_0,
        )
        || !valid_vault_loss_unrealized(
            after_vault.loss_unrealized,
            sandbox.rules().enabled(&protocol::fix_cleanup_3_4_0()),
        )
        || after_vault.assets_total < zero
        || assets_maximum < zero
    {
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

            let Some(pseudo_account) = reads.pseudo_account.as_ref() else {
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
                sandbox.rules().enabled(&protocol::fix_cleanup_3_4_0()),
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
                && agrees_within_one_unit(
                    vault_delta_total,
                    vault_delta_assets,
                    after_vault.asset,
                    min_scale,
                    fix_cleanup_3_4_0,
                )
                && agrees_within_one_unit(
                    vault_delta_available,
                    vault_delta_assets,
                    after_vault.asset,
                    min_scale,
                    fix_cleanup_3_4_0,
                )
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
                    vault_transaction_account_asset_delta(
                        state,
                        account,
                        after_vault.asset,
                        tx_account_paid_fee,
                        fee,
                    )
                    .is_some_and(|delta| {
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
                            && agrees_within_one_unit(
                                -account_delta,
                                local_vault_delta,
                                after_vault.asset,
                                local_scale,
                                fix_cleanup_3_4_0,
                            )
                    })
                })
        }),
        protocol::TxType::VAULT_WITHDRAW => before_vault.is_some_and(|before| {
            let fix_cleanup_3_4_0 = sandbox.rules().enabled(&protocol::fix_cleanup_3_4_0());
            let maybe_pseudo_delta =
                vault_asset_delta(state, after_vault.pseudo_id, after_vault.asset);
            // A fully impaired vault can burn shares for a genuine zero-asset
            // self-withdrawal.  There is then no asset-holding mutation on
            // either side (and post-cleanup must not manufacture an empty
            // IOU/MPT holding merely to create one).  This is not a rounding
            // tolerance: it is permitted only when no effective value backs
            // the shares, and only after fixCleanup3_4_0.
            let zero_value_withdrawal = fix_cleanup_3_4_0
                && maybe_pseudo_delta.is_none()
                && before.assets_total == before.loss_unrealized;
            let pseudo_delta_assets = maybe_pseudo_delta.unwrap_or(VaultAssetDelta {
                delta: RuntimeNumber::zero(),
                scale: None,
            });
            let min_scale = compute_vault_min_scale(
                before,
                after_vault,
                pseudo_delta_assets,
                fix_cleanup_3_4_0,
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
            let destination_valid = if zero_value_withdrawal {
                // Existing zero-balance holdings are untouched, and missing
                // holdings must remain missing.  Either way no recipient
                // asset delta may exist for a genuine zero payout.
                let destination_delta = destination.and_then(|destination| {
                    if Some(destination) == tx_account {
                        vault_transaction_account_asset_delta(
                            state,
                            destination,
                            after_vault.asset,
                            tx_account_paid_fee,
                            fee,
                        )
                    } else {
                        vault_asset_delta(state, destination, after_vault.asset)
                    }
                });
                destination_delta.is_none()
            } else if issuer_withdrawal {
                true
            } else if let Some(destination) = destination {
                let destination_delta = if Some(destination) == tx_account {
                    vault_transaction_account_asset_delta(
                        state,
                        destination,
                        after_vault.asset,
                        tx_account_paid_fee,
                        fee,
                    )
                } else {
                    vault_asset_delta(state, destination, after_vault.asset)
                };
                destination_delta.is_some_and(|delta| {
                    let destination_scale = delta.scale.unwrap_or(0);
                    let local_scale = min_scale.max(destination_scale);
                    let rounded_destination =
                        rounded_vault_delta(after_vault.asset, delta, local_scale);
                    let tolerate_zero_delta = fix_cleanup_3_4_0 && !after_vault.asset.integral();
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
                        && (destroyed_is_sub_ulp
                            || agrees_within_one_unit(
                                -local_pseudo_delta,
                                rounded_destination,
                                after_vault.asset,
                                local_scale,
                                fix_cleanup_3_4_0,
                            ))
                })
            } else {
                false
            };
            after_vault.assets_total <= before.assets_total
                && after_vault.assets_available <= before.assets_available
                && agrees_within_one_unit(
                    vault_delta_total,
                    vault_delta_assets,
                    after_vault.asset,
                    min_scale,
                    fix_cleanup_3_4_0,
                )
                && agrees_within_one_unit(
                    vault_delta_available,
                    vault_delta_assets,
                    after_vault.asset,
                    min_scale,
                    fix_cleanup_3_4_0,
                )
                && (zero_value_withdrawal || vault_delta_assets < RuntimeNumber::zero())
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
                    fix_cleanup_3_4_0,
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
                    && agrees_within_one_unit(
                        vault_delta_assets,
                        vault_delta_total,
                        after_vault.asset,
                        min_scale,
                        fix_cleanup_3_4_0,
                    )
                    && agrees_within_one_unit(
                        vault_delta_assets,
                        vault_delta_available,
                        after_vault.asset,
                        min_scale,
                        fix_cleanup_3_4_0,
                    )
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

#[cfg(test)]
mod tests {
    use super::{V1_1VaultPhase, v1_1_vault_phase, valid_v1_1_vault_lifecycle};
    use basics::base_uint::Uint256;
    use protocol::{LedgerEntryType, STLedgerEntry, TxType, get_field_by_symbol};

    fn sf(name: &str) -> &'static protocol::SField {
        get_field_by_symbol(name)
    }

    fn closed(subscription: u32, redemption: u32) -> STLedgerEntry {
        let mut vault =
            STLedgerEntry::from_type_and_key(LedgerEntryType::Vault, Uint256::from_u64(1));
        vault.set_field_u8(sf("sfVaultKind"), 1);
        vault.set_field_u32(sf("sfSubscriptionDate"), subscription);
        vault.set_field_u32(sf("sfRedemptionDate"), redemption);
        vault
    }

    #[test]
    fn v1_1_vault_phase_has_exact_subscription_and_redemption_boundaries() {
        let vault = closed(100, 280);
        assert_eq!(v1_1_vault_phase(&vault, 99), V1_1VaultPhase::Subscription);
        assert_eq!(v1_1_vault_phase(&vault, 100), V1_1VaultPhase::Subscription);
        assert_eq!(v1_1_vault_phase(&vault, 101), V1_1VaultPhase::Investment);
        assert_eq!(v1_1_vault_phase(&vault, 279), V1_1VaultPhase::Investment);
        assert_eq!(v1_1_vault_phase(&vault, 280), V1_1VaultPhase::Redemption);
    }

    #[test]
    fn v1_1_vault_lifecycle_enforces_180_second_schedule_and_phase_permissions() {
        assert_eq!(
            v1_1_vault_phase(&closed(100, 279), 100),
            V1_1VaultPhase::Invalid
        );
        let vault = closed(100, 280);
        assert!(valid_v1_1_vault_lifecycle(
            TxType::VAULT_DEPOSIT,
            &vault,
            100
        ));
        assert!(!valid_v1_1_vault_lifecycle(
            TxType::VAULT_DEPOSIT,
            &vault,
            101
        ));
        assert!(!valid_v1_1_vault_lifecycle(
            TxType::VAULT_WITHDRAW,
            &vault,
            101
        ));
        assert!(valid_v1_1_vault_lifecycle(
            TxType::VAULT_WITHDRAW,
            &vault,
            280
        ));
        assert!(!valid_v1_1_vault_lifecycle(TxType::LOAN_SET, &vault, 100));
        assert!(valid_v1_1_vault_lifecycle(TxType::LOAN_SET, &vault, 101));
        assert!(!valid_v1_1_vault_lifecycle(TxType::LOAN_SET, &vault, 280));
    }
}
