use super::common::*;
use basics::{base_uint::Uint256, number::NumberParts as RuntimeNumber};
use ledger::{ApplyView, FlowSandbox, ReadView};
use protocol::{AccountID, LedgerEntryType, STLedgerEntry};
use std::collections::BTreeSet;

#[derive(Default)]
pub(super) struct LendingState {
    pub(super) broker_refs: BTreeSet<Uint256>,
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

pub(super) fn validate_loan_entry(before: Option<&STLedgerEntry>, after: &STLedgerEntry) -> bool {
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

pub(super) fn maybe_record_loan_broker_account<V: ApplyView>(
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

pub(super) fn record_lending_state<V: ApplyView>(
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

pub(super) fn validate_zero_owner_count_broker_directory<V: ApplyView>(
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

pub(super) fn validate_loan_broker_entry<V: ApplyView>(
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
