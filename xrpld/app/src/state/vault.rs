use std::sync::Arc;

use basics::{
    base_uint::{Uint160, Uint192, Uint256},
    number::{MantissaScale, NumberParts as RuntimeNumber, RoundingMode, get_mantissa_scale},
};
use ledger::{
    ApplyView, account_root_helpers::create_pseudo_account, adjust_owner_count,
    amm_helpers::stamount_as_number, dir_append, dir_remove,
};
use protocol::{
    AccountID, Asset, LedgerEntryType, MPTIssue, STAmount, STLedgerEntry, STNumber, STTx,
    SerializedTypeId, Ter, VAULT_DEFAULT_IOU_SCALE, XRPAmount, account_keylet, associate_asset,
    feature_id, get_field_by_symbol, mpt_issuance_keylet, mpt_issuance_keylet_from_mptid,
    mptoken_keylet_from_mptid, owner_dir_keylet, to_amount_from_number,
};
use tx::{
    MPT_CAN_ESCROW_FLAG, MPT_CAN_TRADE_FLAG, MPT_CAN_TRANSFER_FLAG, MPT_REQUIRE_AUTH_FLAG,
    VAULT_PRIVATE_FLAG, VAULT_SHARE_NON_TRANSFERABLE_FLAG, VaultClawbackPreflightFacts,
    VaultCreatePreflightFacts, VaultDeletePreflightFacts, VaultDepositPreflightFacts,
    VaultSetPreflightFacts, VaultWithdrawPreflightFacts, run_vault_clawback_preflight,
    run_vault_create_preflight, run_vault_delete_preflight, run_vault_deposit_preflight,
    run_vault_set_preflight, run_vault_withdraw_preflight,
};

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

fn to_160(account: &AccountID) -> Uint160 {
    Uint160::from_void(account.data())
}

fn amount_number(amount: &STAmount) -> RuntimeNumber {
    stamount_as_number(amount)
}

fn asset_number(asset: Asset, value: RuntimeNumber) -> STNumber {
    let mut number = STNumber::from(value);
    number.associate_asset(asset);
    number
}

fn zero_amount(asset: Asset) -> STAmount {
    to_amount_from_number(
        asset,
        RuntimeNumber::zero(),
        basics::number::RoundingMode::TowardsZero,
    )
    .expect("zero amount should be representable")
}

fn tx_asset_field(sttx: &STTx, field: &'static protocol::SField) -> Asset {
    match sttx.peek_at_pfield(field).map(|value| value.stype()) {
        Some(SerializedTypeId::Issue) => sttx.get_field_issue(field).asset(),
        Some(SerializedTypeId::Amount) => sttx.get_field_amount(field).asset(),
        _ => sttx.get_field_issue(field).asset(),
    }
}

fn feature_enabled<V: ApplyView>(view: &V, name: &str) -> bool {
    view.rules().enabled(&feature_id(name))
}

fn data_len(sttx: &STTx, field: &'static protocol::SField) -> Option<usize> {
    sttx.is_field_present(field)
        .then(|| sttx.get_field_vl(field).len())
}

fn tx_number_value_field(sttx: &STTx, field: &'static protocol::SField) -> Option<RuntimeNumber> {
    match sttx.peek_at_pfield(field).map(|value| value.stype()) {
        Some(SerializedTypeId::Number) => Some(sttx.get_field_number(field).value()),
        Some(SerializedTypeId::Amount) => Some(amount_number(&sttx.get_field_amount(field))),
        _ => None,
    }
}

fn vault_create_preflight<V: ApplyView>(view: &V, sttx: &STTx) -> Ter {
    if !feature_enabled(view, "SingleAssetVault") || !feature_enabled(view, "MPTokensV1") {
        return Ter::TEM_DISABLED;
    }

    let domain_id_present = sttx.is_field_present(sf("sfDomainID"));
    if domain_id_present && !feature_enabled(view, "PermissionedDomains") {
        return Ter::TEM_DISABLED;
    }

    let asset = tx_asset_field(sttx, sf("sfAsset"));
    let facts = VaultCreatePreflightFacts {
        data_len: data_len(sttx, sf("sfData")),
        withdrawal_policy: sttx
            .is_field_present(sf("sfWithdrawalPolicy"))
            .then(|| sttx.get_field_u8(sf("sfWithdrawalPolicy"))),
        domain_id_present,
        domain_id_is_zero: domain_id_present && sttx.get_field_h256(sf("sfDomainID")).is_zero(),
        is_private: sttx.get_field_u32(sf("sfFlags")) & VAULT_PRIVATE_FLAG != 0,
        assets_maximum_is_negative: tx_number_value_field(sttx, sf("sfAssetsMaximum"))
            .is_some_and(|value| value < RuntimeNumber::zero()),
        mptoken_metadata_len: data_len(sttx, sf("sfMPTokenMetadata")),
        scale: sttx
            .is_field_present(sf("sfScale"))
            .then(|| sttx.get_field_u8(sf("sfScale"))),
        asset_is_mpt: matches!(asset, Asset::MPTIssue(_)),
        asset_is_native: asset.native(),
    };
    run_vault_create_preflight(facts)
}

fn vault_set_preflight<V: ApplyView>(view: &V, sttx: &STTx) -> Ter {
    if !feature_enabled(view, "SingleAssetVault") {
        return Ter::TEM_DISABLED;
    }

    if sttx.is_field_present(sf("sfDomainID")) && !feature_enabled(view, "PermissionedDomains") {
        return Ter::TEM_DISABLED;
    }

    let facts = VaultSetPreflightFacts {
        vault_id_is_zero: sttx.get_field_h256(sf("sfVaultID")).is_zero(),
        data_len: data_len(sttx, sf("sfData")),
        assets_maximum_is_negative: tx_number_value_field(sttx, sf("sfAssetsMaximum"))
            .is_some_and(|value| value < RuntimeNumber::zero()),
        domain_id_present: sttx.is_field_present(sf("sfDomainID")),
        assets_maximum_present: sttx.is_field_present(sf("sfAssetsMaximum")),
        data_present: sttx.is_field_present(sf("sfData")),
    };
    run_vault_set_preflight(facts)
}

fn vault_delete_preflight<V: ApplyView>(view: &V, sttx: &STTx) -> Ter {
    if !feature_enabled(view, "SingleAssetVault") {
        return Ter::TEM_DISABLED;
    }

    run_vault_delete_preflight(VaultDeletePreflightFacts {
        vault_id_is_zero: sttx.get_field_h256(sf("sfVaultID")).is_zero(),
        has_memo_data: sttx.is_field_present(sf("sfMemoData")),
        lending_protocol_v1_1_enabled: feature_enabled(view, "LendingProtocolV1_1"),
        memo_data_length_valid: !sttx.is_field_present(sf("sfMemoData"))
            || data_len(sttx, sf("sfMemoData"))
                .map_or(true, |len| len <= tx::VAULT_DELETE_MAX_DATA_PAYLOAD_LENGTH),
    })
}

fn vault_deposit_preflight<V: ApplyView>(view: &V, sttx: &STTx) -> Ter {
    if !feature_enabled(view, "SingleAssetVault") {
        return Ter::TEM_DISABLED;
    }

    run_vault_deposit_preflight(VaultDepositPreflightFacts {
        vault_id_is_zero: sttx.get_field_h256(sf("sfVaultID")).is_zero(),
        amount_is_positive: sttx.get_field_amount(sf("sfAmount")).signum() > 0,
    })
}

fn vault_withdraw_preflight<V: ApplyView>(view: &V, sttx: &STTx) -> Ter {
    if !feature_enabled(view, "SingleAssetVault") {
        return Ter::TEM_DISABLED;
    }

    let destination_present = sttx.is_field_present(sf("sfDestination"));
    run_vault_withdraw_preflight(VaultWithdrawPreflightFacts {
        vault_id_is_zero: sttx.get_field_h256(sf("sfVaultID")).is_zero(),
        amount_is_positive: sttx.get_field_amount(sf("sfAmount")).signum() > 0,
        destination_present,
        destination_is_zero: destination_present
            && sttx.get_account_id(sf("sfDestination")).is_zero(),
    })
}

fn vault_clawback_preflight<V: ApplyView>(view: &V, sttx: &STTx) -> Ter {
    if !feature_enabled(view, "SingleAssetVault") {
        return Ter::TEM_DISABLED;
    }

    let amount_present = sttx.is_field_present(sf("sfAmount"));
    let amount = amount_present.then(|| sttx.get_field_amount(sf("sfAmount")));
    run_vault_clawback_preflight(VaultClawbackPreflightFacts {
        vault_id_is_zero: sttx.get_field_h256(sf("sfVaultID")).is_zero(),
        amount_present,
        amount_is_negative: amount.as_ref().is_some_and(STAmount::negative),
        amount_asset_is_xrp: amount
            .as_ref()
            .is_some_and(|amount| amount.asset().native()),
    })
}

fn runtime_to_amount(asset: Asset, value: RuntimeNumber) -> Option<STAmount> {
    to_amount_from_number(asset, value, basics::number::RoundingMode::TowardsZero).ok()
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

fn amount_is_zero_at_scale(asset: Asset, amount: &STAmount, scale: i32) -> bool {
    amount.signum() == 0
        || round_number_to_asset_with_scale(
            asset,
            amount_number(amount),
            scale,
            RoundingMode::ToNearest,
        ) == RuntimeNumber::zero()
}

fn vault_deposit_amount_at_scale(
    vault: &LoadedVault,
    amount: &STAmount,
    fix_cleanup_3_2_0: bool,
) -> Option<STAmount> {
    if !fix_cleanup_3_2_0 || amount.integral() {
        return Some(amount.clone());
    }

    let posterior_total =
        round_number_to_asset(vault.asset, vault.assets_total + amount_number(amount));
    let scale = vault
        .asset
        .amount(posterior_total)
        .map(|amount| amount.exponent())
        .unwrap_or(0);
    let rounded = round_number_to_asset_with_scale(
        vault.asset,
        amount_number(amount),
        scale,
        RoundingMode::Downward,
    );
    runtime_to_amount(vault.asset, rounded)
}

fn number_to_mpt_units_truncated(value: RuntimeNumber) -> Option<u64> {
    value
        .truncate(MantissaScale::Large)
        .try_to_i64()
        .ok()
        .and_then(|value| u64::try_from(value).ok())
}

fn vault_scale(vault: &LoadedVault) -> i32 {
    if vault.entry.is_field_present(sf("sfScale")) {
        vault.entry.get_field_u8(sf("sfScale")) as i32
    } else {
        0
    }
}

fn mpt_id_for(account: &AccountID, sequence: u32) -> Uint192 {
    let mut bytes = [0u8; 24];
    bytes[..4].copy_from_slice(&sequence.to_be_bytes());
    bytes[4..].copy_from_slice(account.data());
    Uint192::from_slice(&bytes).expect("MPT issuance id width")
}

fn share_asset(mpt_id: Uint192) -> Asset {
    Asset::MPTIssue(MPTIssue::new(mpt_id))
}

fn read_balance_drops<V: ApplyView>(view: &mut V, account: &AccountID) -> Option<XRPAmount> {
    view.peek(account_keylet(to_160(account)))
        .ok()
        .flatten()
        .map(|sle| sle.get_field_amount(sf("sfBalance")).xrp())
}

fn ensure_holding<V: ApplyView>(view: &mut V, account: &AccountID, asset: Asset) -> Ter {
    let prior = read_balance_drops(view, account).unwrap_or_default();
    ledger::add_empty_holding(view, account, prior, &asset)
}

fn load_vault<V: ApplyView>(view: &mut V, vault_id: Uint256) -> Option<LoadedVault> {
    let sle = view
        .peek(protocol::vault_keylet_from_key(vault_id))
        .ok()
        .flatten()?;
    let asset = sle.get_field_issue(sf("sfAsset")).asset();
    Some(LoadedVault {
        key: *sle.key(),
        entry: (*sle).clone(),
        asset,
        owner: sle.get_account_id(sf("sfOwner")),
        pseudo: sle.get_account_id(sf("sfAccount")),
        share_id: sle.get_field_h192(sf("sfShareMPTID")),
        assets_total: sle.get_field_number(sf("sfAssetsTotal")).value(),
        assets_available: sle.get_field_number(sf("sfAssetsAvailable")).value(),
        loss_unrealized: sle.get_field_number(sf("sfLossUnrealized")).value(),
    })
}

fn persist_vault<V: ApplyView>(view: &mut V, vault: &mut LoadedVault) -> Ter {
    vault.entry.set_field_number(
        sf("sfAssetsTotal"),
        asset_number(vault.asset, vault.assets_total),
    );
    vault.entry.set_field_number(
        sf("sfAssetsAvailable"),
        asset_number(vault.asset, vault.assets_available),
    );
    vault.entry.set_field_number(
        sf("sfLossUnrealized"),
        asset_number(vault.asset, vault.loss_unrealized),
    );
    associate_asset(&mut vault.entry, vault.asset);
    view.update(Arc::new(vault.entry.clone()))
        .map(|_| Ter::TES_SUCCESS)
        .unwrap_or(Ter::TEF_BAD_LEDGER)
}

fn load_issuance<V: ApplyView>(view: &mut V, mpt_id: Uint192) -> Option<LoadedIssuance> {
    let sle = view
        .peek(mpt_issuance_keylet_from_mptid(mpt_id))
        .ok()
        .flatten()?;
    Some(LoadedIssuance {
        key: *sle.key(),
        entry: (*sle).clone(),
        issuer: sle.get_account_id(sf("sfIssuer")),
        sequence: sle.get_field_u32(sf("sfSequence")),
        owner_node: sle.get_field_u64(sf("sfOwnerNode")),
        outstanding_amount: sle.get_field_u64(sf("sfOutstandingAmount")),
    })
}

fn persist_issuance<V: ApplyView>(view: &mut V, issuance: &LoadedIssuance) -> Ter {
    view.update(Arc::new(issuance.entry.clone()))
        .map(|_| Ter::TES_SUCCESS)
        .unwrap_or(Ter::TEF_BAD_LEDGER)
}

fn token_balance<V: ApplyView>(view: &mut V, mpt_id: Uint192, account: &AccountID) -> Option<u64> {
    view.peek(mptoken_keylet_from_mptid(mpt_id, to_160(account)))
        .ok()
        .flatten()
        .map(|sle| sle.get_field_u64(sf("sfMPTAmount")))
}

fn set_token_balance<V: ApplyView>(
    view: &mut V,
    mpt_id: Uint192,
    account: &AccountID,
    balance: u64,
) -> Ter {
    let keylet = mptoken_keylet_from_mptid(mpt_id, to_160(account));
    let Ok(Some(sle)) = view.peek(keylet) else {
        return Ter::TEF_BAD_LEDGER;
    };
    let mut obj = sle.clone_as_object();
    obj.set_field_u64(sf("sfMPTAmount"), balance);
    view.update(Arc::new(STLedgerEntry::from_stobject(obj, *sle.key())))
        .map(|_| Ter::TES_SUCCESS)
        .unwrap_or(Ter::TEF_BAD_LEDGER)
}

fn transfer_mpt<V: ApplyView>(
    view: &mut V,
    mpt_id: Uint192,
    from: &AccountID,
    to: &AccountID,
    amount: u64,
) -> Ter {
    if amount == 0 || from == to {
        return Ter::TES_SUCCESS;
    }

    let Some(mut issuance) = load_issuance(view, mpt_id) else {
        return Ter::TEF_BAD_LEDGER;
    };
    let issuer = issuance.issuer;

    if *from != issuer {
        let Some(balance) = token_balance(view, mpt_id, from) else {
            return Ter::TEF_BAD_LEDGER;
        };
        if balance < amount {
            return Ter::TEC_INSUFFICIENT_FUNDS;
        }
        let ter = set_token_balance(view, mpt_id, from, balance - amount);
        if ter != Ter::TES_SUCCESS {
            return ter;
        }
    }

    if *to != issuer {
        let ter = ensure_holding(view, to, share_asset(mpt_id));
        if ter != Ter::TES_SUCCESS && ter != Ter::TEC_DUPLICATE {
            return ter;
        }
        let Some(balance) = token_balance(view, mpt_id, to) else {
            return Ter::TEF_BAD_LEDGER;
        };
        let Some(next) = balance.checked_add(amount) else {
            return Ter::TEF_INTERNAL;
        };
        let ter = set_token_balance(view, mpt_id, to, next);
        if ter != Ter::TES_SUCCESS {
            return ter;
        }
    }

    match (*from == issuer, *to == issuer) {
        (true, false) => {
            issuance.outstanding_amount = issuance.outstanding_amount.saturating_add(amount)
        }
        (false, true) => {
            issuance.outstanding_amount = issuance.outstanding_amount.saturating_sub(amount)
        }
        _ => {}
    }
    issuance
        .entry
        .set_field_u64(sf("sfOutstandingAmount"), issuance.outstanding_amount);
    persist_issuance(view, &issuance)
}

fn account_send<V: ApplyView>(
    view: &mut V,
    from: &AccountID,
    to: &AccountID,
    amount: &STAmount,
) -> Ter {
    match amount.asset() {
        Asset::MPTIssue(issue) => transfer_mpt(
            view,
            issue.mpt_id(),
            from,
            to,
            amount.mpt().value().unsigned_abs(),
        ),
        Asset::Issue(issue) if issue.native() => transfer_xrp(view, from, to, amount),
        Asset::Issue(_) => transfer_iou_no_fee(view, from, to, amount),
    }
}

fn transfer_xrp<V: ApplyView>(
    view: &mut V,
    from: &AccountID,
    to: &AccountID,
    amount: &STAmount,
) -> Ter {
    let from_keylet = account_keylet(to_160(from));
    let to_keylet = account_keylet(to_160(to));
    let Ok(Some(from_sle)) = view.peek(from_keylet) else {
        return Ter::TEF_BAD_LEDGER;
    };
    let Ok(Some(to_sle)) = view.peek(to_keylet) else {
        return Ter::TEF_BAD_LEDGER;
    };
    let from_balance = from_sle.get_field_amount(sf("sfBalance")).xrp().drops();
    let to_balance = to_sle.get_field_amount(sf("sfBalance")).xrp().drops();
    let drops = amount.xrp().drops();
    if from_balance < drops {
        return Ter::TEC_INSUFFICIENT_FUNDS;
    }

    let mut from_obj = from_sle.clone_as_object();
    from_obj.set_field_amount(
        sf("sfBalance"),
        STAmount::from_xrp_amount(XRPAmount::from_drops(from_balance - drops)),
    );
    let mut to_obj = to_sle.clone_as_object();
    to_obj.set_field_amount(
        sf("sfBalance"),
        STAmount::from_xrp_amount(XRPAmount::from_drops(to_balance + drops)),
    );
    let _ = view.update(Arc::new(STLedgerEntry::from_stobject(
        from_obj,
        *from_sle.key(),
    )));
    let _ = view.update(Arc::new(STLedgerEntry::from_stobject(
        to_obj,
        *to_sle.key(),
    )));
    Ter::TES_SUCCESS
}

fn transfer_iou_no_fee<V: ApplyView>(
    view: &mut V,
    from: &AccountID,
    to: &AccountID,
    amount: &STAmount,
) -> Ter {
    if amount.signum() <= 0 || from == to {
        return Ter::TES_SUCCESS;
    }

    let issue = amount.issue();
    if *from == issue.account || *to == issue.account || issue.account.is_zero() {
        return ledger::ripple_state_helpers::direct_send_no_fee_iou_pub(view, from, to, amount);
    }

    let res =
        ledger::ripple_state_helpers::direct_send_no_fee_iou_pub(view, &issue.account, to, amount);
    if res != Ter::TES_SUCCESS {
        return res;
    }
    ledger::ripple_state_helpers::direct_send_no_fee_iou_pub(view, from, &issue.account, amount)
}

fn account_holds_vault_asset<V: ApplyView>(
    view: &mut V,
    account: &AccountID,
    asset: Asset,
) -> STAmount {
    let Asset::Issue(issue) = asset else {
        return zero_amount(asset);
    };
    if issue.native() {
        return read_balance_drops(view, account)
            .map(STAmount::from_xrp_amount)
            .unwrap_or_else(|| zero_amount(asset));
    }
    if issue.issuer() == *account {
        return asset
            .amount(RuntimeNumber::max(get_mantissa_scale()))
            .unwrap_or_else(|_| zero_amount(asset));
    }
    let mut amount = view
        .peek(protocol::line(*account, issue.issuer(), issue.currency))
        .ok()
        .flatten()
        .map(|line| line.get_field_amount(sf("sfBalance")))
        .unwrap_or_else(|| zero_amount(asset));
    if *account > issue.issuer() {
        amount.negate();
    }
    amount.set_issuer(issue.issuer());
    amount
}

fn account_holds_vault_asset_full_balance<V: ApplyView>(
    view: &mut V,
    account: &AccountID,
    asset: Asset,
) -> STAmount {
    let balance = account_holds_vault_asset(view, account, asset);
    let Asset::Issue(issue) = asset else {
        return balance;
    };
    if issue.native() || issue.issuer() == *account {
        return balance;
    }

    let Some(line) = view
        .peek(protocol::line(*account, issue.issuer(), issue.currency))
        .ok()
        .flatten()
    else {
        return balance;
    };
    let opposite_limit = if *account > issue.issuer() {
        line.get_field_amount(sf("sfLowLimit"))
    } else {
        line.get_field_amount(sf("sfHighLimit"))
    };
    runtime_to_amount(
        asset,
        amount_number(&balance) + amount_number(&opposite_limit),
    )
    .unwrap_or(balance)
}

fn assets_to_shares_deposit(
    vault: &LoadedVault,
    issuance: &LoadedIssuance,
    amount: &STAmount,
) -> Option<u64> {
    if amount.negative() || amount.asset() != vault.asset {
        return None;
    }
    let assets = amount_number(amount);
    let shares = if vault.assets_total == RuntimeNumber::zero() {
        RuntimeNumber::try_from_external_parts(
            amount.mantissa() as i64,
            amount.exponent() + vault_scale(vault),
            get_mantissa_scale(),
        )
        .ok()?
    } else {
        RuntimeNumber::from_i64(issuance.outstanding_amount as i64) * assets / vault.assets_total
    };
    number_to_mpt_units_truncated(shares)
}

fn shares_to_assets(
    vault: &LoadedVault,
    issuance: &LoadedIssuance,
    shares: u64,
    withdraw: bool,
    waive_unrealized_loss: bool,
) -> Option<STAmount> {
    let share_total = RuntimeNumber::from_i64(issuance.outstanding_amount as i64);
    let share_number = RuntimeNumber::from_i64(shares as i64);
    let asset_total = if withdraw {
        if waive_unrealized_loss {
            vault.assets_total
        } else {
            vault.assets_total - vault.loss_unrealized
        }
    } else {
        vault.assets_total
    };
    let amount = if asset_total == RuntimeNumber::zero() {
        RuntimeNumber::try_from_external_parts(
            shares as i64,
            -vault_scale(vault),
            get_mantissa_scale(),
        )
        .ok()?
    } else {
        asset_total * share_number / share_total
    };
    runtime_to_amount(vault.asset, amount)
}

fn assets_to_shares_withdraw(
    vault: &LoadedVault,
    issuance: &LoadedIssuance,
    amount: &STAmount,
    waive_unrealized_loss: bool,
) -> Option<u64> {
    if amount.negative() || amount.asset() != vault.asset {
        return None;
    }
    let asset_total = if waive_unrealized_loss {
        vault.assets_total
    } else {
        vault.assets_total - vault.loss_unrealized
    };
    if asset_total == RuntimeNumber::zero() {
        return Some(0);
    }
    let shares = RuntimeNumber::from_i64(issuance.outstanding_amount as i64)
        * amount_number(amount)
        / asset_total;
    shares.try_to_i64().ok().map(|value| value.unsigned_abs())
}

fn is_sole_shareholder<V: ApplyView>(
    view: &mut V,
    account: &AccountID,
    issuance: &LoadedIssuance,
) -> bool {
    if issuance.outstanding_amount == 0 {
        return false;
    }
    token_balance(
        view,
        mpt_id_for(&issuance.issuer, issuance.sequence),
        account,
    )
    .is_some_and(|balance| balance == issuance.outstanding_amount)
}

fn should_waive_withdrawal<V: ApplyView>(
    view: &mut V,
    account: &AccountID,
    issuance: &LoadedIssuance,
) -> bool {
    view.rules().enabled(&feature_id("fixCleanup3_2_0"))
        && is_sole_shareholder(view, account, issuance)
}

#[derive(Clone)]
struct LoadedVault {
    key: Uint256,
    entry: STLedgerEntry,
    asset: Asset,
    owner: AccountID,
    pseudo: AccountID,
    share_id: Uint192,
    assets_total: RuntimeNumber,
    assets_available: RuntimeNumber,
    loss_unrealized: RuntimeNumber,
}

#[derive(Clone)]
struct LoadedIssuance {
    key: Uint256,
    entry: STLedgerEntry,
    issuer: AccountID,
    sequence: u32,
    owner_node: u64,
    outstanding_amount: u64,
}

pub fn apply_vault_create<V: ApplyView>(view: &mut V, sttx: &STTx) -> Ter {
    let preflight = vault_create_preflight(view, sttx);
    if preflight != Ter::TES_SUCCESS {
        return preflight;
    }

    let owner = sttx.get_account_id(sf("sfAccount"));
    let sequence = sttx.get_seq_value();
    let asset = tx_asset_field(sttx, sf("sfAsset"));
    let keylet = protocol::vault_keylet(to_160(&owner), sequence);

    let pseudo = match create_pseudo_account(view, keylet.key, sf("sfVaultID")) {
        Ok(sle) => sle.get_account_id(sf("sfAccount")),
        Err(err) => return err,
    };

    let ter = ensure_holding(view, &pseudo, asset);
    if ter != Ter::TES_SUCCESS && ter != Ter::TEC_DUPLICATE {
        return ter;
    }

    let share_id = mpt_id_for(&pseudo, 1);
    let issuance_keylet = mpt_issuance_keylet(1, to_160(&pseudo));
    let issuance_page = match dir_append(
        view,
        &owner_dir_keylet(to_160(&pseudo)),
        issuance_keylet.key,
        &|_| {},
    ) {
        Ok(Some(page)) => page,
        _ => return Ter::TEF_BAD_LEDGER,
    };
    let mut issuance = STLedgerEntry::new(issuance_keylet);
    issuance.set_account_id(sf("sfIssuer"), pseudo);
    issuance.set_field_u32(sf("sfSequence"), 1);
    issuance.set_field_u64(sf("sfOutstandingAmount"), 0);
    issuance.set_field_u64(sf("sfOwnerNode"), issuance_page);
    let mut issuance_flags = 0;
    if sttx.get_field_u32(sf("sfFlags")) & VAULT_SHARE_NON_TRANSFERABLE_FLAG == 0 {
        issuance_flags |= MPT_CAN_ESCROW_FLAG | MPT_CAN_TRADE_FLAG | MPT_CAN_TRANSFER_FLAG;
    }
    if sttx.get_field_u32(sf("sfFlags")) & VAULT_PRIVATE_FLAG != 0 {
        issuance_flags |= MPT_REQUIRE_AUTH_FLAG;
    }
    issuance.set_field_u32(sf("sfFlags"), issuance_flags);
    if sttx.is_field_present(sf("sfMPTokenMetadata")) {
        issuance.set_field_vl(
            sf("sfMPTokenMetadata"),
            &sttx.get_field_vl(sf("sfMPTokenMetadata")),
        );
    }
    if sttx.is_field_present(sf("sfDomainID")) {
        issuance.set_field_h256(sf("sfDomainID"), sttx.get_field_h256(sf("sfDomainID")));
    }
    if view.rules().enabled(&feature_id("fixCleanup3_2_0")) && !asset.native() {
        let reference_holding = match asset {
            Asset::MPTIssue(issue) => {
                mptoken_keylet_from_mptid(issue.mpt_id(), to_160(&pseudo)).key
            }
            Asset::Issue(issue) => protocol::line(pseudo, issue.account, issue.currency).key,
        };
        issuance.set_field_h256(sf("sfReferenceHolding"), reference_holding);
    }
    let share_asset_scale = if asset.integral() {
        0
    } else if sttx.is_field_present(sf("sfScale")) {
        sttx.get_field_u8(sf("sfScale"))
    } else {
        VAULT_DEFAULT_IOU_SCALE
    };
    issuance.set_field_u8(sf("sfAssetScale"), share_asset_scale);
    let _ = view.insert(Arc::new(issuance));
    if let Ok(Some(pseudo_root)) = view.peek(account_keylet(to_160(&pseudo))) {
        let _ = adjust_owner_count(view, &pseudo_root, 1);
    }

    let ter = ensure_holding(view, &owner, share_asset(share_id));
    if ter != Ter::TES_SUCCESS && ter != Ter::TEC_DUPLICATE {
        return ter;
    }

    let owner_page = match dir_append(view, &owner_dir_keylet(to_160(&owner)), keylet.key, &|_| {})
    {
        Ok(Some(page)) => page,
        _ => return Ter::TEF_BAD_LEDGER,
    };

    let mut vault = STLedgerEntry::from_type_and_key(LedgerEntryType::Vault, keylet.key);
    vault.set_field_u32(
        sf("sfFlags"),
        sttx.get_field_u32(sf("sfFlags")) & VAULT_PRIVATE_FLAG,
    );
    vault.set_field_u32(sf("sfSequence"), sequence);
    vault.set_field_u64(sf("sfOwnerNode"), owner_page);
    vault.set_account_id(sf("sfOwner"), owner);
    vault.set_account_id(sf("sfAccount"), pseudo);
    vault.set_field_issue(
        sf("sfAsset"),
        protocol::STIssue::new_with_asset(sf("sfAsset"), asset),
    );
    vault.set_field_number(
        sf("sfAssetsTotal"),
        asset_number(asset, RuntimeNumber::zero()),
    );
    vault.set_field_number(
        sf("sfAssetsAvailable"),
        asset_number(asset, RuntimeNumber::zero()),
    );
    vault.set_field_number(
        sf("sfLossUnrealized"),
        asset_number(asset, RuntimeNumber::zero()),
    );
    if sttx.is_field_present(sf("sfAssetsMaximum")) {
        let Some(assets_maximum) = tx_number_value_field(sttx, sf("sfAssetsMaximum")) else {
            return Ter::TEM_MALFORMED;
        };
        vault.set_field_number(sf("sfAssetsMaximum"), asset_number(asset, assets_maximum));
    }
    vault.set_field_h192(sf("sfShareMPTID"), share_id);
    if sttx.is_field_present(sf("sfData")) {
        vault.set_field_vl(sf("sfData"), &sttx.get_field_vl(sf("sfData")));
    }
    vault.set_field_u8(
        sf("sfWithdrawalPolicy"),
        if sttx.is_field_present(sf("sfWithdrawalPolicy")) {
            sttx.get_field_u8(sf("sfWithdrawalPolicy"))
        } else {
            0
        },
    );
    if share_asset_scale != 0 {
        vault.set_field_u8(sf("sfScale"), share_asset_scale);
    }
    associate_asset(&mut vault, asset);
    let _ = view.insert(Arc::new(vault));

    if let Ok(Some(owner_root)) = view.peek(account_keylet(to_160(&owner))) {
        let _ = adjust_owner_count(view, &owner_root, 2);
    }

    Ter::TES_SUCCESS
}

pub fn apply_vault_set<V: ApplyView>(view: &mut V, sttx: &STTx) -> Ter {
    let preflight = vault_set_preflight(view, sttx);
    if preflight != Ter::TES_SUCCESS {
        return preflight;
    }

    let vault_id = sttx.get_field_h256(sf("sfVaultID"));
    let Some(mut vault) = load_vault(view, vault_id) else {
        return Ter::TEC_NO_ENTRY;
    };

    let tx_account = sttx.get_account_id(sf("sfAccount"));
    if tx_account != vault.owner {
        return Ter::TEC_NO_PERMISSION;
    }

    let mut issuance = if sttx.is_field_present(sf("sfDomainID")) {
        let Some(issuance) = load_issuance(view, vault.share_id) else {
            return Ter::TEF_INTERNAL;
        };
        if vault.entry.get_field_u32(sf("sfFlags")) & VAULT_PRIVATE_FLAG == 0 {
            return Ter::TEC_NO_PERMISSION;
        }
        if issuance.entry.get_field_u32(sf("sfFlags")) & MPT_REQUIRE_AUTH_FLAG == 0 {
            return Ter::TEF_INTERNAL;
        }
        let domain = sttx.get_field_h256(sf("sfDomainID"));
        if !domain.is_zero()
            && view
                .read(protocol::permissioned_domain_keylet_from_id(domain))
                .ok()
                .flatten()
                .is_none()
        {
            return Ter::TEC_OBJECT_NOT_FOUND;
        }
        Some(issuance)
    } else {
        None
    };

    if sttx.is_field_present(sf("sfData")) {
        vault
            .entry
            .set_field_vl(sf("sfData"), &sttx.get_field_vl(sf("sfData")));
    }
    if sttx.is_field_present(sf("sfAssetsMaximum")) {
        let Some(assets_maximum) = tx_number_value_field(sttx, sf("sfAssetsMaximum")) else {
            return Ter::TEM_MALFORMED;
        };
        if assets_maximum != RuntimeNumber::zero() && assets_maximum < vault.assets_total {
            return Ter::TEC_LIMIT_EXCEEDED;
        }
        vault.entry.set_field_number(
            sf("sfAssetsMaximum"),
            asset_number(vault.asset, assets_maximum),
        );
    }
    if sttx.is_field_present(sf("sfDomainID")) {
        let domain = sttx.get_field_h256(sf("sfDomainID"));
        let issuance = issuance.as_mut().expect("domain update loaded issuance");
        if domain.is_zero() {
            issuance.entry.make_field_absent(sf("sfDomainID"));
        } else {
            issuance.entry.set_field_h256(sf("sfDomainID"), domain);
        }
        let ter = persist_issuance(view, issuance);
        if ter != Ter::TES_SUCCESS {
            return ter;
        }
    }

    persist_vault(view, &mut vault)
}

pub fn apply_vault_delete<V: ApplyView>(view: &mut V, sttx: &STTx) -> Ter {
    let preflight = vault_delete_preflight(view, sttx);
    if preflight != Ter::TES_SUCCESS {
        return preflight;
    }

    let vault_id = sttx.get_field_h256(sf("sfVaultID"));
    let owner = sttx.get_account_id(sf("sfAccount"));
    let Some(vault) = load_vault(view, vault_id) else {
        return Ter::TEC_NO_ENTRY;
    };

    if owner != vault.owner {
        return Ter::TEC_NO_PERMISSION;
    }

    if vault.assets_available != RuntimeNumber::zero()
        || vault.assets_total != RuntimeNumber::zero()
    {
        return Ter::TEC_HAS_OBLIGATIONS;
    }

    let Some(issuance) = load_issuance(view, vault.share_id) else {
        return Ter::TEC_OBJECT_NOT_FOUND;
    };
    if issuance.issuer != vault.pseudo {
        return Ter::TEC_NO_PERMISSION;
    }
    if issuance.outstanding_amount != 0 {
        return Ter::TEC_HAS_OBLIGATIONS;
    }

    let ter = ledger::remove_empty_holding(view, &vault.pseudo, &vault.asset);
    if ter != Ter::TES_SUCCESS && ter != Ter::TEC_OBJECT_NOT_FOUND {
        return ter;
    }

    if token_balance(view, vault.share_id, &owner).is_some() {
        let ter = ledger::remove_empty_holding(view, &owner, &share_asset(vault.share_id));
        if ter != Ter::TES_SUCCESS && ter != Ter::TEC_OBJECT_NOT_FOUND {
            return ter;
        }
    }

    let _ = dir_remove(
        view,
        &owner_dir_keylet(to_160(&issuance.issuer)),
        issuance.owner_node,
        issuance.key,
        false,
    );
    let _ = view.erase(Arc::new(issuance.entry));
    if let Ok(Some(pseudo_root)) = view.peek(account_keylet(to_160(&vault.pseudo))) {
        let _ = adjust_owner_count(view, &pseudo_root, -1);
    }

    if let Ok(Some(pseudo_root)) = view.peek(account_keylet(to_160(&vault.pseudo))) {
        if pseudo_root.get_field_amount(sf("sfBalance")).xrp().drops() == 0
            && pseudo_root.get_field_u32(sf("sfOwnerCount")) == 0
        {
            let _ = view.erase(pseudo_root);
        }
    }

    let _ = dir_remove(
        view,
        &owner_dir_keylet(to_160(&vault.owner)),
        vault.entry.get_field_u64(sf("sfOwnerNode")),
        vault.key,
        false,
    );
    let _ = view.erase(Arc::new(vault.entry));
    if let Ok(Some(owner_root)) = view.peek(account_keylet(to_160(&vault.owner))) {
        let _ = adjust_owner_count(view, &owner_root, -2);
    }
    Ter::TES_SUCCESS
}

pub fn apply_vault_deposit<V: ApplyView>(view: &mut V, sttx: &STTx) -> Ter {
    let preflight = vault_deposit_preflight(view, sttx);
    if preflight != Ter::TES_SUCCESS {
        return preflight;
    }

    let vault_id = sttx.get_field_h256(sf("sfVaultID"));
    let account = sttx.get_account_id(sf("sfAccount"));
    let tx_amount = sttx.get_field_amount(sf("sfAmount"));
    let Some(mut vault) = load_vault(view, vault_id) else {
        return Ter::TEC_NO_ENTRY;
    };
    let Some(issuance) = load_issuance(view, vault.share_id) else {
        return Ter::TEF_BAD_LEDGER;
    };
    let Some(amount) = vault_deposit_amount_at_scale(
        &vault,
        &tx_amount,
        view.rules().enabled(&feature_id("fixCleanup3_2_0")),
    ) else {
        return Ter::TEC_INTERNAL;
    };
    if amount.signum() == 0 {
        return Ter::TEC_PRECISION_LOSS;
    }

    let Some(shares_created) = assets_to_shares_deposit(&vault, &issuance, &amount) else {
        return Ter::TEC_INTERNAL;
    };
    if shares_created == 0 {
        return Ter::TEC_PRECISION_LOSS;
    }

    if view.rules().enabled(&feature_id("fixCleanup3_2_0"))
        && !amount.integral()
        && account != amount.issue().issuer()
    {
        let balance = account_holds_vault_asset_full_balance(view, &account, vault.asset);
        if balance < amount {
            return Ter::TEC_INSUFFICIENT_FUNDS;
        }
        let trustline_balance = account_holds_vault_asset(view, &account, vault.asset);
        if amount_is_zero_at_scale(vault.asset, &tx_amount, trustline_balance.exponent()) {
            return Ter::TEC_PRECISION_LOSS;
        }
    }

    let Some(assets_deposited) = shares_to_assets(&vault, &issuance, shares_created, false, false)
    else {
        return Ter::TEC_INTERNAL;
    };
    if assets_deposited.signum() <= 0 || amount_number(&assets_deposited) > amount_number(&amount) {
        return Ter::TEC_INTERNAL;
    }

    let deposited_number = amount_number(&assets_deposited);
    if vault.entry.is_field_present(sf("sfAssetsMaximum")) {
        let maximum = vault.entry.get_field_number(sf("sfAssetsMaximum")).value();
        if maximum != RuntimeNumber::zero() && vault.assets_total + deposited_number > maximum {
            return Ter::TEC_LIMIT_EXCEEDED;
        }
    }

    let ter = ensure_holding(view, &account, share_asset(vault.share_id));
    if ter != Ter::TES_SUCCESS && ter != Ter::TEC_DUPLICATE {
        return ter;
    }

    let ter = account_send(view, &account, &vault.pseudo, &assets_deposited);
    if ter != Ter::TES_SUCCESS {
        return ter;
    }

    let ter = transfer_mpt(
        view,
        vault.share_id,
        &vault.pseudo,
        &account,
        shares_created,
    );
    if ter != Ter::TES_SUCCESS {
        return ter;
    }

    vault.assets_total += deposited_number;
    vault.assets_available += deposited_number;

    persist_vault(view, &mut vault)
}

pub fn apply_vault_withdraw<V: ApplyView>(view: &mut V, sttx: &STTx) -> Ter {
    let preflight = vault_withdraw_preflight(view, sttx);
    if preflight != Ter::TES_SUCCESS {
        return preflight;
    }

    let vault_id = sttx.get_field_h256(sf("sfVaultID"));
    let account = sttx.get_account_id(sf("sfAccount"));
    let destination = if sttx.is_field_present(sf("sfDestination")) {
        sttx.get_account_id(sf("sfDestination"))
    } else {
        account
    };
    let amount = sttx.get_field_amount(sf("sfAmount"));
    let Some(mut vault) = load_vault(view, vault_id) else {
        return Ter::TEC_NO_ENTRY;
    };
    let Some(issuance) = load_issuance(view, vault.share_id) else {
        return Ter::TEF_BAD_LEDGER;
    };
    let waive_unrealized_loss = should_waive_withdrawal(view, &account, &issuance);

    let (shares_redeemed, assets_withdrawn) = if amount.asset() == vault.asset {
        let Some(shares) =
            assets_to_shares_withdraw(&vault, &issuance, &amount, waive_unrealized_loss)
        else {
            return Ter::TEC_INTERNAL;
        };
        if shares == 0 {
            return Ter::TEC_PRECISION_LOSS;
        }
        let Some(assets) = shares_to_assets(&vault, &issuance, shares, true, waive_unrealized_loss)
        else {
            return Ter::TEC_INTERNAL;
        };
        (shares, assets)
    } else if amount.asset() == share_asset(vault.share_id) {
        let shares = amount.mpt().value().unsigned_abs();
        let Some(assets) = shares_to_assets(&vault, &issuance, shares, true, waive_unrealized_loss)
        else {
            return Ter::TEC_INTERNAL;
        };
        (shares, assets)
    } else {
        return Ter::TEF_INTERNAL;
    };

    if token_balance(view, vault.share_id, &account).unwrap_or_default() < shares_redeemed {
        return Ter::TEC_INSUFFICIENT_FUNDS;
    }
    let final_withdrawal = view.rules().enabled(&feature_id("fixCleanup3_2_0"))
        && shares_redeemed == issuance.outstanding_amount;
    let mut assets_withdrawn = assets_withdrawn;
    if vault.assets_available < amount_number(&assets_withdrawn) {
        return Ter::TEC_INSUFFICIENT_FUNDS;
    }
    if final_withdrawal {
        if vault.loss_unrealized != RuntimeNumber::zero() {
            return Ter::TEF_INTERNAL;
        }
        assets_withdrawn = runtime_to_amount(vault.asset, vault.assets_available)
            .unwrap_or_else(|| zero_amount(vault.asset));
    }

    let ter = transfer_mpt(
        view,
        vault.share_id,
        &account,
        &vault.pseudo,
        shares_redeemed,
    );
    if ter != Ter::TES_SUCCESS {
        return ter;
    }
    if account != vault.owner {
        let ter = ledger::remove_empty_holding(view, &account, &share_asset(vault.share_id));
        if ter != Ter::TES_SUCCESS
            && ter != Ter::TEC_HAS_OBLIGATIONS
            && ter != Ter::TEC_OBJECT_NOT_FOUND
        {
            return ter;
        }
    }

    let ter = account_send(view, &vault.pseudo, &destination, &assets_withdrawn);
    if ter != Ter::TES_SUCCESS {
        return ter;
    }

    let withdrawn_number = amount_number(&assets_withdrawn);
    if final_withdrawal {
        vault.assets_total = RuntimeNumber::zero();
        vault.assets_available = RuntimeNumber::zero();
    } else {
        vault.assets_total -= withdrawn_number;
        vault.assets_available -= withdrawn_number;
    }
    persist_vault(view, &mut vault)
}

pub fn apply_vault_clawback<V: ApplyView>(view: &mut V, sttx: &STTx) -> Ter {
    let preflight = vault_clawback_preflight(view, sttx);
    if preflight != Ter::TES_SUCCESS {
        return preflight;
    }

    let vault_id = sttx.get_field_h256(sf("sfVaultID"));
    let account = sttx.get_account_id(sf("sfAccount"));
    let holder = sttx.get_account_id(sf("sfHolder"));
    let Some(mut vault) = load_vault(view, vault_id) else {
        return Ter::TEF_BAD_LEDGER;
    };
    let Some(issuance) = load_issuance(view, vault.share_id) else {
        return Ter::TEF_BAD_LEDGER;
    };

    let (shares_destroyed, assets_recovered) = match sttx.is_field_present(sf("sfAmount")) {
        false if account == vault.owner => {
            let shares = token_balance(view, vault.share_id, &holder).unwrap_or_default();
            let assets = shares_to_assets(&vault, &issuance, shares, true, false)
                .unwrap_or_else(|| zero_amount(vault.asset));
            (shares, assets)
        }
        false => {
            let shares = token_balance(view, vault.share_id, &holder).unwrap_or_default();
            (shares, zero_amount(vault.asset))
        }
        true => {
            let amount = sttx.get_field_amount(sf("sfAmount"));
            if amount.asset() == share_asset(vault.share_id) && account == vault.owner {
                let shares = token_balance(view, vault.share_id, &holder).unwrap_or_default();
                (shares, zero_amount(vault.asset))
            } else {
                let Some(shares) = assets_to_shares_withdraw(&vault, &issuance, &amount, false)
                else {
                    return Ter::TEC_INTERNAL;
                };
                let Some(assets) = shares_to_assets(&vault, &issuance, shares, true, false) else {
                    return Ter::TEC_INTERNAL;
                };
                (shares, assets)
            }
        }
    };

    let ter = transfer_mpt(
        view,
        vault.share_id,
        &holder,
        &vault.pseudo,
        shares_destroyed,
    );
    if ter != Ter::TES_SUCCESS {
        return ter;
    }
    if holder != vault.owner {
        let ter = ledger::remove_empty_holding(view, &holder, &share_asset(vault.share_id));
        if ter != Ter::TES_SUCCESS
            && ter != Ter::TEC_HAS_OBLIGATIONS
            && ter != Ter::TEC_OBJECT_NOT_FOUND
        {
            return ter;
        }
    }

    if assets_recovered.signum() > 0 {
        let ter = account_send(view, &vault.pseudo, &account, &assets_recovered);
        if ter != Ter::TES_SUCCESS {
            return ter;
        }
        let recovered = amount_number(&assets_recovered);
        vault.assets_total -= recovered;
        // Clamp to assets_available to prevent underflow (C++ parity: fixCleanup3_1_3)
        let clamped = if recovered > vault.assets_available {
            vault.assets_available
        } else {
            recovered
        };
        vault.assets_available -= clamped;
    }

    persist_vault(view, &mut vault)
}
