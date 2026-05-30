use std::collections::{BTreeMap, BTreeSet};

use basics::{
    base_uint::{Uint160, Uint256},
    number::{NumberParts as RuntimeNumber, RoundingMode, get_mantissa_scale, root2},
};
use ledger::{ApplyView, FlowSandbox, ReadView, flow_sandbox::Action};
use protocol::{
    AccountID, Asset, IOUAmount, Issue, LedgerEntryType, MPTAmount, MPTID, STAmount, STLedgerEntry,
    STNumber, STTx, Ter, XRPAmount, get_field_by_symbol,
};

#[derive(Default)]
struct MptAccounting {
    outstanding_before: i128,
    outstanding_after: i128,
    amount_delta: i128,
    overflow: bool,
}

#[derive(Default)]
struct MptTransferAmount {
    before: Option<u64>,
    after: Option<u64>,
    authorized_before: bool,
    authorized_after: bool,
    locked_before: bool,
    locked_after: bool,
}

#[derive(Default)]
struct MptIssuanceLifecycle {
    reference_holding_set_on_create: bool,
    reference_holding_mutated: bool,
    vault_holding_deleted: bool,
    issuances_created: u32,
    issuances_deleted: u32,
    tokens_created: u32,
    tokens_deleted: u32,
    token_created_by_issuer: bool,
}

#[derive(Default)]
struct PermissionedDexState {
    domains: BTreeSet<Uint256>,
    regular_offers_old: bool,
    regular_offers: bool,
    bad_hybrids_old: bool,
    bad_hybrids: bool,
}

struct PermissionedDomainStatus {
    credentials_size: usize,
    sorted: bool,
    unique: bool,
    deleted: bool,
}

#[derive(Default)]
struct PermissionedDomainState {
    statuses: Vec<PermissionedDomainStatus>,
}

#[derive(Default)]
struct AmmState {
    amm_after: bool,
    amm_account: Option<AccountID>,
    asset: Option<Asset>,
    asset2: Option<Asset>,
    amount: Option<STAmount>,
    amount2: Option<STAmount>,
    lpt_balance_before: Option<STAmount>,
    lpt_balance_after: Option<STAmount>,
    pool_changed: bool,
}

struct VaultSnapshot {
    key: Uint256,
    asset: Asset,
    pseudo_id: AccountID,
    share_mpt_id: MPTID,
    scale: Option<i32>,
    assets_total: RuntimeNumber,
    assets_available: RuntimeNumber,
    loss_unrealized: RuntimeNumber,
}

#[derive(Clone)]
struct VaultSharesSnapshot {
    share_mpt_id: MPTID,
    issuer: AccountID,
    shares_total: u64,
    shares_maximum: u64,
}

#[derive(Clone, Copy)]
struct VaultAssetDelta {
    delta: RuntimeNumber,
    scale: Option<i32>,
}

#[derive(Default)]
struct VaultState {
    before_vaults: Vec<VaultSnapshot>,
    after_vaults: Vec<VaultSnapshot>,
    before_shares: Vec<VaultSharesSnapshot>,
    after_shares: Vec<VaultSharesSnapshot>,
    share_issuance_delta: BTreeMap<MPTID, i128>,
    share_holder_delta: BTreeMap<MPTID, BTreeMap<AccountID, i128>>,
    asset_delta: BTreeMap<(AccountID, Asset), VaultAssetDelta>,
}

#[derive(Default)]
struct LendingState {
    broker_refs: BTreeSet<Uint256>,
}

#[derive(Default)]
struct ClawbackState {
    trustlines_changed: u32,
    mptokens_changed: u32,
}

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

fn raw_account_id(account: AccountID) -> Uint160 {
    Uint160::from_slice(account.data()).expect("account width")
}

fn optional_u64(sle: &STLedgerEntry, field: &'static protocol::SField) -> u64 {
    if sle.is_field_present(field) {
        sle.get_field_u64(field)
    } else {
        0
    }
}

fn max_mpt_token_amount() -> u64 {
    protocol::MAX_MP_TOKEN_AMOUNT as u64
}

fn mpt_id_from_issuance(sle: &STLedgerEntry) -> MPTID {
    protocol::make_mpt_id(
        sle.get_field_u32(sf("sfSequence")),
        sle.get_account_id(sf("sfIssuer")),
    )
}

fn mpt_max_amount(sle: &STLedgerEntry) -> u64 {
    if sle.is_field_present(sf("sfMaximumAmount")) {
        sle.get_field_u64(sf("sfMaximumAmount"))
    } else {
        max_mpt_token_amount()
    }
}

fn record_mpt_accounting(
    data: &mut BTreeMap<MPTID, MptAccounting>,
    sle: &STLedgerEntry,
    before: bool,
) {
    match sle.get_type() {
        LedgerEntryType::MPTokenIssuance => {
            let outstanding = optional_u64(sle, sf("sfOutstandingAmount"));
            let id = mpt_id_from_issuance(sle);
            let entry = data.entry(id).or_default();
            if outstanding > mpt_max_amount(sle) {
                entry.overflow = true;
            }
            if before {
                entry.outstanding_before = i128::from(outstanding);
            } else {
                entry.outstanding_after = i128::from(outstanding);
            }
        }
        LedgerEntryType::MPToken => {
            let mpt_amount = optional_u64(sle, sf("sfMPTAmount"));
            let locked_amount = optional_u64(sle, sf("sfLockedAmount"));
            let id = sle.get_field_h192(sf("sfMPTokenIssuanceID"));
            let entry = data.entry(id).or_default();
            let max_amount = max_mpt_token_amount();
            if mpt_amount > max_amount
                || locked_amount > max_amount
                || locked_amount > max_amount.saturating_sub(mpt_amount)
            {
                entry.overflow = true;
                return;
            }
            let holder_total = i128::from(mpt_amount) + i128::from(locked_amount);
            if before {
                entry.amount_delta -= holder_total;
            } else {
                entry.amount_delta += holder_total;
            }
        }
        _ => {}
    }
}

fn validates_mpt_accounting(data: &BTreeMap<MPTID, MptAccounting>, enforce: bool) -> bool {
    if !enforce {
        return true;
    }

    data.values().all(|entry| {
        !entry.overflow && entry.outstanding_after == entry.outstanding_before + entry.amount_delta
    })
}

fn record_mpt_transfer(
    transfers: &mut BTreeMap<MPTID, BTreeMap<AccountID, MptTransferAmount>>,
    sle: &STLedgerEntry,
    before: bool,
) {
    if sle.get_type() != LedgerEntryType::MPToken {
        return;
    }

    let id = sle.get_field_h192(sf("sfMPTokenIssuanceID"));
    let account = sle.get_account_id(sf("sfAccount"));
    let amount = optional_u64(sle, sf("sfMPTAmount"));
    let entry = transfers.entry(id).or_default().entry(account).or_default();
    if before {
        entry.before = Some(amount);
        entry.authorized_before = sle.is_flag(protocol::lsfMPTAuthorized);
        entry.locked_before = sle.is_flag(protocol::lsfMPTLocked);
    } else {
        entry.after = Some(amount);
        entry.authorized_after = sle.is_flag(protocol::lsfMPTAuthorized);
        entry.locked_after = sle.is_flag(protocol::lsfMPTLocked);
    }
}

fn mpt_transfer_waives_can_transfer(txn_type: protocol::TxType, fix_cleanup_3_2_0: bool) -> bool {
    txn_type == protocol::TxType::AMM_WITHDRAW
        || (fix_cleanup_3_2_0
            && matches!(
                txn_type,
                protocol::TxType::VAULT_WITHDRAW
                    | protocol::TxType::LOAN_BROKER_COVER_WITHDRAW
                    | protocol::TxType::LOAN_PAY
            ))
}

fn mpt_transfer_is_dex(txn_type: protocol::TxType, cross_currency_payment: bool) -> bool {
    if txn_type == protocol::TxType::PAYMENT {
        return cross_currency_payment;
    }

    matches!(
        txn_type,
        protocol::TxType::AMM_CREATE
            | protocol::TxType::AMM_DEPOSIT
            | protocol::TxType::OFFER_CREATE
    )
}

fn validates_mpt_transfers<V: ApplyView>(
    sandbox: &FlowSandbox<V>,
    txn_type: protocol::TxType,
    cross_currency_payment: bool,
    fix_cleanup_3_2_0: bool,
    mptokens_v2_enabled: bool,
    transfers: &BTreeMap<MPTID, BTreeMap<AccountID, MptTransferAmount>>,
) -> bool {
    if txn_type == protocol::TxType::AMM_CLAWBACK {
        return true;
    }

    for (mpt_id, holders) in transfers {
        let Ok(Some(issuance)) = sandbox.read(protocol::mpt_issuance_keylet_from_mptid(*mpt_id))
        else {
            continue;
        };

        let can_transfer = issuance.is_flag(protocol::lsfMPTCanTransfer)
            || mpt_transfer_waives_can_transfer(txn_type, fix_cleanup_3_2_0);
        let can_trade = issuance.is_flag(protocol::lsfMPTCanTrade);
        let req_auth = issuance.is_flag(protocol::lsfMPTRequireAuth);
        let issue = protocol::MPTIssue::new(*mpt_id);

        let mut senders = 0_u16;
        let mut receivers = 0_u16;
        let mut invalid_transfer = issuance.is_flag(protocol::lsfMPTLocked);
        for (account, value) in holders {
            let Some(after) = value.after else {
                continue;
            };
            let before = value.before.unwrap_or(0);
            if before == after {
                continue;
            }
            if after > before {
                receivers = receivers.saturating_add(1);
            } else {
                senders = senders.saturating_add(1);
            }
            let frozen =
                ledger::mptoken_helpers::is_frozen_mpt(sandbox, account, &issue).unwrap_or(true);
            let authorized = if req_auth {
                ledger::mptoken_helpers::require_auth_mpt(sandbox, &issue, account)
                    .is_ok_and(protocol::is_tes_success)
            } else {
                true
            };
            if frozen || value.locked_before || value.locked_after || !authorized {
                invalid_transfer = true;
            }
        }

        if senders > 0
            && receivers > 0
            && (invalid_transfer
                || !can_transfer
                || (mpt_transfer_is_dex(txn_type, cross_currency_payment) && !can_trade))
        {
            return !mptokens_v2_enabled;
        }
    }

    true
}

fn same_optional_h256(
    before: &STLedgerEntry,
    after: &STLedgerEntry,
    field: &'static protocol::SField,
) -> bool {
    let before_present = before.is_field_present(field);
    let after_present = after.is_field_present(field);
    before_present == after_present
        && (!before_present || before.get_field_h256(field) == after.get_field_h256(field))
}

fn record_mpt_issuance_lifecycle<V: ApplyView>(
    sandbox: &FlowSandbox<V>,
    txn_type: protocol::TxType,
    lifecycle: &mut MptIssuanceLifecycle,
    is_delete: bool,
    before: Option<&STLedgerEntry>,
    after: Option<&STLedgerEntry>,
    deleted: &STLedgerEntry,
) {
    if let Some(after) = after
        && after.get_type() == LedgerEntryType::MPTokenIssuance
    {
        if before.is_none() {
            lifecycle.issuances_created = lifecycle.issuances_created.saturating_add(1);
            lifecycle.reference_holding_set_on_create |= after
                .is_field_present(sf("sfReferenceHolding"))
                && txn_type != protocol::TxType::VAULT_CREATE;
        } else if let Some(before) = before {
            lifecycle.reference_holding_mutated |=
                !same_optional_h256(before, after, sf("sfReferenceHolding"));
        }
    }

    if is_delete && deleted.get_type() == LedgerEntryType::MPTokenIssuance {
        lifecycle.issuances_deleted = lifecycle.issuances_deleted.saturating_add(1);
    }

    if let Some(after) = after
        && after.get_type() == LedgerEntryType::MPToken
        && before.is_none()
    {
        lifecycle.tokens_created = lifecycle.tokens_created.saturating_add(1);
        let id = after.get_field_h192(sf("sfMPTokenIssuanceID"));
        lifecycle.token_created_by_issuer |=
            protocol::MPTIssue::new(id).issuer() == after.get_account_id(sf("sfAccount"));
    }

    if is_delete && deleted.get_type() == LedgerEntryType::MPToken {
        lifecycle.tokens_deleted = lifecycle.tokens_deleted.saturating_add(1);
    }

    if !is_delete || txn_type == protocol::TxType::VAULT_DELETE {
        return;
    }

    lifecycle.vault_holding_deleted |= match deleted.get_type() {
        LedgerEntryType::MPToken => {
            let holder = deleted.get_account_id(sf("sfAccount"));
            is_vault_pseudo_account(sandbox, holder)
        }
        LedgerEntryType::RippleState => {
            let low_counterparty = deleted.get_field_amount(sf("sfLowLimit")).issue().account;
            let high_counterparty = deleted.get_field_amount(sf("sfHighLimit")).issue().account;
            is_vault_pseudo_account(sandbox, low_counterparty)
                || is_vault_pseudo_account(sandbox, high_counterparty)
        }
        _ => false,
    };
}

fn is_vault_pseudo_account<V: ApplyView>(sandbox: &FlowSandbox<V>, account: AccountID) -> bool {
    sandbox
        .read(protocol::account_keylet(raw_account_id(account)))
        .ok()
        .flatten()
        .is_some_and(|sle| sle.is_field_present(sf("sfVaultID")))
}

fn validates_mpt_issuance_lifecycle(lifecycle: &MptIssuanceLifecycle) -> bool {
    !lifecycle.reference_holding_set_on_create
        && !lifecycle.reference_holding_mutated
        && !lifecycle.vault_holding_deleted
}

fn has_create_mpt_issuance_privilege(txn_type: protocol::TxType) -> bool {
    matches!(
        txn_type,
        protocol::TxType::MPTOKEN_ISSUANCE_CREATE | protocol::TxType::VAULT_CREATE
    )
}

fn has_destroy_mpt_issuance_privilege(txn_type: protocol::TxType) -> bool {
    matches!(
        txn_type,
        protocol::TxType::MPTOKEN_ISSUANCE_DESTROY | protocol::TxType::VAULT_DELETE
    )
}

fn has_must_authorize_mpt_privilege(txn_type: protocol::TxType) -> bool {
    txn_type == protocol::TxType::MPTOKEN_AUTHORIZE
}

fn has_may_authorize_mpt_privilege(txn_type: protocol::TxType) -> bool {
    matches!(
        txn_type,
        protocol::TxType::AMM_CLAWBACK
            | protocol::TxType::AMM_WITHDRAW
            | protocol::TxType::VAULT_DEPOSIT
            | protocol::TxType::VAULT_WITHDRAW
            | protocol::TxType::LOAN_BROKER_SET
            | protocol::TxType::LOAN_BROKER_DELETE
            | protocol::TxType::LOAN_BROKER_COVER_WITHDRAW
            | protocol::TxType::LOAN_SET
            | protocol::TxType::LOAN_PAY
    )
}

fn has_may_create_mpt_privilege(txn_type: protocol::TxType) -> bool {
    matches!(
        txn_type,
        protocol::TxType::PAYMENT
            | protocol::TxType::OFFER_CREATE
            | protocol::TxType::CHECK_CASH
            | protocol::TxType::AMM_CREATE
    )
}

fn has_may_delete_mpt_privilege(txn_type: protocol::TxType) -> bool {
    matches!(
        txn_type,
        protocol::TxType::AMM_DELETE
            | protocol::TxType::VAULT_WITHDRAW
            | protocol::TxType::VAULT_CLAWBACK
    )
}

fn validates_mpt_lifecycle_counts(
    txn_type: protocol::TxType,
    result: Ter,
    tx_has_holder: bool,
    single_asset_vault_enabled: bool,
    lending_protocol_enabled: bool,
    mptokens_v2_enabled: bool,
    lifecycle: &MptIssuanceLifecycle,
) -> bool {
    let applies =
        protocol::is_tes_success(result) || (mptokens_v2_enabled && result == Ter::TEC_INCOMPLETE);
    if !applies {
        return true;
    }

    if lifecycle.token_created_by_issuer && (single_asset_vault_enabled || lending_protocol_enabled)
    {
        return false;
    }

    if has_create_mpt_issuance_privilege(txn_type) {
        return lifecycle.issuances_created == 1 && lifecycle.issuances_deleted == 0;
    }

    if has_destroy_mpt_issuance_privilege(txn_type) {
        return lifecycle.issuances_created == 0 && lifecycle.issuances_deleted == 1;
    }

    let enforce_escrow_finish = txn_type == protocol::TxType::ESCROW_FINISH
        && (single_asset_vault_enabled || lending_protocol_enabled);
    if has_must_authorize_mpt_privilege(txn_type)
        || has_may_authorize_mpt_privilege(txn_type)
        || enforce_escrow_finish
    {
        if lifecycle.issuances_created > 0 || lifecycle.issuances_deleted > 0 {
            return false;
        }

        if mptokens_v2_enabled
            && has_may_authorize_mpt_privilege(txn_type)
            && matches!(
                txn_type,
                protocol::TxType::AMM_WITHDRAW | protocol::TxType::AMM_CLAWBACK
            )
        {
            if tx_has_holder
                && txn_type == protocol::TxType::AMM_WITHDRAW
                && lifecycle.tokens_created > 0
            {
                return false;
            }
            return lifecycle.tokens_created <= 2 && lifecycle.tokens_deleted <= 2;
        }

        if lending_protocol_enabled && lifecycle.tokens_created + lifecycle.tokens_deleted > 1 {
            return false;
        }

        if tx_has_holder && (lifecycle.tokens_created > 0 || lifecycle.tokens_deleted > 0) {
            return false;
        }

        if !tx_has_holder
            && has_must_authorize_mpt_privilege(txn_type)
            && lifecycle.tokens_created + lifecycle.tokens_deleted != 1
        {
            return false;
        }

        return true;
    }

    if has_may_create_mpt_privilege(txn_type) {
        if lifecycle.issuances_created > 0
            || lifecycle.issuances_deleted > 0
            || lifecycle.tokens_deleted > 0
            || tx_has_holder
        {
            return false;
        }
        if (txn_type == protocol::TxType::AMM_CREATE && lifecycle.tokens_created > 2)
            || (txn_type == protocol::TxType::CHECK_CASH && lifecycle.tokens_created > 1)
        {
            return false;
        }
        return true;
    }

    if has_may_delete_mpt_privilege(txn_type)
        && lifecycle.tokens_created == 0
        && lifecycle.issuances_created == 0
        && lifecycle.issuances_deleted == 0
        && ((txn_type == protocol::TxType::AMM_DELETE && lifecycle.tokens_deleted <= 2)
            || lifecycle.tokens_deleted == 1)
    {
        return true;
    }

    lifecycle.issuances_created == 0
        && lifecycle.issuances_deleted == 0
        && lifecycle.tokens_created == 0
        && lifecycle.tokens_deleted == 0
}

fn is_root_book_directory(sle: &STLedgerEntry) -> bool {
    [
        "sfExchangeRate",
        "sfTakerPaysCurrency",
        "sfTakerPaysIssuer",
        "sfTakerPaysMPT",
        "sfTakerGetsCurrency",
        "sfTakerGetsIssuer",
        "sfTakerGetsMPT",
        "sfDomainID",
    ]
    .iter()
    .any(|field| sle.is_field_present(sf(field)))
}

fn bad_book_exchange_rate(sle: &STLedgerEntry) -> bool {
    is_root_book_directory(sle)
        && (!sle.is_field_present(sf("sfExchangeRate"))
            || sle.get_field_u64(sf("sfExchangeRate")) != protocol::quality_from_key(*sle.key()))
}

fn maybe_record_directory_root(
    roots: &mut BTreeSet<Uint256>,
    is_delete: bool,
    before: Option<&STLedgerEntry>,
    after: Option<&STLedgerEntry>,
) -> bool {
    if is_delete || after.is_none_or(|sle| sle.get_type() != LedgerEntryType::DirectoryNode) {
        return true;
    }
    let after = after.expect("checked above");
    let root_index = after.get_field_h256(sf("sfRootIndex"));

    if before.is_some_and(|sle| sle.get_field_h256(sf("sfRootIndex")) == root_index) {
        return true;
    }

    if *after.key() == root_index {
        return !bad_book_exchange_rate(after);
    }

    roots.insert(root_index);
    true
}

fn validate_vault_entry(sle: &STLedgerEntry) -> bool {
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

fn vault_snapshot(sle: &STLedgerEntry) -> VaultSnapshot {
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

fn vault_shares_snapshot(sle: &STLedgerEntry) -> VaultSharesSnapshot {
    VaultSharesSnapshot {
        share_mpt_id: mpt_id_from_issuance(sle),
        issuer: sle.get_account_id(sf("sfIssuer")),
        shares_total: optional_u64(sle, sf("sfOutstandingAmount")),
        shares_maximum: mpt_max_amount(sle),
    }
}

fn add_vault_asset_delta(
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

fn signed_delta(value: RuntimeNumber, before: bool) -> RuntimeNumber {
    if before { -value } else { value }
}

fn record_vault_asset_delta(state: &mut VaultState, sle: &STLedgerEntry, before: bool) {
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

fn record_vault_state(
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

fn vault_must_modify(txn_type: protocol::TxType) -> bool {
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

fn vault_may_modify(txn_type: protocol::TxType) -> bool {
    txn_type == protocol::TxType::LOAN_MANAGE
}

fn find_vault_share<'a>(
    shares: &'a [VaultSharesSnapshot],
    share_mpt_id: MPTID,
) -> Option<&'a VaultSharesSnapshot> {
    shares
        .iter()
        .find(|candidate| candidate.share_mpt_id == share_mpt_id)
}

fn read_vault_share<V: ApplyView>(
    sandbox: &FlowSandbox<V>,
    share_mpt_id: MPTID,
) -> Option<VaultSharesSnapshot> {
    sandbox
        .read(protocol::mpt_issuance_keylet_from_mptid(share_mpt_id))
        .ok()
        .flatten()
        .map(|sle| vault_shares_snapshot(&sle))
}

fn vault_share_issuance_delta(state: &VaultState, share_mpt_id: MPTID) -> i128 {
    state
        .share_issuance_delta
        .get(&share_mpt_id)
        .copied()
        .unwrap_or_default()
}

fn vault_share_issuance_delta_if_updated(state: &VaultState, share_mpt_id: MPTID) -> Option<i128> {
    state.share_issuance_delta.get(&share_mpt_id).copied()
}

fn vault_share_holder_delta(
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

fn valid_vault_share_delta(
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

fn asset_issuer(asset: Asset) -> AccountID {
    match asset {
        Asset::Issue(issue) => issue.account,
        Asset::MPTIssue(issue) => issue.issuer(),
    }
}

fn round_runtime_to_scale(
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

fn round_number_to_asset(asset: Asset, value: RuntimeNumber) -> RuntimeNumber {
    let mut number = STNumber::from(value);
    number.associate_asset(asset);
    number.value()
}

fn round_number_to_asset_with_scale(
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

fn number_scale(asset: Asset, value: RuntimeNumber) -> i32 {
    if asset.integral() {
        0
    } else {
        asset
            .amount(value)
            .map(|amount| amount.exponent())
            .unwrap_or(0)
    }
}

fn vault_delta_scale(before: RuntimeNumber, after: RuntimeNumber, asset: Asset) -> i32 {
    number_scale(asset, before).max(number_scale(asset, after))
}

fn compute_vault_min_scale(
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

fn rounded_vault_delta(asset: Asset, delta: VaultAssetDelta, scale: i32) -> RuntimeNumber {
    round_number_to_asset_with_scale(asset, delta.delta, scale, RoundingMode::ToNearest)
}

fn vault_account_asset_delta(
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

fn vault_asset_delta(
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

fn validates_vault_state<V: ApplyView>(
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

fn number_field_value(sle: &STLedgerEntry, field: &'static protocol::SField) -> RuntimeNumber {
    sle.get_field_number(field).value()
}

fn number_field_negative(sle: &STLedgerEntry, field: &'static protocol::SField) -> bool {
    number_field_value(sle, field) < RuntimeNumber::zero()
}

fn validate_loan_entry(before: Option<&STLedgerEntry>, after: &STLedgerEntry) -> bool {
    let zero = RuntimeNumber::zero();
    let payment_remaining = after.get_field_u32(sf("sfPaymentRemaining"));
    let total_value = number_field_value(after, sf("sfTotalValueOutstanding"));
    let principal = number_field_value(after, sf("sfPrincipalOutstanding"));
    let management_fee = number_field_value(after, sf("sfManagementFeeOutstanding"));

    if payment_remaining == 0
        && (total_value != zero || principal != zero || management_fee != zero)
    {
        return false;
    }
    if payment_remaining != 0 && total_value == zero && principal == zero && management_fee == zero
    {
        return false;
    }
    if before.is_some_and(|before| {
        before.is_flag(protocol::lsfLoanOverpayment) != after.is_flag(protocol::lsfLoanOverpayment)
    }) {
        return false;
    }

    for field in [
        sf("sfLoanServiceFee"),
        sf("sfLatePaymentFee"),
        sf("sfClosePaymentFee"),
        sf("sfPrincipalOutstanding"),
        sf("sfTotalValueOutstanding"),
        sf("sfManagementFeeOutstanding"),
    ] {
        if number_field_negative(after, field) {
            return false;
        }
    }

    number_field_value(after, sf("sfPeriodicPayment")) > zero
}

fn maybe_record_loan_broker_account<V: ApplyView>(
    sandbox: &FlowSandbox<V>,
    state: &mut LendingState,
    account: AccountID,
) {
    if let Ok(Some(root)) = sandbox.read(protocol::account_keylet(raw_account_id(account)))
        && root.is_field_present(sf("sfLoanBrokerID"))
    {
        state
            .broker_refs
            .insert(root.get_field_h256(sf("sfLoanBrokerID")));
    }
}

fn record_lending_state<V: ApplyView>(
    sandbox: &FlowSandbox<V>,
    state: &mut LendingState,
    after: Option<&STLedgerEntry>,
) {
    let Some(after) = after else {
        return;
    };

    match after.get_type() {
        LedgerEntryType::AccountRoot => {
            if after.is_field_present(sf("sfLoanBrokerID")) {
                state
                    .broker_refs
                    .insert(after.get_field_h256(sf("sfLoanBrokerID")));
            }
        }
        LedgerEntryType::LoanBroker => {
            state.broker_refs.insert(*after.key());
        }
        LedgerEntryType::RippleState => {
            maybe_record_loan_broker_account(
                sandbox,
                state,
                after.get_field_amount(sf("sfLowLimit")).issue().account,
            );
            maybe_record_loan_broker_account(
                sandbox,
                state,
                after.get_field_amount(sf("sfHighLimit")).issue().account,
            );
        }
        LedgerEntryType::MPToken => {
            maybe_record_loan_broker_account(sandbox, state, after.get_account_id(sf("sfAccount")));
        }
        _ => {}
    }
}

fn validate_zero_owner_count_broker_directory<V: ApplyView>(
    sandbox: &FlowSandbox<V>,
    broker: &STLedgerEntry,
) -> bool {
    if broker.get_field_u32(sf("sfOwnerCount")) != 0 {
        return true;
    }

    let Ok(Some(dir)) = sandbox.read(protocol::owner_dir_keylet(raw_account_id(
        broker.get_account_id(sf("sfAccount")),
    ))) else {
        return true;
    };

    if dir.is_field_present(sf("sfIndexPrevious")) && dir.get_field_u64(sf("sfIndexPrevious")) != 0
    {
        return false;
    }
    if dir.is_field_present(sf("sfIndexNext")) && dir.get_field_u64(sf("sfIndexNext")) != 0 {
        return false;
    }

    let indexes = dir.get_field_v256(sf("sfIndexes"));
    if indexes.value().len() > 1 {
        return false;
    }

    if let Some(index) = indexes.value().first() {
        let Ok(Some(indexed)) = sandbox.read(protocol::unchecked_keylet(*index)) else {
            return false;
        };
        matches!(
            indexed.get_type(),
            LedgerEntryType::RippleState | LedgerEntryType::MPToken
        )
    } else {
        true
    }
}

fn amount_to_number(amount: &STAmount) -> RuntimeNumber {
    if amount.native() {
        RuntimeNumber::from(amount.xrp())
    } else if amount.holds_mpt_issue() {
        RuntimeNumber::from(amount.mpt())
    } else {
        RuntimeNumber::from(amount.iou())
    }
}

fn account_holds_asset_amount<V: ApplyView>(
    sandbox: &FlowSandbox<V>,
    account: AccountID,
    asset: Asset,
    field: &'static protocol::SField,
) -> Option<STAmount> {
    match asset {
        Asset::Issue(issue) if issue.native() => Some(
            sandbox
                .read(protocol::account_keylet(raw_account_id(account)))
                .ok()
                .flatten()
                .map(|sle| sle.get_field_amount(sf("sfBalance")))
                .unwrap_or_else(|| STAmount::from_xrp_amount(XRPAmount::new())),
        ),
        Asset::Issue(issue) => {
            if issue.issuer() == account {
                return Some(STAmount::from_iou_amount(field, IOUAmount::new(), issue));
            }
            let mut amount = sandbox
                .read(protocol::line(account, issue.issuer(), issue.currency))
                .ok()
                .flatten()
                .map(|sle| sle.get_field_amount(sf("sfBalance")))
                .unwrap_or_else(|| STAmount::from_iou_amount(field, IOUAmount::new(), issue));
            if account > issue.issuer() {
                amount.negate();
            }
            amount.set_issuer(issue.issuer());
            Some(amount)
        }
        Asset::MPTIssue(issue) => {
            let value = sandbox
                .read(protocol::mptoken_keylet_from_mptid(
                    issue.mpt_id(),
                    raw_account_id(account),
                ))
                .ok()
                .flatten()
                .map(|sle| optional_u64(&sle, sf("sfMPTAmount")))
                .unwrap_or(0);
            let value = i64::try_from(value).ok()?;
            Some(STAmount::from_mpt_amount(
                field,
                MPTAmount::from_value(value),
                issue,
            ))
        }
    }
}

fn account_holds_asset_number<V: ApplyView>(
    sandbox: &FlowSandbox<V>,
    account: AccountID,
    asset: Asset,
) -> Option<RuntimeNumber> {
    match asset {
        Asset::Issue(issue) if issue.native() => {
            let amount = sandbox
                .read(protocol::account_keylet(raw_account_id(account)))
                .ok()
                .flatten()
                .map(|sle| sle.get_field_amount(sf("sfBalance")))
                .unwrap_or_else(|| STAmount::from_xrp_amount(XRPAmount::new()));
            Some(amount_to_number(&amount))
        }
        Asset::Issue(issue) => {
            if issue.issuer() == account {
                return Some(RuntimeNumber::zero());
            }
            let amount = sandbox
                .read(protocol::line(account, issue.issuer(), issue.currency))
                .ok()
                .flatten()
                .map(|sle| {
                    let mut balance = sle.get_field_amount(sf("sfBalance"));
                    if account > issue.issuer() {
                        balance.negate();
                    }
                    balance
                })
                .unwrap_or_else(|| STAmount::new_with_asset(sf("sfBalance"), issue, 0, 0, false));
            Some(amount_to_number(&amount))
        }
        Asset::MPTIssue(issue) => {
            let amount = sandbox
                .read(protocol::mptoken_keylet_from_mptid(
                    issue.mpt_id(),
                    raw_account_id(account),
                ))
                .ok()
                .flatten()
                .map(|sle| RuntimeNumber::from_i64(sle.get_field_u64(sf("sfMPTAmount")) as i64))
                .unwrap_or_else(RuntimeNumber::zero);
            Some(amount)
        }
    }
}

fn validate_loan_broker_entry<V: ApplyView>(
    sandbox: &FlowSandbox<V>,
    txn_type: protocol::TxType,
    fix_cleanup_3_1_3: bool,
    before: Option<&STLedgerEntry>,
    after: &STLedgerEntry,
) -> bool {
    if before.is_some_and(|before| {
        before.get_field_u32(sf("sfLoanSequence")) > after.get_field_u32(sf("sfLoanSequence"))
    }) {
        return false;
    }
    if number_field_negative(after, sf("sfDebtTotal"))
        || number_field_negative(after, sf("sfCoverAvailable"))
    {
        return false;
    }
    let Ok(Some(vault)) = sandbox.read(protocol::vault_keylet_from_key(
        after.get_field_h256(sf("sfVaultID")),
    )) else {
        return false;
    };
    if !validate_zero_owner_count_broker_directory(sandbox, after) {
        return false;
    }

    let cover_available = number_field_value(after, sf("sfCoverAvailable"));
    let vault_asset = vault.get_field_issue(sf("sfAsset")).asset();
    let Some(pseudo_balance) =
        account_holds_asset_number(sandbox, after.get_account_id(sf("sfAccount")), vault_asset)
    else {
        return false;
    };

    if cover_available < pseudo_balance {
        return false;
    }
    if fix_cleanup_3_1_3 && txn_type != protocol::TxType::LOAN_BROKER_DELETE {
        if cover_available > pseudo_balance {
            return false;
        }
    }

    true
}

fn validate_mpt_entry(sle: &STLedgerEntry) -> bool {
    match sle.get_type() {
        LedgerEntryType::MPTokenIssuance => {
            let outstanding = optional_u64(sle, sf("sfOutstandingAmount"));
            let locked = optional_u64(sle, sf("sfLockedAmount"));
            outstanding <= mpt_max_amount(sle) && locked <= outstanding
        }
        LedgerEntryType::MPToken => {
            let account = sle.get_account_id(sf("sfAccount"));
            let id = sle.get_field_h192(sf("sfMPTokenIssuanceID"));
            let mpt_amount = optional_u64(sle, sf("sfMPTAmount"));
            let locked = optional_u64(sle, sf("sfLockedAmount"));
            let max_amount = max_mpt_token_amount();
            account != protocol::MPTIssue::new(id).issuer()
                && mpt_amount <= max_amount
                && locked <= max_amount.saturating_sub(mpt_amount)
        }
        _ => true,
    }
}

fn validate_amm_entry(sle: &STLedgerEntry) -> bool {
    let asset = sle.get_field_issue(sf("sfAsset")).asset();
    let asset2 = sle.get_field_issue(sf("sfAsset2")).asset();
    if asset == asset2 {
        return false;
    }

    let lp_tokens = sle.get_field_amount(sf("sfLPTokenBalance"));
    if lp_tokens.negative() || lp_tokens.signum() == 0 {
        return false;
    }

    if sle.is_field_present(sf("sfTradingFee")) && sle.get_field_u16(sf("sfTradingFee")) > 1000 {
        return false;
    }

    if sle.is_field_present(sf("sfAuctionSlot")) {
        let slot = sle.get_field_object(sf("sfAuctionSlot"));
        if slot.is_field_present(sf("sfAuthAccounts"))
            && slot.get_field_array(sf("sfAuthAccounts")).iter().count() > 4
        {
            return false;
        }
    }

    true
}

fn validate_ripple_state_entry(sle: &STLedgerEntry) -> bool {
    if sle.get_field_amount(sf("sfLowLimit")).asset().native()
        || sle.get_field_amount(sf("sfHighLimit")).asset().native()
    {
        return false;
    }

    let flags = if sle.is_field_present(sf("sfFlags")) {
        sle.get_field_u32(sf("sfFlags"))
    } else {
        0
    };
    let low_freeze = (flags & protocol::lsfLowFreeze) != 0;
    let low_deep_freeze = (flags & protocol::lsfLowDeepFreeze) != 0;
    let high_freeze = (flags & protocol::lsfHighFreeze) != 0;
    let high_deep_freeze = (flags & protocol::lsfHighDeepFreeze) != 0;

    !(low_deep_freeze && !low_freeze || high_deep_freeze && !high_freeze)
}

fn credential_sort_key(credential: &protocol::STObject) -> (AccountID, Vec<u8>) {
    (
        credential.get_account_id(sf("sfIssuer")),
        credential.get_field_vl(sf("sfCredentialType")),
    )
}

fn record_permissioned_domain_state(
    state: &mut PermissionedDomainState,
    is_delete: bool,
    before: Option<&STLedgerEntry>,
    after: Option<&STLedgerEntry>,
) {
    let candidate = if is_delete { before } else { after };
    let Some(sle) = candidate else {
        return;
    };
    if sle.get_type() != LedgerEntryType::PermissionedDomain {
        return;
    }

    let credentials = sle.get_field_array(sf("sfAcceptedCredentials"));
    let keys = credentials
        .iter()
        .map(credential_sort_key)
        .collect::<Vec<_>>();
    let mut sorted_keys = keys.clone();
    sorted_keys.sort();
    sorted_keys.dedup();

    state.statuses.push(PermissionedDomainStatus {
        credentials_size: keys.len(),
        sorted: keys == sorted_keys,
        unique: keys.len() == sorted_keys.len(),
        deleted: is_delete,
    });
}

fn validates_permissioned_domain_status(status: &PermissionedDomainStatus) -> bool {
    const MAX_PERMISSIONED_DOMAIN_CREDENTIALS_ARRAY_SIZE: usize = 10;

    status.credentials_size > 0
        && status.credentials_size <= MAX_PERMISSIONED_DOMAIN_CREDENTIALS_ARRAY_SIZE
        && status.unique
        && status.sorted
}

fn validates_permissioned_domain(
    txn_type: protocol::TxType,
    result: Ter,
    fix_cleanup_3_1_3: bool,
    state: &PermissionedDomainState,
) -> bool {
    if fix_cleanup_3_1_3 {
        if !protocol::is_tes_success(result) {
            return state.statuses.is_empty();
        }
        if state.statuses.len() > 1 {
            return false;
        }

        match txn_type {
            protocol::TxType::PERMISSIONED_DOMAIN_SET => {
                let Some(status) = state.statuses.first() else {
                    return false;
                };
                !status.deleted && validates_permissioned_domain_status(status)
            }
            protocol::TxType::PERMISSIONED_DOMAIN_DELETE => {
                state.statuses.first().is_some_and(|status| status.deleted)
            }
            _ => state.statuses.is_empty(),
        }
    } else {
        if txn_type != protocol::TxType::PERMISSIONED_DOMAIN_SET
            || !protocol::is_tes_success(result)
            || state.statuses.is_empty()
        {
            return true;
        }
        validates_permissioned_domain_status(&state.statuses[0])
    }
}

fn record_amm_state(
    state: &mut AmmState,
    is_delete: bool,
    before: Option<&STLedgerEntry>,
    after: Option<&STLedgerEntry>,
) {
    if is_delete {
        return;
    }

    let Some(after) = after else {
        return;
    };

    if let Some(before) = before
        && before.get_type() == LedgerEntryType::AMM
    {
        state.lpt_balance_before = Some(before.get_field_amount(sf("sfLPTokenBalance")));
    }

    match after.get_type() {
        LedgerEntryType::AMM => {
            state.amm_after = true;
            state.amm_account = Some(after.get_account_id(sf("sfAccount")));
            state.asset = Some(after.get_field_issue(sf("sfAsset")).asset());
            state.asset2 = Some(after.get_field_issue(sf("sfAsset2")).asset());
            if after.is_field_present(sf("sfAmount")) {
                state.amount = Some(after.get_field_amount(sf("sfAmount")));
            }
            if after.is_field_present(sf("sfAmount2")) {
                state.amount2 = Some(after.get_field_amount(sf("sfAmount2")));
            }
            state.lpt_balance_after = Some(after.get_field_amount(sf("sfLPTokenBalance")));
        }
        LedgerEntryType::RippleState if after.is_flag(protocol::lsfAMMNode) => {
            state.pool_changed = true;
        }
        LedgerEntryType::AccountRoot if after.is_field_present(sf("sfAMMID")) => {
            state.pool_changed = true;
        }
        LedgerEntryType::MPToken if after.is_flag(protocol::lsfMPTAMM) => {
            let before_amount = before.map(|sle| optional_u64(sle, sf("sfMPTAmount")));
            let after_amount = optional_u64(after, sf("sfMPTAmount"));
            if before_amount != Some(after_amount) {
                state.pool_changed = true;
            }
        }
        _ => {}
    }
}

fn amm_invariant_result_applies(result: Ter) -> bool {
    protocol::is_tes_success(result) || result == Ter::TEC_INCOMPLETE
}

fn valid_amm_balances(
    amount: &STAmount,
    amount2: &STAmount,
    lp_tokens: &STAmount,
    zero_allowed: bool,
) -> bool {
    if amount.signum() > 0 && amount2.signum() > 0 && lp_tokens.signum() > 0 {
        return true;
    }
    zero_allowed && amount.signum() == 0 && amount2.signum() == 0 && lp_tokens.signum() == 0
}

fn amm_pool_holds<V: ApplyView>(
    sandbox: &FlowSandbox<V>,
    state: &AmmState,
) -> Option<(STAmount, STAmount)> {
    if let (Some(amount), Some(amount2)) = (&state.amount, &state.amount2) {
        return Some((amount.clone(), amount2.clone()));
    }

    let account = state.amm_account?;
    let asset = state.asset?;
    let asset2 = state.asset2?;
    Some((
        account_holds_asset_amount(sandbox, account, asset, sf("sfAmount"))?,
        account_holds_asset_amount(sandbox, account, asset2, sf("sfAmount2"))?,
    ))
}

fn validates_amm_create<V: ApplyView>(sandbox: &FlowSandbox<V>, state: &AmmState) -> bool {
    let Some(lp_tokens) = &state.lpt_balance_after else {
        return false;
    };
    let Some((amount, amount2)) = amm_pool_holds(sandbox, state) else {
        return false;
    };

    valid_amm_balances(&amount, &amount2, lp_tokens, false)
        && ledger::amm_helpers::amm_lp_tokens(&amount, &amount2, lp_tokens.issue()) == *lp_tokens
}

fn validates_amm_general<V: ApplyView>(
    sandbox: &FlowSandbox<V>,
    state: &AmmState,
    zero_allowed: bool,
) -> bool {
    let Some(lp_tokens) = &state.lpt_balance_after else {
        return false;
    };
    let Some((amount, amount2)) = amm_pool_holds(sandbox, state) else {
        return false;
    };

    if !valid_amm_balances(&amount, &amount2, lp_tokens, zero_allowed) {
        return false;
    }

    let Some(pool_product_mean) = root2(
        ledger::amm_helpers::stamount_as_number(&amount)
            * ledger::amm_helpers::stamount_as_number(&amount2),
    )
    .ok() else {
        return false;
    };
    let lp_number = ledger::amm_helpers::stamount_as_number(lp_tokens);
    if pool_product_mean >= lp_number {
        return true;
    }
    if lp_number == RuntimeNumber::zero() {
        return false;
    }
    let distance = RuntimeNumber::try_from_external_parts(1, -11, get_mantissa_scale())
        .expect("relative distance constant");
    ledger::amm_helpers::within_relative_distance_amount(pool_product_mean, lp_number, distance)
}

fn validates_amm_state<V: ApplyView>(
    sandbox: &FlowSandbox<V>,
    txn_type: protocol::TxType,
    result: Ter,
    state: &AmmState,
) -> bool {
    if !amm_invariant_result_applies(result) {
        return true;
    }

    match txn_type {
        protocol::TxType::AMM_BID => {
            if state.pool_changed {
                return false;
            }
            if let (Some(before), Some(after)) =
                (&state.lpt_balance_before, &state.lpt_balance_after)
                && (after > before || after.signum() <= 0)
            {
                return false;
            }
            true
        }
        protocol::TxType::AMM_VOTE => {
            !state.pool_changed && state.lpt_balance_before == state.lpt_balance_after
        }
        protocol::TxType::AMM_CREATE => state.amm_after && validates_amm_create(sandbox, state),
        protocol::TxType::AMM_DEPOSIT => {
            state.amm_after && validates_amm_general(sandbox, state, false)
        }
        protocol::TxType::AMM_WITHDRAW | protocol::TxType::AMM_CLAWBACK => {
            !state.amm_after || validates_amm_general(sandbox, state, true)
        }
        protocol::TxType::AMM_DELETE => !state.amm_after,
        protocol::TxType::CHECK_CASH
        | protocol::TxType::OFFER_CREATE
        | protocol::TxType::PAYMENT => !state.amm_after,
        _ => true,
    }
}

fn record_permissioned_dex(
    state: &mut PermissionedDexState,
    is_delete: bool,
    before: Option<&STLedgerEntry>,
    after: Option<&STLedgerEntry>,
) {
    let Some(after) = after.or(if is_delete { before } else { None }) else {
        return;
    };

    match after.get_type() {
        LedgerEntryType::DirectoryNode => {
            if after.is_field_present(sf("sfDomainID")) {
                state.domains.insert(after.get_field_h256(sf("sfDomainID")));
            }
        }
        LedgerEntryType::Offer => {
            if after.is_field_present(sf("sfDomainID")) {
                state.domains.insert(after.get_field_h256(sf("sfDomainID")));
            } else {
                state.regular_offers_old = true;
                if !is_delete {
                    state.regular_offers = true;
                }
            }

            if after.is_flag(protocol::lsfHybrid) {
                let has_domain = after.is_field_present(sf("sfDomainID"));
                let additional_len = if after.is_field_present(sf("sfAdditionalBooks")) {
                    Some(after.get_field_array(sf("sfAdditionalBooks")).len())
                } else {
                    None
                };

                if !has_domain || additional_len.is_none_or(|len| len > 1) {
                    state.bad_hybrids_old = true;
                }
                if !has_domain || additional_len != Some(1) {
                    state.bad_hybrids = true;
                }
            }
        }
        _ => {}
    }
}

fn validates_permissioned_dex<V: ApplyView>(
    sandbox: &FlowSandbox<V>,
    txn_type: protocol::TxType,
    result: Ter,
    tx_domain: Option<Uint256>,
    fix_cleanup_3_1_3: bool,
    fix_cleanup_3_2_0: bool,
    state: &PermissionedDexState,
) -> bool {
    if !matches!(
        txn_type,
        protocol::TxType::PAYMENT | protocol::TxType::OFFER_CREATE
    ) || !protocol::is_tes_success(result)
    {
        return true;
    }

    let malformed_hybrid = if fix_cleanup_3_1_3 {
        state.bad_hybrids
    } else {
        state.bad_hybrids_old
    };
    if txn_type == protocol::TxType::OFFER_CREATE && malformed_hybrid {
        return false;
    }

    let Some(domain) = tx_domain else {
        return true;
    };

    if !matches!(
        sandbox.read(protocol::permissioned_domain_keylet_from_id(domain)),
        Ok(Some(_))
    ) {
        return false;
    }

    if state.domains.iter().any(|candidate| *candidate != domain) {
        return false;
    }

    let has_regular_offers = if fix_cleanup_3_2_0 {
        state.regular_offers
    } else {
        state.regular_offers_old
    };
    !has_regular_offers
}

fn record_clawback_state(state: &mut ClawbackState, before: Option<&STLedgerEntry>) {
    match before.map(STLedgerEntry::get_type) {
        Some(LedgerEntryType::RippleState) => {
            state.trustlines_changed = state.trustlines_changed.saturating_add(1);
        }
        Some(LedgerEntryType::MPToken) => {
            state.mptokens_changed = state.mptokens_changed.saturating_add(1);
        }
        _ => {}
    }
}

fn validates_clawback<V: ApplyView>(
    sandbox: &FlowSandbox<V>,
    txn_type: protocol::TxType,
    result: Ter,
    tx_account: Option<AccountID>,
    tx_holder: Option<AccountID>,
    tx_amount: Option<&STAmount>,
    mptokens_v2_enabled: bool,
    state: &ClawbackState,
) -> bool {
    if txn_type != protocol::TxType::CLAWBACK {
        return true;
    }

    if !protocol::is_tes_success(result) {
        return state.trustlines_changed == 0 && state.mptokens_changed == 0;
    }

    if state.trustlines_changed > 1 || state.mptokens_changed > 1 {
        return false;
    }

    let should_check_balance =
        state.trustlines_changed == 1 || (mptokens_v2_enabled && state.mptokens_changed == 1);
    if !should_check_balance {
        return true;
    }

    let (Some(issuer), Some(amount)) = (tx_account, tx_amount) else {
        return false;
    };

    let (holder, asset) = match amount.asset() {
        Asset::Issue(issue) => (
            issue.account,
            Asset::Issue(Issue {
                currency: issue.currency,
                account: issuer,
            }),
        ),
        Asset::MPTIssue(issue) => {
            let Some(holder) = tx_holder else {
                return false;
            };
            (holder, Asset::MPTIssue(issue))
        }
    };

    account_holds_asset_amount(sandbox, holder, asset, sf("sfAmount"))
        .is_some_and(|balance| balance.signum() >= 0)
}

pub fn check_invariants_for_tx<V: ApplyView>(
    sandbox: &FlowSandbox<V>,
    tx: &STTx,
    result: Ter,
    fee: XRPAmount,
) -> Ter {
    let txn_type = tx.get_txn_type();
    let tx_domain = tx
        .is_field_present(sf("sfDomainID"))
        .then(|| tx.get_field_h256(sf("sfDomainID")));
    let tx_account = tx
        .is_field_present(sf("sfAccount"))
        .then(|| tx.get_account_id(sf("sfAccount")));
    let tx_destination = tx
        .is_field_present(sf("sfDestination"))
        .then(|| tx.get_account_id(sf("sfDestination")));
    let tx_holder = tx
        .is_field_present(sf("sfHolder"))
        .then(|| tx.get_account_id(sf("sfHolder")));
    let tx_amount = tx
        .is_field_present(sf("sfAmount"))
        .then(|| tx.get_field_amount(sf("sfAmount")));
    let tx_has_holder = tx.is_field_present(sf("sfHolder"));
    let cross_currency_payment = payment_is_cross_currency(tx);
    check_invariants_inner(
        sandbox,
        txn_type,
        tx_domain,
        tx_account,
        tx_destination,
        tx_holder,
        tx_amount,
        tx_has_holder,
        cross_currency_payment,
        result,
        fee,
    )
}

pub fn check_invariants<V: ApplyView>(
    sandbox: &FlowSandbox<V>,
    txn_type: protocol::TxType,
    result: Ter,
    fee: XRPAmount,
) -> Ter {
    check_invariants_inner(
        sandbox, txn_type, None, None, None, None, None, false, false, result, fee,
    )
}

fn payment_is_cross_currency(tx: &STTx) -> bool {
    if tx.get_txn_type() != protocol::TxType::PAYMENT || !tx.is_field_present(sf("sfAmount")) {
        return false;
    }

    let amount = tx.get_field_amount(sf("sfAmount"));
    let send_max = if tx.is_field_present(sf("sfSendMax")) {
        tx.get_field_amount(sf("sfSendMax"))
    } else {
        amount.clone()
    };
    send_max.asset() != amount.asset()
}

fn check_invariants_inner<V: ApplyView>(
    sandbox: &FlowSandbox<V>,
    txn_type: protocol::TxType,
    tx_domain: Option<Uint256>,
    tx_account: Option<AccountID>,
    tx_destination: Option<AccountID>,
    tx_holder: Option<AccountID>,
    tx_amount: Option<STAmount>,
    tx_has_holder: bool,
    cross_currency_payment: bool,
    result: Ter,
    fee: XRPAmount,
) -> Ter {
    let mut xrp_balance_change: i64 = 0;
    let fix_cleanup_3_1_3 = sandbox
        .rules()
        .enabled(&protocol::feature_id("fixCleanup3_1_3"));
    let fix_cleanup_3_2_0 = sandbox
        .rules()
        .enabled(&protocol::feature_id("fixCleanup3_2_0"));
    let amm_invariant_enabled =
        fix_cleanup_3_2_0 || sandbox.rules().enabled(&protocol::fix_ammv1_3());
    let single_asset_vault_enabled = sandbox
        .rules()
        .enabled(&protocol::feature_id("SingleAssetVault"));
    let vault_invariant_enabled = fix_cleanup_3_2_0 || single_asset_vault_enabled;
    let lending_protocol_enabled = sandbox
        .rules()
        .enabled(&protocol::feature_id("LendingProtocol"));
    let mptokens_v2_enabled = sandbox.rules().enabled(&protocol::feature_id("MPTokensV2"));
    let mpt_transfer_invariant_enabled = fix_cleanup_3_2_0 || mptokens_v2_enabled;
    let permissioned_dex_invariant_enabled = fix_cleanup_3_2_0
        || sandbox
            .rules()
            .enabled(&protocol::feature_id("PermissionedDEX"));
    let mut directory_roots = BTreeSet::new();
    let mut mpt_accounting = BTreeMap::new();
    let mut mpt_transfers = BTreeMap::new();
    let mut mpt_issuance_lifecycle = MptIssuanceLifecycle::default();
    let mut permissioned_domain = PermissionedDomainState::default();
    let mut permissioned_dex = PermissionedDexState::default();
    let mut amm = AmmState::default();
    let mut vault = VaultState::default();
    let mut lending = LendingState::default();
    let mut clawback = ClawbackState::default();

    for (index, entry) in sandbox.items() {
        let is_delete = entry.action == Action::Erase;
        let after = if is_delete { None } else { Some(&entry.sle) };
        let before = sandbox
            .peek_parent(protocol::Keylet::new(
                after
                    .map(|a| a.get_type())
                    .unwrap_or_else(|| entry.sle.get_type()),
                *index,
            ))
            .ok()
            .flatten();

        let before_sle = before.as_deref();
        let after_sle = after.map(|s| &**s);

        // 4. LedgerEntryTypesMatch
        if let (Some(b), Some(a)) = (before_sle, after_sle) {
            if b.get_type() != a.get_type() {
                return Ter::TEC_INVARIANT_FAILED;
            }
        }

        // 2. AccountRootsNotDeleted
        if is_delete {
            let sle_to_delete = before_sle.unwrap_or(&*entry.sle);
            if sle_to_delete.get_type() == LedgerEntryType::AccountRoot {
                if txn_type != protocol::TxType::ACCOUNT_DELETE {
                    return Ter::TEC_INVARIANT_FAILED;
                }
            }
        }

        let sle_type = after_sle
            .map(|s| s.get_type())
            .unwrap_or_else(|| before_sle.unwrap_or(&*entry.sle).get_type());

        if amm_invariant_enabled {
            record_amm_state(&mut amm, is_delete, before_sle, after_sle);
        }
        if vault_invariant_enabled {
            record_vault_state(&mut vault, is_delete, before_sle, after_sle);
        }
        if lending_protocol_enabled {
            record_lending_state(sandbox, &mut lending, after_sle);
        }
        if fix_cleanup_3_1_3 || txn_type == protocol::TxType::PERMISSIONED_DOMAIN_SET {
            record_permissioned_domain_state(
                &mut permissioned_domain,
                is_delete,
                before_sle,
                after_sle,
            );
        }

        if mpt_transfer_invariant_enabled {
            if let Some(b) = before_sle {
                record_mpt_accounting(&mut mpt_accounting, b, true);
                record_mpt_transfer(&mut mpt_transfers, b, true);
            }
            if let Some(a) = after_sle {
                record_mpt_accounting(&mut mpt_accounting, a, false);
                record_mpt_transfer(&mut mpt_transfers, a, false);
                if fix_cleanup_3_2_0 && protocol::has_invalid_amount(&a.clone_as_object()) {
                    return Ter::TEC_INVARIANT_FAILED;
                }
            }
        }

        if permissioned_dex_invariant_enabled {
            record_permissioned_dex(&mut permissioned_dex, is_delete, before_sle, after_sle);
        }
        record_clawback_state(&mut clawback, before_sle);

        if fix_cleanup_3_2_0 {
            let deleted_sle = before_sle.unwrap_or(&entry.sle);
            record_mpt_issuance_lifecycle(
                sandbox,
                txn_type,
                &mut mpt_issuance_lifecycle,
                is_delete,
                before_sle,
                after_sle,
                deleted_sle,
            );
            if !maybe_record_directory_root(&mut directory_roots, is_delete, before_sle, after_sle)
            {
                return Ter::TEC_INVARIANT_FAILED;
            }
        }

        match sle_type {
            LedgerEntryType::AccountRoot => {
                // 8. XRPBalanceChecks
                if let Some(a) = after_sle {
                    let balance_field = get_field_by_symbol("sfBalance");
                    if a.is_field_present(balance_field) {
                        let bal = a.get_field_amount(balance_field);
                        if bal.negative() || bal.xrp().drops() > protocol::INITIAL_XRP.drops() {
                            return Ter::TEC_INVARIANT_FAILED;
                        }
                    }
                }

                // 7. ValidNewAccountRoot
                // when DeletableAccounts is enabled (always on testnet/mainnet).
                if entry.action == Action::Insert {
                    if let Some(a) = after_sle {
                        let seq = a.get_field_u32(get_field_by_symbol("sfSequence"));
                        let expected_seq = sandbox.header().seq;
                        if seq != expected_seq && seq != 0 {
                            return Ter::TEC_INVARIANT_FAILED;
                        }
                    }
                }

                // 1. XRPNotCreated (AccountRoot)
                let bal_before = before_sle
                    .map(|b| {
                        b.get_field_amount(get_field_by_symbol("sfBalance"))
                            .xrp()
                            .drops() as i64
                    })
                    .unwrap_or(0);
                let bal_after = after_sle
                    .map(|a| {
                        a.get_field_amount(get_field_by_symbol("sfBalance"))
                            .xrp()
                            .drops() as i64
                    })
                    .unwrap_or(0);
                xrp_balance_change += bal_after - bal_before;
            }
            LedgerEntryType::Escrow => {
                // 6. NoZeroEscrow
                if let Some(a) = after_sle {
                    let amt = a.get_field_amount(get_field_by_symbol("sfAmount"));
                    if amt.signum() <= 0 {
                        return Ter::TEC_INVARIANT_FAILED;
                    }
                }

                // 1. XRPNotCreated (Escrow). Token escrows are covered by
                // token-specific accounting; only native amounts affect XRP.
                let bal_before = before_sle
                    .map(|b| b.get_field_amount(get_field_by_symbol("sfAmount")))
                    .filter(|amount| amount.native())
                    .map(|amount| amount.xrp().drops() as i64)
                    .unwrap_or(0);
                let bal_after = after_sle
                    .map(|a| a.get_field_amount(get_field_by_symbol("sfAmount")))
                    .filter(|amount| amount.native())
                    .map(|amount| amount.xrp().drops() as i64)
                    .unwrap_or(0);
                xrp_balance_change += bal_after - bal_before;
            }
            LedgerEntryType::PayChannel => {
                // 1. XRPNotCreated (PayChannel)
                let bal_before = before_sle
                    .map(|b| {
                        b.get_field_amount(get_field_by_symbol("sfAmount"))
                            .xrp()
                            .drops() as i64
                    })
                    .unwrap_or(0);
                let bal_after = after_sle
                    .map(|a| {
                        a.get_field_amount(get_field_by_symbol("sfAmount"))
                            .xrp()
                            .drops() as i64
                    })
                    .unwrap_or(0);
                xrp_balance_change += bal_after - bal_before;
            }
            LedgerEntryType::Offer => {
                // 5. NoBadOffers
                if let Some(a) = after_sle {
                    let gets = a.get_field_amount(get_field_by_symbol("sfTakerGets"));
                    let pays = a.get_field_amount(get_field_by_symbol("sfTakerPays"));
                    if gets.negative()
                        || gets.mantissa() == 0
                        || pays.negative()
                        || pays.mantissa() == 0
                    {
                        return Ter::TEC_INVARIANT_FAILED;
                    }
                }
            }
            LedgerEntryType::DirectoryNode => {}
            LedgerEntryType::RippleState => {
                if let Some(a) = after_sle
                    && !validate_ripple_state_entry(a)
                {
                    return Ter::TEC_INVARIANT_FAILED;
                }
            }
            LedgerEntryType::MPTokenIssuance | LedgerEntryType::MPToken => {
                if fix_cleanup_3_2_0
                    && let Some(a) = after_sle
                    && !validate_mpt_entry(a)
                {
                    return Ter::TEC_INVARIANT_FAILED;
                }
            }
            LedgerEntryType::Vault => {
                if vault_invariant_enabled
                    && let Some(a) = after_sle
                    && !validate_vault_entry(a)
                {
                    return Ter::TEC_INVARIANT_FAILED;
                }
            }
            LedgerEntryType::AMM => {
                if amm_invariant_enabled
                    && let Some(a) = after_sle
                    && !validate_amm_entry(a)
                {
                    return Ter::TEC_INVARIANT_FAILED;
                }
            }
            LedgerEntryType::Loan => {
                if lending_protocol_enabled
                    && let Some(a) = after_sle
                    && !validate_loan_entry(before_sle, a)
                {
                    return Ter::TEC_INVARIANT_FAILED;
                }
            }
            LedgerEntryType::LoanBroker => {
                if lending_protocol_enabled
                    && let Some(a) = after_sle
                    && !validate_loan_broker_entry(
                        sandbox,
                        txn_type,
                        fix_cleanup_3_1_3,
                        before_sle,
                        a,
                    )
                {
                    return Ter::TEC_INVARIANT_FAILED;
                }
            }
            _ => {}
        }
    }

    if (fix_cleanup_3_1_3 || txn_type == protocol::TxType::PERMISSIONED_DOMAIN_SET)
        && !validates_permissioned_domain(txn_type, result, fix_cleanup_3_1_3, &permissioned_domain)
    {
        return Ter::TEC_INVARIANT_FAILED;
    }

    if permissioned_dex_invariant_enabled {
        if !validates_permissioned_dex(
            sandbox,
            txn_type,
            result,
            tx_domain,
            fix_cleanup_3_1_3,
            fix_cleanup_3_2_0,
            &permissioned_dex,
        ) {
            return Ter::TEC_INVARIANT_FAILED;
        }
    }

    if !validates_clawback(
        sandbox,
        txn_type,
        result,
        tx_account,
        tx_holder,
        tx_amount.as_ref(),
        mptokens_v2_enabled,
        &clawback,
    ) {
        return Ter::TEC_INVARIANT_FAILED;
    }

    if fix_cleanup_3_2_0 {
        if !validates_mpt_issuance_lifecycle(&mpt_issuance_lifecycle) {
            return Ter::TEC_INVARIANT_FAILED;
        }
        if !validates_mpt_lifecycle_counts(
            txn_type,
            result,
            tx_has_holder,
            single_asset_vault_enabled,
            lending_protocol_enabled,
            mptokens_v2_enabled,
            &mpt_issuance_lifecycle,
        ) {
            return Ter::TEC_INVARIANT_FAILED;
        }
        for root_index in directory_roots {
            if !matches!(
                sandbox.read(protocol::Keylet::new(
                    LedgerEntryType::DirectoryNode,
                    root_index
                )),
                Ok(Some(_))
            ) {
                return Ter::TEC_INVARIANT_FAILED;
            }
        }
    }

    if mpt_transfer_invariant_enabled {
        if !validates_mpt_accounting(&mpt_accounting, mptokens_v2_enabled) {
            return Ter::TEC_INVARIANT_FAILED;
        }
        if !validates_mpt_transfers(
            sandbox,
            txn_type,
            cross_currency_payment,
            fix_cleanup_3_2_0,
            mptokens_v2_enabled,
            &mpt_transfers,
        ) {
            return Ter::TEC_INVARIANT_FAILED;
        }
    }

    if amm_invariant_enabled && !validates_amm_state(sandbox, txn_type, result, &amm) {
        return Ter::TEC_INVARIANT_FAILED;
    }

    if vault_invariant_enabled
        && !validates_vault_state(
            sandbox,
            txn_type,
            tx_account,
            tx_destination,
            tx_holder,
            tx_amount.as_ref(),
            fix_cleanup_3_2_0,
            result,
            fee,
            &vault,
        )
    {
        return Ter::TEC_INVARIANT_FAILED;
    }

    if lending_protocol_enabled {
        for broker_id in lending.broker_refs {
            if !matches!(
                sandbox.read(protocol::loan_broker_keylet_from_key(broker_id)),
                Ok(Some(_))
            ) {
                return Ter::TEC_INVARIANT_FAILED;
            }
        }
    }

    // 1. XRPNotCreated (finalize)
    // Since our sandbox does not contain the fee deduction (it's applied to the parent view),
    // the net XRP change inside the sandbox MUST be <= 0.
    if xrp_balance_change > 0 {
        return Ter::TEC_INVARIANT_FAILED;
    }

    // 3. TransactionFeeCheck
    if fee.drops() < 0 || fee.drops() > protocol::INITIAL_XRP.drops() {
        return Ter::TEC_INVARIANT_FAILED;
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(byte: u8) -> AccountID {
        AccountID::from_array([byte; 20])
    }

    fn usd_asset() -> Asset {
        Asset::Issue(Issue {
            currency: protocol::currency_from_string("USD"),
            account: account(0xA1),
        })
    }

    fn vault_snapshot_with_scale(scale: Option<i32>) -> VaultSnapshot {
        VaultSnapshot {
            key: Uint256::from_u64(1),
            asset: usd_asset(),
            pseudo_id: account(0xA2),
            share_mpt_id: protocol::MPTIssue::new(protocol::make_mpt_id(1, account(0xA2))).mpt_id(),
            scale,
            assets_total: RuntimeNumber::from_i64(1),
            assets_available: RuntimeNumber::from_i64(1),
            loss_unrealized: RuntimeNumber::zero(),
        }
    }

    #[test]
    fn vault_invariant_min_scale_prefers_explicit_vault_scale_after_cleanup_3_2_0() {
        let before = vault_snapshot_with_scale(Some(-2));
        let after = vault_snapshot_with_scale(Some(-2));
        let delta = VaultAssetDelta {
            delta: RuntimeNumber::try_from_external_parts(12345, -4, get_mantissa_scale())
                .expect("valid delta"),
            scale: Some(-4),
        };

        assert_eq!(compute_vault_min_scale(&before, &after, delta, true), -2);
        assert_eq!(
            rounded_vault_delta(after.asset, delta, -2),
            RuntimeNumber::try_from_external_parts(123, -2, get_mantissa_scale())
                .expect("vault-scale rounded delta")
        );
    }

    #[test]
    fn vault_invariant_min_scale_preserves_legacy_coarsest_scale_before_cleanup_3_2_0() {
        let before = vault_snapshot_with_scale(Some(-2));
        let mut after = vault_snapshot_with_scale(Some(-2));
        after.assets_total =
            RuntimeNumber::try_from_external_parts(10001, -4, get_mantissa_scale())
                .expect("valid total");
        after.assets_available = after.assets_total;
        let delta = VaultAssetDelta {
            delta: RuntimeNumber::try_from_external_parts(1, -4, get_mantissa_scale())
                .expect("valid delta"),
            scale: Some(-4),
        };

        assert_eq!(compute_vault_min_scale(&before, &after, delta, false), -4);
    }
}
