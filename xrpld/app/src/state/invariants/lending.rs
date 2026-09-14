use super::common::*;
use basics::{base_uint::Uint256, number::NumberParts as RuntimeNumber};
use ledger::{ApplyView, FlowSandbox, ReadView};
use protocol::{AccountID, LedgerEntryType, STLedgerEntry};
use std::collections::BTreeSet;

#[derive(Default)]
pub(super) struct LendingState {
    pub(super) broker_refs: BTreeSet<Uint256>,
    deleted_loans: Vec<STLedgerEntry>,
    deleted_brokers: Vec<STLedgerEntry>,
}

pub(super) fn number_field_value(
    sle: &STLedgerEntry,
    field: &'static protocol::SField,
) -> RuntimeNumber {
    sle.get_field_number(field).value()
}

pub(super) fn number_field_negative(sle: &STLedgerEntry, field: &'static protocol::SField) -> bool {
    number_field_value(sle, field) < RuntimeNumber::zero()
}

pub(super) fn validate_loan_entry<V: ApplyView + ?Sized>(
    sandbox: &FlowSandbox<V>,
    txn_type: protocol::TxType,
    result: protocol::Ter,
    lending_protocol_v1_1: bool,
    before: Option<&STLedgerEntry>,
    after: &STLedgerEntry,
) -> Result<bool, ledger::ViewError> {
    let zero = RuntimeNumber::zero();
    let payment_remaining = after.get_field_u32(sf("sfPaymentRemaining"));
    let total_value = number_field_value(after, sf("sfTotalValueOutstanding"));
    let principal = number_field_value(after, sf("sfPrincipalOutstanding"));
    let management_fee = number_field_value(after, sf("sfManagementFeeOutstanding"));

    if payment_remaining == 0
        && (total_value != zero || principal != zero || management_fee != zero)
    {
        return Ok(false);
    }
    if payment_remaining != 0 && total_value == zero && principal == zero && management_fee == zero
    {
        return Ok(false);
    }
    if before.is_some_and(|before| {
        before.is_flag(protocol::lsfLoanOverpayment) != after.is_flag(protocol::lsfLoanOverpayment)
    }) {
        return Ok(false);
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
            return Ok(false);
        }
    }
    if number_field_value(after, sf("sfPeriodicPayment")) <= zero {
        return Ok(false);
    }

    if !lending_protocol_v1_1 {
        return Ok(true);
    }

    if before.is_none() && txn_type != protocol::TxType::LOAN_SET {
        return Ok(false);
    }
    if payment_remaining == 0 && after.get_field_u32(sf("sfNextPaymentDueDate")) != 0 {
        return Ok(false);
    }
    if let Some(before) = before {
        let impaired_changed =
            before.is_flag(protocol::lsfLoanImpaired) != after.is_flag(protocol::lsfLoanImpaired);
        let default_changed =
            before.is_flag(protocol::lsfLoanDefault) != after.is_flag(protocol::lsfLoanDefault);
        if (impaired_changed
            && !matches!(
                txn_type,
                protocol::TxType::LOAN_MANAGE | protocol::TxType::LOAN_PAY
            ))
            || (default_changed && txn_type != protocol::TxType::LOAN_MANAGE)
        {
            return Ok(false);
        }
        if protocol::is_tes_success(result)
            && txn_type == protocol::TxType::LOAN_PAY
            && payment_remaining != 0
            && (!(principal < number_field_value(before, sf("sfPrincipalOutstanding")))
                || payment_remaining >= before.get_field_u32(sf("sfPaymentRemaining")))
        {
            return Ok(false);
        }
        if protocol::is_tes_success(result)
            && txn_type == protocol::TxType::LOAN_PAY
            && payment_remaining != 0
        {
            let before_due = before.get_field_u32(sf("sfNextPaymentDueDate"));
            let after_due = after.get_field_u32(sf("sfNextPaymentDueDate"));
            let interval = after.get_field_u32(sf("sfPaymentInterval"));
            if after_due <= before_due
                || interval == 0
                || !(after_due - before_due).is_multiple_of(interval)
            {
                return Ok(false);
            }
        }
    }

    let Some(broker) = sandbox.read(protocol::loan_broker_keylet_from_key(
        after.get_field_h256(sf("sfLoanBrokerID")),
    ))?
    else {
        return Ok(false);
    };
    let Some(vault) = sandbox.read(protocol::vault_keylet_from_key(
        broker.get_field_h256(sf("sfVaultID")),
    ))?
    else {
        return Ok(false);
    };

    let interest_due = total_value - principal - management_fee;
    let tolerance = if vault.get_field_issue(sf("sfAsset")).asset().integral() {
        zero
    } else {
        RuntimeNumber::try_from_external_parts(
            -1,
            after.get_field_i32(sf("sfLoanScale")),
            basics::number::get_mantissa_scale(),
        )
        .unwrap_or(zero)
    };
    if interest_due < tolerance {
        return Ok(false);
    }

    if before.is_none()
        && protocol::is_tes_success(result)
        && vault.get_field_u8(sf("sfVaultKind")) == 1
    {
        let final_payment = u64::from(after.get_field_u32(sf("sfStartDate")))
            + u64::from(after.get_field_u32(sf("sfPaymentInterval")))
                * u64::from(payment_remaining);
        let redemption = u64::from(vault.get_field_u32(sf("sfRedemptionDate")));
        if final_payment + 60 > redemption {
            return Ok(false);
        }
    }

    Ok(true)
}

pub(super) fn maybe_record_loan_broker_account<V: ApplyView + ?Sized>(
    sandbox: &FlowSandbox<V>,
    state: &mut LendingState,
    account: AccountID,
) -> Result<(), ledger::ViewError> {
    if let Some(root) = sandbox.read(protocol::account_keylet(raw_account_id(account)))?
        && root.is_field_present(sf("sfLoanBrokerID"))
    {
        state
            .broker_refs
            .insert(root.get_field_h256(sf("sfLoanBrokerID")));
    }
    Ok(())
}

pub(super) fn record_lending_state<V: ApplyView + ?Sized>(
    sandbox: &FlowSandbox<V>,
    state: &mut LendingState,
    is_delete: bool,
    before: Option<&STLedgerEntry>,
    after: Option<&STLedgerEntry>,
) -> Result<(), ledger::ViewError> {
    if is_delete {
        if let Some(before) = before {
            match before.get_type() {
                LedgerEntryType::Loan => state.deleted_loans.push(before.clone()),
                LedgerEntryType::LoanBroker => state.deleted_brokers.push(before.clone()),
                _ => {}
            }
        }
        return Ok(());
    }
    let Some(after) = after else {
        return Ok(());
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
            )?;
            maybe_record_loan_broker_account(
                sandbox,
                state,
                after.get_field_amount(sf("sfHighLimit")).issue().account,
            )?;
        }
        LedgerEntryType::MPToken => {
            maybe_record_loan_broker_account(
                sandbox,
                state,
                after.get_account_id(sf("sfAccount")),
            )?;
        }
        _ => {}
    }
    Ok(())
}

pub(super) fn validate_zero_owner_count_broker_directory<V: ApplyView + ?Sized>(
    sandbox: &FlowSandbox<V>,
    broker: &STLedgerEntry,
) -> Result<bool, ledger::ViewError> {
    if broker.get_field_u32(sf("sfOwnerCount")) != 0 {
        return Ok(true);
    }

    let Some(dir) = sandbox.read(protocol::owner_dir_keylet(raw_account_id(
        broker.get_account_id(sf("sfAccount")),
    )))?
    else {
        return Ok(true);
    };

    if dir.is_field_present(sf("sfIndexPrevious")) && dir.get_field_u64(sf("sfIndexPrevious")) != 0
    {
        return Ok(false);
    }
    if dir.is_field_present(sf("sfIndexNext")) && dir.get_field_u64(sf("sfIndexNext")) != 0 {
        return Ok(false);
    }

    let indexes = dir.get_field_v256(sf("sfIndexes"));
    if indexes.value().len() > 1 {
        return Ok(false);
    }

    if let Some(index) = indexes.value().first() {
        let Some(indexed) = sandbox.read(protocol::unchecked_keylet(*index))? else {
            return Ok(false);
        };
        Ok(matches!(
            indexed.get_type(),
            LedgerEntryType::RippleState | LedgerEntryType::MPToken
        ))
    } else {
        Ok(true)
    }
}

pub(super) fn validate_loan_broker_entry<V: ApplyView + ?Sized>(
    sandbox: &FlowSandbox<V>,
    txn_type: protocol::TxType,
    fix_cleanup_3_1_3: bool,
    before: Option<&STLedgerEntry>,
    after: &STLedgerEntry,
) -> Result<bool, ledger::ViewError> {
    if before.is_some_and(|before| {
        before.get_field_u32(sf("sfLoanSequence")) > after.get_field_u32(sf("sfLoanSequence"))
    }) {
        return Ok(false);
    }
    if number_field_negative(after, sf("sfDebtTotal"))
        || number_field_negative(after, sf("sfCoverAvailable"))
    {
        return Ok(false);
    }
    let Some(vault) = sandbox.read(protocol::vault_keylet_from_key(
        after.get_field_h256(sf("sfVaultID")),
    ))?
    else {
        return Ok(false);
    };
    if !validate_zero_owner_count_broker_directory(sandbox, after)? {
        return Ok(false);
    }

    let cover_available = number_field_value(after, sf("sfCoverAvailable"));
    let vault_asset = vault.get_field_issue(sf("sfAsset")).asset();
    let Some(pseudo_balance) =
        account_holds_asset_number(sandbox, after.get_account_id(sf("sfAccount")), vault_asset)?
    else {
        return Ok(false);
    };

    if cover_available < pseudo_balance {
        return Ok(false);
    }
    if fix_cleanup_3_1_3 && txn_type != protocol::TxType::LOAN_BROKER_DELETE {
        if cover_available > pseudo_balance {
            return Ok(false);
        }
    }

    Ok(true)
}

/// V1.1 deletion rules must consult the erased pre-state, just like the
/// corresponding preclaims. Existing legacy lending ledgers retain their prior
/// behavior because this function is called only when LendingProtocolV1_1 is
/// enabled.
pub(super) fn validate_v1_1_lending_deletions<V: ApplyView + ?Sized>(
    sandbox: &FlowSandbox<V>,
    txn_type: protocol::TxType,
    state: &LendingState,
) -> Result<bool, ledger::ViewError> {
    if (!state.deleted_loans.is_empty() && txn_type != protocol::TxType::LOAN_DELETE)
        || state.deleted_brokers.len() > 1
        || (!state.deleted_brokers.is_empty() && txn_type != protocol::TxType::LOAN_BROKER_DELETE)
    {
        return Ok(false);
    }

    for broker in &state.deleted_brokers {
        if broker.get_field_u32(sf("sfOwnerCount")) != 0 {
            return Ok(false);
        }
        let debt = number_field_value(broker, sf("sfDebtTotal"));
        if debt == RuntimeNumber::zero() {
            continue;
        }
        let Some(vault) = sandbox.read(protocol::vault_keylet_from_key(
            broker.get_field_h256(sf("sfVaultID")),
        ))?
        else {
            return Ok(false);
        };
        let asset = vault.get_field_issue(sf("sfAsset")).asset();
        let scale = if asset.integral() {
            0
        } else {
            asset
                .amount(number_field_value(&vault, sf("sfAssetsTotal")))
                .map(|amount| amount.exponent())
                .unwrap_or(0)
        };
        if ledger::vault_helpers::round_number_to_asset_with_scale(
            asset,
            debt,
            scale,
            basics::number::RoundingMode::TowardsZero,
        ) != RuntimeNumber::zero()
        {
            return Ok(false);
        }
    }
    Ok(true)
}
