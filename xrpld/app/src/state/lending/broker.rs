use basics::number::{NumberParts as RuntimeNumber, RoundingMode};
use ledger::{RelativeDistanceAmount, views::apply_view::ApplyView};
use protocol::{STLedgerEntry, STTx, Ter, account_keylet, feature_id, owner_dir_keylet};

use super::common::*;
use super::helpers::*;

pub fn apply_loan_broker_delete<V: ApplyView>(view: &mut V, sttx: &STTx) -> Ter {
    if !view.rules().enabled(&feature_id("LendingProtocol")) {
        return Ter::TEM_DISABLED;
    }

    let account = sttx.get_account_id(sf("sfAccount"));
    let broker_id = sttx.get_field_h256(sf("sfLoanBrokerID"));

    let Ok(Some(broker_sle)) = view.peek(protocol::loan_broker_keylet_from_key(broker_id)) else {
        return Ter::TEF_BAD_LEDGER;
    };
    let Ok(Some(vault_sle)) = view.peek(protocol::vault_keylet_from_key(
        broker_sle.get_field_h256(sf("sfVaultID")),
    )) else {
        return Ter::TEF_BAD_LEDGER;
    };
    let vault_pseudo_id = vault_sle.get_account_id(sf("sfAccount"));
    let vault_asset = vault_sle.get_field_issue(sf("sfAsset")).asset();
    let broker_pseudo_id = broker_sle.get_account_id(sf("sfAccount"));

    if account != broker_sle.get_account_id(sf("sfOwner")) {
        return Ter::TEC_NO_PERMISSION;
    }
    if broker_sle.get_field_u32(sf("sfOwnerCount")) != 0 {
        return Ter::TEC_HAS_OBLIGATIONS;
    }
    let debt_total = broker_sle.get_field_number(sf("sfDebtTotal")).value();
    if debt_total != RuntimeNumber::zero() {
        let rounded = round_number_to_asset_with_scale(
            vault_asset,
            debt_total,
            vault_scale(&vault_sle, vault_asset),
            RoundingMode::TowardsZero,
        );
        if rounded != RuntimeNumber::zero() {
            return Ter::TEC_HAS_OBLIGATIONS;
        }
    }
    let cover_available = broker_sle.get_field_number(sf("sfCoverAvailable")).value();
    if view.rules().enabled(&feature_id("fixCleanup3_2_0"))
        && cover_available > RuntimeNumber::zero()
    {
        let ter = check_cover_sendable(view, &broker_pseudo_id, vault_asset);
        if ter != Ter::TES_SUCCESS {
            return ter;
        }
        let ter = check_mpt_cover_transfer(
            view,
            &broker_pseudo_id,
            &account,
            &account,
            vault_asset,
            true,
        );
        if ter != Ter::TES_SUCCESS {
            return ter;
        }
    }

    if !matches!(
        ledger::dir_remove(
            view,
            &owner_dir_keylet(to_160(&account)),
            broker_sle.get_field_u64(sf("sfOwnerNode")),
            *broker_sle.key(),
            false,
        ),
        Ok(true)
    ) {
        return Ter::TEF_BAD_LEDGER;
    }
    if !matches!(
        ledger::dir_remove(
            view,
            &owner_dir_keylet(to_160(&vault_pseudo_id)),
            broker_sle.get_field_u64(sf("sfVaultNode")),
            *broker_sle.key(),
            false,
        ),
        Ok(true)
    ) {
        return Ter::TEF_BAD_LEDGER;
    }

    let Ok(cover_amount) = vault_asset.amount(cover_available) else {
        return Ter::TEF_BAD_LEDGER;
    };
    let waive_mpt_can_transfer = view.rules().enabled(&feature_id("fixCleanup3_2_0"));
    let payout = account_send_with_mpt_transfer_waiver(
        view,
        &broker_pseudo_id,
        &account,
        &cover_amount,
        waive_mpt_can_transfer,
    );
    if payout != Ter::TES_SUCCESS {
        return payout;
    }

    let cleanup = ledger::remove_empty_holding(view, &broker_pseudo_id, &vault_asset);
    if cleanup != Ter::TES_SUCCESS {
        return cleanup;
    }

    let Ok(Some(pseudo_sle)) = view.peek(account_keylet(to_160(&broker_pseudo_id))) else {
        return Ter::TEF_BAD_LEDGER;
    };
    if account_balance_drops(&pseudo_sle) != 0 {
        return Ter::TEC_HAS_OBLIGATIONS;
    }
    if pseudo_sle.get_field_u32(sf("sfOwnerCount")) != 0 {
        return Ter::TEC_HAS_OBLIGATIONS;
    }
    if view
        .read(owner_dir_keylet(to_160(&broker_pseudo_id)))
        .ok()
        .flatten()
        .is_some()
    {
        return Ter::TEC_HAS_OBLIGATIONS;
    }

    if view.erase(pseudo_sle).is_err() || view.erase(broker_sle.clone()).is_err() {
        return Ter::TEF_BAD_LEDGER;
    }
    let Ok(Some(owner_sle)) = view.peek(account_keylet(to_160(&account))) else {
        return Ter::TEF_BAD_LEDGER;
    };
    let _ = ledger::adjust_owner_count(view, &owner_sle, -2);
    Ter::TES_SUCCESS
}

pub fn apply_loan_delete<V: ApplyView>(view: &mut V, sttx: &STTx) -> Ter {
    if !view.rules().enabled(&feature_id("LendingProtocol")) {
        return Ter::TEM_DISABLED;
    }

    let loan_id = sttx.get_field_h256(sf("sfLoanID"));

    let Ok(Some(loan_sle)) = view.peek(protocol::loan_keylet_from_key(loan_id)) else {
        return Ter::TEF_BAD_LEDGER;
    };
    let borrower_id = loan_sle.get_account_id(sf("sfBorrower"));
    let Ok(Some(borrower_sle)) = view.peek(account_keylet(to_160(&borrower_id))) else {
        return Ter::TEF_BAD_LEDGER;
    };

    let broker_id = loan_sle.get_field_h256(sf("sfLoanBrokerID"));
    let Ok(Some(broker_sle)) = view.peek(protocol::loan_broker_keylet_from_key(broker_id)) else {
        return Ter::TEF_BAD_LEDGER;
    };
    let broker_pseudo_id = broker_sle.get_account_id(sf("sfAccount"));

    let vault_id = broker_sle.get_field_h256(sf("sfVaultID"));
    let Ok(Some(vault_sle)) = view.peek(protocol::vault_keylet_from_key(vault_id)) else {
        return Ter::TEF_BAD_LEDGER;
    };
    let vault_asset = vault_sle.get_field_issue(sf("sfAsset")).asset();

    if !matches!(
        ledger::dir_remove(
            view,
            &owner_dir_keylet(to_160(&broker_pseudo_id)),
            loan_sle.get_field_u64(sf("sfLoanBrokerNode")),
            loan_id,
            false,
        ),
        Ok(true)
    ) {
        return Ter::TEF_BAD_LEDGER;
    }
    if !matches!(
        ledger::dir_remove(
            view,
            &owner_dir_keylet(to_160(&borrower_id)),
            loan_sle.get_field_u64(sf("sfOwnerNode")),
            loan_id,
            false,
        ),
        Ok(true)
    ) {
        return Ter::TEF_BAD_LEDGER;
    }

    if view.erase(loan_sle.clone()).is_err() {
        return Ter::TEF_BAD_LEDGER;
    }

    let Some(broker_owner_count) = broker_sle.get_field_u32(sf("sfOwnerCount")).checked_sub(1)
    else {
        return Ter::TEF_BAD_LEDGER;
    };
    let mut broker_obj = broker_sle.clone_as_object();
    broker_obj.set_field_u32(sf("sfOwnerCount"), broker_owner_count);
    if broker_owner_count == 0 {
        let debt_total = broker_sle.get_field_number(sf("sfDebtTotal"));
        if debt_total.value() != RuntimeNumber::zero() {
            let rounded = round_number_to_asset(vault_asset, debt_total.value());
            if rounded != RuntimeNumber::zero() {
                return Ter::TEF_BAD_LEDGER;
            }
            broker_obj.set_field_number(
                sf("sfDebtTotal"),
                with_asset_number(RuntimeNumber::zero(), vault_asset),
            );
        }
    }
    let mut broker = STLedgerEntry::from_stobject(broker_obj, *broker_sle.key());
    associate_asset_entry(&mut broker, vault_asset);
    let broker_result = persist_entry(view, broker);
    if broker_result != Ter::TES_SUCCESS {
        return broker_result;
    }

    let _ = ledger::adjust_owner_count(view, &borrower_sle, -1);

    let mut loan = STLedgerEntry::from_stobject(loan_sle.clone_as_object(), *loan_sle.key());
    associate_asset_entry(&mut loan, vault_asset);
    let mut vault = STLedgerEntry::from_stobject(vault_sle.clone_as_object(), *vault_sle.key());
    associate_asset_entry(&mut vault, vault_asset);

    Ter::TES_SUCCESS
}

pub fn apply_loan_broker_cover_deposit<V: ApplyView>(view: &mut V, sttx: &STTx) -> Ter {
    if !view.rules().enabled(&feature_id("LendingProtocol")) {
        return Ter::TEM_DISABLED;
    }

    let account = sttx.get_account_id(sf("sfAccount"));
    let broker_id = sttx.get_field_h256(sf("sfLoanBrokerID"));
    let amount = sttx.get_field_amount(sf("sfAmount"));

    let Some(mut broker) = load_broker(view, broker_id) else {
        return Ter::TEF_INTERNAL;
    };
    let Some(vault) = load_vault(view, broker.vault_id) else {
        return Ter::TEF_INTERNAL;
    };
    if account != broker.owner {
        return Ter::TEC_NO_PERMISSION;
    }
    if amount.asset() != vault.asset {
        return Ter::TEC_WRONG_ASSET;
    }

    let Some((deposit_amount, added)) = rounded_cover_deposit_amount(
        vault.asset,
        broker.cover_available,
        &amount,
        view.rules().enabled(&feature_id("fixCleanup3_2_0")),
    ) else {
        return Ter::TEF_INTERNAL;
    };
    if added == RuntimeNumber::zero() {
        return Ter::TEC_PRECISION_LOSS;
    }

    let transfer_result = account_send(view, &account, &broker.pseudo_account, &deposit_amount);
    if transfer_result != Ter::TES_SUCCESS {
        return transfer_result;
    }

    broker.cover_available += added;
    broker.cover_asset = vault.asset;

    persist_broker_cover(view, broker_id, &broker)
}

pub fn apply_loan_broker_cover_withdraw<V: ApplyView>(
    view: &mut V,
    sttx: &STTx,
    _pre_fee_balance_drops: i64,
) -> Ter {
    if !view.rules().enabled(&feature_id("LendingProtocol")) {
        return Ter::TEM_DISABLED;
    }

    let broker_id = sttx.get_field_h256(sf("sfLoanBrokerID"));
    let amount = sttx.get_field_amount(sf("sfAmount"));
    let destination = if sttx.is_field_present(sf("sfDestination")) {
        sttx.get_account_id(sf("sfDestination"))
    } else {
        sttx.get_account_id(sf("sfAccount"))
    };
    let account = sttx.get_account_id(sf("sfAccount"));

    let Some(mut broker) = load_broker(view, broker_id) else {
        return Ter::TEC_INTERNAL;
    };
    let Ok(Some(vault_sle)) = view.peek(protocol::vault_keylet_from_key(broker.vault_id)) else {
        return Ter::TEC_INTERNAL;
    };
    let vault_asset = vault_sle.get_field_issue(sf("sfAsset")).asset();

    if is_pseudo_account(view, &destination) {
        return Ter::TEC_PSEUDO_ACCOUNT;
    }
    if account != broker.owner {
        return Ter::TEC_NO_PERMISSION;
    }
    if amount.asset() != vault_asset {
        return Ter::TEC_WRONG_ASSET;
    }
    if cover_amount_is_zero_at_cover_scale(
        vault_asset,
        broker.cover_available,
        &amount,
        view.rules().enabled(&feature_id("fixCleanup3_2_0")),
    ) {
        return Ter::TEC_PRECISION_LOSS;
    }

    let deducted = amount.as_number();
    if broker.cover_available < deducted {
        return Ter::TEC_INSUFFICIENT_FUNDS;
    }
    let minimum_cover = minimum_broker_cover(
        vault_asset,
        broker.debt_total,
        broker.cover_rate_minimum,
        &vault_sle,
        view.rules().enabled(&feature_id("fixCleanup3_2_0")),
    );
    if broker.cover_available - deducted < minimum_cover {
        return Ter::TEC_INSUFFICIENT_FUNDS;
    }

    let waive_mpt_can_transfer = view.rules().enabled(&feature_id("fixCleanup3_2_0"));
    let transfer_check = check_mpt_cover_transfer(
        view,
        &broker.pseudo_account,
        &destination,
        &account,
        vault_asset,
        waive_mpt_can_transfer,
    );
    if transfer_check != Ter::TES_SUCCESS {
        return transfer_check;
    }

    broker.cover_available -= deducted;
    broker.cover_asset = vault_asset;

    let update_result = persist_broker_cover(view, broker_id, &broker);
    if update_result != Ter::TES_SUCCESS {
        return update_result;
    }

    account_send_with_mpt_transfer_waiver(
        view,
        &broker.pseudo_account,
        &destination,
        &amount,
        waive_mpt_can_transfer,
    )
}

pub fn apply_loan_broker_cover_clawback<V: ApplyView>(view: &mut V, sttx: &STTx) -> Ter {
    if !view.rules().enabled(&feature_id("LendingProtocol")) {
        return Ter::TEM_DISABLED;
    }

    let account = sttx.get_account_id(sf("sfAccount"));
    let broker_id = sttx.get_field_h256(sf("sfLoanBrokerID"));

    let Some(mut broker) = load_broker(view, broker_id) else {
        return Ter::TEC_INTERNAL;
    };
    let Some(vault) = load_vault(view, broker.vault_id) else {
        return Ter::TEC_INTERNAL;
    };

    if vault.asset.native() || asset_issuer(vault.asset) != account {
        return Ter::TEC_NO_PERMISSION;
    }

    let requested = sttx
        .is_field_present(sf("sfAmount"))
        .then(|| sttx.get_field_amount(sf("sfAmount")));
    if let Some(amount) = &requested
        && amount.asset() != vault.asset
    {
        return Ter::TEC_WRONG_ASSET;
    }

    let minimum_cover = minimum_broker_cover(
        vault.asset,
        broker.debt_total,
        broker.cover_rate_minimum,
        &vault.entry,
        view.rules().enabled(&feature_id("fixCleanup3_2_0")),
    );
    let max_claw_amount = broker.cover_available - minimum_cover;
    if max_claw_amount <= RuntimeNumber::zero() {
        return Ter::TEC_INSUFFICIENT_FUNDS;
    }

    let deducted = requested
        .as_ref()
        .filter(|amount| amount.signum() > 0)
        .map(amount_number)
        .unwrap_or(max_claw_amount)
        .min(max_claw_amount);
    let Some(claw_amount) = runtime_to_amount(vault.asset, deducted, RoundingMode::Downward) else {
        return Ter::TEC_INTERNAL;
    };
    if claw_amount.signum() == 0 {
        return Ter::TEC_PRECISION_LOSS;
    }
    if cover_amount_is_zero_at_cover_scale(
        vault.asset,
        broker.cover_available,
        &claw_amount,
        view.rules().enabled(&feature_id("fixCleanup3_2_0")),
    ) {
        return Ter::TEC_PRECISION_LOSS;
    }

    broker.cover_available -= deducted;
    broker.cover_asset = vault.asset;

    let update_result = persist_broker_cover(view, broker_id, &broker);
    if update_result != Ter::TES_SUCCESS {
        return update_result;
    }

    account_send(view, &broker.pseudo_account, &account, &claw_amount)
}
