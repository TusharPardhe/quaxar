use super::common::*;
use super::freeze::LoanDefaultFreezeExemptAccounts;
use ledger::{ApplyView, FlowSandbox, ReadView};
use protocol::{AccountID, LedgerEntryType, MPTID, STLedgerEntry, Ter};
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct MptAccounting {
    outstanding_before: i128,
    outstanding_after: i128,
    amount_delta: i128,
    overflow: bool,
}

#[derive(Default)]
pub(super) struct MptTransferAmount {
    before: Option<u64>,
    after: Option<u64>,
    authorized_before: bool,
    authorized_after: bool,
    deleted: bool,
}

#[derive(Default)]
pub(super) struct MptIssuanceLifecycle {
    reference_holding_set_on_create: bool,
    reference_holding_mutated: bool,
    vault_holding_deleted: bool,
    issuances_created: u32,
    issuances_deleted: u32,
    tokens_created: u32,
    tokens_deleted: u32,
    token_created_by_issuer: bool,
    /// rippled #8152: issuance flags cleared by a modification, excluding the
    /// legally clearable lsfMPTLocked. Any nonzero value is an invariant
    /// violation under fixCleanup3_5_0.
    issuance_flags_cleared: u32,
    /// rippled #8209: an MPToken erased with a non-zero public sfMPTAmount.
    /// Rejected under fixCleanup3_5_0 (the check moved here from the
    /// confidential gate).
    mptoken_deleted_with_balance: bool,
}

#[derive(Default)]
pub(super) struct ConfidentialMptChange {
    mpt_amount_delta: i128,
    coa_delta: i128,
    outstanding_delta: i128,
    issuance: Option<STLedgerEntry>,
    deleted_with_encrypted: bool,
    /// rippled #8209: pre-fixCleanup3_5_0 only, a non-zero public balance at
    /// erase time. Pre-amendment this fed the confidential (COA) gate; post
    /// amendment the public-balance check moves to ValidMPTIssuance.
    deleted_with_balance_before: bool,
    bad_consistency: bool,
    bad_coa: bool,
    changes_confidential_fields: bool,
    bad_version: bool,
}

fn capped_delta(value: u64) -> i128 {
    i128::from(value.min(max_mpt_token_amount()))
}

fn optional_u32_value(sle: &STLedgerEntry, field: &'static protocol::SField) -> Option<u32> {
    sle.is_field_present(field)
        .then(|| sle.get_field_u32(field))
}

fn optional_vl_value(sle: &STLedgerEntry, field: &'static protocol::SField) -> Option<Vec<u8>> {
    sle.is_field_present(field).then(|| sle.get_field_vl(field))
}

pub(super) fn record_confidential_mpt(
    changes: &mut BTreeMap<MPTID, ConfidentialMptChange>,
    is_delete: bool,
    before: Option<&STLedgerEntry>,
    after: &STLedgerEntry,
    fix_cleanup_3_5_0: bool,
) {
    let id = |sle: &STLedgerEntry| match sle.get_type() {
        LedgerEntryType::MPToken => sle.get_field_h192(sf("sfMPTokenIssuanceID")),
        LedgerEntryType::MPTokenIssuance => mpt_id_from_issuance(sle),
        _ => MPTID::default(),
    };
    if let Some(before) = before.filter(|sle| sle.get_type() == LedgerEntryType::MPToken) {
        let change = changes.entry(id(before)).or_default();
        change.mpt_amount_delta -= capped_delta(before.get_field_u64(sf("sfMPTAmount")));
        if is_delete {
            // changes is keyed by issuance, so sibling holders erased by the
            // same transaction share this entry. Only ever SET these flags,
            // never clear them, or an empty sibling visited later would mask a
            // funded MPToken (rippled #8209).
            //
            // Pre-fixCleanup3_5_0 the non-zero public balance fed the COA gate
            // below; post-amendment the public-balance check moves to
            // ValidMPTIssuance::finalize, so only ciphertext fields feed the
            // confidential gate here.
            if !fix_cleanup_3_5_0 && before.get_field_u64(sf("sfMPTAmount")) > 0 {
                change.deleted_with_balance_before = true;
            }
            if [
                "sfConfidentialBalanceSpending",
                "sfConfidentialBalanceInbox",
                "sfIssuerEncryptedBalance",
                "sfAuditorEncryptedBalance",
            ]
            .iter()
            .any(|field| before.is_field_present(sf(field)))
            {
                change.deleted_with_encrypted = true;
            }
        }
    }
    if after.get_type() == LedgerEntryType::MPToken {
        let change = changes.entry(id(after)).or_default();
        change.mpt_amount_delta += capped_delta(after.get_field_u64(sf("sfMPTAmount")));
        let issuer = after.is_field_present(sf("sfIssuerEncryptedBalance"));
        let inbox = after.is_field_present(sf("sfConfidentialBalanceInbox"));
        let spending = after.is_field_present(sf("sfConfidentialBalanceSpending"));
        let auditor = after.is_field_present(sf("sfAuditorEncryptedBalance"));
        change.bad_consistency |= inbox != spending || inbox != issuer || (auditor && !issuer);
        for field in [
            "sfConfidentialBalanceInbox",
            "sfConfidentialBalanceSpending",
            "sfIssuerEncryptedBalance",
            "sfAuditorEncryptedBalance",
        ] {
            let field = sf(field);
            if after.is_field_present(field)
                && before.is_none_or(|before| {
                    before.get_type() != LedgerEntryType::MPToken
                        || optional_vl_value(before, field) != optional_vl_value(after, field)
                })
            {
                change.changes_confidential_fields = true;
            }
        }
    }
    if let Some(before) = before.filter(|sle| sle.get_type() == LedgerEntryType::MPTokenIssuance) {
        let change = changes.entry(id(before)).or_default();
        if before.is_field_present(sf("sfConfidentialOutstandingAmount")) {
            change.coa_delta -=
                capped_delta(before.get_field_u64(sf("sfConfidentialOutstandingAmount")));
        }
        change.outstanding_delta -= capped_delta(before.get_field_u64(sf("sfOutstandingAmount")));
    }
    if after.get_type() == LedgerEntryType::MPTokenIssuance {
        let change = changes.entry(id(after)).or_default();
        let coa = optional_u64(after, sf("sfConfidentialOutstandingAmount"));
        if after.is_field_present(sf("sfConfidentialOutstandingAmount")) {
            change.coa_delta += capped_delta(coa);
        }
        let outstanding = after.get_field_u64(sf("sfOutstandingAmount"));
        change.outstanding_delta += capped_delta(outstanding);
        change.issuance = Some(after.clone());
        change.bad_coa |= coa > outstanding;
    }
    if let Some(before) = before.filter(|sle| sle.get_type() == LedgerEntryType::MPToken)
        && after.get_type() == LedgerEntryType::MPToken
    {
        let spending_before = optional_vl_value(before, sf("sfConfidentialBalanceSpending"));
        let spending_after = optional_vl_value(after, sf("sfConfidentialBalanceSpending"));
        if spending_before.is_some() && spending_before != spending_after {
            changes.entry(id(after)).or_default().bad_version |=
                optional_u32_value(before, sf("sfConfidentialBalanceVersion"))
                    == optional_u32_value(after, sf("sfConfidentialBalanceVersion"));
        }
    }
}

/// rippled `kConfidentialMptTxTypes` (MPTInvariant.cpp, incl. #8192/#8266).
fn is_confidential_mpt_tx(txn_type: protocol::TxType) -> bool {
    matches!(
        txn_type,
        protocol::TxType::CONFIDENTIAL_MPT_SEND
            | protocol::TxType::CONFIDENTIAL_MPT_CONVERT
            | protocol::TxType::CONFIDENTIAL_MPT_CONVERT_BACK
            | protocol::TxType::CONFIDENTIAL_MPT_MERGE_INBOX
            | protocol::TxType::CONFIDENTIAL_MPT_CLAWBACK
            | protocol::TxType::CONFIDENTIAL_MPT_MIRROR_UPDATE
            | protocol::TxType::CONFIDENTIAL_MPT_HOLDER_KEY_UPDATE
    )
}

pub(super) fn validates_confidential_mpt<V: ApplyView + ?Sized>(
    view: &FlowSandbox<V>,
    txn_type: protocol::TxType,
    result: Ter,
    changes: &BTreeMap<MPTID, ConfidentialMptChange>,
) -> bool {
    if !protocol::is_tes_success(result) {
        return true;
    }
    let confidential_tx = is_confidential_mpt_tx(txn_type);
    // rippled #8209: before fixCleanup3_5_0 the COA gate also absorbed the
    // pre-transaction public balance, so a drain-then-erase of an MPToken was
    // rejected whenever an unrelated holder of the same issuance held a
    // confidential balance. Post-amendment the public-balance check lives in
    // ValidMPTIssuance and only ciphertext feeds this gate.
    let fix_cleanup_3_5_0 = view
        .rules()
        .enabled(&protocol::feature_id("fixCleanup3_5_0"));
    changes.iter().all(|(id, change)| {
        let issuance = if let Some(issuance) = change.issuance.clone() {
            Some(issuance)
        } else {
            match view.read(protocol::mpt_issuance_keylet_from_mptid(*id)) {
                Ok(value) => value.map(|sle| (*sle).clone()),
                // A SHAMap/storage failure is not evidence that the issuance
                // was deleted. Fail the invariant closed instead of silently
                // skipping all confidential-balance checks.
                Err(_) => return false,
            }
        };
        let Some(issuance) = issuance else {
            return true;
        };
        let deleted_with_encrypted = if fix_cleanup_3_5_0 {
            change.deleted_with_encrypted
        } else {
            change.deleted_with_encrypted || change.deleted_with_balance_before
        };
        if deleted_with_encrypted
            && optional_u64(&issuance, sf("sfConfidentialOutstandingAmount")) > 0
        {
            return false;
        }
        if change.bad_consistency || change.bad_coa || change.bad_version {
            return false;
        }
        if change.changes_confidential_fields
            && !issuance.is_flag(protocol::lsfMPTCanHoldConfidentialBalance)
        {
            return false;
        }
        if change.coa_delta != 0 {
            change.mpt_amount_delta + change.coa_delta == change.outstanding_delta
        } else if confidential_tx {
            change.mpt_amount_delta == 0 && change.outstanding_delta == 0
        } else {
            true
        }
    })
}

pub(super) fn max_mpt_token_amount() -> u64 {
    protocol::MAX_MP_TOKEN_AMOUNT as u64
}

pub(super) fn mpt_id_from_issuance(sle: &STLedgerEntry) -> MPTID {
    protocol::make_mpt_id(
        sle.get_field_u32(sf("sfSequence")),
        sle.get_account_id(sf("sfIssuer")),
    )
}

pub(super) fn mpt_max_amount(sle: &STLedgerEntry) -> u64 {
    if sle.is_field_present(sf("sfMaximumAmount")) {
        sle.get_field_u64(sf("sfMaximumAmount"))
    } else {
        max_mpt_token_amount()
    }
}

pub(super) fn record_mpt_accounting(
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

pub(super) fn validates_mpt_accounting(
    data: &BTreeMap<MPTID, MptAccounting>,
    enforce: bool,
) -> bool {
    if !enforce {
        return true;
    }

    data.values().all(|entry| {
        !entry.overflow && entry.outstanding_after == entry.outstanding_before + entry.amount_delta
    })
}

/// Mirrors rippled's `ValidMPTBalanceChanges::finalize`: confidential MPT
/// transactions move value between the public amount and encrypted balances.
/// Their accounting is owned by `ValidConfidentialMPToken`, so applying the
/// ordinary `sfMPTAmount`/`sfOutstandingAmount` equation as well would reject
/// valid convert, convert-back, and clawback transactions.
pub(super) fn validates_mpt_accounting_for_transaction(
    data: &BTreeMap<MPTID, MptAccounting>,
    enforce: bool,
    txn_type: protocol::TxType,
    result: Ter,
    fix_cleanup_3_4_0: bool,
) -> bool {
    // Confidential MPT operations have their own encrypted-balance invariant;
    // do not subject them to public amount accounting on either outcome.
    if is_confidential_mpt_tx(txn_type) {
        return true;
    }
    if fix_cleanup_3_4_0
        && !protocol::is_tes_success(result)
        && data.values().any(|entry| entry.amount_delta != 0)
    {
        return false;
    }
    validates_mpt_accounting(data, enforce)
}

pub(super) fn record_mpt_transfer(
    transfers: &mut BTreeMap<MPTID, BTreeMap<AccountID, MptTransferAmount>>,
    sle: &STLedgerEntry,
    before: bool,
    is_delete: bool,
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
        entry.deleted |= is_delete;
    } else {
        entry.after = Some(amount);
        entry.authorized_after = sle.is_flag(protocol::lsfMPTAuthorized);
    }
}

pub(super) fn mpt_transfer_waives_can_transfer(
    txn_type: protocol::TxType,
    fix_cleanup_3_2_0: bool,
) -> bool {
    txn_type == protocol::TxType::AMM_WITHDRAW
        || (fix_cleanup_3_2_0
            && matches!(
                txn_type,
                protocol::TxType::VAULT_WITHDRAW
                    | protocol::TxType::LOAN_BROKER_COVER_WITHDRAW
                    | protocol::TxType::LOAN_PAY
            ))
}

pub(super) fn mpt_transfer_is_dex(
    txn_type: protocol::TxType,
    cross_currency_payment: bool,
) -> bool {
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

pub(super) fn validates_mpt_transfers<V: ApplyView + ?Sized>(
    sandbox: &FlowSandbox<V>,
    txn_type: protocol::TxType,
    result: Ter,
    cross_currency_payment: bool,
    fix_cleanup_3_2_0: bool,
    fix_cleanup_3_4_0: bool,
    mptokens_v2_enabled: bool,
    loan_default_accounts: Option<&LoanDefaultFreezeExemptAccounts>,
    pseudo_accounts_before: &BTreeMap<AccountID, bool>,
    transfers: &BTreeMap<MPTID, BTreeMap<AccountID, MptTransferAmount>>,
) -> Result<bool, ledger::ViewError> {
    // AMMClawback has rippled's OverrideFreeze privilege and bypasses this
    // invariant. Confidential transfers do not: public-balance mutations must
    // still be caught here, especially on the post-reset failure pass.
    if txn_type == protocol::TxType::AMM_CLAWBACK {
        return Ok(true);
    }
    let enforce = mptokens_v2_enabled || fix_cleanup_3_4_0;

    for (mpt_id, holders) in transfers {
        let issuance = sandbox.read(protocol::mpt_issuance_keylet_from_mptid(*mpt_id))?;
        if issuance.is_none() {
            // Orphaned zero-balance MPToken entries may be deleted later, but
            // no transaction may change their balance after the issuance is
            // gone.
            if holders.values().any(|value| {
                value
                    .after
                    .is_some_and(|after| value.before.unwrap_or(0) != after)
            }) {
                return Ok(!enforce);
            }
            continue;
        }
        let issuance = issuance.expect("checked above");

        let can_transfer = issuance.is_flag(protocol::lsfMPTCanTransfer)
            || mpt_transfer_waives_can_transfer(txn_type, fix_cleanup_3_2_0);
        let can_trade = issuance.is_flag(protocol::lsfMPTCanTrade);
        let req_auth = issuance.is_flag(protocol::lsfMPTRequireAuth);
        let issue = protocol::MPTIssue::new(*mpt_id);
        let loan_default_asset = loan_default_accounts.is_some_and(|accounts| {
            matches!(accounts.asset, protocol::Asset::MPTIssue(expected) if expected.mpt_id() == *mpt_id)
        });

        let mut senders = 0_u16;
        let mut receivers = 0_u16;
        // `is_frozen_mpt` includes the issuance-wide lock.  Evaluate that
        // condition per affected account so fixCleanup3_4_0 can exempt only
        // the resolved LoanBroker/Vault pseudo-account legs of a default;
        // any unrelated participant in the same locked issuance remains
        // rejected. Authorization is checked independently below.
        let mut invalid_transfer = false;
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
            let frozen = ledger::mptoken_helpers::is_frozen_mpt(sandbox, account, &issue)?;
            let exempt_from_freeze = loan_default_asset
                && loan_default_accounts.is_some_and(|accounts| {
                    *account == accounts.broker || *account == accounts.vault
                });
            // A pseudo account can be erased with the MPToken transfer that
            // drains it. Its poststate AccountRoot is then gone, so prefer the
            // classification captured from every touched prestate root.
            let authorized = if pseudo_accounts_before.get(account).copied() == Some(true) {
                true
            } else if req_auth && value.deleted {
                value.authorized_before
            } else if req_auth {
                protocol::is_tes_success(ledger::mptoken_helpers::require_auth_mpt(
                    sandbox, &issue, account,
                )?)
            } else {
                true
            };
            if (!exempt_from_freeze && frozen) || !authorized {
                invalid_transfer = true;
            }
        }

        // `Transactor::reset` discards every ordinary failed MPT mutation.
        // After fixCleanup3_4_0, seeing even one changed public balance in a
        // failed transaction is therefore an invariant failure.
        if fix_cleanup_3_4_0 && !protocol::is_tes_success(result) && (senders > 0 || receivers > 0)
        {
            return Ok(false);
        }

        if senders > 0
            && receivers > 0
            && (invalid_transfer
                || !can_transfer
                || (mpt_transfer_is_dex(txn_type, cross_currency_payment) && !can_trade))
        {
            return Ok(!(enforce || loan_default_accounts.is_some()));
        }
    }

    Ok(true)
}

pub(super) fn same_optional_h256(
    before: &STLedgerEntry,
    after: &STLedgerEntry,
    field: &'static protocol::SField,
) -> bool {
    let before_present = before.is_field_present(field);
    let after_present = after.is_field_present(field);
    before_present == after_present
        && (!before_present || before.get_field_h256(field) == after.get_field_h256(field))
}

pub(super) fn record_mpt_issuance_lifecycle<V: ApplyView + ?Sized>(
    sandbox: &FlowSandbox<V>,
    txn_type: protocol::TxType,
    lifecycle: &mut MptIssuanceLifecycle,
    is_delete: bool,
    before: Option<&STLedgerEntry>,
    after: Option<&STLedgerEntry>,
    deleted: &STLedgerEntry,
) -> Result<(), ledger::ViewError> {
    let fix_cleanup_3_2_0 = sandbox
        .rules()
        .enabled(&protocol::feature_id("fixCleanup3_2_0"));
    if let Some(after) = after
        && after.get_type() == LedgerEntryType::MPTokenIssuance
    {
        if before.is_none() {
            lifecycle.issuances_created = lifecycle.issuances_created.saturating_add(1);
            lifecycle.reference_holding_set_on_create |= fix_cleanup_3_2_0
                && after.is_field_present(sf("sfReferenceHolding"))
                && txn_type != protocol::TxType::VAULT_CREATE;
        } else if let Some(before) = before {
            // rippled #8152: lsfMPTLocked is the only issuance flag with a
            // legal clear path (tfMPTUnlock); every other flag is fixed at
            // creation or set-once. Accumulate any other cleared flag.
            lifecycle.issuance_flags_cleared |=
                before.get_flags() & !after.get_flags() & !protocol::MPT_LOCKED_LEDGER_FLAG;
            if fix_cleanup_3_2_0 {
                lifecycle.reference_holding_mutated |=
                    !same_optional_h256(before, after, sf("sfReferenceHolding"));
            }
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
        // rippled #8209: deleting an MPToken with a non-zero public balance is
        // rejected under fixCleanup3_5_0 (checked in the verdict).
        if deleted.get_field_u64(sf("sfMPTAmount")) > 0 {
            lifecycle.mptoken_deleted_with_balance = true;
        }
    }

    if !fix_cleanup_3_2_0 || !is_delete || txn_type == protocol::TxType::VAULT_DELETE {
        return Ok(());
    }

    lifecycle.vault_holding_deleted |= match deleted.get_type() {
        LedgerEntryType::MPToken => {
            let holder = deleted.get_account_id(sf("sfAccount"));
            is_vault_pseudo_account(sandbox, holder)?
        }
        LedgerEntryType::RippleState => {
            let low_counterparty = deleted.get_field_amount(sf("sfLowLimit")).issue().account;
            let high_counterparty = deleted.get_field_amount(sf("sfHighLimit")).issue().account;
            is_vault_pseudo_account(sandbox, low_counterparty)?
                || is_vault_pseudo_account(sandbox, high_counterparty)?
        }
        _ => false,
    };
    Ok(())
}

pub(super) fn is_vault_pseudo_account<V: ApplyView + ?Sized>(
    sandbox: &FlowSandbox<V>,
    account: AccountID,
) -> Result<bool, ledger::ViewError> {
    Ok(sandbox
        .read(protocol::account_keylet(raw_account_id(account)))?
        .is_some_and(|sle| sle.is_field_present(sf("sfVaultID"))))
}

pub(super) fn validates_mpt_issuance_lifecycle(
    lifecycle: &MptIssuanceLifecycle,
    fix_cleanup_3_5_0: bool,
) -> bool {
    // rippled #8152: post-fixCleanup3_5_0, clearing any issuance flag other
    // than lsfMPTLocked is an invariant violation.
    if fix_cleanup_3_5_0 && lifecycle.issuance_flags_cleared != 0 {
        return false;
    }
    // rippled #8209: post-fixCleanup3_5_0, erasing an MPToken with a non-zero
    // public balance is an invariant violation.
    if fix_cleanup_3_5_0 && lifecycle.mptoken_deleted_with_balance {
        return false;
    }
    !lifecycle.reference_holding_set_on_create
        && !lifecycle.reference_holding_mutated
        && !lifecycle.vault_holding_deleted
}

pub(super) fn has_create_mpt_issuance_privilege(txn_type: protocol::TxType) -> bool {
    matches!(
        txn_type,
        protocol::TxType::MPTOKEN_ISSUANCE_CREATE | protocol::TxType::VAULT_CREATE
    )
}

pub(super) fn has_destroy_mpt_issuance_privilege(txn_type: protocol::TxType) -> bool {
    matches!(
        txn_type,
        protocol::TxType::MPTOKEN_ISSUANCE_DESTROY | protocol::TxType::VAULT_DELETE
    )
}

pub(super) fn has_must_authorize_mpt_privilege(txn_type: protocol::TxType) -> bool {
    txn_type == protocol::TxType::MPTOKEN_AUTHORIZE
}

pub(super) fn has_may_authorize_mpt_privilege(txn_type: protocol::TxType) -> bool {
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

pub(super) fn has_may_create_mpt_privilege(txn_type: protocol::TxType) -> bool {
    matches!(
        txn_type,
        protocol::TxType::PAYMENT
            | protocol::TxType::OFFER_CREATE
            | protocol::TxType::CHECK_CASH
            | protocol::TxType::AMM_CREATE
    )
}

pub(super) fn has_may_delete_mpt_privilege(txn_type: protocol::TxType) -> bool {
    matches!(
        txn_type,
        protocol::TxType::AMM_DELETE
            | protocol::TxType::VAULT_WITHDRAW
            | protocol::TxType::VAULT_CLAWBACK
    )
}

pub(super) fn validates_mpt_lifecycle_counts(
    txn_type: protocol::TxType,
    result: Ter,
    tx_has_holder: bool,
    single_asset_vault_enabled: bool,
    lending_protocol_enabled: bool,
    mptokens_v2_enabled: bool,
    fix_cleanup_3_4_0: bool,
    lifecycle: &MptIssuanceLifecycle,
) -> bool {
    let applies =
        protocol::is_tes_success(result) || (mptokens_v2_enabled && result == Ter::TEC_INCOMPLETE);
    if !applies {
        return lifecycle.issuances_created == 0
            && lifecycle.issuances_deleted == 0
            && lifecycle.tokens_created == 0
            && lifecycle.tokens_deleted == 0;
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
            // An AMM has two pool assets. Either AMMWithdraw or
            // AMMClawback can therefore recreate zero, one, or both missing
            // recipient MPT holdings while deleting up to two emptied pool
            // holdings. This matches the bounded lifecycle exception in
            // rippled's MPT invariant.
            return lifecycle.tokens_created <= 2 && lifecycle.tokens_deleted <= 2;
        }

        // rippled a18839d92d: fixCleanup3_4_0 relaxes the LendingProtocol
        // MayAuthorizeMpt cap only for these two transaction shapes. LoanSet
        // may create its borrower and origination-fee holdings; VaultWithdraw
        // may create a destination asset holding while deleting emptied shares.
        let mptokens_exceed_authorize_cap = if !lending_protocol_enabled {
            false
        } else if fix_cleanup_3_4_0 {
            match txn_type {
                protocol::TxType::LOAN_SET => {
                    lifecycle.tokens_deleted != 0 || lifecycle.tokens_created > 2
                }
                protocol::TxType::VAULT_WITHDRAW => {
                    lifecycle.tokens_created > 1 || lifecycle.tokens_deleted > 1
                }
                _ => lifecycle.tokens_created + lifecycle.tokens_deleted > 1,
            }
        } else {
            lifecycle.tokens_created + lifecycle.tokens_deleted > 1
        };
        if mptokens_exceed_authorize_cap {
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

    if txn_type == protocol::TxType::ESCROW_FINISH {
        // EscrowFinish may create the destination MPToken. The stricter
        // SingleAssetVault/LendingProtocol case was validated above; rippled
        // unconditionally permits the legacy lifecycle shape here.
        debug_assert!(!enforce_escrow_finish);
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

#[cfg(test)]
mod tests {
    use super::{
        MptIssuanceLifecycle, validates_mpt_issuance_lifecycle, validates_mpt_lifecycle_counts,
    };
    use protocol::{Ter, TxType};

    fn lifecycle(tokens_created: u32, tokens_deleted: u32) -> MptIssuanceLifecycle {
        MptIssuanceLifecycle {
            tokens_created,
            tokens_deleted,
            ..MptIssuanceLifecycle::default()
        }
    }

    #[test]
    fn issuance_flag_clear_rejected_only_under_cleanup_3_5_0() {
        // rippled #8152: clearing a non-lock issuance flag fails post-3.5.0.
        let cleared = MptIssuanceLifecycle {
            issuance_flags_cleared: 0x0000_0002, // not lsfMPTLocked (0x1)
            ..MptIssuanceLifecycle::default()
        };
        assert!(
            validates_mpt_issuance_lifecycle(&cleared, false),
            "pre-3.5.0 the cleared flag is tolerated"
        );
        assert!(
            !validates_mpt_issuance_lifecycle(&cleared, true),
            "post-3.5.0 a cleared non-lock issuance flag is an invariant failure"
        );

        // Clearing only lsfMPTLocked is always allowed (it is masked out).
        let clean = MptIssuanceLifecycle::default();
        assert!(validates_mpt_issuance_lifecycle(&clean, true));
    }

    #[test]
    fn cleanup_3_4_0_relaxes_only_the_loan_set_and_vault_withdraw_mpt_caps() {
        let validates = |txn_type, fix_cleanup_3_4_0, tokens_created, tokens_deleted| {
            validates_mpt_lifecycle_counts(
                txn_type,
                Ter::TES_SUCCESS,
                false,
                true,
                true,
                true,
                fix_cleanup_3_4_0,
                &lifecycle(tokens_created, tokens_deleted),
            )
        };

        assert!(!validates(TxType::LOAN_SET, false, 2, 0));
        assert!(validates(TxType::LOAN_SET, true, 2, 0));
        assert!(!validates(TxType::LOAN_SET, true, 0, 1));
        assert!(validates(TxType::VAULT_WITHDRAW, true, 1, 1));
        assert!(!validates(TxType::VAULT_WITHDRAW, true, 2, 0));
        assert!(!validates(TxType::VAULT_WITHDRAW, true, 0, 2));
    }
}
