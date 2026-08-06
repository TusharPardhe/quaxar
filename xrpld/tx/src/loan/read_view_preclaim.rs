//! Immutable `ReadView` preclaim helpers for Loan and LoanBroker transactions.
//!
//! These helpers own the lending transaction types only. They derive every
//! result from immutable `ReadView` reads and return `None` for unowned types;
//! no apply path, sandbox, or success fallback is used.

use basics::base_uint::{Uint160, Uint256};
use ledger::{ReadView, RelativeDistanceAmount};
use protocol::{
    AccountID, Asset, STAmount, STLedgerEntry, STTx, Ter, TxType, get_field_by_symbol,
    lsfAllowTrustLineClawback, lsfGlobalFreeze, lsfHighAuth, lsfHighDeepFreeze, lsfHighFreeze,
    lsfLoanDefault, lsfLoanImpaired, lsfLoanOverpayment, lsfLowAuth, lsfLowDeepFreeze,
    lsfLowFreeze, lsfMPTCanClawback, lsfNoFreeze, lsfRequireAuth, tfLoanDefault, tfLoanImpair,
    tfLoanOverpayment, tfLoanUnimpair,
};

use crate::{
    LoanBrokerCoverClawbackAmountKind, LoanBrokerCoverClawbackPreclaimFacts,
    LoanBrokerCoverDepositPreclaimFacts, LoanBrokerCoverWithdrawPreclaimFacts,
    LoanBrokerDeletePreclaimFacts, LoanBrokerSetPreclaimFacts, LoanManagePreclaimFacts,
    LoanPayPreclaimFacts, LoanSetScheduleGuardInputs, check_loan_set_schedule_guard,
    run_loan_broker_cover_clawback_preclaim, run_loan_broker_cover_deposit_preclaim,
    run_loan_broker_cover_withdraw_preclaim, run_loan_broker_delete_preclaim,
    run_loan_broker_set_preclaim, run_loan_manage_preclaim, run_loan_pay_preclaim,
};

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}
fn read_error() -> Ter {
    Ter::TEF_BAD_LEDGER
}
fn account_keylet(account: AccountID) -> protocol::Keylet {
    protocol::account_keylet(Uint160::from_void(account.data()))
}
fn read<V: ReadView>(
    view: &V,
    keylet: protocol::Keylet,
) -> Result<Option<std::sync::Arc<STLedgerEntry>>, Ter> {
    view.read(keylet).map_err(|_| read_error())
}
fn account<V: ReadView>(
    view: &V,
    id: AccountID,
) -> Result<Option<std::sync::Arc<STLedgerEntry>>, Ter> {
    read(view, account_keylet(id))
}
fn broker<V: ReadView>(
    view: &V,
    id: Uint256,
) -> Result<Option<std::sync::Arc<STLedgerEntry>>, Ter> {
    read(view, protocol::loan_broker_keylet_from_key(id))
}
fn loan<V: ReadView>(view: &V, id: Uint256) -> Result<Option<std::sync::Arc<STLedgerEntry>>, Ter> {
    read(view, protocol::loan_keylet_from_key(id))
}
fn vault<V: ReadView>(view: &V, id: Uint256) -> Result<Option<std::sync::Arc<STLedgerEntry>>, Ter> {
    read(view, protocol::vault_keylet_from_key(id))
}

fn frozen<V: ReadView>(view: &V, id: AccountID, asset: Asset, deep: bool) -> Result<Ter, Ter> {
    match asset {
        Asset::Issue(issue) if issue.native() || issue.account == id => Ok(Ter::TES_SUCCESS),
        Asset::Issue(issue) => {
            let globally_frozen =
                account(view, issue.account)?.is_some_and(|sle| sle.is_flag(lsfGlobalFreeze));
            let line = read(view, protocol::line(id, issue.account, issue.currency))?;
            let flag = if deep {
                if id > issue.account {
                    lsfHighDeepFreeze
                } else {
                    lsfLowDeepFreeze
                }
            } else if id > issue.account {
                lsfHighFreeze
            } else {
                lsfLowFreeze
            };
            Ok(
                if globally_frozen || line.is_some_and(|sle| sle.is_flag(flag)) {
                    Ter::TEC_FROZEN
                } else {
                    Ter::TES_SUCCESS
                },
            )
        }
        Asset::MPTIssue(issue) => Ok(
            if ledger::mptoken_helpers::is_frozen_mpt(view, &id, &issue)
                .map_err(|_| read_error())?
            {
                Ter::TEC_LOCKED
            } else {
                Ter::TES_SUCCESS
            },
        ),
    }
}

fn auth<V: ReadView>(view: &V, id: AccountID, asset: Asset, strong: bool) -> Result<Ter, Ter> {
    match asset {
        Asset::Issue(issue) if issue.native() || issue.account == id => Ok(Ter::TES_SUCCESS),
        Asset::Issue(issue) => {
            let line = read(view, protocol::line(id, issue.account, issue.currency))?;
            if line.is_none() && strong {
                return Ok(Ter::TEC_NO_LINE);
            }
            if !account(view, issue.account)?.is_some_and(|sle| sle.is_flag(lsfRequireAuth)) {
                return Ok(Ter::TES_SUCCESS);
            }
            let Some(line) = line else {
                return Ok(Ter::TEC_NO_LINE);
            };
            Ok(
                if line.is_flag(if id > issue.account {
                    lsfLowAuth
                } else {
                    lsfHighAuth
                }) {
                    Ter::TES_SUCCESS
                } else {
                    Ter::TEC_NO_AUTH
                },
            )
        }
        Asset::MPTIssue(issue) => ledger::mptoken_helpers::require_auth_mpt_with_type(
            view,
            &issue,
            &id,
            if strong {
                ledger::mptoken_helpers::MPTAuthType::Strong
            } else {
                ledger::mptoken_helpers::MPTAuthType::Weak
            },
        )
        .map_err(|_| read_error()),
    }
}

fn holds_at_least<V: ReadView>(view: &V, id: AccountID, amount: &STAmount) -> Result<bool, Ter> {
    match amount.asset() {
        Asset::Issue(issue) if issue.native() => {
            Ok(account(view, id)?
                .is_some_and(|sle| sle.get_field_amount(sf("sfBalance")) >= *amount))
        }
        Asset::Issue(issue) if issue.account == id => Ok(true),
        Asset::Issue(issue) => {
            let Some(line) = read(view, protocol::line(id, issue.account, issue.currency))? else {
                return Ok(false);
            };
            let mut held = line.get_field_amount(sf("sfBalance"));
            if id > issue.account {
                held.negate();
            }
            held.set_issuer(issue.account);
            Ok(held >= *amount)
        }
        Asset::MPTIssue(issue) if issue.issuer() == id => Ok(true),
        Asset::MPTIssue(issue) => Ok(read(
            view,
            protocol::mptoken_keylet_from_mptid(issue.mpt_id(), Uint160::from_void(id.data())),
        )?
        .is_some_and(|sle| sle.get_field_u64(sf("sfMPTAmount")) as i64 >= amount.mpt().value())),
    }
}

fn transfer<V: ReadView>(
    view: &V,
    asset: Asset,
    from: AccountID,
    to: AccountID,
    waive_mpt_can_transfer: bool,
) -> Result<Ter, Ter> {
    match asset {
        Asset::MPTIssue(issue) if !waive_mpt_can_transfer => {
            ledger::mptoken_helpers::can_transfer_mpt(view, &issue, &from, &to)
                .map_err(|_| read_error())
        }
        _ => Ok(Ter::TES_SUCCESS),
    }
}

fn preclaim_loan_set<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    // Mirrors LoanSet.cpp's overflow check before its first ledger lookup.
    let schedule = LoanSetScheduleGuardInputs {
        start_date: view.parent_close_time().as_seconds(),
        payment_interval: tx
            .is_field_present(sf("sfPaymentInterval"))
            .then(|| tx.get_field_u32(sf("sfPaymentInterval"))),
        payment_total: tx
            .is_field_present(sf("sfPaymentTotal"))
            .then(|| tx.get_field_u32(sf("sfPaymentTotal"))),
        grace_period: tx
            .is_field_present(sf("sfGracePeriod"))
            .then(|| tx.get_field_u32(sf("sfGracePeriod"))),
        default_payment_interval: 60,
        default_payment_total: 1,
        default_grace_period: 60,
    };
    if check_loan_set_schedule_guard(schedule).is_err() {
        return Ok(Ter::TEC_KILLED);
    }
    let account_id = tx.get_account_id(sf("sfAccount"));
    let Some(broker_sle) = broker(view, tx.get_field_h256(sf("sfLoanBrokerID")))? else {
        return Ok(Ter::TEC_NO_ENTRY);
    };
    let broker_owner = broker_sle.get_account_id(sf("sfOwner"));
    let borrower = tx
        .is_field_present(sf("sfCounterparty"))
        .then(|| tx.get_account_id(sf("sfCounterparty")))
        .unwrap_or(account_id);
    if account_id != broker_owner && borrower != broker_owner {
        return Ok(Ter::TEC_NO_PERMISSION);
    }
    if account(view, borrower)?.is_none() {
        return Ok(Ter::TER_NO_ACCOUNT);
    }
    let Some(vault_sle) = vault(view, broker_sle.get_field_h256(sf("sfVaultID")))? else {
        return Ok(Ter::TEF_BAD_LEDGER);
    };
    if vault_sle.get_field_number(sf("sfAssetsMaximum")).value()
        != basics::number::NumberParts::zero()
        && vault_sle.get_field_number(sf("sfAssetsTotal")).value()
            >= vault_sle.get_field_number(sf("sfAssetsMaximum")).value()
    {
        return Ok(Ter::TEC_LIMIT_EXCEEDED);
    }
    let asset = vault_sle.get_field_issue(sf("sfAsset")).asset();
    let can_add = ledger::can_add_holding(view, &asset);
    if can_add != Ter::TES_SUCCESS {
        return Ok(can_add);
    }
    for (id, deep) in [
        (vault_sle.get_account_id(sf("sfAccount")), false),
        (broker_sle.get_account_id(sf("sfAccount")), true),
        (borrower, false),
        (broker_owner, true),
    ] {
        let result = frozen(view, id, asset, deep)?;
        if result != Ter::TES_SUCCESS {
            return Ok(result);
        }
    }
    Ok(Ter::TES_SUCCESS)
}

fn preclaim_manage<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let account_id = tx.get_account_id(sf("sfAccount"));
    let Some(loan_sle) = loan(view, tx.get_field_h256(sf("sfLoanID")))? else {
        return Ok(run_loan_manage_preclaim(LoanManagePreclaimFacts::default()));
    };
    let broker_sle = broker(view, loan_sle.get_field_h256(sf("sfLoanBrokerID")))?;
    let expiry = loan_sle
        .get_field_u32(sf("sfNextPaymentDueDate"))
        .saturating_add(loan_sle.get_field_u32(sf("sfGracePeriod")));
    Ok(run_loan_manage_preclaim(LoanManagePreclaimFacts {
        loan_exists: true,
        loan_is_defaulted: loan_sle.is_flag(lsfLoanDefault),
        loan_is_impaired: loan_sle.is_flag(lsfLoanImpaired),
        tx_requests_impair: tx.is_flag(tfLoanImpair),
        tx_requests_unimpair: tx.is_flag(tfLoanUnimpair),
        tx_requests_default: tx.is_flag(tfLoanDefault),
        payment_remaining_is_zero: loan_sle.get_field_u32(sf("sfPaymentRemaining")) == 0,
        default_is_too_soon: tx.is_flag(tfLoanDefault)
            && view.parent_close_time().as_seconds() < expiry,
        broker_exists: broker_sle.is_some(),
        submitter_is_broker_owner: broker_sle
            .as_ref()
            .is_some_and(|sle| sle.get_account_id(sf("sfOwner")) == account_id),
    }))
}

fn preclaim_pay<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let account_id = tx.get_account_id(sf("sfAccount"));
    let amount = tx.get_field_amount(sf("sfAmount"));
    let Some(loan_sle) = loan(view, tx.get_field_h256(sf("sfLoanID")))? else {
        return Ok(run_loan_pay_preclaim(LoanPayPreclaimFacts::default()));
    };
    let broker_sle = broker(view, loan_sle.get_field_h256(sf("sfLoanBrokerID")))?;
    let vault_sle = match broker_sle.as_ref() {
        Some(b) => vault(view, b.get_field_h256(sf("sfVaultID")))?,
        None => None,
    };
    let asset = vault_sle
        .as_ref()
        .map(|sle| sle.get_field_issue(sf("sfAsset")).asset());
    Ok(run_loan_pay_preclaim(LoanPayPreclaimFacts {
        loan_exists: true,
        submitter_is_borrower: loan_sle.get_account_id(sf("sfBorrower")) == account_id,
        tx_requests_overpayment: tx.is_flag(tfLoanOverpayment),
        loan_allows_overpayment: loan_sle.is_flag(lsfLoanOverpayment),
        fix_cleanup_3_1_3_enabled: view
            .rules()
            .enabled(&protocol::feature_id("fixCleanup3_1_3")),
        principal_outstanding_is_zero: loan_sle
            .get_field_number(sf("sfPrincipalOutstanding"))
            .value()
            == basics::number::NumberParts::zero(),
        payment_remaining_is_zero: loan_sle.get_field_u32(sf("sfPaymentRemaining")) == 0,
        broker_exists: broker_sle.is_some(),
        vault_exists: vault_sle.is_some(),
        amount_matches_vault_asset: asset == Some(amount.asset()),
        frozen_result: asset
            .map(|a| frozen(view, account_id, a, false).unwrap_or(Ter::TEF_BAD_LEDGER))
            .unwrap_or(Ter::TES_SUCCESS),
        deep_frozen_result: match (asset, vault_sle.as_ref()) {
            (Some(a), Some(v)) => frozen(view, v.get_account_id(sf("sfAccount")), a, true)
                .unwrap_or(Ter::TEF_BAD_LEDGER),
            _ => Ter::TES_SUCCESS,
        },
        require_auth_result: asset
            .map(|a| auth(view, account_id, a, false).unwrap_or(Ter::TEF_BAD_LEDGER))
            .unwrap_or(Ter::TES_SUCCESS),
        balance_is_less_than_amount: asset
            .map(|_| {
                holds_at_least(view, account_id, &amount)
                    .map(|has| !has)
                    .unwrap_or(true)
            })
            .unwrap_or(false),
    }))
}

fn preclaim_delete<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let Some(loan_sle) = loan(view, tx.get_field_h256(sf("sfLoanID")))? else {
        return Ok(Ter::TEC_NO_ENTRY);
    };
    if loan_sle.get_field_u32(sf("sfPaymentRemaining")) > 0 {
        return Ok(Ter::TEC_HAS_OBLIGATIONS);
    }
    let Some(broker_sle) = broker(view, loan_sle.get_field_h256(sf("sfLoanBrokerID")))? else {
        return Ok(Ter::TEC_INTERNAL);
    };
    let account_id = tx.get_account_id(sf("sfAccount"));
    Ok(
        if broker_sle.get_account_id(sf("sfOwner")) != account_id
            && loan_sle.get_account_id(sf("sfBorrower")) != account_id
        {
            Ter::TEC_NO_PERMISSION
        } else {
            Ter::TES_SUCCESS
        },
    )
}

fn preclaim_broker_set<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let account_id = tx.get_account_id(sf("sfAccount"));
    let vault_id = tx.get_field_h256(sf("sfVaultID"));
    let Some(vault_sle) = vault(view, vault_id)? else {
        return Ok(run_loan_broker_set_preclaim(
            LoanBrokerSetPreclaimFacts::default(),
        ));
    };
    let asset = vault_sle.get_field_issue(sf("sfAsset")).asset();
    let existing = if tx.is_field_present(sf("sfLoanBrokerID")) {
        broker(view, tx.get_field_h256(sf("sfLoanBrokerID")))?
    } else {
        None
    };
    let debt_max = tx
        .is_field_present(sf("sfDebtMaximum"))
        .then(|| tx.get_field_number(sf("sfDebtMaximum")).value());
    Ok(run_loan_broker_set_preclaim(LoanBrokerSetPreclaimFacts {
        vault_exists: true,
        submitter_is_vault_owner: vault_sle.get_account_id(sf("sfOwner")) == account_id,
        broker_id_is_present: tx.is_field_present(sf("sfLoanBrokerID")),
        broker_exists: existing.is_some(),
        vault_id_matches_existing_broker: existing
            .as_ref()
            .is_some_and(|b| b.get_field_h256(sf("sfVaultID")) == vault_id),
        submitter_is_broker_owner: existing
            .as_ref()
            .is_some_and(|b| b.get_account_id(sf("sfOwner")) == account_id),
        debt_maximum_is_zero_or_not_below_current_debt: debt_max.is_none_or(|d| {
            d == basics::number::NumberParts::zero()
                || existing
                    .as_ref()
                    .is_some_and(|b| d >= b.get_field_number(sf("sfDebtTotal")).value())
        }),
        debt_maximum_is_present: debt_max.is_some(),
        debt_maximum_is_representable: debt_max.is_none_or(|d| asset.amount(d).is_ok()),
        can_add_holding_result: ledger::can_add_holding(view, &asset),
        check_frozen_result: frozen(
            view,
            vault_sle.get_account_id(sf("sfAccount")),
            asset,
            false,
        )?,
    }))
}

fn preclaim_cover_deposit<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let account_id = tx.get_account_id(sf("sfAccount"));
    let amount = tx.get_field_amount(sf("sfAmount"));
    let broker_sle = broker(view, tx.get_field_h256(sf("sfLoanBrokerID")))?;
    let vault_sle = match broker_sle.as_ref() {
        Some(b) => vault(view, b.get_field_h256(sf("sfVaultID")))?,
        None => None,
    };
    let asset = vault_sle
        .as_ref()
        .map(|v| v.get_field_issue(sf("sfAsset")).asset());
    Ok(run_loan_broker_cover_deposit_preclaim(
        LoanBrokerCoverDepositPreclaimFacts {
            broker_exists: broker_sle.is_some(),
            submitter_is_broker_owner: broker_sle
                .as_ref()
                .is_some_and(|b| b.get_account_id(sf("sfOwner")) == account_id),
            vault_exists: vault_sle.is_some(),
            amount_matches_vault_asset: asset == Some(amount.asset()),
            can_transfer_result: match (asset, broker_sle.as_ref()) {
                (Some(a), Some(b)) => transfer(
                    view,
                    a,
                    account_id,
                    b.get_account_id(sf("sfAccount")),
                    false,
                )?,
                _ => Ter::TES_SUCCESS,
            },
            frozen_result: asset
                .map(|a| frozen(view, account_id, a, false).unwrap_or(Ter::TEF_BAD_LEDGER))
                .unwrap_or(Ter::TES_SUCCESS),
            deep_frozen_result: match (asset, broker_sle.as_ref()) {
                (Some(a), Some(b)) => frozen(view, b.get_account_id(sf("sfAccount")), a, true)
                    .unwrap_or(Ter::TEF_BAD_LEDGER),
                _ => Ter::TES_SUCCESS,
            },
            require_auth_result: asset
                .map(|a| auth(view, account_id, a, true).unwrap_or(Ter::TEF_BAD_LEDGER))
                .unwrap_or(Ter::TES_SUCCESS),
            balance_is_less_than_amount: asset
                .map(|_| {
                    holds_at_least(view, account_id, &amount)
                        .map(|has| !has)
                        .unwrap_or(true)
                })
                .unwrap_or(false),
        },
    ))
}

fn preclaim_cover_withdraw<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let account_id = tx.get_account_id(sf("sfAccount"));
    let destination = tx
        .is_field_present(sf("sfDestination"))
        .then(|| tx.get_account_id(sf("sfDestination")))
        .unwrap_or(account_id);
    let amount = tx.get_field_amount(sf("sfAmount"));
    let broker_sle = broker(view, tx.get_field_h256(sf("sfLoanBrokerID")))?;
    let vault_sle = match broker_sle.as_ref() {
        Some(b) => vault(view, b.get_field_h256(sf("sfVaultID")))?,
        None => None,
    };
    let asset = vault_sle
        .as_ref()
        .map(|v| v.get_field_issue(sf("sfAsset")).asset());
    let cover = broker_sle
        .as_ref()
        .map(|b| b.get_field_number(sf("sfCoverAvailable")).value())
        .unwrap_or_else(basics::number::NumberParts::zero);
    let pseudo_destination = account(view, destination)?.is_some_and(|sle| {
        sle.is_field_present(sf("sfVaultID"))
            || sle.is_field_present(sf("sfLoanBrokerID"))
            || sle.is_field_present(sf("sfAMMID"))
    });
    Ok(run_loan_broker_cover_withdraw_preclaim(
        LoanBrokerCoverWithdrawPreclaimFacts {
            destination_is_pseudo_account: pseudo_destination,
            broker_exists: broker_sle.is_some(),
            submitter_is_broker_owner: broker_sle
                .as_ref()
                .is_some_and(|b| b.get_account_id(sf("sfOwner")) == account_id),
            vault_exists: vault_sle.is_some(),
            amount_matches_vault_asset: asset == Some(amount.asset()),
            destination_is_submitter: destination == account_id,
            destination_is_vault_asset_issuer: asset.is_some_and(|a| destination == a.issuer()),
            cover_available_at_least_amount: cover >= amount.as_number(),
            cover_after_withdraw_at_least_minimum: cover >= amount.as_number(),
            pseudo_balance_at_least_amount: match broker_sle.as_ref() {
                Some(b) => holds_at_least(view, b.get_account_id(sf("sfAccount")), &amount)?,
                None => true,
            },
            can_transfer_result: match (asset, broker_sle.as_ref()) {
                (Some(a), Some(b)) => transfer(
                    view,
                    a,
                    b.get_account_id(sf("sfAccount")),
                    destination,
                    view.rules()
                        .enabled(&protocol::feature_id("fixCleanup3_2_0")),
                )?,
                _ => Ter::TES_SUCCESS,
            },
            can_withdraw_result: Ter::TES_SUCCESS,
            require_auth_result: asset
                .map(|a| {
                    auth(view, destination, a, destination != account_id)
                        .unwrap_or(Ter::TEF_BAD_LEDGER)
                })
                .unwrap_or(Ter::TES_SUCCESS),
            source_frozen_result: match (asset, broker_sle.as_ref()) {
                (Some(a), Some(b)) => frozen(view, b.get_account_id(sf("sfAccount")), a, false)
                    .unwrap_or(Ter::TEF_BAD_LEDGER),
                _ => Ter::TES_SUCCESS,
            },
            destination_deep_frozen_result: asset
                .map(|a| frozen(view, destination, a, true).unwrap_or(Ter::TEF_BAD_LEDGER))
                .unwrap_or(Ter::TES_SUCCESS),
        },
    ))
}

fn preclaim_cover_clawback<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let Some(broker_sle) = broker(view, tx.get_field_h256(sf("sfLoanBrokerID")))? else {
        return Ok(run_loan_broker_cover_clawback_preclaim(
            LoanBrokerCoverClawbackPreclaimFacts {
                broker_id_resolution_result: Ter::TES_SUCCESS,
                ..Default::default()
            },
        ));
    };
    let Some(vault_sle) = vault(view, broker_sle.get_field_h256(sf("sfVaultID")))? else {
        return Ok(Ter::TEF_BAD_LEDGER);
    };
    let account_id = tx.get_account_id(sf("sfAccount"));
    let asset = vault_sle.get_field_issue(sf("sfAsset")).asset();
    let amount = tx
        .is_field_present(sf("sfAmount"))
        .then(|| tx.get_field_amount(sf("sfAmount")));
    let issuer_root = match asset {
        Asset::Issue(issue) => account(view, issue.account)?,
        _ => None,
    };
    let issuance = match asset {
        Asset::MPTIssue(issue) => read(
            view,
            protocol::mpt_issuance_keylet_from_mptid(issue.mpt_id()),
        )?,
        _ => None,
    };
    Ok(run_loan_broker_cover_clawback_preclaim(
        LoanBrokerCoverClawbackPreclaimFacts {
            broker_id_resolution_result: Ter::TES_SUCCESS,
            broker_exists: true,
            vault_exists: true,
            vault_asset_is_native: asset.native(),
            submitter_is_vault_asset_issuer: asset.issuer() == account_id,
            amount_is_present: amount.is_some(),
            amount_asset_matches_vault_asset: amount.as_ref().is_none_or(|a| a.asset() == asset),
            claw_amount_can_be_determined: broker_sle
                .get_field_number(sf("sfCoverAvailable"))
                .value()
                > basics::number::NumberParts::zero(),
            pseudo_balance_at_least_claw_amount: true,
            issuer_account_exists: issuer_root.is_some() || issuance.is_some(),
            amount_kind: if matches!(asset, Asset::MPTIssue(_)) {
                LoanBrokerCoverClawbackAmountKind::Mpt
            } else {
                LoanBrokerCoverClawbackAmountKind::Issue
            },
            mpt_issuance_exists: issuance.is_some(),
            mpt_can_clawback: issuance
                .as_ref()
                .is_some_and(|i| i.is_flag(lsfMPTCanClawback)),
            mpt_issuer_matches_submitter: issuance
                .as_ref()
                .is_some_and(|i| i.get_account_id(sf("sfIssuer")) == account_id),
            issuer_allows_trustline_clawback: issuer_root
                .as_ref()
                .is_some_and(|i| i.is_flag(lsfAllowTrustLineClawback)),
            issuer_has_no_freeze: issuer_root.as_ref().is_some_and(|i| i.is_flag(lsfNoFreeze)),
        },
    ))
}

fn preclaim_broker_delete<V: ReadView>(view: &V, tx: &STTx) -> Result<Ter, Ter> {
    let account_id = tx.get_account_id(sf("sfAccount"));
    let broker_sle = broker(view, tx.get_field_h256(sf("sfLoanBrokerID")))?;
    let vault_sle = match broker_sle.as_ref() {
        Some(b) => vault(view, b.get_field_h256(sf("sfVaultID")))?,
        None => None,
    };
    let asset = vault_sle
        .as_ref()
        .map(|v| v.get_field_issue(sf("sfAsset")).asset());
    let cover = broker_sle
        .as_ref()
        .map(|b| b.get_field_number(sf("sfCoverAvailable")).value())
        .unwrap_or_else(basics::number::NumberParts::zero);
    Ok(run_loan_broker_delete_preclaim(
        LoanBrokerDeletePreclaimFacts {
            broker_exists: broker_sle.is_some(),
            submitter_is_broker_owner: broker_sle
                .as_ref()
                .is_some_and(|b| b.get_account_id(sf("sfOwner")) == account_id),
            owner_count_is_zero: broker_sle
                .as_ref()
                .is_some_and(|b| b.get_field_u32(sf("sfOwnerCount")) == 0),
            vault_exists: vault_sle.is_some(),
            rounded_debt_total_is_zero: broker_sle.as_ref().is_none_or(|b| {
                b.get_field_number(sf("sfDebtTotal")).value() == basics::number::NumberParts::zero()
            }),
            cover_available_is_positive: cover > basics::number::NumberParts::zero(),
            deep_frozen_result: asset
                .map(|a| frozen(view, account_id, a, true).unwrap_or(Ter::TEF_BAD_LEDGER))
                .unwrap_or(Ter::TES_SUCCESS),
        },
    ))
}

/// Runs an owned Loan/LoanBroker preclaim against an immutable view.
/// `None` identifies an unowned type and is never a success fallback.
pub fn run_loan_read_view_preclaim<V: ReadView>(
    view: &V,
    tx: &STTx,
    txn_type: TxType,
) -> Option<Ter> {
    let result = match txn_type {
        TxType::LOAN_SET => preclaim_loan_set(view, tx),
        TxType::LOAN_MANAGE => preclaim_manage(view, tx),
        TxType::LOAN_PAY => preclaim_pay(view, tx),
        TxType::LOAN_DELETE => preclaim_delete(view, tx),
        TxType::LOAN_BROKER_SET => preclaim_broker_set(view, tx),
        TxType::LOAN_BROKER_DELETE => preclaim_broker_delete(view, tx),
        TxType::LOAN_BROKER_COVER_DEPOSIT => preclaim_cover_deposit(view, tx),
        TxType::LOAN_BROKER_COVER_WITHDRAW => preclaim_cover_withdraw(view, tx),
        TxType::LOAN_BROKER_COVER_CLAWBACK => preclaim_cover_clawback(view, tx),
        _ => return None,
    };
    Some(result.unwrap_or_else(|ter| ter))
}

#[cfg(test)]
mod tests {
    use super::{run_loan_read_view_preclaim, sf};
    use basics::base_uint::Uint256;
    use ledger::{Fees, LedgerHeader, ReadView, ReadViewTx, Rules, ViewError};
    use protocol::{STLedgerEntry, STTx, Ter, TxType};
    use std::{collections::BTreeMap, sync::Arc};
    #[derive(Debug, Default)]
    struct View {
        entries: BTreeMap<Uint256, Arc<STLedgerEntry>>,
    }
    impl ReadView for View {
        fn open(&self) -> bool {
            false
        }
        fn header(&self) -> LedgerHeader {
            LedgerHeader::default()
        }
        fn fees(&self) -> Fees {
            Fees::default()
        }
        fn rules(&self) -> Rules {
            Rules::default()
        }
        fn exists(&self, k: protocol::Keylet) -> Result<bool, ViewError> {
            Ok(self.entries.contains_key(&k.key))
        }
        fn succ(&self, _: Uint256, _: Option<Uint256>) -> Result<Option<Uint256>, ViewError> {
            Ok(None)
        }
        fn read(&self, k: protocol::Keylet) -> Result<Option<Arc<STLedgerEntry>>, ViewError> {
            Ok(self.entries.get(&k.key).cloned())
        }
        fn sles(&self) -> Result<Vec<Arc<STLedgerEntry>>, ViewError> {
            Ok(self.entries.values().cloned().collect())
        }
        fn tx_exists(&self, _: Uint256) -> Result<bool, ViewError> {
            Ok(false)
        }
        fn tx_read(&self, _: Uint256) -> Result<Option<ReadViewTx>, ViewError> {
            Ok(None)
        }
        fn txs(&self) -> Result<Vec<ReadViewTx>, ViewError> {
            Ok(Vec::new())
        }
    }
    #[test]
    fn loan_helper_has_no_unowned_success_default() {
        let tx = STTx::new(TxType::PAYMENT, |_| {});
        assert_eq!(
            run_loan_read_view_preclaim(&View::default(), &tx, TxType::PAYMENT),
            None
        );
    }
    #[test]
    fn loan_manage_reads_missing_loan_without_mutation() {
        let tx = STTx::new(TxType::LOAN_MANAGE, |tx| {
            tx.set_field_h256(sf("sfLoanID"), Uint256::from_u64(9));
        });
        let view = View::default();
        assert_eq!(
            run_loan_read_view_preclaim(&view, &tx, TxType::LOAN_MANAGE),
            Some(Ter::TEC_NO_ENTRY)
        );
        assert!(view.entries.is_empty());
    }
}
