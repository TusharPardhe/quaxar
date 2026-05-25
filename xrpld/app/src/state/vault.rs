use std::sync::Arc;

use basics::{
    base_uint::{Uint160, Uint192, Uint256},
    number::NumberParts as RuntimeNumber,
};
use ledger::{
    ApplyView, account_root_helpers::create_pseudo_account, adjust_owner_count,
    amm_helpers::stamount_as_number, dir_append, dir_remove,
};
use protocol::{
    AccountID, Asset, LedgerEntryType, MPTIssue, STAmount, STLedgerEntry, STNumber, STTx, Ter,
    XRPAmount, account_keylet, associate_asset, feature_id, get_field_by_symbol,
    mpt_issuance_keylet, mpt_issuance_keylet_from_mptid, mptoken_keylet_from_mptid,
    owner_dir_keylet, to_amount_from_number,
};
use tx::{
    MPT_CAN_ESCROW_FLAG, MPT_CAN_TRADE_FLAG, MPT_CAN_TRANSFER_FLAG, MPT_REQUIRE_AUTH_FLAG,
    VAULT_PRIVATE_FLAG, VAULT_SHARE_NON_TRANSFERABLE_FLAG,
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

fn runtime_to_amount(asset: Asset, value: RuntimeNumber) -> Option<STAmount> {
    to_amount_from_number(asset, value, basics::number::RoundingMode::TowardsZero).ok()
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
        Asset::Issue(_) => ledger::ripple_state_helpers::account_send(view, from, to, amount),
    }
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
        assets
    } else {
        RuntimeNumber::from_i64(issuance.outstanding_amount as i64) * assets / vault.assets_total
    };
    shares.try_to_i64().ok().map(|value| value.unsigned_abs())
}

fn shares_to_assets(
    vault: &LoadedVault,
    issuance: &LoadedIssuance,
    shares: u64,
    withdraw: bool,
) -> Option<STAmount> {
    let share_total = RuntimeNumber::from_i64(issuance.outstanding_amount as i64);
    let share_number = RuntimeNumber::from_i64(shares as i64);
    let asset_total = if withdraw {
        vault.assets_total - vault.loss_unrealized
    } else {
        vault.assets_total
    };
    let amount = if asset_total == RuntimeNumber::zero() {
        share_number
    } else {
        asset_total * share_number / share_total
    };
    runtime_to_amount(vault.asset, amount)
}

fn assets_to_shares_withdraw(
    vault: &LoadedVault,
    issuance: &LoadedIssuance,
    amount: &STAmount,
) -> Option<u64> {
    if amount.negative() || amount.asset() != vault.asset {
        return None;
    }
    let asset_total = vault.assets_total - vault.loss_unrealized;
    if asset_total == RuntimeNumber::zero() {
        return Some(0);
    }
    let shares = RuntimeNumber::from_i64(issuance.outstanding_amount as i64)
        * amount_number(amount)
        / asset_total;
    shares.try_to_i64().ok().map(|value| value.unsigned_abs())
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
    owner_node: u64,
    outstanding_amount: u64,
}

pub fn apply_vault_create<V: ApplyView>(view: &mut V, sttx: &STTx) -> Ter {
    if !view.rules().enabled(&feature_id("SingleAssetVault")) {
        return Ter::TEM_DISABLED;
    }

    let owner = sttx.get_account_id(sf("sfAccount"));
    let sequence = sttx.get_seq_value();
    let asset = sttx.get_field_amount(sf("sfAsset")).asset();
    let keylet = protocol::vault_keylet(to_160(&owner), sequence);

    let pseudo = match create_pseudo_account(view, keylet.key, sf("sfRegularKey")) {
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
        vault.set_field_number(
            sf("sfAssetsMaximum"),
            asset_number(
                asset,
                amount_number(&sttx.get_field_amount(sf("sfAssetsMaximum"))),
            ),
        );
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
    if sttx.is_field_present(sf("sfScale")) {
        vault.set_field_u8(sf("sfScale"), sttx.get_field_u8(sf("sfScale")));
    }
    associate_asset(&mut vault, asset);
    let _ = view.insert(Arc::new(vault));

    if let Ok(Some(owner_root)) = view.peek(account_keylet(to_160(&owner))) {
        let _ = adjust_owner_count(view, &owner_root, 1);
    }

    Ter::TES_SUCCESS
}

pub fn apply_vault_set<V: ApplyView>(view: &mut V, sttx: &STTx) -> Ter {
    if !view.rules().enabled(&feature_id("SingleAssetVault")) {
        return Ter::TEM_DISABLED;
    }

    let vault_id = sttx.get_field_h256(sf("sfVaultID"));
    let Some(mut vault) = load_vault(view, vault_id) else {
        return Ter::TEF_BAD_LEDGER;
    };

    let tx_account = sttx.get_account_id(sf("sfAccount"));
    if tx_account != vault.owner {
        return Ter::TEC_NO_PERMISSION;
    }

    if sttx.is_field_present(sf("sfData")) {
        vault
            .entry
            .set_field_vl(sf("sfData"), &sttx.get_field_vl(sf("sfData")));
    }
    if sttx.is_field_present(sf("sfAssetsMaximum")) {
        vault.entry.set_field_number(
            sf("sfAssetsMaximum"),
            asset_number(
                vault.asset,
                amount_number(&sttx.get_field_amount(sf("sfAssetsMaximum"))),
            ),
        );
    }
    if sttx.is_field_present(sf("sfDomainID")) {
        let Some(mut issuance) = load_issuance(view, vault.share_id) else {
            return Ter::TEF_BAD_LEDGER;
        };
        let domain = sttx.get_field_h256(sf("sfDomainID"));
        if domain.is_zero() {
            issuance.entry.make_field_absent(sf("sfDomainID"));
        } else {
            issuance.entry.set_field_h256(sf("sfDomainID"), domain);
        }
        let ter = persist_issuance(view, &issuance);
        if ter != Ter::TES_SUCCESS {
            return ter;
        }
    }

    persist_vault(view, &mut vault)
}

pub fn apply_vault_delete<V: ApplyView>(view: &mut V, sttx: &STTx) -> Ter {
    if !view.rules().enabled(&feature_id("SingleAssetVault")) {
        return Ter::TEM_DISABLED;
    }

    let vault_id = sttx.get_field_h256(sf("sfVaultID"));
    let owner = sttx.get_account_id(sf("sfAccount"));
    let Some(vault) = load_vault(view, vault_id) else {
        return Ter::TEF_BAD_LEDGER;
    };

    if owner != vault.owner {
        return Ter::TEC_NO_PERMISSION;
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

    if let Some(issuance) = load_issuance(view, vault.share_id) {
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
        let _ = adjust_owner_count(view, &owner_root, -1);
    }
    Ter::TES_SUCCESS
}

pub fn apply_vault_deposit<V: ApplyView>(view: &mut V, sttx: &STTx) -> Ter {
    if !view.rules().enabled(&feature_id("SingleAssetVault")) {
        return Ter::TEM_DISABLED;
    }

    let vault_id = sttx.get_field_h256(sf("sfVaultID"));
    let account = sttx.get_account_id(sf("sfAccount"));
    let amount = sttx.get_field_amount(sf("sfAmount"));
    let Some(mut vault) = load_vault(view, vault_id) else {
        return Ter::TEF_BAD_LEDGER;
    };
    let Some(issuance) = load_issuance(view, vault.share_id) else {
        return Ter::TEF_BAD_LEDGER;
    };

    let Some(shares_created) = assets_to_shares_deposit(&vault, &issuance, &amount) else {
        return Ter::TEC_INTERNAL;
    };
    let Some(assets_deposited) = shares_to_assets(&vault, &issuance, shares_created, false) else {
        return Ter::TEC_INTERNAL;
    };

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

    let deposited_number = amount_number(&assets_deposited);
    vault.assets_total += deposited_number;
    vault.assets_available += deposited_number;
    if vault.entry.is_field_present(sf("sfAssetsMaximum")) {
        let maximum = vault.entry.get_field_number(sf("sfAssetsMaximum")).value();
        let ter = persist_vault(view, &mut vault);
        if ter != Ter::TES_SUCCESS {
            return ter;
        }
        if maximum != RuntimeNumber::zero() && vault.assets_total > maximum {
            return Ter::TEC_LIMIT_EXCEEDED;
        }
        return Ter::TES_SUCCESS;
    }

    persist_vault(view, &mut vault)
}

pub fn apply_vault_withdraw<V: ApplyView>(view: &mut V, sttx: &STTx) -> Ter {
    if !view.rules().enabled(&feature_id("SingleAssetVault")) {
        return Ter::TEM_DISABLED;
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
        return Ter::TEF_BAD_LEDGER;
    };
    let Some(issuance) = load_issuance(view, vault.share_id) else {
        return Ter::TEF_BAD_LEDGER;
    };

    let (shares_redeemed, assets_withdrawn) = if amount.asset() == vault.asset {
        let Some(shares) = assets_to_shares_withdraw(&vault, &issuance, &amount) else {
            return Ter::TEC_INTERNAL;
        };
        let Some(assets) = shares_to_assets(&vault, &issuance, shares, true) else {
            return Ter::TEC_INTERNAL;
        };
        (shares, assets)
    } else if amount.asset() == share_asset(vault.share_id) {
        let shares = amount.mpt().value().unsigned_abs();
        let Some(assets) = shares_to_assets(&vault, &issuance, shares, true) else {
            return Ter::TEC_INTERNAL;
        };
        (shares, assets)
    } else {
        return Ter::TEF_INTERNAL;
    };

    if token_balance(view, vault.share_id, &account).unwrap_or_default() < shares_redeemed {
        return Ter::TEC_INSUFFICIENT_FUNDS;
    }
    if vault.assets_available < amount_number(&assets_withdrawn) {
        return Ter::TEC_INSUFFICIENT_FUNDS;
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
    vault.assets_total -= withdrawn_number;
    vault.assets_available -= withdrawn_number;
    persist_vault(view, &mut vault)
}

pub fn apply_vault_clawback<V: ApplyView>(view: &mut V, sttx: &STTx) -> Ter {
    if !view.rules().enabled(&feature_id("SingleAssetVault")) {
        return Ter::TEM_DISABLED;
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
            let assets = shares_to_assets(&vault, &issuance, shares, true)
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
                let Some(shares) = assets_to_shares_withdraw(&vault, &issuance, &amount) else {
                    return Ter::TEC_INTERNAL;
                };
                let Some(assets) = shares_to_assets(&vault, &issuance, shares, true) else {
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
        vault.assets_available -= recovered;
    }

    persist_vault(view, &mut vault)
}
