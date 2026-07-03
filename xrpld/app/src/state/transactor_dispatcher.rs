//! Transactor dispatcher — routes `TxType` to real view-backed engines.

use crate::state::transactor_apply_bridge::*;
use crate::state::transactor_escrow_bridge::*;
use basics::math::base_uint::{Uint160, Uint256};
use basics::number::{NumberParts as RuntimeNumber, RoundingMode};
use protocol::{
    AUCTION_SLOT_DISCOUNTED_FEE_FRACTION, AccountID, Asset, IOUAmount, Keylet, LedgerEntryType,
    MPTAmount, STAmount, STArray, STIssue, STLedgerEntry, STObject, STTx, Ter, TxType,
    VOTE_MAX_SLOTS, VOTE_WEIGHT_SCALE_FACTOR, XRPAmount, get_field_by_symbol, is_tes_success,
    lsfDisableMaster, owner_dir_keylet, signers_keylet,
};
use std::sync::Arc;
use tx::*;

fn sf(name: &str) -> &'static protocol::SField {
    get_field_by_symbol(name)
}

fn tx_amm_asset(tx: &STTx, field: &'static protocol::SField) -> Asset {
    if let Some(value) = tx.peek_at_pfield(field) {
        if let Some(issue) = value.as_any().downcast_ref::<STIssue>() {
            return issue.asset();
        }
        if let Some(amount) = value.as_any().downcast_ref::<STAmount>() {
            return amount.asset();
        }
    }
    tx.get_field_issue(field).asset()
}

fn optional_tx_amount(tx: &STTx, field: &'static protocol::SField) -> Option<STAmount> {
    tx.is_field_present(field)
        .then(|| tx.get_field_amount(field))
}

fn check_amm_mptokens_v2_gate<V: ledger::ApplyView>(view: &V, assets: &[Asset]) -> Ter {
    if view.rules().enabled(&protocol::feature_id("MPTokensV2")) {
        return Ter::TES_SUCCESS;
    }

    if assets
        .iter()
        .any(|asset| matches!(asset, Asset::MPTIssue(_)))
    {
        return Ter::TEM_DISABLED;
    }

    Ter::TES_SUCCESS
}

fn zero_amount_for_asset(field: &'static protocol::SField, asset: Asset) -> STAmount {
    match asset {
        Asset::Issue(issue) if issue.native() => STAmount::from_xrp_amount(XRPAmount::new()),
        Asset::Issue(issue) => STAmount::from_iou_amount(field, IOUAmount::new(), issue),
        Asset::MPTIssue(issue) => STAmount::from_mpt_amount(field, MPTAmount::from_value(0), issue),
    }
}

fn account_holds_amm_asset<V: ledger::ApplyView>(
    view: &V,
    account: &AccountID,
    asset: Asset,
    field: &'static protocol::SField,
) -> Option<STAmount> {
    match asset {
        Asset::Issue(issue) if issue.native() => Some(
            view.read(protocol::account_keylet(Uint160::from_void(account.data())))
                .ok()
                .flatten()
                .map(|sle| sle.get_field_amount(sf("sfBalance")))
                .unwrap_or_else(|| STAmount::from_xrp_amount(XRPAmount::new())),
        ),
        Asset::Issue(issue) => {
            if issue.account == *account {
                return Some(STAmount::from_iou_amount(field, IOUAmount::new(), issue));
            }
            let mut amount = view
                .read(protocol::line(*account, issue.account, issue.currency))
                .ok()
                .flatten()
                .map(|sle| sle.get_field_amount(sf("sfBalance")))
                .unwrap_or_else(|| STAmount::from_iou_amount(field, IOUAmount::new(), issue));
            if *account > issue.account {
                amount.negate();
            }
            amount.set_issuer(issue.account);
            Some(amount)
        }
        Asset::MPTIssue(issue) => {
            let value = view
                .read(protocol::mptoken_keylet_from_mptid(
                    issue.mpt_id(),
                    Uint160::from_void(account.data()),
                ))
                .ok()
                .flatten()
                .map(|sle| {
                    if sle.is_field_present(sf("sfMPTAmount")) {
                        sle.get_field_u64(sf("sfMPTAmount"))
                    } else {
                        0
                    }
                })
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

fn amm_deposit_asset<V: ledger::ApplyView>(
    view: &mut V,
    from: &AccountID,
    amm_account: &AccountID,
    amount: &STAmount,
) -> Ter {
    match amount.asset() {
        Asset::Issue(issue) if issue.native() => {
            ledger::ripple_state_helpers::account_send(view, from, amm_account, amount)
        }
        Asset::Issue(_) => amm_transfer_iou_no_fee(view, from, amm_account, amount),
        Asset::MPTIssue(_) => amm_transfer_mpt_no_fee(view, from, amm_account, amount),
    }
}

fn amm_transfer_iou_no_fee<V: ledger::ApplyView>(
    view: &mut V,
    from: &AccountID,
    to: &AccountID,
    amount: &STAmount,
) -> Ter {
    let issue = amount.issue();
    if *from == issue.account || *to == issue.account || issue.account.is_zero() {
        return ledger::ripple_state_helpers::direct_send_no_fee_iou_pub(view, from, to, amount);
    }

    let result =
        ledger::ripple_state_helpers::direct_send_no_fee_iou_pub(view, &issue.account, to, amount);
    if result != Ter::TES_SUCCESS {
        return result;
    }
    ledger::ripple_state_helpers::direct_send_no_fee_iou_pub(view, from, &issue.account, amount)
}

fn amm_transfer_mpt_no_fee<V: ledger::ApplyView>(
    view: &mut V,
    from: &AccountID,
    to: &AccountID,
    amount: &STAmount,
) -> Ter {
    let Asset::MPTIssue(issue) = amount.asset() else {
        return Ter::TEC_INTERNAL;
    };
    let value = amount.mpt().value();
    if value <= 0 || from == to {
        return Ter::TES_SUCCESS;
    }
    let Ok(units) = u64::try_from(value) else {
        return Ter::TEC_INTERNAL;
    };
    let debit_keylet =
        protocol::mptoken_keylet_from_mptid(issue.mpt_id(), Uint160::from_void(from.data()));
    let credit_keylet =
        protocol::mptoken_keylet_from_mptid(issue.mpt_id(), Uint160::from_void(to.data()));
    let Some(debit_token) = view.peek(debit_keylet).ok().flatten() else {
        return Ter::TEC_AMM_BALANCE;
    };
    let Some(credit_token) = view.peek(credit_keylet).ok().flatten() else {
        return Ter::TEC_NO_AUTH;
    };
    let debit_balance = debit_token.get_field_u64(sf("sfMPTAmount"));
    let Some(next_debit) = debit_balance.checked_sub(units) else {
        return Ter::TEC_AMM_BALANCE;
    };
    let credit_balance = credit_token.get_field_u64(sf("sfMPTAmount"));
    let Some(next_credit) = credit_balance.checked_add(units) else {
        return Ter::TEC_INTERNAL;
    };

    let mut debit_obj = debit_token.clone_as_object();
    debit_obj.set_field_u64(sf("sfMPTAmount"), next_debit);
    if view
        .update(Arc::new(STLedgerEntry::from_stobject(
            debit_obj,
            *debit_token.key(),
        )))
        .is_err()
    {
        return Ter::TEF_BAD_LEDGER;
    }

    let mut credit_obj = credit_token.clone_as_object();
    credit_obj.set_field_u64(sf("sfMPTAmount"), next_credit);
    if view
        .update(Arc::new(STLedgerEntry::from_stobject(
            credit_obj,
            *credit_token.key(),
        )))
        .is_err()
    {
        return Ter::TEF_BAD_LEDGER;
    }

    Ter::TES_SUCCESS
}

fn amm_withdraw_asset<V: ledger::ApplyView>(
    view: &mut V,
    amm_account: &AccountID,
    account: &AccountID,
    amount: &STAmount,
) -> Ter {
    match amount.asset() {
        Asset::Issue(issue) if issue.native() => {
            ledger::ripple_state_helpers::account_send(view, amm_account, account, amount)
        }
        Asset::Issue(_) => amm_transfer_iou_no_fee(view, amm_account, account, amount),
        Asset::MPTIssue(_) => amm_transfer_mpt_no_fee(view, amm_account, account, amount),
    }
}

fn amount_from_number(
    asset: Asset,
    value: RuntimeNumber,
    rounding: RoundingMode,
) -> Option<STAmount> {
    protocol::to_amount_from_number(asset, value, rounding).ok()
}

fn amm_clawback_proportional_amount(
    balance: &STAmount,
    frac: RuntimeNumber,
    rounding: RoundingMode,
) -> Option<STAmount> {
    amount_from_number(
        balance.asset(),
        ledger::amm_helpers::stamount_as_number(balance) * frac,
        rounding,
    )
}

fn amm_clawback_lp_tokens(
    lp_total: &STAmount,
    frac: RuntimeNumber,
    rounding: RoundingMode,
) -> Option<STAmount> {
    amount_from_number(
        lp_total.asset(),
        ledger::amm_helpers::stamount_as_number(lp_total) * frac,
        rounding,
    )
}

fn amm_clawback_math(
    amount: Option<&STAmount>,
    pool1: &STAmount,
    pool2: &STAmount,
    lp_total: &STAmount,
    holder_lp: &STAmount,
    rules: protocol::Rules,
) -> Result<tx::AMMWithdrawApplyMathResult, Ter> {
    let full_withdraw = |holder_lp: &STAmount| -> Result<tx::AMMWithdrawApplyMathResult, Ter> {
        if holder_lp.signum() == 0 {
            return Err(Ter::TEC_AMM_BALANCE);
        }
        let frac = ledger::amm_helpers::stamount_as_number(holder_lp)
            / ledger::amm_helpers::stamount_as_number(lp_total);
        let amount1 = amm_clawback_proportional_amount(pool1, frac, RoundingMode::Downward)
            .ok_or(Ter::TEC_INTERNAL)?;
        let amount2 = amm_clawback_proportional_amount(pool2, frac, RoundingMode::Downward)
            .ok_or(Ter::TEC_INTERNAL)?;
        if amount1.signum() == 0 || amount2.signum() == 0 {
            return Err(Ter::TEC_AMM_FAILED);
        }
        Ok(tx::AMMWithdrawApplyMathResult {
            amount1: Some(amount1),
            amount2: Some(amount2),
            lp_tokens: holder_lp.clone(),
            new_lp_token_balance: lp_total.clone() - holder_lp.clone(),
        })
    };

    let Some(amount) = amount else {
        return full_withdraw(holder_lp);
    };

    let frac = ledger::amm_helpers::stamount_as_number(amount)
        / ledger::amm_helpers::stamount_as_number(pool1);
    let lp_tokens = amm_clawback_lp_tokens(lp_total, frac, RoundingMode::TowardsZero)
        .ok_or(Ter::TEC_INTERNAL)?;
    if lp_tokens > *holder_lp {
        return full_withdraw(holder_lp);
    }

    let (amount1, amount2, lp_tokens) =
        if rules.enabled(&protocol::feature_id("fixAMMClawbackRounding")) {
            let tokens = ledger::amm_helpers::get_rounded_lp_tokens(
                &rules,
                lp_total,
                frac,
                ledger::amm_helpers::IsDeposit::No,
            );
            if tokens.signum() == 0 {
                return Err(Ter::TEC_AMM_INVALID_TOKENS);
            }
            let adjusted_frac =
                ledger::amm_helpers::adjust_frac_by_tokens(&rules, lp_total, &tokens, frac);
            let amount1 =
                amm_clawback_proportional_amount(pool1, adjusted_frac, RoundingMode::Downward)
                    .ok_or(Ter::TEC_INTERNAL)?;
            let amount2 =
                amm_clawback_proportional_amount(pool2, adjusted_frac, RoundingMode::Downward)
                    .ok_or(Ter::TEC_INTERNAL)?;
            (amount1, amount2, tokens)
        } else {
            let amount2 = amm_clawback_proportional_amount(pool2, frac, RoundingMode::TowardsZero)
                .ok_or(Ter::TEC_INTERNAL)?;
            (amount.clone(), amount2, lp_tokens)
        };

    if lp_tokens.signum() <= 0 || lp_tokens > *holder_lp || amount1 > *pool1 || amount2 > *pool2 {
        return Err(Ter::TEC_AMM_INVALID_TOKENS);
    }

    Ok(tx::AMMWithdrawApplyMathResult {
        amount1: Some(amount1),
        amount2: Some(amount2),
        lp_tokens: lp_tokens.clone(),
        new_lp_token_balance: lp_total.clone() - lp_tokens,
    })
}

fn direct_send_mpt_no_fee<V: ledger::ApplyView>(
    view: &mut V,
    from: &AccountID,
    to: &AccountID,
    amount: &STAmount,
) -> Ter {
    if amount.signum() <= 0 || from == to {
        return Ter::TES_SUCCESS;
    }
    let Asset::MPTIssue(issue) = amount.asset() else {
        return Ter::TEC_INTERNAL;
    };
    let value = amount.mpt().value();
    let Ok(units) = u64::try_from(value) else {
        return Ter::TEC_INTERNAL;
    };
    let issuer = issue.issuer();
    let issuance_keylet = protocol::mpt_issuance_keylet_from_mptid(issue.mpt_id());
    let Some(issuance) = view.peek(issuance_keylet).ok().flatten() else {
        return Ter::TEC_OBJECT_NOT_FOUND;
    };
    let outstanding = issuance.get_field_u64(sf("sfOutstandingAmount"));

    if *from == issuer {
        let Some(next) = outstanding.checked_add(units) else {
            return Ter::TEC_INTERNAL;
        };
        let mut obj = issuance.clone_as_object();
        obj.set_field_u64(sf("sfOutstandingAmount"), next);
        let _ = view.update(Arc::new(STLedgerEntry::from_stobject(obj, *issuance.key())));
    } else {
        let debit_keylet =
            protocol::mptoken_keylet_from_mptid(issue.mpt_id(), Uint160::from_void(from.data()));
        let Some(token) = view.peek(debit_keylet).ok().flatten() else {
            return Ter::TEC_NO_AUTH;
        };
        let balance = token.get_field_u64(sf("sfMPTAmount"));
        let Some(next) = balance.checked_sub(units) else {
            return Ter::TEC_INSUFFICIENT_FUNDS;
        };
        let mut obj = token.clone_as_object();
        obj.set_field_u64(sf("sfMPTAmount"), next);
        let _ = view.update(Arc::new(STLedgerEntry::from_stobject(obj, *token.key())));
    }

    if *to == issuer {
        let Some(issuance) = view.peek(issuance_keylet).ok().flatten() else {
            return Ter::TEC_OBJECT_NOT_FOUND;
        };
        let outstanding = issuance.get_field_u64(sf("sfOutstandingAmount"));
        let Some(next) = outstanding.checked_sub(units) else {
            return Ter::TEC_INTERNAL;
        };
        let mut obj = issuance.clone_as_object();
        obj.set_field_u64(sf("sfOutstandingAmount"), next);
        let _ = view.update(Arc::new(STLedgerEntry::from_stobject(obj, *issuance.key())));
    } else {
        let credit_keylet =
            protocol::mptoken_keylet_from_mptid(issue.mpt_id(), Uint160::from_void(to.data()));
        let Some(token) = view.peek(credit_keylet).ok().flatten() else {
            return Ter::TEC_NO_AUTH;
        };
        let balance = token.get_field_u64(sf("sfMPTAmount"));
        let Some(next) = balance.checked_add(units) else {
            return Ter::TEC_INTERNAL;
        };
        let mut obj = token.clone_as_object();
        obj.set_field_u64(sf("sfMPTAmount"), next);
        let _ = view.update(Arc::new(STLedgerEntry::from_stobject(obj, *token.key())));
    }

    Ter::TES_SUCCESS
}

fn amm_clawback_send_amount<V: ledger::ApplyView>(
    view: &mut V,
    holder: &AccountID,
    issuer: &AccountID,
    amount: &STAmount,
) -> Ter {
    match amount.asset() {
        Asset::Issue(issue) if issue.native() => Ter::TEM_MALFORMED,
        Asset::Issue(_) => amm_transfer_iou_no_fee(view, holder, issuer, amount),
        Asset::MPTIssue(_) => direct_send_mpt_no_fee(view, holder, issuer, amount),
    }
}

fn amm_clawback_asset_allowed<V: ledger::ApplyView>(
    view: &mut V,
    issuer: &AccountID,
    issuer_sle: &STLedgerEntry,
    asset: Asset,
) -> bool {
    match asset {
        Asset::Issue(issue) => {
            !issue.native()
                && issue.account == *issuer
                && issuer_sle.is_flag(protocol::lsfAllowTrustLineClawback)
                && !issuer_sle.is_flag(protocol::lsfNoFreeze)
        }
        Asset::MPTIssue(issue) => view
            .peek(protocol::mpt_issuance_keylet_from_mptid(issue.mpt_id()))
            .ok()
            .flatten()
            .is_some_and(|sle| {
                sle.is_flag(protocol::lsfMPTCanClawback)
                    && sle.get_account_id(sf("sfIssuer")) == *issuer
            }),
    }
}

fn legacy_amm_clawback_direct_dispatch<V: ledger::ApplyView>(view: &mut V, sttx: &STTx) -> Ter {
    let issuer = sttx.get_account_id(sf("sfAccount"));
    let holder = sttx.get_account_id(sf("sfHolder"));
    let amount = sttx.get_field_amount(sf("sfAmount"));
    let Asset::Issue(issue) = amount.asset() else {
        return Ter::TES_SUCCESS;
    };
    if issue.native() {
        return Ter::TES_SUCCESS;
    }

    let line_keylet = protocol::line(issuer, holder, issue.currency);
    let Ok(Some(line)) = view.peek(line_keylet) else {
        return Ter::TES_SUCCESS;
    };

    let b_high = holder > issuer;
    let current_balance = line.get_field_amount(sf("sfBalance"));
    let holder_balance = if b_high {
        let mut balance = current_balance.clone();
        balance.negate();
        balance
    } else {
        current_balance.clone()
    };
    let mut normalized_amount = amount;
    normalized_amount.set_issue(protocol::Issue {
        account: issuer,
        currency: issue.currency,
    });
    let clawback_actual = if normalized_amount > holder_balance {
        holder_balance
    } else {
        normalized_amount
    };
    let new_balance = if b_high {
        current_balance + clawback_actual
    } else {
        current_balance - clawback_actual
    };

    let mut obj = line.clone_as_object();
    obj.set_field_amount(sf("sfBalance"), new_balance);
    if view
        .update(Arc::new(STLedgerEntry::from_stobject(obj, *line.key())))
        .is_err()
    {
        return Ter::TEF_INTERNAL;
    }

    Ter::TES_SUCCESS
}

fn escrow_mpt_unlock_amounts<V: ledger::ApplyView>(
    view: &V,
    amount: &STAmount,
    locked_rate: u32,
    sender: &AccountID,
    receiver: &AccountID,
) -> (STAmount, STAmount) {
    let Asset::MPTIssue(issue) = amount.asset() else {
        return (amount.clone(), amount.clone());
    };
    let issuer = issue.issuer();
    let mut rate = protocol::Rate::new(locked_rate);
    if let Ok(current_rate) = ledger::mptoken_helpers::transfer_rate_mpt(view, issue.mpt_id())
        && current_rate < rate
    {
        rate = current_rate;
    }

    if sender != &issuer && receiver != &issuer && rate != protocol::PARITY_RATE {
        return (protocol::divide_round(amount, rate, true), amount.clone());
    }
    (amount.clone(), amount.clone())
}

fn check_mpt_check_create_allowed<V: ledger::ApplyView>(
    view: &V,
    source: &AccountID,
    destination: &AccountID,
    amount: &STAmount,
) -> Ter {
    let Asset::MPTIssue(issue) = amount.asset() else {
        return Ter::TES_SUCCESS;
    };
    let issuer = issue.issuer();

    if source != &issuer
        && ledger::mptoken_helpers::is_frozen_mpt(view, source, &issue).unwrap_or(true)
    {
        return Ter::TEC_LOCKED;
    }
    if destination != &issuer
        && ledger::mptoken_helpers::is_frozen_mpt(view, destination, &issue).unwrap_or(true)
    {
        return Ter::TEC_LOCKED;
    }

    ledger::mptoken_helpers::can_transfer_mpt(view, &issue, source, destination)
        .unwrap_or(Ter::TEF_INTERNAL)
}

fn check_mpt_check_cash_allowed<V: ledger::ApplyView>(
    view: &mut V,
    source: &AccountID,
    destination: &AccountID,
    amount: &STAmount,
) -> Ter {
    let Asset::MPTIssue(issue) = amount.asset() else {
        return Ter::TES_SUCCESS;
    };
    let issuer = issue.issuer();
    if view
        .peek(protocol::account_keylet(Uint160::from_void(issuer.data())))
        .ok()
        .flatten()
        .is_none()
    {
        return Ter::TEC_NO_ISSUER;
    }
    let auth = ledger::mptoken_helpers::require_auth_mpt(view, &issue, destination)
        .unwrap_or(Ter::TEF_INTERNAL);
    if auth != Ter::TES_SUCCESS {
        return auth;
    }
    if destination != &issuer
        && ledger::mptoken_helpers::is_frozen_mpt(view, destination, &issue).unwrap_or(true)
    {
        return Ter::TEC_LOCKED;
    }
    let transfer = ledger::mptoken_helpers::can_transfer_mpt(view, &issue, source, destination)
        .unwrap_or(Ter::TEF_INTERNAL);
    if transfer != Ter::TES_SUCCESS {
        return transfer;
    }
    ledger::mptoken_helpers::check_create_mpt(view, &issue, destination)
        .unwrap_or(Ter::TEF_INTERNAL)
}

fn check_mpt_amm_asset_allowed<V: ledger::ApplyView>(
    view: &V,
    account: &AccountID,
    asset: Asset,
    require_holding: bool,
) -> Ter {
    let Asset::MPTIssue(issue) = asset else {
        return Ter::TES_SUCCESS;
    };
    let issuer = issue.issuer();

    if require_holding && account != &issuer {
        match view.read(protocol::mptoken_keylet_from_mptid(
            issue.mpt_id(),
            Uint160::from_void(account.data()),
        )) {
            Ok(Some(_)) => {}
            Ok(None) => return Ter::TEC_NO_AUTH,
            Err(_) => return Ter::TEF_INTERNAL,
        }
    }

    let auth = ledger::mptoken_helpers::require_auth_mpt(view, &issue, account)
        .unwrap_or(Ter::TEF_INTERNAL);
    if auth != Ter::TES_SUCCESS {
        return auth;
    }

    if ledger::mptoken_helpers::is_frozen_mpt(view, account, &issue).unwrap_or(true) {
        return Ter::TEC_LOCKED;
    }

    ledger::mptoken_helpers::can_mpt_trade_and_transfer(view, &asset, account, account)
        .unwrap_or(Ter::TEF_INTERNAL)
}

fn check_mpt_amm_withdraw_asset_allowed<V: ledger::ApplyView>(
    view: &V,
    account: &AccountID,
    asset: Asset,
) -> Ter {
    let Asset::MPTIssue(issue) = asset else {
        return Ter::TES_SUCCESS;
    };

    // #7040: AMMWithdraw is a recovery path. It must not require CanTransfer
    // or CanTrade, but it still rejects globally/individually locked MPTs.
    if ledger::mptoken_helpers::is_frozen_mpt(view, account, &issue).unwrap_or(true) {
        return Ter::TEC_LOCKED;
    }

    let auth = ledger::mptoken_helpers::require_auth_mpt(view, &issue, account)
        .unwrap_or(Ter::TEF_INTERNAL);
    if auth != Ter::TES_SUCCESS {
        return auth;
    }

    Ter::TES_SUCCESS
}

fn check_mpt_amm_pool_asset_unlocked<V: ledger::ApplyView>(
    view: &V,
    amm_account: &AccountID,
    asset: Asset,
) -> Ter {
    let Asset::MPTIssue(issue) = asset else {
        return Ter::TES_SUCCESS;
    };

    if ledger::mptoken_helpers::is_frozen_mpt(view, amm_account, &issue).unwrap_or(true) {
        return Ter::TEC_LOCKED;
    }

    Ter::TES_SUCCESS
}

fn nft_page_mask() -> Uint256 {
    protocol::nft_page_mask()
}

fn nft_owner_min(owner: &AccountID) -> Keylet {
    protocol::nft_page_min_keylet(Uint160::from_void(owner.data()))
}

fn nft_owner_max(owner: &AccountID) -> Keylet {
    protocol::nft_page_max_keylet(Uint160::from_void(owner.data()))
}

fn nft_page_for_token_keylet(owner: &AccountID, token_id: Uint256) -> Keylet {
    protocol::nft_page_keylet(nft_owner_min(owner), token_id)
}

fn nft_compare_tokens(left: Uint256, right: Uint256) -> std::cmp::Ordering {
    let mask = nft_page_mask();
    let left_low = left & mask;
    let right_low = right & mask;
    left_low.cmp(&right_low).then_with(|| left.cmp(&right))
}

fn starray_from_tokens(tokens: Vec<STObject>) -> STArray {
    let mut array = STArray::new(sf("sfNFTokens"));
    array.reserve(tokens.len());
    for token in tokens {
        array.push_back(token);
    }
    array
}

fn number_from_i64(value: i64) -> RuntimeNumber {
    RuntimeNumber::from_i64(value)
}

fn amm_lp_holds_in_view<V: ledger::ApplyView>(
    view: &mut V,
    amm_sle: &STLedgerEntry,
    lp_account: AccountID,
) -> Result<Option<STAmount>, ledger::ViewError> {
    let lp_tokens = amm_sle.get_field_amount(sf("sfLPTokenBalance"));
    let Asset::Issue(lp_issue) = lp_tokens.asset() else {
        return Ok(None);
    };
    let amm_account = amm_sle.get_account_id(sf("sfAccount"));
    let keylet = protocol::line(lp_account, amm_account, lp_issue.currency);
    let Some(sle) = view.peek(keylet)? else {
        return Ok(None);
    };
    let mut amount = sle.get_field_amount(sf("sfBalance"));
    if lp_account > amm_account {
        amount.negate();
    }
    amount.set_issuer(amm_account);
    Ok(Some(amount))
}

fn nft_locate_page<V: ledger::ApplyView>(
    view: &mut V,
    owner: &AccountID,
    token_id: Uint256,
) -> Result<Option<Arc<STLedgerEntry>>, ledger::ViewError> {
    let first = nft_page_for_token_keylet(owner, token_id);
    let last = nft_owner_max(owner);
    let candidate = view
        .succ(first.key, Some(last.key.next()))?
        .unwrap_or(last.key);
    view.peek(Keylet::new(LedgerEntryType::NFTokenPage, candidate))
}

fn nft_find_token_and_page<V: ledger::ApplyView>(
    view: &mut V,
    owner: &AccountID,
    token_id: Uint256,
) -> Result<Option<(STObject, Arc<STLedgerEntry>)>, ledger::ViewError> {
    let Some(page) = nft_locate_page(view, owner, token_id)? else {
        return Ok(None);
    };

    for token in page.get_field_array(sf("sfNFTokens")).iter() {
        if token.get_field_h256(sf("sfNFTokenID")) == token_id {
            return Ok(Some((token.clone(), page)));
        }
    }

    Ok(None)
}

fn nft_page_link<V: ledger::ApplyView>(
    view: &mut V,
    page: &Arc<STLedgerEntry>,
    field: &'static protocol::SField,
) -> Result<Option<Arc<STLedgerEntry>>, ledger::ViewError> {
    if !page.is_field_present(field) {
        return Ok(None);
    }

    let key = page.get_field_h256(field);
    view.peek(Keylet::new(LedgerEntryType::NFTokenPage, key))
}

fn nft_merge_pages<V: ledger::ApplyView>(
    view: &mut V,
    first: Arc<STLedgerEntry>,
    second: Arc<STLedgerEntry>,
) -> Result<bool, ledger::ViewError> {
    if first.key() >= second.key() {
        return Ok(false);
    }
    if !first.is_field_present(sf("sfNextPageMin"))
        || first.get_field_h256(sf("sfNextPageMin")) != *second.key()
        || !second.is_field_present(sf("sfPreviousPageMin"))
        || second.get_field_h256(sf("sfPreviousPageMin")) != *first.key()
    {
        return Ok(false);
    }

    let first_tokens: Vec<_> = first
        .get_field_array(sf("sfNFTokens"))
        .iter()
        .cloned()
        .collect();
    let second_tokens: Vec<_> = second
        .get_field_array(sf("sfNFTokens"))
        .iter()
        .cloned()
        .collect();
    if first_tokens.len() + second_tokens.len() > protocol::DIR_MAX_TOKENS_PER_PAGE {
        return Ok(false);
    }

    let mut merged = first_tokens;
    merged.extend(second_tokens);
    merged.sort_by(|left, right| {
        nft_compare_tokens(
            left.get_field_h256(sf("sfNFTokenID")),
            right.get_field_h256(sf("sfNFTokenID")),
        )
    });

    let mut second_obj = second.clone_as_object();
    second_obj.set_field_array(sf("sfNFTokens"), starray_from_tokens(merged));
    if second_obj.is_field_present(sf("sfPreviousPageMin")) {
        second_obj.make_field_absent(sf("sfPreviousPageMin"));
    }

    if first.is_field_present(sf("sfPreviousPageMin")) {
        let previous_key = first.get_field_h256(sf("sfPreviousPageMin"));
        if let Some(previous) =
            view.peek(Keylet::new(LedgerEntryType::NFTokenPage, previous_key))?
        {
            let mut previous_obj = previous.clone_as_object();
            previous_obj.set_field_h256(sf("sfNextPageMin"), *second.key());
            view.update(Arc::new(STLedgerEntry::from_stobject(
                previous_obj,
                *previous.key(),
            )))?;
            second_obj.set_field_h256(sf("sfPreviousPageMin"), previous_key);
        } else {
            return Ok(false);
        }
    }

    view.update(Arc::new(STLedgerEntry::from_stobject(
        second_obj,
        *second.key(),
    )))?;
    view.erase(first)?;

    Ok(true)
}

fn nft_remove_token_from_page<V: ledger::ApplyView>(
    view: &mut V,
    owner: &AccountID,
    token_id: Uint256,
    current: Arc<STLedgerEntry>,
) -> Ter {
    let tokens = current.get_field_array(sf("sfNFTokens"));
    let mut kept = Vec::new();
    let mut removed = false;
    for token in tokens.iter() {
        if token.get_field_h256(sf("sfNFTokenID")) == token_id {
            removed = true;
        } else {
            kept.push(token.clone());
        }
    }

    if !removed {
        return Ter::TEC_NO_ENTRY;
    }

    let previous = match nft_page_link(view, &current, sf("sfPreviousPageMin")) {
        Ok(page) => page,
        Err(_) => return Ter::TEF_INTERNAL,
    };
    let next = match nft_page_link(view, &current, sf("sfNextPageMin")) {
        Ok(page) => page,
        Err(_) => return Ter::TEF_INTERNAL,
    };

    if !kept.is_empty() {
        let mut obj = current.clone_as_object();
        obj.set_field_array(sf("sfNFTokens"), starray_from_tokens(kept));
        let updated_current = Arc::new(STLedgerEntry::from_stobject(obj, *current.key()));
        if view.update(updated_current.clone()).is_err() {
            return Ter::TEF_INTERNAL;
        }

        let mut owner_count_delta = 0;
        if let Some(prev) = previous.clone() {
            match nft_merge_pages(view, prev, updated_current.clone()) {
                Ok(true) => owner_count_delta -= 1,
                Ok(false) => {}
                Err(_) => return Ter::TEF_INTERNAL,
            }
        }
        if let Some(next_page) = next {
            match nft_merge_pages(view, updated_current, next_page) {
                Ok(true) => owner_count_delta -= 1,
                Ok(false) => {}
                Err(_) => return Ter::TEF_INTERNAL,
            }
        }
        if owner_count_delta != 0 {
            if let Ok(Some(account)) =
                view.peek(protocol::account_keylet(Uint160::from_void(owner.data())))
            {
                if ledger::adjust_owner_count(view, &account, owner_count_delta).is_err() {
                    return Ter::TEF_INTERNAL;
                }
            }
        }
        return Ter::TES_SUCCESS;
    }

    if let Some(prev) = previous.clone() {
        if view
            .rules()
            .enabled(&protocol::feature_id("fixNFTokenPageLinks"))
            && (*current.key() & nft_page_mask()) == nft_page_mask()
        {
            let mut current_obj = current.clone_as_object();
            current_obj.set_field_array(sf("sfNFTokens"), prev.get_field_array(sf("sfNFTokens")));
            if prev.is_field_present(sf("sfPreviousPageMin")) {
                let prev_link = prev.get_field_h256(sf("sfPreviousPageMin"));
                current_obj.set_field_h256(sf("sfPreviousPageMin"), prev_link);
                match view.peek(Keylet::new(LedgerEntryType::NFTokenPage, prev_link)) {
                    Ok(Some(new_prev)) => {
                        let mut new_prev_obj = new_prev.clone_as_object();
                        new_prev_obj.set_field_h256(sf("sfNextPageMin"), *current.key());
                        if view
                            .update(Arc::new(STLedgerEntry::from_stobject(
                                new_prev_obj,
                                *new_prev.key(),
                            )))
                            .is_err()
                        {
                            return Ter::TEF_INTERNAL;
                        }
                    }
                    _ => return Ter::TEF_INTERNAL,
                }
            } else if current_obj.is_field_present(sf("sfPreviousPageMin")) {
                current_obj.make_field_absent(sf("sfPreviousPageMin"));
            }

            if let Ok(Some(account)) =
                view.peek(protocol::account_keylet(Uint160::from_void(owner.data())))
            {
                if ledger::adjust_owner_count(view, &account, -1).is_err() {
                    return Ter::TEF_INTERNAL;
                }
            }
            if view
                .update(Arc::new(STLedgerEntry::from_stobject(
                    current_obj,
                    *current.key(),
                )))
                .is_err()
                || view.erase(prev).is_err()
            {
                return Ter::TEF_INTERNAL;
            }
            return Ter::TES_SUCCESS;
        }

        let mut prev_obj = prev.clone_as_object();
        if let Some(next_page) = next.clone() {
            prev_obj.set_field_h256(sf("sfNextPageMin"), *next_page.key());
        } else if prev_obj.is_field_present(sf("sfNextPageMin")) {
            prev_obj.make_field_absent(sf("sfNextPageMin"));
        }
        if view
            .update(Arc::new(STLedgerEntry::from_stobject(
                prev_obj,
                *prev.key(),
            )))
            .is_err()
        {
            return Ter::TEF_INTERNAL;
        }
    }

    if let Some(next_page) = next.clone() {
        let mut next_obj = next_page.clone_as_object();
        if let Some(prev) = previous.clone() {
            next_obj.set_field_h256(sf("sfPreviousPageMin"), *prev.key());
        } else if next_obj.is_field_present(sf("sfPreviousPageMin")) {
            next_obj.make_field_absent(sf("sfPreviousPageMin"));
        }
        if view
            .update(Arc::new(STLedgerEntry::from_stobject(
                next_obj,
                *next_page.key(),
            )))
            .is_err()
        {
            return Ter::TEF_INTERNAL;
        }
    }

    if view.erase(current).is_err() {
        return Ter::TEF_INTERNAL;
    }

    let mut owner_count_delta = -1;
    if let (Some(prev), Some(next_page)) = (previous, next) {
        match nft_merge_pages(view, prev, next_page) {
            Ok(true) => owner_count_delta -= 1,
            Ok(false) => {}
            Err(_) => return Ter::TEF_INTERNAL,
        }
    }

    if let Ok(Some(account)) = view.peek(protocol::account_keylet(Uint160::from_void(owner.data())))
    {
        if ledger::adjust_owner_count(view, &account, owner_count_delta).is_err() {
            return Ter::TEF_INTERNAL;
        }
    }

    Ter::TES_SUCCESS
}

fn nft_get_page_for_token<V: ledger::ApplyView>(
    view: &mut V,
    owner: &AccountID,
    token_id: Uint256,
) -> Result<Option<Arc<STLedgerEntry>>, ledger::ViewError> {
    let base = nft_owner_min(owner);
    let first = protocol::nft_page_keylet(base, token_id);
    let last = nft_owner_max(owner);
    let candidate = view
        .succ(first.key, Some(last.key.next()))?
        .unwrap_or(last.key);

    if let Some(page) = view.peek(Keylet::new(LedgerEntryType::NFTokenPage, candidate))? {
        if page.get_field_array(sf("sfNFTokens")).len() != protocol::DIR_MAX_TOKENS_PER_PAGE {
            return Ok(Some(page));
        }

        let mut tokens: Vec<_> = page
            .get_field_array(sf("sfNFTokens"))
            .iter()
            .cloned()
            .collect();
        let split_cmp = tokens[(protocol::DIR_MAX_TOKENS_PER_PAGE / 2) - 1]
            .get_field_h256(sf("sfNFTokenID"))
            & nft_page_mask();
        let mut split_index = (protocol::DIR_MAX_TOKENS_PER_PAGE / 2..tokens.len())
            .find(|index| {
                (tokens[*index].get_field_h256(sf("sfNFTokenID")) & nft_page_mask()) != split_cmp
            })
            .unwrap_or(tokens.len());
        if split_index == tokens.len() {
            split_index = tokens
                .iter()
                .position(|token| {
                    (token.get_field_h256(sf("sfNFTokenID")) & nft_page_mask()) == split_cmp
                })
                .unwrap_or(tokens.len());
        }
        if split_index == tokens.len() {
            return Ok(None);
        }
        if split_index == 0 {
            match (token_id & nft_page_mask()).cmp(&split_cmp) {
                std::cmp::Ordering::Equal => return Ok(None),
                std::cmp::Ordering::Greater => split_index = tokens.len(),
                std::cmp::Ordering::Less => {}
            }
        }

        let carried = tokens.split_off(split_index);
        let token_id_for_new_page = if tokens.len() == protocol::DIR_MAX_TOKENS_PER_PAGE {
            tokens[protocol::DIR_MAX_TOKENS_PER_PAGE - 1]
                .get_field_h256(sf("sfNFTokenID"))
                .next()
        } else {
            carried[0].get_field_h256(sf("sfNFTokenID"))
        };

        let new_page_keylet = protocol::nft_page_keylet(base, token_id_for_new_page);
        let mut new_page = STLedgerEntry::new(new_page_keylet);
        new_page.set_field_array(sf("sfNFTokens"), starray_from_tokens(tokens));
        new_page.set_field_h256(sf("sfNextPageMin"), *page.key());

        if page.is_field_present(sf("sfPreviousPageMin")) {
            let previous_key = page.get_field_h256(sf("sfPreviousPageMin"));
            new_page.set_field_h256(sf("sfPreviousPageMin"), previous_key);
            if let Some(previous) =
                view.peek(Keylet::new(LedgerEntryType::NFTokenPage, previous_key))?
            {
                let mut previous_obj = previous.clone_as_object();
                previous_obj.set_field_h256(sf("sfNextPageMin"), new_page_keylet.key);
                view.update(Arc::new(STLedgerEntry::from_stobject(
                    previous_obj,
                    *previous.key(),
                )))?;
            }
        }

        view.insert(Arc::new(new_page))?;

        let mut page_obj = page.clone_as_object();
        page_obj.set_field_array(sf("sfNFTokens"), starray_from_tokens(carried));
        page_obj.set_field_h256(sf("sfPreviousPageMin"), new_page_keylet.key);
        view.update(Arc::new(STLedgerEntry::from_stobject(
            page_obj,
            *page.key(),
        )))?;

        if let Ok(Some(account)) =
            view.peek(protocol::account_keylet(Uint160::from_void(owner.data())))
        {
            let _ = ledger::adjust_owner_count(view, &account, 1);
        }

        return if first.key < new_page_keylet.key {
            view.peek(new_page_keylet)
        } else {
            view.peek(Keylet::new(LedgerEntryType::NFTokenPage, *page.key()))
        };
    }

    let mut page = STLedgerEntry::new(last);
    page.set_field_array(sf("sfNFTokens"), STArray::new(sf("sfNFTokens")));
    let page = Arc::new(page);
    view.insert(page.clone())?;
    if let Ok(Some(account)) = view.peek(protocol::account_keylet(Uint160::from_void(owner.data())))
    {
        let _ = ledger::adjust_owner_count(view, &account, 1);
    }
    Ok(Some(page))
}

fn nft_insert_token<V: ledger::ApplyView>(view: &mut V, owner: &AccountID, token: STObject) -> Ter {
    let token_id = token.get_field_h256(sf("sfNFTokenID"));
    let page = match nft_get_page_for_token(view, owner, token_id) {
        Ok(Some(page)) => page,
        Ok(None) => return Ter::TEC_NO_SUITABLE_NFTOKEN_PAGE,
        Err(_) => return Ter::TEF_INTERNAL,
    };

    let mut tokens: Vec<_> = page
        .get_field_array(sf("sfNFTokens"))
        .iter()
        .cloned()
        .collect();
    tokens.push(token);
    tokens.sort_by(|left, right| {
        nft_compare_tokens(
            left.get_field_h256(sf("sfNFTokenID")),
            right.get_field_h256(sf("sfNFTokenID")),
        )
    });
    let mut page_obj = page.clone_as_object();
    page_obj.set_field_array(sf("sfNFTokens"), starray_from_tokens(tokens));
    if view
        .update(Arc::new(STLedgerEntry::from_stobject(
            page_obj,
            *page.key(),
        )))
        .is_err()
    {
        return Ter::TEF_INTERNAL;
    }

    Ter::TES_SUCCESS
}

fn nft_transfer_token<V: ledger::ApplyView>(
    view: &mut V,
    buyer: &AccountID,
    seller: &AccountID,
    token_id: Uint256,
) -> Ter {
    let (token, page) = match nft_find_token_and_page(view, seller, token_id) {
        Ok(Some(found)) => found,
        Ok(None) => return Ter::TEC_INTERNAL,
        Err(_) => return Ter::TEF_INTERNAL,
    };

    let remove_result = nft_remove_token_from_page(view, seller, token_id, page);
    if !is_tes_success(remove_result) {
        return remove_result;
    }

    nft_insert_token(view, buyer, token)
}

struct DispatcherTicketCreateSink<'a, V> {
    view: &'a mut V,
    account: AccountID,
    tx_sequence: u32,
    pre_fee_balance_drops: Option<i64>,
}

impl<V: ledger::ApplyView> TicketCreateDoApplySink for DispatcherTicketCreateSink<'_, V> {
    type OwnerNode = u64;

    fn account_exists(&mut self) -> bool {
        self.view
            .exists(protocol::account_keylet(Uint160::from_void(
                self.account.data(),
            )))
            .unwrap_or(false)
    }

    fn has_reserve(&mut self, ticket_count: u32) -> bool {
        let account_keylet = protocol::account_keylet(Uint160::from_void(self.account.data()));
        let Ok(Some(account_root)) = self.view.peek(account_keylet) else {
            return false;
        };

        let owner_count = account_root.get_field_u32(sf("sfOwnerCount"));
        let reserve =
            self.view
                .fees()
                .account_reserve(owner_count as usize + ticket_count as usize) as i64;
        let balance = self
            .pre_fee_balance_drops
            .unwrap_or_else(|| account_root.get_field_amount(sf("sfBalance")).xrp().drops());
        balance >= reserve
    }

    fn first_ticket_sequence(&mut self) -> u32 {
        self.tx_sequence.saturating_add(1)
    }

    fn tx_sequence(&mut self) -> u32 {
        self.tx_sequence
    }

    fn create_ticket(&mut self, ticket_sequence: u32) {
        let ticket_keylet =
            protocol::ticket_keylet(Uint160::from_void(self.account.data()), ticket_sequence);
        let mut sle = STLedgerEntry::new(ticket_keylet);
        sle.set_account_id(sf("sfAccount"), self.account);
        sle.set_field_u32(sf("sfTicketSequence"), ticket_sequence);
        let _ = self.view.insert(Arc::new(sle));
    }

    fn dir_insert_ticket(&mut self, ticket_sequence: u32) -> Option<Self::OwnerNode> {
        let ticket_keylet =
            protocol::ticket_keylet(Uint160::from_void(self.account.data()), ticket_sequence);
        ledger::dir_append(
            self.view,
            &owner_dir_keylet(Uint160::from_void(self.account.data())),
            ticket_keylet.key,
            &|_| {},
        )
        .ok()
        .flatten()
    }

    fn set_ticket_owner_node(&mut self, ticket_sequence: u32, page: Self::OwnerNode) {
        let ticket_keylet =
            protocol::ticket_keylet(Uint160::from_void(self.account.data()), ticket_sequence);
        let Ok(Some(ticket)) = self.view.peek(ticket_keylet) else {
            return;
        };

        let mut obj = ticket.clone_as_object();
        obj.set_field_u64(sf("sfOwnerNode"), page);
        let _ = self
            .view
            .update(Arc::new(STLedgerEntry::from_stobject(obj, *ticket.key())));
    }

    fn old_ticket_count(&mut self) -> u32 {
        let account_keylet = protocol::account_keylet(Uint160::from_void(self.account.data()));
        let Ok(Some(account_root)) = self.view.peek(account_keylet) else {
            return 0;
        };

        if account_root.is_field_present(sf("sfTicketCount")) {
            account_root.get_field_u32(sf("sfTicketCount"))
        } else {
            0
        }
    }

    fn set_ticket_count(&mut self, ticket_count: u32) {
        let account_keylet = protocol::account_keylet(Uint160::from_void(self.account.data()));
        let Ok(Some(account_root)) = self.view.peek(account_keylet) else {
            return;
        };

        let mut obj = account_root.clone_as_object();
        obj.set_field_u32(sf("sfTicketCount"), ticket_count);
        let _ = self.view.update(Arc::new(STLedgerEntry::from_stobject(
            obj,
            *account_root.key(),
        )));
    }

    fn adjust_owner_count(&mut self, ticket_count: u32) {
        let account_keylet = protocol::account_keylet(Uint160::from_void(self.account.data()));
        if let Ok(Some(account_root)) = self.view.peek(account_keylet) {
            let _ = ledger::adjust_owner_count(self.view, &account_root, ticket_count as i32);
        }
    }

    fn set_account_sequence(&mut self, sequence: u32) {
        let account_keylet = protocol::account_keylet(Uint160::from_void(self.account.data()));
        let Ok(Some(account_root)) = self.view.peek(account_keylet) else {
            return;
        };

        let mut obj = account_root.clone_as_object();
        obj.set_field_u32(sf("sfSequence"), sequence);
        let _ = self.view.update(Arc::new(STLedgerEntry::from_stobject(
            obj,
            *account_root.key(),
        )));
    }
}

#[derive(Debug, Clone, Copy)]
struct LedgerSignerList {
    flags: u32,
    signer_entries_len: usize,
    owner_node: u64,
}

impl SignerListSetLedgerEntry for LedgerSignerList {
    fn flags(&self) -> u32 {
        self.flags
    }

    fn signer_entries_len(&self) -> usize {
        self.signer_entries_len
    }

    fn owner_node(&self) -> u64 {
        self.owner_node
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct DispatcherSignerEntry {
    account: AccountID,
    weight: u16,
    wallet_locator: Option<Uint256>,
}

impl SignerListSetWriteEntry for DispatcherSignerEntry {
    type AccountId = AccountID;
    type WalletLocator = Uint256;

    fn account(&self) -> &Self::AccountId {
        &self.account
    }

    fn weight(&self) -> u16 {
        self.weight
    }

    fn wallet_locator(&self) -> Option<&Self::WalletLocator> {
        self.wallet_locator.as_ref()
    }
}

fn parse_signer_entries(
    sttx: &STTx,
) -> Result<
    (
        Vec<SignerListSetEntry<AccountID>>,
        Vec<DispatcherSignerEntry>,
    ),
    Ter,
> {
    if !sttx.is_field_present(sf("sfSignerEntries")) {
        return Ok((Vec::new(), Vec::new()));
    }

    let signer_entries = sttx.get_field_array(sf("sfSignerEntries"));
    let mut operation_entries = Vec::with_capacity(signer_entries.len());
    let mut write_entries = Vec::with_capacity(signer_entries.len());

    for signer in signer_entries.iter() {
        let signer_account = signer.get_account_id(sf("sfAccount"));
        let weight = signer.get_field_u16(sf("sfSignerWeight"));
        let wallet_locator = signer
            .is_field_present(sf("sfWalletLocator"))
            .then(|| signer.get_field_h256(sf("sfWalletLocator")));

        operation_entries.push(SignerListSetEntry {
            account: signer_account,
            weight,
        });
        write_entries.push(DispatcherSignerEntry {
            account: signer_account,
            weight,
            wallet_locator,
        });
    }

    write_entries.sort();
    Ok((operation_entries, write_entries))
}

fn remove_signer_list<V: ledger::ApplyView>(view: &mut V, account: AccountID) -> Ter {
    let account_keylet = protocol::account_keylet(Uint160::from_void(account.data()));
    let owner_dir = owner_dir_keylet(Uint160::from_void(account.data()));
    let signer_keylet = signers_keylet(Uint160::from_void(account.data()));
    let signer_list = match view.peek(signer_keylet) {
        Ok(Some(sle)) => Some(LedgerSignerList {
            flags: sle.get_field_u32(sf("sfFlags")),
            signer_entries_len: sle.get_field_array(sf("sfSignerEntries")).len(),
            owner_node: sle.get_field_u64(sf("sfOwnerNode")),
        }),
        Ok(None) => None,
        Err(_) => return Ter::TEF_INTERNAL,
    };

    // Inline removal logic to avoid multiple mutable borrows
    if let Some(ref sl) = signer_list {
        let owner_node = sl.owner_node;
        let _ = ledger::dir_remove(view, &owner_dir, owner_node, signer_keylet.key, false);
        let delta = -(sl.signer_entries_len as i32 + 2);
        if let Ok(Some(account_sle)) = view.peek(account_keylet) {
            let _ = ledger::adjust_owner_count(view, &account_sle, delta);
        }
        if let Ok(Some(signer_sle)) = view.peek(signer_keylet) {
            let _ = view.erase(signer_sle);
        }
    }
    Ter::TES_SUCCESS
}

fn destroy_signer_list<V: ledger::ApplyView>(view: &mut V, account: AccountID) -> Ter {
    let account_keylet = protocol::account_keylet(Uint160::from_void(account.data()));
    let account_sle = match view.peek(account_keylet) {
        Ok(account_sle) => account_sle,
        Err(_) => return Ter::TEF_INTERNAL,
    };

    let master_disabled = account_sle
        .as_ref()
        .is_some_and(|sle| sle.get_field_u32(sf("sfFlags")) & lsfDisableMaster != 0);
    let regular_key_present = account_sle
        .as_ref()
        .is_some_and(|sle| sle.is_field_present(sf("sfRegularKey")));

    run_signer_list_set_destroy_signer_list(
        account_sle.is_some(),
        master_disabled,
        regular_key_present,
        || remove_signer_list(view, account),
    )
}

fn replace_signer_list<V: ledger::ApplyView>(
    view: &mut V,
    account: AccountID,
    quorum: u32,
    signers: &[DispatcherSignerEntry],
    pre_fee_balance_drops: Option<i64>,
) -> Ter {
    let ter = remove_signer_list(view, account);
    if ter != Ter::TES_SUCCESS {
        return ter;
    }

    let account_keylet = protocol::account_keylet(Uint160::from_void(account.data()));
    let owner_dir = owner_dir_keylet(Uint160::from_void(account.data()));
    let signer_keylet = signers_keylet(Uint160::from_void(account.data()));
    let account_sle = match view.peek(account_keylet) {
        Ok(Some(sle)) => sle,
        Ok(None) => return Ter::TEF_INTERNAL,
        Err(_) => return Ter::TEF_INTERNAL,
    };

    let pre_fee_balance = pre_fee_balance_drops
        .map(XRPAmount::from_drops)
        .unwrap_or_else(|| account_sle.get_field_amount(sf("sfBalance")).xrp());
    let old_owner_count = account_sle.get_field_u32(sf("sfOwnerCount"));
    let new_reserve =
        XRPAmount::from_drops(view.fees().account_reserve(old_owner_count as usize + 1) as i64);
    if pre_fee_balance < new_reserve {
        return Ter::TEC_INSUFFICIENT_RESERVE;
    }

    let plan = build_signer_list_set_ledger_write_plan(
        false,
        account,
        quorum,
        LSF_ONE_OWNER_COUNT,
        signers,
    );

    let owner_page = match ledger::dir_insert(view, &owner_dir, signer_keylet.key, &|_| {}) {
        Ok(Some(page)) => page,
        Ok(None) => return Ter::TEC_DIR_FULL,
        Err(_) => return Ter::TEF_INTERNAL,
    };

    let mut signer_list = STLedgerEntry::new(signer_keylet);
    if let Some(owner) = plan.owner {
        signer_list.set_account_id(sf("sfOwner"), owner);
    }
    signer_list.set_field_u32(sf("sfSignerQuorum"), plan.signer_quorum);
    signer_list.set_field_u32(sf("sfSignerListID"), plan.signer_list_id);
    if let Some(flags) = plan.flags {
        signer_list.set_field_u32(sf("sfFlags"), flags);
    }
    signer_list.set_field_u64(sf("sfOwnerNode"), owner_page);

    let mut signer_array = STArray::new(sf("sfSignerEntries"));
    signer_array.reserve(plan.signer_entries.len());
    for signer in plan.signer_entries {
        let mut signer_entry = STObject::make_inner_object(sf("sfSignerEntry"));
        signer_entry.set_account_id(sf("sfAccount"), signer.account);
        signer_entry.set_field_u16(sf("sfSignerWeight"), signer.weight);
        if let Some(wallet_locator) = signer.wallet_locator {
            signer_entry.set_field_h256(sf("sfWalletLocator"), wallet_locator);
        }
        signer_array.push_back(signer_entry);
    }
    signer_list.set_field_array(sf("sfSignerEntries"), signer_array);

    if view.insert(Arc::new(signer_list)).is_err() {
        return Ter::TEF_INTERNAL;
    }
    if ledger::adjust_owner_count(view, &account_sle, 1).is_err() {
        return Ter::TEF_INTERNAL;
    }

    Ter::TES_SUCCESS
}

pub fn handle_real_dispatch<V: ledger::ApplyView>(
    view: &mut V,
    sttx: &STTx,
    txn_type: TxType,
    pre_fee_balance_drops: Option<i64>,
) -> Ter {
    let tx_hash = sttx.get_hash(protocol::HashPrefix::TransactionId);
    tracing::trace!(target: "tx", tx_type = %format!("{:?}", txn_type), hash = %tx_hash, "Transaction preflight");
    let result = handle_real_dispatch_inner(view, sttx, txn_type, pre_fee_balance_drops);

    if protocol::is_tes_success(result) || protocol::is_tec_claim(result) {
        tracing::debug!(target: "tx", tx_type = %format!("{:?}", txn_type), hash = %tx_hash, result = %format!("{:?}", result), "Transaction applied");
    } else {
        tracing::debug!(target: "tx", tx_type = %format!("{:?}", txn_type), hash = %tx_hash, result = %format!("{:?}", result), "Transaction not applied");
    }

    // Comprehensive per-tx debug log — logs every tx with key fields and result.
    // Controlled by a global counter so we don't flood the log.
    static TX_LOG: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let c = TX_LOG.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if c < 5000 {
        let account = sttx.get_account_id(sf("sfAccount"));
        let flags = sttx.get_field_u32(sf("sfFlags"));
        let seq = sttx.get_seq_value();

        // Key amounts for each tx type
        let detail = match txn_type {
            TxType::OFFER_CREATE => {
                let tp = sttx.get_field_amount(sf("sfTakerPays"));
                let tg = sttx.get_field_amount(sf("sfTakerGets"));
                format!(
                    "TakerPays_native={} TakerGets_native={} TakerPays_signum={} TakerGets_signum={}",
                    tp.native(),
                    tg.native(),
                    tp.signum(),
                    tg.signum()
                )
            }
            TxType::PAYMENT => {
                let amt = sttx.get_field_amount(sf("sfAmount"));
                let has_sm = sttx.is_field_present(sf("sfSendMax"));
                let has_paths = sttx.is_field_present(sf("sfPaths"));
                let sm_native = if has_sm {
                    sttx.get_field_amount(sf("sfSendMax")).native()
                } else {
                    true
                };
                format!(
                    "Amount_native={} has_sendmax={} sendmax_native={} has_paths={} partial={}",
                    amt.native(),
                    has_sm,
                    sm_native,
                    has_paths,
                    (flags & 0x0002_0000) != 0
                )
            }
            TxType::CHECK_CASH => {
                let has_amt = sttx.is_field_present(sf("sfAmount"));
                let has_dmin = sttx.is_field_present(sf("sfDeliverMin"));
                format!("has_amount={} has_deliver_min={}", has_amt, has_dmin)
            }
            _ => String::new(),
        };

        tracing::debug!(target: "tx",
            "[tx_trace] type={:?} seq={} flags=0x{:08x} acct={:02x}{:02x}{:02x}{:02x} result={:?} {}",
            txn_type,
            seq,
            flags,
            account.data()[0],
            account.data()[1],
            account.data()[2],
            account.data()[3],
            result,
            detail,
        );
    }

    result
}

fn handle_real_dispatch_inner<V: ledger::ApplyView>(
    view: &mut V,
    sttx: &STTx,
    txn_type: TxType,
    pre_fee_balance_drops: Option<i64>,
) -> Ter {
    // C++ Transactor::checkSign parity: reject transactions signed with
    // master key when lsfDisableMaster is set on the account, and reject
    // multi-signed transactions that lack a valid signer list or fail to
    // meet the quorum requirement.
    let account = sttx.get_account_id(sf("sfAccount"));
    let signing_pub_key = sttx.get_field_vl(sf("sfSigningPubKey"));
    if !signing_pub_key.is_empty() {
        // Non-empty SigningPubKey means single-signed (not multi-sign).
        // C++ parity: Transactor::checkSingleSign validates that the signing
        // key corresponds to either the account's master key (if enabled) or
        // its regular key (if set). Any other key is rejected with tefBAD_AUTH.
        use sha2::Digest;
        let sha = sha2::Sha256::digest(&signing_pub_key);
        let ripe = ripemd::Ripemd160::digest(sha);
        let id_signer = protocol::AccountID::from_slice(&ripe).expect("20 bytes");

        let acct_keylet = protocol::account_keylet(Uint160::from_void(account.data()));
        if let Ok(Some(acct_sle)) = view.peek(acct_keylet) {
            let is_master_disabled =
                acct_sle.get_field_u32(sf("sfFlags")) & lsfDisableMaster != 0;

            // Check 1: Signed with regular key
            let regular_key_field = sf("sfRegularKey");
            if acct_sle.is_field_present(regular_key_field) {
                let regular_key = acct_sle.get_account_id(regular_key_field);
                if id_signer == regular_key {
                    // Valid: signed with the account's regular key.
                    // (pass through to transaction dispatch below)
                } else if !is_master_disabled && id_signer == account {
                    // Check 2: Signed with enabled master key
                    // (pass through)
                } else if is_master_disabled && id_signer == account {
                    // Check 3: Signed with disabled master key
                    return Ter::TEF_MASTER_DISABLED;
                } else {
                    // Check 4: Signed with unknown key
                    return Ter::TEF_BAD_AUTH;
                }
            } else {
                // No regular key set on account
                if !is_master_disabled && id_signer == account {
                    // Signed with enabled master key
                    // (pass through)
                } else if is_master_disabled && id_signer == account {
                    // Signed with disabled master key
                    return Ter::TEF_MASTER_DISABLED;
                } else {
                    // Signed with unknown key (not master, no regular key set)
                    return Ter::TEF_BAD_AUTH;
                }
            }
        }
    } else if sttx.is_field_present(sf("sfSigners")) {
        // Empty SigningPubKey with sfSigners present means multi-signed.
        // Validate that the account has a signer list and that the
        // provided signers meet the quorum requirement.
        let signer_keylet = signers_keylet(Uint160::from_void(account.data()));
        let signer_list_sle = match view.read(signer_keylet) {
            Ok(Some(sle)) => sle,
            _ => return Ter::TEF_NOT_MULTI_SIGNING,
        };

        let quorum = signer_list_sle.get_field_u32(sf("sfSignerQuorum"));
        let signer_entries = signer_list_sle.get_field_array(sf("sfSignerEntries"));

        // Build a map of authorized signer account → weight from the on-ledger list.
        let mut authorized: std::collections::HashMap<AccountID, u32> =
            std::collections::HashMap::new();
        for entry in signer_entries.iter() {
            let signer_account = entry.get_account_id(sf("sfAccount"));
            let weight = entry.get_field_u16(sf("sfSignerWeight")) as u32;
            authorized.insert(signer_account, weight);
        }

        // Iterate over the transaction's signers and accumulate weight.
        let tx_signers = sttx.get_field_array(sf("sfSigners"));
        let mut weight_sum: u32 = 0;
        for tx_signer in tx_signers.iter() {
            let signer_account = tx_signer.get_account_id(sf("sfAccount"));
            if let Some(&weight) = authorized.get(&signer_account) {
                weight_sum = weight_sum.saturating_add(weight);
            }
            // Signers not in the list contribute zero weight but are not
            // rejected here — the cryptographic signature check already
            // validated their identity during preflight.
        }

        if weight_sum < quorum {
            return Ter::TEF_BAD_QUORUM;
        }
    }

    match txn_type {
        // --- XChain Bridge ---
        TxType::XCHAIN_CREATE_BRIDGE => {
            if !view.rules().enabled(&protocol::feature_id("XChainBridge")) {
                return Ter::TEM_DISABLED;
            }
            crate::state::xchain::apply_xchain_create_bridge(view, sttx)
        }
        TxType::XCHAIN_MODIFY_BRIDGE => {
            if !view.rules().enabled(&protocol::feature_id("XChainBridge")) {
                return Ter::TEM_DISABLED;
            }
            crate::state::xchain::apply_xchain_modify_bridge(view, sttx)
        }
        TxType::XCHAIN_CLAIM => {
            if !view.rules().enabled(&protocol::feature_id("XChainBridge")) {
                return Ter::TEM_DISABLED;
            }
            crate::state::xchain::apply_xchain_claim(view, sttx)
        }
        TxType::XCHAIN_COMMIT => {
            if !view.rules().enabled(&protocol::feature_id("XChainBridge")) {
                return Ter::TEM_DISABLED;
            }
            crate::state::xchain::apply_xchain_commit(view, sttx, pre_fee_balance_drops)
        }
        TxType::XCHAIN_CREATE_CLAIM_ID => {
            if !view.rules().enabled(&protocol::feature_id("XChainBridge")) {
                return Ter::TEM_DISABLED;
            }
            crate::state::xchain::apply_xchain_create_claim_id(view, sttx)
        }
        TxType::XCHAIN_ADD_CLAIM_ATTESTATION => {
            if !view.rules().enabled(&protocol::feature_id("XChainBridge")) {
                return Ter::TEM_DISABLED;
            }
            crate::state::xchain::apply_xchain_add_claim_attestation(view, sttx)
        }
        TxType::XCHAIN_ADD_ACCOUNT_CREATE_ATTESTATION => {
            if !view.rules().enabled(&protocol::feature_id("XChainBridge")) {
                return Ter::TEM_DISABLED;
            }
            crate::state::xchain::apply_xchain_add_account_create_attestation(view, sttx)
        }
        TxType::XCHAIN_ACCOUNT_CREATE_COMMIT => {
            if !view.rules().enabled(&protocol::feature_id("XChainBridge")) {
                return Ter::TEM_DISABLED;
            }
            crate::state::xchain::apply_xchain_account_create_commit(
                view,
                sttx,
                pre_fee_balance_drops,
            )
        }

        // --- Vault / Loan / Batch / Delegate ---
        TxType::VAULT_CREATE => crate::state::vault::apply_vault_create(view, sttx),
        TxType::VAULT_SET => crate::state::vault::apply_vault_set(view, sttx),
        TxType::VAULT_DELETE => crate::state::vault::apply_vault_delete(view, sttx),
        TxType::VAULT_DEPOSIT => crate::state::vault::apply_vault_deposit(view, sttx),
        TxType::VAULT_WITHDRAW => crate::state::vault::apply_vault_withdraw(view, sttx),
        TxType::VAULT_CLAWBACK => crate::state::vault::apply_vault_clawback(view, sttx),
        TxType::BATCH => {
            if !view.rules().enabled(&protocol::feature_id("Batch")) {
                return Ter::TEM_DISABLED;
            }
            crate::state::batch::apply_batch(view, sttx)
        }
        TxType::LOAN_SET => crate::state::lending::apply_loan_set(
            view,
            sttx,
            pre_fee_balance_drops.unwrap_or(10_000_000_000),
        ),
        TxType::LOAN_DELETE => crate::state::lending::apply_loan_delete(view, sttx),
        TxType::LOAN_MANAGE => crate::state::lending::apply_loan_manage(view, sttx),
        TxType::LOAN_PAY => crate::state::lending::apply_loan_pay(view, sttx),
        TxType::LOAN_BROKER_SET => crate::state::lending::apply_loan_broker_set(
            view,
            sttx,
            pre_fee_balance_drops.unwrap_or(10_000_000_000),
        ),
        TxType::LOAN_BROKER_DELETE => crate::state::lending::apply_loan_broker_delete(view, sttx),
        TxType::LOAN_BROKER_COVER_DEPOSIT => {
            crate::state::lending::apply_loan_broker_cover_deposit(view, sttx)
        }
        TxType::LOAN_BROKER_COVER_WITHDRAW => {
            crate::state::lending::apply_loan_broker_cover_withdraw(
                view,
                sttx,
                pre_fee_balance_drops.unwrap_or(10_000_000_000),
            )
        }
        TxType::LOAN_BROKER_COVER_CLAWBACK => {
            crate::state::lending::apply_loan_broker_cover_clawback(view, sttx)
        }
        TxType::DELEGATE_SET => {
            if !view
                .rules()
                .enabled(&protocol::feature_id("PermissionDelegationV1_1"))
            {
                return Ter::TEM_DISABLED;
            }
            let account = sttx.get_account_id(sf("sfAccount"));
            let authorize = sttx.get_account_id(sf("sfAuthorize"));
            let permissions = sttx
                .get_field_array(sf("sfPermissions"))
                .iter()
                .map(|permission| permission.get_field_u32(sf("sfPermissionValue")))
                .collect::<Vec<_>>();
            let balance_for_reserve = pre_fee_balance_drops.unwrap_or_else(|| {
                view.peek(protocol::account_keylet(Uint160::from_void(account.data())))
                    .ok()
                    .flatten()
                    .map(|sle| sle.get_field_amount(sf("sfBalance")).xrp().drops())
                    .unwrap_or(0)
            });
            let mut sink =
                ViewBackedDelegateSetSink::new(view, account, authorize, balance_for_reserve);
            run_delegate_set_do_apply(&permissions, &mut sink)
        }

        // --- Payment: full compatibility (payment.rs) ---
        TxType::PAYMENT => crate::state::payment::do_payment(view, sttx, pre_fee_balance_drops),

        // --- TrustSet: full flag handling ---
        // --- TrustSet: full compatibility (trust_set.rs) ---
        TxType::TRUST_SET => {
            crate::state::trust_set::do_trust_set(view, sttx, pre_fee_balance_drops)
        }

        // --- OfferCreate: full compatibility (offer_create.rs) ---
        TxType::OFFER_CREATE => {
            crate::state::offer_create::do_offer_create(view, sttx, pre_fee_balance_drops)
        }

        // --- OfferCancel ---
        TxType::OFFER_CANCEL => {
            let account = sttx.get_account_id(sf("sfAccount"));
            let seq = sttx.get_field_u32(sf("sfOfferSequence"));
            let keylet = protocol::offer_keylet(Uint160::from_void(account.data()), seq);
            if let Ok(Some(offer)) = view.peek(keylet) {
                let _ = crate::state::offer_create::offer_delete_pub(view, &account, offer);
            }
            Ter::TES_SUCCESS
        }

        // --- Account operations ---
        TxType::ACCOUNT_SET => {
            let account = sttx.get_account_id(sf("sfAccount"));
            let keylet = protocol::account_keylet(Uint160::from_void(account.data()));
            if let Ok(Some(sle)) = view.peek(keylet) {
                let mut obj = sle.clone_as_object();
                if sttx.is_field_present(sf("sfDomain")) {
                    obj.set_stbase(protocol::STBlob::from_buffer(
                        sf("sfDomain"),
                        basics::buffer::Buffer::from(&sttx.get_field_vl(sf("sfDomain"))[..]),
                    ));
                }
                if sttx.is_field_present(sf("sfTransferRate")) {
                    let rate = sttx.get_field_u32(sf("sfTransferRate"));
                    if rate == 0 || rate == 1_000_000_000 {
                        obj.make_field_absent(sf("sfTransferRate"));
                    } else {
                        obj.set_field_u32(sf("sfTransferRate"), rate);
                    }
                }
                if sttx.is_field_present(sf("sfTickSize")) {
                    let tick = sttx.get_field_u8(sf("sfTickSize"));
                    if tick == 0 {
                        obj.make_field_absent(sf("sfTickSize"));
                    } else {
                        obj.set_field_u8(sf("sfTickSize"), tick);
                    }
                }
                if sttx.is_field_present(sf("sfEmailHash")) {
                    let hash = sttx.get_field_h128(sf("sfEmailHash"));
                    if hash.is_zero() {
                        obj.make_field_absent(sf("sfEmailHash"));
                    } else {
                        obj.set_field_h128(sf("sfEmailHash"), hash);
                    }
                }
                if sttx.is_field_present(sf("sfMessageKey")) {
                    let vl = sttx.get_field_vl(sf("sfMessageKey"));
                    if vl.is_empty() {
                        obj.make_field_absent(sf("sfMessageKey"));
                    } else {
                        obj.set_stbase(protocol::STBlob::from_buffer(
                            sf("sfMessageKey"),
                            basics::buffer::Buffer::from(&vl[..]),
                        ));
                    }
                }
                // sfSetFlag / sfClearFlag — modify account flags
                let mut flags = obj.get_field_u32(sf("sfFlags"));
                if sttx.is_field_present(sf("sfSetFlag")) {
                    let set_flag = sttx.get_field_u32(sf("sfSetFlag"));
                    let lsf = asf_to_lsf(set_flag);
                    if lsf != 0 {
                        flags |= lsf;
                    }
                    // asfAccountTxnID (5) — add the field if not present
                    if set_flag == 5 && !obj.is_field_present(sf("sfAccountTxnID")) {
                        obj.set_field_h256(sf("sfAccountTxnID"), Uint256::default());
                    }
                }
                if sttx.is_field_present(sf("sfClearFlag")) {
                    let clear_flag = sttx.get_field_u32(sf("sfClearFlag"));
                    let lsf = asf_to_lsf(clear_flag);
                    if lsf != 0 {
                        flags &= !lsf;
                    }
                    // asfAccountTxnID (5) — remove the field
                    if clear_flag == 5 {
                        obj.make_field_absent(sf("sfAccountTxnID"));
                    }
                    // asfAuthorizedNFTokenMinter (10) — remove sfNFTokenMinter field
                    if clear_flag == 10 {
                        obj.make_field_absent(sf("sfNFTokenMinter"));
                    }
                }
                obj.set_field_u32(sf("sfFlags"), flags);
                let _ = view.update(Arc::new(STLedgerEntry::from_stobject(obj, *sle.key())));
            }
            Ter::TES_SUCCESS
        }

        TxType::ACCOUNT_DELETE => {
            let account = sttx.get_account_id(sf("sfAccount"));
            let destination = sttx.get_account_id(sf("sfDestination"));
            // C++ preclaim checks
            if account == destination {
                return Ter::TEM_DST_IS_SRC;
            }
            let src_keylet = protocol::account_keylet(Uint160::from_void(account.data()));
            let dst_keylet = protocol::account_keylet(Uint160::from_void(destination.data()));
            let Some(src) = view.peek(src_keylet).ok().flatten() else {
                return Ter::TEF_INTERNAL;
            };
            if view.peek(dst_keylet).ok().flatten().is_none() {
                return Ter::TEC_NO_DST;
            }
            // Sequence gap: account must be old enough (256 ledgers)
            let acct_seq = src.get_field_u32(sf("sfSequence"));
            let ledger_seq = view.header().seq;
            if ledger_seq.saturating_sub(acct_seq) < 256 {
                return Ter::TEC_TOO_SOON;
            }
            // Scan owner directory — only tickets and credentials are deletable
            let owner_dir = owner_dir_keylet(Uint160::from_void(account.data()));
            if view.exists(owner_dir).unwrap_or(false) {
                // Collect all directory entries
                let mut page = 0_u64;
                let mut all_entries: Vec<basics::math::base_uint::Uint256> = Vec::new();
                loop {
                    let page_keylet = protocol::page_keylet(owner_dir, page);
                    let Some(node) = view.peek(page_keylet).ok().flatten() else {
                        break;
                    };
                    all_entries.extend(
                        node.get_field_v256(sf("sfIndexes")).value().iter().copied(),
                    );
                    let next = node.get_field_u64(sf("sfIndexNext"));
                    if next == 0 || next == page {
                        break;
                    }
                    page = next;
                }
                // Check each entry: tickets/credentials are deletable, everything else is not
                for entry_key in &all_entries {
                    let Some(entry_sle) = view
                        .peek(protocol::child_keylet(*entry_key))
                        .ok()
                        .flatten()
                    else {
                        return Ter::TEF_BAD_LEDGER;
                    };
                    match entry_sle.get_type() {
                        LedgerEntryType::Ticket | LedgerEntryType::Credential => {
                            // deletable — will be cleaned up below
                        }
                        _ => {
                            return Ter::TEC_HAS_OBLIGATIONS;
                        }
                    }
                }
                // Delete all deletable entries
                for entry_key in all_entries {
                    if let Ok(Some(entry_sle)) =
                        view.peek(protocol::child_keylet(entry_key))
                    {
                        let owner_node = entry_sle.get_field_u64(sf("sfOwnerNode"));
                        let _ = ledger::dir_remove(
                            view,
                            &owner_dir,
                            owner_node,
                            entry_key,
                            false,
                        );
                        if let Ok(Some(acct)) = view.peek(src_keylet) {
                            let _ = ledger::adjust_owner_count(view, &acct, -1);
                        }
                        let _ = view.erase(entry_sle);
                    }
                }
            }
            // Transfer remaining XRP to destination, delete account
            if let (Ok(Some(src)), Ok(Some(dst))) = (view.peek(src_keylet), view.peek(dst_keylet)) {
                let balance = src.get_field_amount(sf("sfBalance")).xrp();
                let mut dst_obj = dst.clone_as_object();
                let dst_bal = dst.get_field_amount(sf("sfBalance")).xrp();
                dst_obj.set_field_amount(
                    sf("sfBalance"),
                    STAmount::from_xrp_amount(XRPAmount::from_drops(
                        dst_bal.drops() + balance.drops(),
                    )),
                );
                let _ = view.update(Arc::new(STLedgerEntry::from_stobject(dst_obj, *dst.key())));
                let _ = view.erase(src);
            }
            Ter::TES_SUCCESS
        }

        TxType::LEDGER_STATE_FIX => apply_ledger_state_fix(view, sttx),

        TxType::REGULAR_KEY_SET => {
            let account = sttx.get_account_id(sf("sfAccount"));
            // C++ preflight: RegularKey == Account → temBAD_REGKEY
            if sttx.is_field_present(sf("sfRegularKey"))
                && sttx.get_account_id(sf("sfRegularKey")) == account
            {
                return Ter::TEM_BAD_REGKEY;
            }
            let keylet = protocol::account_keylet(Uint160::from_void(account.data()));
            if let Ok(Some(sle)) = view.peek(keylet) {
                let mut obj = sle.clone_as_object();
                if sttx.is_field_present(sf("sfRegularKey")) {
                    obj.set_account_id(sf("sfRegularKey"), sttx.get_account_id(sf("sfRegularKey")));
                } else {
                    obj.make_field_absent(sf("sfRegularKey"));
                }
                let _ = view.update(Arc::new(STLedgerEntry::from_stobject(obj, *sle.key())));
            }
            Ter::TES_SUCCESS
        }

        TxType::SIGNER_LIST_SET => {
            let account = sttx.get_account_id(sf("sfAccount"));
            let quorum = sttx.get_field_u32(sf("sfSignerQuorum"));
            let (operation_entries, write_entries) = match parse_signer_entries(sttx) {
                Ok(parsed) => parsed,
                Err(err) => return err,
            };

            if !operation_entries.is_empty() {
                let validation = tx::run_signer_list_set_validate_quorum_and_signer_entries(
                    quorum,
                    &operation_entries,
                    &account,
                );
                if validation != Ter::TES_SUCCESS {
                    return validation;
                }
            }

            let operation = run_signer_list_set_determine_operation(
                quorum,
                sttx.is_field_present(sf("sfSignerEntries")),
                Ok(operation_entries),
            );
            if operation.result != Ter::TES_SUCCESS {
                return operation.result;
            }

            run_signer_list_set_do_apply(
                operation.operation,
                || Ter::TES_SUCCESS, // replace handled below
                || Ter::TES_SUCCESS, // destroy handled below
            );
            match operation.operation {
                SignerListSetOperation::Set => replace_signer_list(
                    view,
                    account,
                    operation.quorum,
                    &write_entries,
                    pre_fee_balance_drops,
                ),
                SignerListSetOperation::Destroy => destroy_signer_list(view, account),
                _ => Ter::TES_SUCCESS,
            }
        }

        TxType::DEPOSIT_PREAUTH => {
            let account = sttx.get_account_id(sf("sfAccount"));
            if sttx.is_field_present(sf("sfAuthorize")) {
                let auth_account = sttx.get_account_id(sf("sfAuthorize"));
                let preauth_keylet = protocol::deposit_preauth_keylet(
                    Uint160::from_void(account.data()),
                    Uint160::from_void(auth_account.data()),
                );
                let mut sle = STLedgerEntry::new(preauth_keylet);
                sle.set_account_id(sf("sfAccount"), account);
                sle.set_account_id(sf("sfAuthorize"), auth_account);
                // Add to owner directory
                let owner_dir = owner_dir_keylet(Uint160::from_void(account.data()));
                if let Ok(Some(page)) =
                    ledger::dir_append(view, &owner_dir, preauth_keylet.key, &|_| {})
                {
                    sle.set_field_u64(sf("sfOwnerNode"), page);
                }
                let _ = view.insert(Arc::new(sle));
                if let Ok(Some(acct)) =
                    view.peek(protocol::account_keylet(Uint160::from_void(account.data())))
                {
                    let _ = ledger::adjust_owner_count(view, &acct, 1);
                }
            } else if sttx.is_field_present(sf("sfUnauthorize")) {
                let unauth_account = sttx.get_account_id(sf("sfUnauthorize"));
                let preauth_keylet = protocol::deposit_preauth_keylet(
                    Uint160::from_void(account.data()),
                    Uint160::from_void(unauth_account.data()),
                );
                if let Ok(Some(preauth_sle)) = view.peek(preauth_keylet) {
                    let owner_node = preauth_sle.get_field_u64(sf("sfOwnerNode"));
                    let owner_dir = owner_dir_keylet(Uint160::from_void(account.data()));
                    let _ =
                        ledger::dir_remove(view, &owner_dir, owner_node, *preauth_sle.key(), false);
                    let _ = view.erase(preauth_sle);
                    if let Ok(Some(acct)) =
                        view.peek(protocol::account_keylet(Uint160::from_void(account.data())))
                    {
                        let _ = ledger::adjust_owner_count(view, &acct, -1);
                    }
                }
            }
            Ter::TES_SUCCESS
        }

        // --- Escrows ---
        TxType::ESCROW_CREATE => {
            let account = sttx.get_account_id(sf("sfAccount"));
            let dst_account = sttx.get_account_id(sf("sfDestination"));
            let amount = sttx.get_field_amount(sf("sfAmount"));
            let finish_after = if sttx.is_field_present(sf("sfFinishAfter")) {
                Some(sttx.get_field_u32(sf("sfFinishAfter")))
            } else {
                None
            };
            let cancel_after = if sttx.is_field_present(sf("sfCancelAfter")) {
                Some(sttx.get_field_u32(sf("sfCancelAfter")))
            } else {
                None
            };
            if let Ok(facts) = build_escrow_create_facts(view, &account, &dst_account, &amount) {
                let source_tag = if sttx.is_field_present(sf("sfSourceTag")) {
                    Some(sttx.get_field_u32(sf("sfSourceTag")))
                } else {
                    None
                };
                let destination_tag = if sttx.is_field_present(sf("sfDestinationTag")) {
                    Some(sttx.get_field_u32(sf("sfDestinationTag")))
                } else {
                    None
                };
                let mut sink = ViewBackedEscrowCreateSink {
                    view,
                    account,
                    dst_account,
                    amount,
                    escrow_key: Uint256::default(),
                    escrow_seq: sttx.get_seq_value(),
                    finish_after,
                    cancel_after,
                    source_tag,
                    destination_tag,
                };
                run_escrow_create_do_apply(facts, &mut sink)
            } else {
                Ter::TEF_INTERNAL
            }
        }
        TxType::ESCROW_FINISH => {
            let owner = sttx.get_account_id(sf("sfOwner"));
            let offer_seq = sttx.get_field_u32(sf("sfOfferSequence"));
            let escrow_keylet =
                protocol::escrow_keylet(Uint160::from_void(owner.data()), offer_seq);
            if let Ok(Some(escrow_sle)) = view.peek(escrow_keylet) {
                if escrow_sle.is_field_present(sf("sfFinishAfter")) {
                    let finish_after = escrow_sle.get_field_u32(sf("sfFinishAfter"));
                    if view.header().parent_close_time < finish_after {
                        return Ter::TEC_NO_PERMISSION;
                    }
                }
                let destination = escrow_sle.get_account_id(sf("sfDestination"));
                let amount = escrow_sle.get_field_amount(sf("sfAmount"));
                if amount.native() {
                    let amount_drops = amount.xrp().drops();
                    let dst_keylet =
                        protocol::account_keylet(Uint160::from_void(destination.data()));
                    if let Ok(Some(dst)) = view.peek(dst_keylet) {
                        let bal = dst.get_field_amount(sf("sfBalance")).xrp();
                        let mut obj = dst.clone_as_object();
                        obj.set_field_amount(
                            sf("sfBalance"),
                            STAmount::from_xrp_amount(XRPAmount::from_drops(
                                bal.drops() + amount_drops,
                            )),
                        );
                        let _ =
                            view.update(Arc::new(STLedgerEntry::from_stobject(obj, *dst.key())));
                    }
                } else {
                    match amount.asset() {
                        protocol::Asset::Issue(_) => {
                            return Ter::TEC_LIMIT_EXCEEDED;
                        }
                        protocol::Asset::MPTIssue(_) => {
                            let locked_rate = if escrow_sle
                                .is_field_present(sf("sfTransferRate")) { escrow_sle.get_field_u32(sf("sfTransferRate")) } else { protocol::PARITY_RATE.value };
                            let (net_amount, gross_amount) = escrow_mpt_unlock_amounts(
                                view,
                                &amount,
                                locked_rate,
                                &owner,
                                &destination,
                            );
                            let gross_amount = if view
                                .rules()
                                .enabled(&protocol::feature_id("fixTokenEscrowV1"))
                            {
                                &gross_amount
                            } else {
                                &net_amount
                            };
                            let submitter = sttx.get_account_id(sf("sfAccount"));
                            let result = ledger::mptoken_helpers::unlock_escrow_mpt(
                                view,
                                &owner,
                                &destination,
                                &net_amount,
                                gross_amount,
                                destination == submitter,
                                pre_fee_balance_drops,
                            )
                            .unwrap_or(Ter::TEF_INTERNAL);
                            if result != Ter::TES_SUCCESS {
                                return result;
                            }
                        }
                    }
                }
                // Remove from owner directory and adjust owner count
                let owner_node = escrow_sle.get_field_u64(sf("sfOwnerNode"));
                let owner_dir = owner_dir_keylet(Uint160::from_void(owner.data()));
                let _ = ledger::dir_remove(view, &owner_dir, owner_node, *escrow_sle.key(), false);
                // Remove from destination directory if present
                if escrow_sle.is_field_present(sf("sfDestinationNode")) {
                    let dst_node = escrow_sle.get_field_u64(sf("sfDestinationNode"));
                    let dst_dir = owner_dir_keylet(Uint160::from_void(destination.data()));
                    let _ = ledger::dir_remove(view, &dst_dir, dst_node, *escrow_sle.key(), false);
                }
                // Adjust owner count
                let owner_acct_keylet = protocol::account_keylet(Uint160::from_void(owner.data()));
                if let Ok(Some(owner_acct)) = view.peek(owner_acct_keylet) {
                    let _ = ledger::adjust_owner_count(view, &owner_acct, -1);
                }
                let _ = view.erase(escrow_sle);
            } else {
                return Ter::TEC_NO_TARGET;
            }
            Ter::TES_SUCCESS
        }
        TxType::ESCROW_CANCEL => {
            let owner = sttx.get_account_id(sf("sfOwner"));
            let offer_seq = sttx.get_field_u32(sf("sfOfferSequence"));
            let escrow_keylet =
                protocol::escrow_keylet(Uint160::from_void(owner.data()), offer_seq);
            if let Ok(Some(escrow_sle)) = view.peek(escrow_keylet) {
                if escrow_sle.is_field_present(sf("sfCancelAfter")) {
                    let cancel_after = escrow_sle.get_field_u32(sf("sfCancelAfter"));
                    if view.header().parent_close_time < cancel_after {
                        return Ter::TEC_NO_PERMISSION;
                    }
                }
                let amount = escrow_sle.get_field_amount(sf("sfAmount"));
                if amount.native() {
                    // Return XRP funds to owner.
                    let owner_keylet = protocol::account_keylet(Uint160::from_void(owner.data()));
                    if let Ok(Some(owner_acct)) = view.peek(owner_keylet) {
                        let bal = owner_acct.get_field_amount(sf("sfBalance")).xrp();
                        let mut obj = owner_acct.clone_as_object();
                        obj.set_field_amount(
                            sf("sfBalance"),
                            STAmount::from_xrp_amount(XRPAmount::from_drops(
                                bal.drops() + amount.xrp().drops(),
                            )),
                        );
                        let _ = view.update(Arc::new(STLedgerEntry::from_stobject(
                            obj,
                            *owner_acct.key(),
                        )));
                    }
                } else {
                    match amount.asset() {
                        protocol::Asset::Issue(issue) => {
                            let result = ledger::ripple_state_helpers::issue_iou(
                                view, &owner, &amount, &issue,
                            );
                            if result != Ter::TES_SUCCESS {
                                return result;
                            }
                        }
                        protocol::Asset::MPTIssue(_) => {
                            let submitter = sttx.get_account_id(sf("sfAccount"));
                            let (net_amount, gross_amount) = escrow_mpt_unlock_amounts(
                                view,
                                &amount,
                                protocol::PARITY_RATE.value,
                                &owner,
                                &owner,
                            );
                            let gross_amount = if view
                                .rules()
                                .enabled(&protocol::feature_id("fixTokenEscrowV1"))
                            {
                                &gross_amount
                            } else {
                                &net_amount
                            };
                            let result = ledger::mptoken_helpers::unlock_escrow_mpt(
                                view,
                                &owner,
                                &owner,
                                &net_amount,
                                gross_amount,
                                owner == submitter,
                                pre_fee_balance_drops,
                            )
                            .unwrap_or(Ter::TEF_INTERNAL);
                            if result != Ter::TES_SUCCESS {
                                return result;
                            }
                        }
                    }
                }
                // Remove from owner directory and adjust owner count
                let owner_node = escrow_sle.get_field_u64(sf("sfOwnerNode"));
                let owner_dir = owner_dir_keylet(Uint160::from_void(owner.data()));
                let _ = ledger::dir_remove(view, &owner_dir, owner_node, *escrow_sle.key(), false);
                // Remove from destination directory if present
                if escrow_sle.is_field_present(sf("sfDestinationNode")) {
                    let destination = escrow_sle.get_account_id(sf("sfDestination"));
                    let dst_node = escrow_sle.get_field_u64(sf("sfDestinationNode"));
                    let dst_dir = owner_dir_keylet(Uint160::from_void(destination.data()));
                    let _ = ledger::dir_remove(view, &dst_dir, dst_node, *escrow_sle.key(), false);
                }
                if !amount.native() && escrow_sle.is_field_present(sf("sfIssuerNode")) {
                    let issuer = amount.asset().issuer();
                    let issuer_node = escrow_sle.get_field_u64(sf("sfIssuerNode"));
                    let issuer_dir = owner_dir_keylet(Uint160::from_void(issuer.data()));
                    let _ = ledger::dir_remove(
                        view,
                        &issuer_dir,
                        issuer_node,
                        *escrow_sle.key(),
                        false,
                    );
                }
                // Adjust owner count
                if let Ok(Some(oa)) =
                    view.peek(protocol::account_keylet(Uint160::from_void(owner.data())))
                {
                    let _ = ledger::adjust_owner_count(view, &oa, -1);
                }
                let _ = view.erase(escrow_sle);
            } else {
                return Ter::TEC_NO_TARGET;
            }
            Ter::TES_SUCCESS
        }

        // --- Checks ---
        TxType::CHECK_CREATE => {
            let account = sttx.get_account_id(sf("sfAccount"));
            let dst = sttx.get_account_id(sf("sfDestination"));
            let send_max = sttx.get_field_amount(sf("sfSendMax"));
            // Preclaim: destination must exist (matching rippled CheckCreate::preclaim)
            let dst_keylet = protocol::account_keylet(Uint160::from_void(dst.data()));
            let Some(dst_sle) = view.peek(dst_keylet).ok().flatten() else {
                return Ter::TEC_NO_DST;
            };
            // Preclaim: check lsfDisallowIncomingCheck on destination
            if dst_sle.is_flag(protocol::lsfDisallowIncomingCheck) {
                return Ter::TEC_NO_PERMISSION;
            }
            // Preclaim: destination requires DestinationTag
            if dst_sle.is_flag(protocol::lsfRequireDestTag)
                && !sttx.is_field_present(sf("sfDestinationTag"))
            {
                return Ter::TEC_DST_TAG_NEEDED;
            }
            let mpt_result = check_mpt_check_create_allowed(view, &account, &dst, &send_max);
            if mpt_result != Ter::TES_SUCCESS {
                return mpt_result;
            }
            let check_keylet =
                protocol::check_keylet(Uint160::from_void(account.data()), sttx.get_seq_value());
            let mut sle = STLedgerEntry::new(check_keylet);
            sle.set_account_id(sf("sfAccount"), account);
            sle.set_account_id(sf("sfDestination"), dst);
            sle.set_field_amount(sf("sfSendMax"), send_max);
            sle.set_field_u32(sf("sfSequence"), sttx.get_seq_value());
            if sttx.is_field_present(sf("sfSourceTag")) {
                sle.set_field_u32(sf("sfSourceTag"), sttx.get_field_u32(sf("sfSourceTag")));
            }
            if sttx.is_field_present(sf("sfDestinationTag")) {
                sle.set_field_u32(sf("sfDestinationTag"), sttx.get_field_u32(sf("sfDestinationTag")));
            }
            if sttx.is_field_present(sf("sfExpiration")) {
                sle.set_field_u32(sf("sfExpiration"), sttx.get_field_u32(sf("sfExpiration")));
            }
            if sttx.is_field_present(sf("sfInvoiceID")) {
                sle.set_field_h256(sf("sfInvoiceID"), sttx.get_field_h256(sf("sfInvoiceID")));
            }
            // Add to owner directory
            let owner_dir = owner_dir_keylet(Uint160::from_void(account.data()));
            if let Ok(Some(page)) = ledger::dir_append(view, &owner_dir, check_keylet.key, &|_| {})
            {
                sle.set_field_u64(sf("sfOwnerNode"), page);
            }
            // Add to destination directory
            let dst_dir = owner_dir_keylet(Uint160::from_void(dst.data()));
            if let Ok(Some(page)) = ledger::dir_append(view, &dst_dir, check_keylet.key, &|_| {}) {
                sle.set_field_u64(sf("sfDestinationNode"), page);
            }
            let _ = view.insert(Arc::new(sle));
            if let Ok(Some(acct)) =
                view.peek(protocol::account_keylet(Uint160::from_void(account.data())))
            {
                let _ = ledger::adjust_owner_count(view, &acct, 1);
            }
            Ter::TES_SUCCESS
        }
        TxType::CHECK_CANCEL => {
            let tx_account = sttx.get_account_id(sf("sfAccount"));
            let check_id = sttx.get_field_h256(sf("sfCheckID"));
            let check_keylet = protocol::unchecked_keylet(check_id);
            if let Ok(Some(check_sle)) = view.peek(check_keylet) {
                let owner = check_sle.get_account_id(sf("sfAccount"));
                let destination = check_sle.get_account_id(sf("sfDestination"));
                // Preclaim: if check is not expired, only creator or destination may cancel
                let expired = if check_sle.is_field_present(sf("sfExpiration")) {
                    let exp = check_sle.get_field_u32(sf("sfExpiration"));
                    view.header().parent_close_time >= exp
                } else {
                    false
                };
                if !expired && tx_account != owner && tx_account != destination {
                    return Ter::TEC_NO_PERMISSION;
                }
                // Remove from owner directory
                let owner_node = check_sle.get_field_u64(sf("sfOwnerNode"));
                let owner_dir = owner_dir_keylet(Uint160::from_void(owner.data()));
                let _ = ledger::dir_remove(view, &owner_dir, owner_node, *check_sle.key(), false);
                // Remove from destination directory
                if check_sle.is_field_present(sf("sfDestinationNode")) {
                    let dst_node = check_sle.get_field_u64(sf("sfDestinationNode"));
                    let dst_dir = owner_dir_keylet(Uint160::from_void(destination.data()));
                    let _ = ledger::dir_remove(view, &dst_dir, dst_node, *check_sle.key(), false);
                }
                let _ = view.erase(check_sle);
                if let Ok(Some(acct)) =
                    view.peek(protocol::account_keylet(Uint160::from_void(owner.data())))
                {
                    let _ = ledger::adjust_owner_count(view, &acct, -1);
                }
            } else {
                return Ter::TEC_NO_ENTRY;
            }
            Ter::TES_SUCCESS
        }
        TxType::CHECK_CASH => {
            let check_id = sttx.get_field_h256(sf("sfCheckID"));
            let check_keylet = protocol::unchecked_keylet(check_id);
            if let Ok(Some(check_sle)) = view.peek(check_keylet) {
                let source = check_sle.get_account_id(sf("sfAccount"));
                let tx_account = sttx.get_account_id(sf("sfAccount"));
                let check_destination = check_sle.get_account_id(sf("sfDestination"));
                // Preclaim: only the check's destination can cash it (matching rippled)
                if tx_account != check_destination {
                    return Ter::TEC_NO_PERMISSION;
                }
                // Preclaim: check for expiration
                if check_sle.is_field_present(sf("sfExpiration")) {
                    let exp = check_sle.get_field_u32(sf("sfExpiration"));
                    if view.header().parent_close_time >= exp {
                        return Ter::TEC_EXPIRED;
                    }
                }
                let destination = tx_account;
                let requested_amount = if sttx.is_field_present(sf("sfAmount")) {
                    sttx.get_field_amount(sf("sfAmount"))
                } else {
                    check_sle.get_field_amount(sf("sfSendMax"))
                };
                let send_max = check_sle.get_field_amount(sf("sfSendMax"));
                if view
                    .rules()
                    .enabled(&protocol::feature_id("fixCleanup3_2_0"))
                    && !send_max.is_legal_mpt()
                {
                    return Ter::TEF_BAD_LEDGER;
                }
                if sttx.is_field_present(sf("sfAmount")) && requested_amount > send_max {
                    return Ter::TEC_PATH_PARTIAL;
                }
                let mpt_result =
                    check_mpt_check_cash_allowed(view, &source, &destination, &requested_amount);
                if mpt_result != Ter::TES_SUCCESS {
                    return mpt_result;
                }

                if requested_amount.native() {
                    let src_keylet = protocol::account_keylet(Uint160::from_void(source.data()));
                    let Some(src_sle) = view.peek(src_keylet).ok().flatten() else {
                        return Ter::TEC_FAILED_PROCESSING;
                    };
                    let available = src_sle.get_field_amount(sf("sfBalance"));
                    if requested_amount > available {
                        return Ter::TEC_PATH_PARTIAL;
                    }
                    do_xrp_payment(view, &source, &destination, &requested_amount, 0);
                } else {
                    let result = ledger::ripple_state_helpers::account_send_with_fee(
                        view,
                        &source,
                        &destination,
                        &requested_amount,
                    );
                    if !is_tes_success(result) {
                        return Ter::TEC_PATH_PARTIAL;
                    }
                }
                // Remove from owner directory
                let owner_node = check_sle.get_field_u64(sf("sfOwnerNode"));
                let owner_dir = owner_dir_keylet(Uint160::from_void(source.data()));
                let _ = ledger::dir_remove(view, &owner_dir, owner_node, *check_sle.key(), false);
                // Remove from destination directory
                if check_sle.is_field_present(sf("sfDestinationNode")) {
                    let dst = check_sle.get_account_id(sf("sfDestination"));
                    let dst_node = check_sle.get_field_u64(sf("sfDestinationNode"));
                    let dst_dir = owner_dir_keylet(Uint160::from_void(dst.data()));
                    let _ = ledger::dir_remove(view, &dst_dir, dst_node, *check_sle.key(), false);
                }
                let _ = view.erase(check_sle);
                if let Ok(Some(acct)) =
                    view.peek(protocol::account_keylet(Uint160::from_void(source.data())))
                {
                    let _ = ledger::adjust_owner_count(view, &acct, -1);
                }
            }
            Ter::TES_SUCCESS
        }

        // --- PayChans ---
        TxType::PAYCHAN_CREATE => {
            let account = sttx.get_account_id(sf("sfAccount"));
            let dst = sttx.get_account_id(sf("sfDestination"));
            let amount = sttx.get_field_amount(sf("sfAmount"));
            let settle_delay = sttx.get_field_u32(sf("sfSettleDelay"));
            let chan_keylet = protocol::pay_channel_keylet(
                Uint160::from_void(account.data()),
                Uint160::from_void(dst.data()),
                sttx.get_seq_value(),
            );
            let mut sle = STLedgerEntry::new(chan_keylet);
            sle.set_account_id(sf("sfAccount"), account);
            sle.set_account_id(sf("sfDestination"), dst);
            sle.set_field_amount(sf("sfAmount"), amount.clone());
            sle.set_field_amount(sf("sfBalance"), STAmount::from_xrp_amount(XRPAmount::new()));
            sle.set_field_u32(sf("sfSettleDelay"), settle_delay);
            // Copy PublicKey (required by rippled)
            if sttx.is_field_present(sf("sfPublicKey")) {
                let pk = sttx.get_field_vl(sf("sfPublicKey"));
                sle.set_field_vl(sf("sfPublicKey"), &pk);
            }
            // Copy optional fields matching rippled
            if sttx.is_field_present(sf("sfCancelAfter")) {
                sle.set_field_u32(sf("sfCancelAfter"), sttx.get_field_u32(sf("sfCancelAfter")));
            }
            if sttx.is_field_present(sf("sfSourceTag")) {
                sle.set_field_u32(sf("sfSourceTag"), sttx.get_field_u32(sf("sfSourceTag")));
            }
            if sttx.is_field_present(sf("sfDestinationTag")) {
                sle.set_field_u32(sf("sfDestinationTag"), sttx.get_field_u32(sf("sfDestinationTag")));
            }
            // Add to owner directory
            let owner_dir = owner_dir_keylet(Uint160::from_void(account.data()));
            if let Ok(Some(page)) = ledger::dir_append(view, &owner_dir, chan_keylet.key, &|_| {}) {
                sle.set_field_u64(sf("sfOwnerNode"), page);
            }
            let _ = view.insert(Arc::new(sle));
            // Adjust owner count
            if let Ok(Some(acct)) =
                view.peek(protocol::account_keylet(Uint160::from_void(account.data())))
            {
                let _ = ledger::adjust_owner_count(view, &acct, 1);
            }
            // Debit source account
            let src_keylet = protocol::account_keylet(Uint160::from_void(account.data()));
            if let Ok(Some(src_sle)) = view.peek(src_keylet) {
                let bal = src_sle.get_field_amount(sf("sfBalance")).xrp().drops();
                let amount_drops = amount.xrp().drops();
                let owner_count = src_sle.get_field_u32(sf("sfOwnerCount"));
                let reserve = view.fees().account_reserve(owner_count as usize) as i64;
                if bal < amount_drops + reserve {
                    return Ter::TEC_UNFUNDED;
                }
                let mut obj = src_sle.clone_as_object();
                obj.set_field_amount(
                    sf("sfBalance"),
                    STAmount::from_xrp_amount(XRPAmount::from_drops(bal - amount_drops)),
                );
                let _ = view.update(Arc::new(STLedgerEntry::from_stobject(obj, *src_sle.key())));
            }
            Ter::TES_SUCCESS
        }
        TxType::PAYCHAN_FUND => {
            let account = sttx.get_account_id(sf("sfAccount"));
            let channel_id = sttx.get_field_h256(sf("sfChannel"));
            let amount = sttx.get_field_amount(sf("sfAmount"));
            let chan_keylet = protocol::pay_channel_keylet_from_key(channel_id);
            if let Ok(Some(chan)) = view.peek(chan_keylet) {
                // C++ parity: only the channel source can fund it
                let chan_src = chan.get_account_id(sf("sfAccount"));
                if chan_src != account {
                    return Ter::TEC_NO_PERMISSION;
                }
                // Increase channel amount
                let cur = chan.get_field_amount(sf("sfAmount"));
                let mut obj = chan.clone_as_object();
                obj.set_field_amount(sf("sfAmount"), cur + amount.clone());
                let _ = view.update(Arc::new(STLedgerEntry::from_stobject(obj, *chan.key())));
                // Debit source account
                let src_keylet = protocol::account_keylet(Uint160::from_void(account.data()));
                if let Ok(Some(src_sle)) = view.peek(src_keylet) {
                    let bal = src_sle.get_field_amount(sf("sfBalance")).xrp().drops();
                    let mut src_obj = src_sle.clone_as_object();
                    src_obj.set_field_amount(
                        sf("sfBalance"),
                        STAmount::from_xrp_amount(XRPAmount::from_drops(
                            bal - amount.xrp().drops(),
                        )),
                    );
                    let _ = view.update(Arc::new(STLedgerEntry::from_stobject(
                        src_obj,
                        *src_sle.key(),
                    )));
                }
            }
            Ter::TES_SUCCESS
        }
        TxType::PAYCHAN_CLAIM => {
            let channel_id = sttx.get_field_h256(sf("sfChannel"));
            let chan_keylet = protocol::pay_channel_keylet_from_key(channel_id);
            let Some(chan) = view.peek(chan_keylet).ok().flatten() else {
                return Ter::TEC_NO_TARGET;
            };

            let src = chan.get_account_id(sf("sfAccount"));
            let dst = chan.get_account_id(sf("sfDestination"));
            let tx_account = sttx.get_account_id(sf("sfAccount"));
            let tx_flags = sttx.get_field_u32(sf("sfFlags"));

            // reference: check expiration/cancelAfter — close expired channels
            let close_time = view.header().parent_close_time;
            if chan.is_field_present(sf("sfCancelAfter")) {
                let cancel_after = chan.get_field_u32(sf("sfCancelAfter"));
                if close_time >= cancel_after {
                    return close_channel(view, &chan, chan_keylet.key);
                }
            }
            if chan.is_field_present(sf("sfExpiration")) {
                let expiration = chan.get_field_u32(sf("sfExpiration"));
                if close_time >= expiration {
                    return close_channel(view, &chan, chan_keylet.key);
                }
            }

            // reference: permission check
            if tx_account != src && tx_account != dst {
                return Ter::TEC_NO_PERMISSION;
            }

            // reference: balance update
            if sttx.is_field_present(sf("sfBalance")) {
                let chan_balance = chan.get_field_amount(sf("sfBalance")).xrp().drops();
                let chan_funds = chan.get_field_amount(sf("sfAmount")).xrp().drops();
                let req_balance = sttx.get_field_amount(sf("sfBalance")).xrp().drops();

                if req_balance > chan_funds || req_balance <= chan_balance {
                    return Ter::TEC_UNFUNDED_PAYMENT;
                }

                let delta = req_balance - chan_balance;

                // Credit destination
                let dst_keylet = protocol::account_keylet(Uint160::from_void(dst.data()));
                if let Ok(Some(dst_sle)) = view.peek(dst_keylet) {
                    let dst_bal = dst_sle.get_field_amount(sf("sfBalance")).xrp().drops();
                    let mut dst_obj = dst_sle.clone_as_object();
                    dst_obj.set_field_amount(
                        sf("sfBalance"),
                        STAmount::from_xrp_amount(XRPAmount::from_drops(dst_bal + delta)),
                    );
                    let _ = view.update(Arc::new(STLedgerEntry::from_stobject(
                        dst_obj,
                        *dst_sle.key(),
                    )));
                } else {
                    return Ter::TEC_NO_DST;
                }

                // Update channel balance
                let mut obj = chan.clone_as_object();
                obj.set_field_amount(sf("sfBalance"), sttx.get_field_amount(sf("sfBalance")));
                let _ = view.update(Arc::new(STLedgerEntry::from_stobject(obj, *chan.key())));
            }

            // reference: tfRenew — clear expiration (only source can renew)
            if (tx_flags & 0x0001_0000) != 0 {
                if src != tx_account {
                    return Ter::TEC_NO_PERMISSION;
                }
                if let Ok(Some(cur)) = view.peek(chan_keylet) {
                    let mut obj = cur.clone_as_object();
                    obj.make_field_absent(sf("sfExpiration"));
                    let _ =
                        view.update(Arc::new(STLedgerEntry::from_stobject(obj, chan_keylet.key)));
                }
            }

            // reference: tfClose — close channel or set expiration
            if (tx_flags & 0x0002_0000) != 0 {
                if let Ok(Some(cur)) = view.peek(chan_keylet) {
                    let cur_balance = cur.get_field_amount(sf("sfBalance")).xrp().drops();
                    let cur_amount = cur.get_field_amount(sf("sfAmount")).xrp().drops();

                    if dst == tx_account || cur_balance == cur_amount {
                        return close_channel(view, &cur, chan_keylet.key);
                    }

                    let settle_delay = cur.get_field_u32(sf("sfSettleDelay"));
                    let settle_expiration = close_time + settle_delay;

                    let should_update = if cur.is_field_present(sf("sfExpiration")) {
                        cur.get_field_u32(sf("sfExpiration")) > settle_expiration
                    } else {
                        true
                    };

                    if should_update {
                        let mut obj = cur.clone_as_object();
                        obj.set_field_u32(sf("sfExpiration"), settle_expiration);
                        let _ = view
                            .update(Arc::new(STLedgerEntry::from_stobject(obj, chan_keylet.key)));
                    }
                }
            }

            Ter::TES_SUCCESS
        }

        // --- AMM ---
        TxType::AMM_CREATE => {
            let account = sttx.get_account_id(sf("sfAccount"));
            let amount1 = sttx.get_field_amount(sf("sfAmount"));
            let amount2 = sttx.get_field_amount(sf("sfAmount2"));
            let mpt_gate = check_amm_mptokens_v2_gate(view, &[amount1.asset(), amount2.asset()]);
            if mpt_gate != Ter::TES_SUCCESS {
                return mpt_gate;
            }
            let mpt_result = check_mpt_amm_asset_allowed(view, &account, amount1.asset(), true);
            if mpt_result != Ter::TES_SUCCESS {
                return mpt_result;
            }
            let mpt_result = check_mpt_amm_asset_allowed(view, &account, amount2.asset(), true);
            if mpt_result != Ter::TES_SUCCESS {
                return mpt_result;
            }
            let facts = AMMCreateApplyFacts {
                amount1: amount1.clone(),
                amount2: amount2.clone(),
                trading_fee: sttx.get_field_u16(sf("sfTradingFee")),
                account,
                amm_account: account,
            };
            let mut sink = ViewBackedAMMCreateSink {
                view,
                account,
                amount1,
                amount2,
                trading_fee: facts.trading_fee,
                amm_keylet: None,
                amm_account: None,
                lp_tokens: None,
            };
            run_amm_create_do_apply(facts, &mut sink)
        }
        TxType::AMM_DEPOSIT => {
            let account = sttx.get_account_id(sf("sfAccount"));
            let asset1 = tx_amm_asset(sttx, sf("sfAsset"));
            let asset2 = tx_amm_asset(sttx, sf("sfAsset2"));
            let amount = optional_tx_amount(sttx, sf("sfAmount"));
            let amount2 = optional_tx_amount(sttx, sf("sfAmount2"));
            let e_price = optional_tx_amount(sttx, sf("sfEPrice"));
            let lp_token_out = optional_tx_amount(sttx, sf("sfLPTokenOut"));
            let mut gated_assets = vec![asset1, asset2];
            if let Some(amount) = &amount {
                gated_assets.push(amount.asset());
            }
            if let Some(amount2) = &amount2 {
                gated_assets.push(amount2.asset());
            }
            let mpt_gate = check_amm_mptokens_v2_gate(view, &gated_assets);
            if mpt_gate != Ter::TES_SUCCESS {
                return mpt_gate;
            }
            let mpt_result = check_mpt_amm_asset_allowed(view, &account, asset1, false);
            if mpt_result != Ter::TES_SUCCESS {
                return mpt_result;
            }
            let mpt_result = check_mpt_amm_asset_allowed(view, &account, asset2, false);
            if mpt_result != Ter::TES_SUCCESS {
                return mpt_result;
            }
            let amm_keylet = protocol::keylet::amm(asset1, asset2);
            let Some(amm_sle) = view.peek(amm_keylet).ok().flatten() else {
                return Ter::TER_NO_AMM;
            };
            {
                let amm_account = amm_sle.get_account_id(sf("sfAccount"));
                let mpt_result = check_mpt_amm_pool_asset_unlocked(view, &amm_account, asset1);
                if mpt_result != Ter::TES_SUCCESS {
                    return mpt_result;
                }
                let mpt_result = check_mpt_amm_pool_asset_unlocked(view, &amm_account, asset2);
                if mpt_result != Ter::TES_SUCCESS {
                    return mpt_result;
                }
                let flags = sttx.get_flags();
                if let Some(amount) = &amount {
                    let mpt_result =
                        check_mpt_amm_pool_asset_unlocked(view, &amm_account, amount.asset());
                    if mpt_result != Ter::TES_SUCCESS {
                        return mpt_result;
                    }
                    let mpt_result =
                        check_mpt_amm_asset_allowed(view, &account, amount.asset(), true);
                    if mpt_result != Ter::TES_SUCCESS {
                        return mpt_result;
                    }
                }
                if let Some(amount2) = &amount2 {
                    let mpt_result =
                        check_mpt_amm_pool_asset_unlocked(view, &amm_account, amount2.asset());
                    if mpt_result != Ter::TES_SUCCESS {
                        return mpt_result;
                    }
                    let mpt_result =
                        check_mpt_amm_asset_allowed(view, &account, amount2.asset(), true);
                    if mpt_result != Ter::TES_SUCCESS {
                        return mpt_result;
                    }
                }
                let lp_tokens = amm_sle.get_field_amount(sf("sfLPTokenBalance"));
                let pool_asset1 = amount.as_ref().map(STAmount::asset).unwrap_or(asset1);
                let pool_asset2 = amount2.as_ref().map(STAmount::asset).unwrap_or(asset2);
                let pool1 =
                    account_holds_amm_asset(view, &amm_account, pool_asset1, sf("sfAmount"))
                        .unwrap_or_else(|| zero_amount_for_asset(sf("sfAmount"), pool_asset1));
                let pool2 =
                    account_holds_amm_asset(view, &amm_account, pool_asset2, sf("sfAmount2"))
                        .unwrap_or_else(|| zero_amount_for_asset(sf("sfAmount2"), pool_asset2));
                let trading_fee = if lp_tokens.signum() == 0 {
                    if sttx.is_field_present(sf("sfTradingFee")) { sttx.get_field_u16(sf("sfTradingFee")) } else { 0 }
                } else {
                    amm_sle.get_field_u16(sf("sfTradingFee"))
                };
                let math =
                    match tx::run_amm_deposit_apply_math_facts(&tx::AMMDepositApplyMathFacts {
                        amount1: amount,
                        amount2,
                        e_price,
                        lp_token_out,
                        pool_amount1: pool1,
                        pool_amount2: pool2,
                        lp_token_balance: lp_tokens,
                        trading_fee,
                        rules: view.rules(),
                        flags,
                    }) {
                        Ok(math) => math,
                        Err(ter) => return ter,
                    };

                if let Some(amount) = &math.amount1 {
                    let mpt_result =
                        check_mpt_amm_pool_asset_unlocked(view, &amm_account, amount.asset());
                    if mpt_result != Ter::TES_SUCCESS {
                        return mpt_result;
                    }
                    let mpt_result =
                        check_mpt_amm_asset_allowed(view, &account, amount.asset(), true);
                    if mpt_result != Ter::TES_SUCCESS {
                        return mpt_result;
                    }
                    let deposit_result = amm_deposit_asset(view, &account, &amm_account, amount);
                    if deposit_result != Ter::TES_SUCCESS {
                        return deposit_result;
                    }
                }
                if let Some(amount2) = &math.amount2 {
                    let mpt_result =
                        check_mpt_amm_pool_asset_unlocked(view, &amm_account, amount2.asset());
                    if mpt_result != Ter::TES_SUCCESS {
                        return mpt_result;
                    }
                    let mpt_result =
                        check_mpt_amm_asset_allowed(view, &account, amount2.asset(), true);
                    if mpt_result != Ter::TES_SUCCESS {
                        return mpt_result;
                    }
                    let deposit2_result = amm_deposit_asset(view, &account, &amm_account, amount2);
                    if deposit2_result != Ter::TES_SUCCESS {
                        return deposit2_result;
                    }
                }

                let mut obj = amm_sle.clone_as_object();
                obj.set_field_amount(sf("sfLPTokenBalance"), math.new_lp_token_balance);
                if math.empty_pool_reinit
                    && view
                        .rules()
                        .enabled(&protocol::feature_id("fixCleanup3_2_0"))
                    && obj.is_field_present(sf("sfAuctionSlot"))
                {
                    let mut auction_slot = obj.peek_field_object(sf("sfAuctionSlot")).clone();
                    if auction_slot.is_field_present(sf("sfAuthAccounts")) {
                        auction_slot.make_field_absent(sf("sfAuthAccounts"));
                        obj.set_field_object(sf("sfAuctionSlot"), auction_slot);
                    }
                }
                let _ = view.update(Arc::new(STLedgerEntry::from_stobject(obj, *amm_sle.key())));
                let lp_result = ledger::ripple_state_helpers::direct_send_no_fee_iou_pub(
                    view,
                    &amm_account,
                    &account,
                    &math.lp_tokens,
                );
                if lp_result != Ter::TES_SUCCESS {
                    return lp_result;
                }
            }
            Ter::TES_SUCCESS
        }
        TxType::AMM_WITHDRAW => {
            let account = sttx.get_account_id(sf("sfAccount"));
            let asset1 = tx_amm_asset(sttx, sf("sfAsset"));
            let asset2 = tx_amm_asset(sttx, sf("sfAsset2"));
            let amount = optional_tx_amount(sttx, sf("sfAmount"));
            let amount2 = optional_tx_amount(sttx, sf("sfAmount2"));
            let e_price = optional_tx_amount(sttx, sf("sfEPrice"));
            let lp_token_in = optional_tx_amount(sttx, sf("sfLPTokenIn"));
            let mut gated_assets = vec![asset1, asset2];
            if let Some(amount) = &amount {
                gated_assets.push(amount.asset());
            }
            if let Some(amount2) = &amount2 {
                gated_assets.push(amount2.asset());
            }
            let mpt_gate = check_amm_mptokens_v2_gate(view, &gated_assets);
            if mpt_gate != Ter::TES_SUCCESS {
                return mpt_gate;
            }
            let mpt_result = check_mpt_amm_withdraw_asset_allowed(view, &account, asset1);
            if mpt_result != Ter::TES_SUCCESS {
                return mpt_result;
            }
            let mpt_result = check_mpt_amm_withdraw_asset_allowed(view, &account, asset2);
            if mpt_result != Ter::TES_SUCCESS {
                return mpt_result;
            }
            let amm_keylet = protocol::keylet::amm(asset1, asset2);
            let Some(amm_sle) = view.peek(amm_keylet).ok().flatten() else {
                return Ter::TER_NO_AMM;
            };
            {
                let amm_account = amm_sle.get_account_id(sf("sfAccount"));
                let mpt_result = check_mpt_amm_pool_asset_unlocked(view, &amm_account, asset1);
                if mpt_result != Ter::TES_SUCCESS {
                    return mpt_result;
                }
                let mpt_result = check_mpt_amm_pool_asset_unlocked(view, &amm_account, asset2);
                if mpt_result != Ter::TES_SUCCESS {
                    return mpt_result;
                }
                let lp_total = amm_sle.get_field_amount(sf("sfLPTokenBalance"));
                let lp_issue = lp_total.issue();
                if sttx.get_flags() & protocol::WITHDRAW_SUB_TX_FLAGS == 0
                    && (matches!(asset1, Asset::MPTIssue(_))
                        || matches!(asset2, Asset::MPTIssue(_)))
                    && let Some(lp_tokens_in) = lp_token_in.clone()
                {
                    let mpt_pool = amm_sle.get_field_amount(sf("sfAmount"));
                    let asset_out = if lp_total.signum() > 0 {
                        mpt_pool
                            .multiply(&lp_tokens_in, mpt_pool.asset())
                            .divide(&lp_total, mpt_pool.asset())
                    } else {
                        mpt_pool.zeroed()
                    };
                    if let Asset::MPTIssue(_) = asset_out.asset() {
                        let withdraw_result =
                            amm_withdraw_asset(view, &amm_account, &account, &asset_out);
                        if withdraw_result != Ter::TES_SUCCESS {
                            return withdraw_result;
                        }
                    }
                    let _ = crate::state::amm_bid_apply::redeem_iou_pub(
                        view,
                        &account,
                        &lp_tokens_in,
                        &lp_issue,
                    );
                    let mut obj = amm_sle.clone_as_object();
                    obj.set_field_amount(sf("sfLPTokenBalance"), lp_total - lp_tokens_in);
                    let _ =
                        view.update(Arc::new(STLedgerEntry::from_stobject(obj, *amm_sle.key())));
                    return Ter::TES_SUCCESS;
                }
                let Some(account_lp_tokens) =
                    amm_lp_holds_in_view(view, &amm_sle, account).ok().flatten()
                else {
                    return Ter::TEC_AMM_BALANCE;
                };
                let pool_asset1 = amount.as_ref().map(STAmount::asset).unwrap_or(asset1);
                let pool_asset2 = amount2.as_ref().map(STAmount::asset).unwrap_or(asset2);
                let pool1 =
                    account_holds_amm_asset(view, &amm_account, pool_asset1, sf("sfAmount"))
                        .unwrap_or_else(|| zero_amount_for_asset(sf("sfAmount"), pool_asset1));
                let pool2 =
                    account_holds_amm_asset(view, &amm_account, pool_asset2, sf("sfAmount2"))
                        .unwrap_or_else(|| zero_amount_for_asset(sf("sfAmount2"), pool_asset2));
                let math =
                    match tx::run_amm_withdraw_apply_math_facts(&tx::AMMWithdrawApplyMathFacts {
                        amount1: amount,
                        amount2,
                        e_price,
                        lp_token_in,
                        pool_amount1: pool1,
                        pool_amount2: pool2,
                        lp_token_balance: lp_total,
                        account_lp_tokens,
                        trading_fee: amm_sle.get_field_u16(sf("sfTradingFee")),
                        rules: view.rules(),
                        flags: sttx.get_flags(),
                    }) {
                        Ok(math) => math,
                        Err(ter) => return ter,
                    };

                if let Some(amount) = &math.amount1 {
                    let withdraw_result = amm_withdraw_asset(view, &amm_account, &account, amount);
                    if withdraw_result != Ter::TES_SUCCESS {
                        return withdraw_result;
                    }
                }
                if let Some(amount2) = &math.amount2 {
                    let withdraw2_result =
                        amm_withdraw_asset(view, &amm_account, &account, amount2);
                    if withdraw2_result != Ter::TES_SUCCESS {
                        return withdraw2_result;
                    }
                }
                let burn_result = crate::state::amm_bid_apply::redeem_iou_pub(
                    view,
                    &account,
                    &math.lp_tokens,
                    &lp_issue,
                );
                if burn_result != Ter::TES_SUCCESS {
                    return burn_result;
                }
                let mut obj = amm_sle.clone_as_object();
                obj.set_field_amount(sf("sfLPTokenBalance"), math.new_lp_token_balance);
                let _ = view.update(Arc::new(STLedgerEntry::from_stobject(obj, *amm_sle.key())));
            }
            Ter::TES_SUCCESS
        }
        TxType::AMM_VOTE => {
            let asset1 = sttx.get_field_issue(sf("sfAsset")).asset();
            let asset2 = sttx.get_field_issue(sf("sfAsset2")).asset();
            let mpt_gate = check_amm_mptokens_v2_gate(view, &[asset1, asset2]);
            if mpt_gate != Ter::TES_SUCCESS {
                return mpt_gate;
            }
            let fee_vote = sttx.get_field_u16(sf("sfTradingFee"));
            let account = sttx.get_account_id(sf("sfAccount"));
            let amm_keylet = protocol::keylet::amm(asset1, asset2);
            let Ok(Some(amm_sle)) = view.peek(amm_keylet) else {
                return Ter::TER_NO_AMM;
            };
            let lp_amm_balance = amm_sle.get_field_amount(sf("sfLPTokenBalance"));
            if lp_amm_balance.signum() == 0 {
                return Ter::TEC_AMM_EMPTY;
            }
            let Ok(Some(lp_tokens_new)) = amm_lp_holds_in_view(view, &amm_sle, account) else {
                return Ter::TEC_AMM_INVALID_TOKENS;
            };
            if lp_tokens_new.signum() == 0 {
                return Ter::TEC_AMM_INVALID_TOKENS;
            }

            let lp_total = ledger::amm_helpers::stamount_as_number(&lp_amm_balance);
            let lp_tokens_new_num = ledger::amm_helpers::stamount_as_number(&lp_tokens_new);
            let mut updated_vote_slots = STArray::new(sf("sfVoteSlots"));
            let mut numerator = RuntimeNumber::zero();
            let mut denominator = RuntimeNumber::zero();
            let mut found_account = false;
            let mut min_tokens: Option<RuntimeNumber> = None;
            let mut min_pos = 0usize;
            let mut min_account = AccountID::from_array([0; 20]);
            let mut min_fee = 0u32;

            let existing_slots = if amm_sle.is_field_present(sf("sfVoteSlots")) {
                amm_sle.get_field_array(sf("sfVoteSlots"))
            } else {
                STArray::new(sf("sfVoteSlots"))
            };

            for entry in existing_slots.iter() {
                let entry_account = entry.get_account_id(sf("sfAccount"));
                let Ok(Some(mut lp_tokens)) = amm_lp_holds_in_view(view, &amm_sle, entry_account)
                else {
                    continue;
                };
                if lp_tokens.signum() == 0 {
                    continue;
                }
                let mut fee_val = u32::from(entry.get_field_u16(sf("sfTradingFee")));
                if entry_account == account {
                    lp_tokens = lp_tokens_new.clone();
                    fee_val = u32::from(fee_vote);
                    found_account = true;
                }
                let lp_tokens_num = ledger::amm_helpers::stamount_as_number(&lp_tokens);
                numerator += number_from_i64(fee_val as i64) * lp_tokens_num;
                denominator += lp_tokens_num;

                let vote_weight =
                    ((lp_tokens_num * number_from_i64(VOTE_WEIGHT_SCALE_FACTOR as i64)) / lp_total)
                        .try_to_i64()
                        .unwrap_or(0)
                        .max(0) as u32;

                let mut new_entry = STObject::make_inner_object(sf("sfVoteEntry"));
                new_entry.set_account_id(sf("sfAccount"), entry_account);
                if fee_val != 0 {
                    new_entry.set_field_u16(sf("sfTradingFee"), fee_val as u16);
                }
                new_entry.set_field_u32(sf("sfVoteWeight"), vote_weight);

                if min_tokens.is_none()
                    || lp_tokens_num < min_tokens.unwrap()
                    || (lp_tokens_num == min_tokens.unwrap()
                        && (fee_val < min_fee
                            || (fee_val == min_fee && entry_account < min_account)))
                {
                    min_tokens = Some(lp_tokens_num);
                    min_pos = updated_vote_slots.len();
                    min_account = entry_account;
                    min_fee = fee_val;
                }

                updated_vote_slots.push_back(new_entry);
            }

            if !found_account {
                let update_entry = |slots: &mut STArray, replace_pos: Option<usize>| {
                    let vote_weight = ((lp_tokens_new_num
                        * number_from_i64(VOTE_WEIGHT_SCALE_FACTOR as i64))
                        / lp_total)
                        .try_to_i64()
                        .unwrap_or(0)
                        .max(0) as u32;
                    let mut new_entry = STObject::make_inner_object(sf("sfVoteEntry"));
                    if fee_vote != 0 {
                        new_entry.set_field_u16(sf("sfTradingFee"), fee_vote);
                    }
                    new_entry.set_field_u32(sf("sfVoteWeight"), vote_weight);
                    new_entry.set_account_id(sf("sfAccount"), account);
                    if let Some(pos) = replace_pos {
                        if let Some(slot) = slots.get_mut(pos) {
                            *slot = new_entry;
                        }
                    } else {
                        slots.push_back(new_entry);
                    }
                };

                if updated_vote_slots.len() < usize::from(VOTE_MAX_SLOTS) {
                    numerator += number_from_i64(i64::from(fee_vote)) * lp_tokens_new_num;
                    denominator += lp_tokens_new_num;
                    update_entry(&mut updated_vote_slots, None);
                } else if let Some(min_tokens) = min_tokens
                    && (lp_tokens_new_num > min_tokens
                        || (lp_tokens_new_num == min_tokens && u32::from(fee_vote) > min_fee))
                {
                    let replaced = updated_vote_slots
                        .get(min_pos)
                        .cloned()
                        .expect("vote slot exists");
                    let replaced_fee =
                        u32::from(if replaced.is_field_present(sf("sfTradingFee")) {
                            replaced.get_field_u16(sf("sfTradingFee"))
                        } else {
                            0
                        });
                    numerator = numerator - number_from_i64(replaced_fee as i64) * min_tokens
                        + number_from_i64(i64::from(fee_vote)) * lp_tokens_new_num;
                    denominator = denominator - min_tokens + lp_tokens_new_num;
                    update_entry(&mut updated_vote_slots, Some(min_pos));
                }
            }

            let mut obj = amm_sle.clone_as_object();
            obj.set_field_array(sf("sfVoteSlots"), updated_vote_slots);
            if denominator.signum() != 0 {
                let fee = (numerator / denominator).try_to_i64().unwrap_or(0).max(0) as u16;
                if fee != 0 {
                    obj.set_field_u16(sf("sfTradingFee"), fee);
                } else if obj.is_field_present(sf("sfTradingFee")) {
                    obj.make_field_absent(sf("sfTradingFee"));
                }
                if obj.is_field_present(sf("sfAuctionSlot")) {
                    let mut auction_slot = obj.peek_field_object(sf("sfAuctionSlot")).clone();
                    let discounted_fee = fee / AUCTION_SLOT_DISCOUNTED_FEE_FRACTION as u16;
                    if discounted_fee != 0 {
                        auction_slot.set_field_u16(sf("sfDiscountedFee"), discounted_fee);
                    } else if auction_slot.is_field_present(sf("sfDiscountedFee")) {
                        auction_slot.make_field_absent(sf("sfDiscountedFee"));
                    }
                    obj.set_field_object(sf("sfAuctionSlot"), auction_slot);
                }
            } else {
                if obj.is_field_present(sf("sfTradingFee")) {
                    obj.make_field_absent(sf("sfTradingFee"));
                }
                if obj.is_field_present(sf("sfAuctionSlot")) {
                    let mut auction_slot = obj.peek_field_object(sf("sfAuctionSlot")).clone();
                    if auction_slot.is_field_present(sf("sfDiscountedFee")) {
                        auction_slot.make_field_absent(sf("sfDiscountedFee"));
                    }
                    obj.set_field_object(sf("sfAuctionSlot"), auction_slot);
                }
            }
            let _ = view.update(Arc::new(STLedgerEntry::from_stobject(obj, *amm_sle.key())));
            Ter::TES_SUCCESS
        }
        TxType::AMM_DELETE => {
            let asset1 = sttx.get_field_issue(sf("sfAsset")).asset();
            let asset2 = sttx.get_field_issue(sf("sfAsset2")).asset();
            let mpt_gate = check_amm_mptokens_v2_gate(view, &[asset1, asset2]);
            if mpt_gate != Ter::TES_SUCCESS {
                return mpt_gate;
            }
            let amm_keylet = protocol::keylet::amm(asset1, asset2);
            let Some(amm_sle) = view.peek(amm_keylet).ok().flatten() else {
                return Ter::TER_NO_AMM;
            };
            let lp_balance = amm_sle.get_field_amount(sf("sfLPTokenBalance"));
            if lp_balance.signum() != 0 {
                return Ter::TEC_AMM_NOT_EMPTY;
            }
            let amm_account = amm_sle.get_account_id(sf("sfAccount"));
            let cleanup = delete_empty_amm_owner_entries(view, &amm_account);
            if cleanup != Ter::TES_SUCCESS {
                return cleanup;
            }

            let owner_dir = protocol::owner_dir_keylet(Uint160::from_void(amm_account.data()));
            let owner_node = amm_sle.get_field_u64(sf("sfOwnerNode"));
            match ledger::dir_remove(view, &owner_dir, owner_node, *amm_sle.key(), false) {
                Ok(true) => {}
                Ok(false) => return Ter::TEC_INTERNAL,
                Err(_) => return Ter::TEC_INTERNAL,
            }

            let account_keylet = protocol::account_keylet(Uint160::from_void(amm_account.data()));
            let Some(amm_root) = view.peek(account_keylet).ok().flatten() else {
                return Ter::TEC_INTERNAL;
            };
            if view.erase(amm_sle).is_err() || view.erase(amm_root).is_err() {
                return Ter::TEC_INTERNAL;
            }

            Ter::TES_SUCCESS
        }

        // --- NFTs ---
        TxType::NFTOKEN_MINT => {
            let account = sttx.get_account_id(sf("sfAccount"));
            // Determine the actual issuer (sfIssuer if present, otherwise minting account)
            let issuer = if sttx.is_field_present(sf("sfIssuer")) {
                sttx.get_account_id(sf("sfIssuer"))
            } else {
                account
            };

            // Get or create the issuer's MintedNFTokens counter (matching rippled doApply)
            let issuer_keylet = protocol::account_keylet(Uint160::from_void(issuer.data()));
            let Some(issuer_sle) = view.peek(issuer_keylet).ok().flatten() else {
                return Ter::TEC_NO_ISSUER;
            };

            let mut issuer_obj = issuer_sle.clone_as_object();

            // Set FirstNFTokenSequence if not present (matching rippled)
            if !issuer_obj.is_field_present(sf("sfFirstNFTokenSequence")) {
                let acct_seq = issuer_obj.get_field_u32(sf("sfSequence"));
                // If minted by owner using sequence (not ticket, not authorized minter):
                // Sequence was already incremented by apply_submit_transactor_shell,
                // so use acct_seq - 1. Otherwise use acct_seq as-is.
                let first_seq = if !sttx.is_field_present(sf("sfIssuer"))
                    && sttx.get_seq_proxy().is_seq()
                {
                    acct_seq.saturating_sub(1)
                } else {
                    acct_seq
                };
                issuer_obj.set_field_u32(sf("sfFirstNFTokenSequence"), first_seq);
            }

            // Get current MintedNFTokens and increment
            let minted_count = if issuer_obj.is_field_present(sf("sfMintedNFTokens")) {
                issuer_obj.get_field_u32(sf("sfMintedNFTokens"))
            } else {
                0
            };
            issuer_obj.set_field_u32(sf("sfMintedNFTokens"), minted_count.saturating_add(1));

            // Compute token sequence = FirstNFTokenSequence + MintedNFTokens (before increment)
            let first_nft_seq = issuer_obj.get_field_u32(sf("sfFirstNFTokenSequence"));
            let token_seq = first_nft_seq.wrapping_add(minted_count);

            // Update issuer account
            let _ = view.update(Arc::new(STLedgerEntry::from_stobject(
                issuer_obj,
                *issuer_sle.key(),
            )));

            // Compute the NFTokenID (matching rippled createNFTokenID exactly)
            let nft_flags = (sttx.get_flags() & 0x0000FFFF) as u16;
            let transfer_fee = if sttx.is_field_present(sf("sfTransferFee")) {
                sttx.get_field_u16(sf("sfTransferFee"))
            } else {
                0
            };
            let taxon = protocol::nft::to_taxon(sttx.get_field_u32(sf("sfNFTokenTaxon")));
            let nftoken_id = protocol::nft::create_nftoken_id(
                nft_flags,
                transfer_fee,
                &issuer,
                taxon,
                token_seq,
            );

            // Read URI from transaction if present
            let uri = if sttx.is_field_present(sf("sfURI")) {
                let uri_bytes = sttx.get_field_vl(sf("sfURI"));
                Some(protocol::STBlob::from_buffer(
                    sf("sfURI"),
                    basics::buffer::Buffer::from(uri_bytes.as_slice()),
                ))
            } else {
                None
            };

            let facts = NFTokenMintApplyFacts {
                nftoken_id,
                issuer,
                owner: account,
                transfer_fee: if transfer_fee != 0 { Some(transfer_fee) } else { None },
                uri,
            };
            let mut sink = ViewBackedNFTokenMintSink { view, account };
            run_nftoken_mint_do_apply(facts, &mut sink)
        }
        TxType::NFTOKEN_BURN => {
            let account = sttx.get_account_id(sf("sfAccount"));
            let owner = if sttx.is_field_present(sf("sfOwner")) {
                sttx.get_account_id(sf("sfOwner"))
            } else {
                account
            };
            let token_id = sttx.get_field_h256(sf("sfNFTokenID"));
            // Use succ-based page lookup (pages are stored at the max key for the
            // owner, not at the token-derived key). nft_locate_page uses succ() to
            // find the correct page containing this token.
            if let Ok(Some(page)) = nft_locate_page(view, &owner, token_id) {
                let tokens = page.get_field_array(sf("sfNFTokens"));
                let mut new_tokens = protocol::STArray::new(sf("sfNFTokens"));
                let mut found = false;
                for token in tokens.iter() {
                    let tid = token.get_field_h256(sf("sfNFTokenID"));
                    if tid != token_id {
                        new_tokens.push_back(token.clone());
                    } else {
                        found = true;
                    }
                }
                if !found {
                    return Ter::TEC_NO_ENTRY;
                }
                if new_tokens.is_empty() {
                    let _ = view.erase(page);
                } else {
                    let mut obj = page.clone_as_object();
                    obj.set_field_array(sf("sfNFTokens"), new_tokens);
                    let _ =
                        view.update(Arc::new(STLedgerEntry::from_stobject(obj, *page.key())));
                }
                if let Ok(Some(acct)) =
                    view.peek(protocol::account_keylet(Uint160::from_void(owner.data())))
                {
                    let _ = ledger::adjust_owner_count(view, &acct, -1);
                }
            } else {
                return Ter::TEC_NO_ENTRY;
            }
            Ter::TES_SUCCESS
        }
        TxType::NFTOKEN_CREATE_OFFER => {
            let account = sttx.get_account_id(sf("sfAccount"));
            let token_id = sttx.get_field_h256(sf("sfNFTokenID"));
            let tx_flags = sttx.get_flags();
            let is_sell = (tx_flags & protocol::tfSellNFToken) != 0;
            let amount = sttx.get_field_amount(sf("sfAmount"));

            // Determine the NFT owner for lookup purposes.
            // For buy offers, sfOwner specifies who owns the token.
            // For sell offers, the tx account must be the owner.
            let nft_owner = if !is_sell && sttx.is_field_present(sf("sfOwner")) {
                sttx.get_account_id(sf("sfOwner"))
            } else {
                account
            };

            // Verify NFT exists: look up the token in the owner's pages.
            let nft_found = match nft_find_token_and_page(view, &nft_owner, token_id) {
                Ok(Some(_)) => true,
                _ => false,
            };
            if !nft_found {
                return Ter::TEC_NO_ENTRY;
            }

            // Sell offers: account must own the NFT (already verified above since nft_owner == account for sell).
            // Buy offers: cannot buy your own NFT.
            if !is_sell {
                if nft_owner == account {
                    // Cannot create a buy offer for your own NFT
                    return Ter::TEC_NO_PERMISSION;
                }
                // Buy offers must have positive amount
                if amount.signum() <= 0 {
                    return Ter::TEM_BAD_AMOUNT;
                }
            }

            // tfOnlyXRP check: if the NFT has the tfOnlyXRP flag set (bit 0x0002 in
            // the high 16 bits of NFTokenID), reject non-XRP amounts.
            let id_bytes = token_id.data();
            let nft_flags_from_id = ((id_bytes[0] as u16) << 8) | (id_bytes[1] as u16);
            if (nft_flags_from_id & 0x0002) != 0 && !amount.native() && amount.signum() != 0 {
                return Ter::TEM_BAD_AMOUNT;
            }

            let offer_keylet = protocol::keylet::nft_offer_keylet_for_owner(
                Uint160::from_void(account.data()),
                sttx.get_seq_value(),
            );

            // Insert into owner directory (matching rippled)
            let owner_dir = protocol::owner_dir_keylet(Uint160::from_void(account.data()));
            let owner_node = match ledger::dir_append(view, &owner_dir, offer_keylet.key, &|_| {}) {
                Ok(Some(page)) => page,
                _ => return Ter::TEC_DIR_FULL,
            };

            // Insert into the token's sell or buy offer directory (matching rippled)
            let token_dir_keylet = if is_sell {
                protocol::nft_sell_offers_keylet(token_id)
            } else {
                protocol::nft_buy_offers_keylet(token_id)
            };
            let offer_node = match ledger::dir_append(view, &token_dir_keylet, offer_keylet.key, &|sle| {
                sle.set_field_u32(sf("sfFlags"), if is_sell { protocol::lsfNFTokenSellOffers } else { protocol::lsfNFTokenBuyOffers });
                sle.set_field_h256(sf("sfNFTokenID"), token_id);
            }) {
                Ok(Some(page)) => page,
                _ => return Ter::TEC_DIR_FULL,
            };

            // Create the offer SLE
            let mut sle = STLedgerEntry::new(offer_keylet);
            sle.set_account_id(sf("sfOwner"), account);
            sle.set_field_h256(sf("sfNFTokenID"), token_id);
            sle.set_field_amount(sf("sfAmount"), amount);
            let mut sle_flags = 0u32;
            if is_sell {
                sle_flags |= protocol::lsfSellNFToken;
            }
            sle.set_field_u32(sf("sfFlags"), sle_flags);
            sle.set_field_u64(sf("sfOwnerNode"), owner_node);
            sle.set_field_u64(sf("sfNFTokenOfferNode"), offer_node);
            if sttx.is_field_present(sf("sfDestination")) {
                sle.set_account_id(sf("sfDestination"), sttx.get_account_id(sf("sfDestination")));
            }
            if sttx.is_field_present(sf("sfExpiration")) {
                sle.set_field_u32(sf("sfExpiration"), sttx.get_field_u32(sf("sfExpiration")));
            }
            let _ = view.insert(Arc::new(sle));

            // Update owner count
            if let Ok(Some(acct)) =
                view.peek(protocol::account_keylet(Uint160::from_void(account.data())))
            {
                let _ = ledger::adjust_owner_count(view, &acct, 1);
            }
            Ter::TES_SUCCESS
        }
        TxType::NFTOKEN_CANCEL_OFFER => {
            let offers = sttx.get_field_v256(sf("sfNFTokenOffers"));
            for offer_id in offers.value() {
                let offer_keylet = protocol::keylet::nft_offer_keylet(*offer_id);
                if let Ok(Some(offer_sle)) = view.peek(offer_keylet) {
                    let offer_owner = offer_sle.get_account_id(sf("sfOwner"));
                    // Remove from owner directory
                    let owner_node = offer_sle.get_field_u64(sf("sfOwnerNode"));
                    let owner_dir =
                        protocol::owner_dir_keylet(Uint160::from_void(offer_owner.data()));
                    let _ = ledger::dir_remove(view, &owner_dir, owner_node, *offer_id, false);

                    // Remove from NFToken directory
                    let nftoken_id = offer_sle.get_field_h256(sf("sfNFTokenID"));
                    let flags = offer_sle.get_field_u32(sf("sfFlags"));
                    let is_sell = (flags & protocol::lsfSellNFToken) != 0;
                    let nft_dir = if is_sell {
                        protocol::nft_sell_offers_keylet(nftoken_id)
                    } else {
                        protocol::nft_buy_offers_keylet(nftoken_id)
                    };
                    let nft_node = offer_sle.get_field_u64(sf("sfNFTokenOfferNode"));
                    let _ = ledger::dir_remove(view, &nft_dir, nft_node, *offer_id, false);

                    let _ = view.erase(offer_sle);
                    if let Ok(Some(acct)) = view.peek(protocol::account_keylet(Uint160::from_void(
                        offer_owner.data(),
                    ))) {
                        let _ = ledger::adjust_owner_count(view, &acct, -1);
                    }
                }
            }
            Ter::TES_SUCCESS
        }
        TxType::NFTOKEN_ACCEPT_OFFER => {
            let tx_account = sttx.get_account_id(sf("sfAccount"));

            // Load offers
            let sell_offer = if sttx.is_field_present(sf("sfNFTokenSellOffer")) {
                let id = sttx.get_field_h256(sf("sfNFTokenSellOffer"));
                view.peek(protocol::keylet::nft_offer_keylet(Uint256::from(id)))
                    .ok()
                    .flatten()
            } else {
                None
            };
            let buy_offer = if sttx.is_field_present(sf("sfNFTokenBuyOffer")) {
                let id = sttx.get_field_h256(sf("sfNFTokenBuyOffer"));
                view.peek(protocol::keylet::nft_offer_keylet(Uint256::from(id)))
                    .ok()
                    .flatten()
            } else {
                None
            };

            // Validate: at least one offer must exist
            if sell_offer.is_none() && buy_offer.is_none() {
                return Ter::TEC_OBJECT_NOT_FOUND;
            }

            // Destination check: if a sell offer has a Destination, only that
            // account (or a broker using both offers) can accept it.
            if let Some(ref so) = sell_offer {
                if so.is_field_present(sf("sfDestination")) {
                    let dest = so.get_account_id(sf("sfDestination"));
                    // In broker mode, the buy offer owner must be the destination.
                    // In direct sell mode, the tx_account must be the destination.
                    if buy_offer.is_some() {
                        if let Some(ref bo) = buy_offer {
                            if bo.get_account_id(sf("sfOwner")) != dest {
                                return Ter::TEC_NO_PERMISSION;
                            }
                        }
                    } else if tx_account != dest {
                        return Ter::TEC_NO_PERMISSION;
                    }
                }
            }

            // Destination check for buy offers
            if let Some(ref bo) = buy_offer {
                if bo.is_field_present(sf("sfDestination")) {
                    let dest = bo.get_account_id(sf("sfDestination"));
                    if sell_offer.is_some() {
                        if let Some(ref so) = sell_offer {
                            if so.get_account_id(sf("sfOwner")) != dest {
                                return Ter::TEC_NO_PERMISSION;
                            }
                        }
                    } else if tx_account != dest {
                        return Ter::TEC_NO_PERMISSION;
                    }
                }
            }

            let delete_offer = |view: &mut V, offer: &Arc<STLedgerEntry>| {
                let owner = offer.get_account_id(sf("sfOwner"));
                let owner_node = offer.get_field_u64(sf("sfOwnerNode"));
                let owner_dir = owner_dir_keylet(Uint160::from_void(owner.data()));
                let _ = ledger::dir_remove(view, &owner_dir, owner_node, *offer.key(), false);
                // Also remove from the NFToken's sell/buy offer directory
                if offer.is_field_present(sf("sfNFTokenOfferNode")) {
                    let offer_node = offer.get_field_u64(sf("sfNFTokenOfferNode"));
                    let nftoken_id = offer.get_field_h256(sf("sfNFTokenID"));
                    let flags = offer.get_field_u32(sf("sfFlags"));
                    let is_sell = (flags & protocol::lsfSellNFToken) != 0;
                    let token_dir = if is_sell {
                        protocol::nft_sell_offers_keylet(nftoken_id)
                    } else {
                        protocol::nft_buy_offers_keylet(nftoken_id)
                    };
                    let _ = ledger::dir_remove(view, &token_dir, offer_node, *offer.key(), false);
                }
                if let Ok(Some(acct)) =
                    view.peek(protocol::account_keylet(Uint160::from_void(owner.data())))
                {
                    let _ = ledger::adjust_owner_count(view, &acct, -1);
                }
                let _ = view.erase(offer.clone());
            };

            // Delete both offers first (reference does this before payment/transfer)
            if let Some(ref bo) = buy_offer {
                delete_offer(view, bo);
            }
            if let Some(ref so) = sell_offer {
                delete_offer(view, so);
            }

            // Determine buyer, seller, amount, nftokenID based on mode
            let (buyer, seller, nftoken_id, amount) =
                if let (Some(bo), Some(so)) = (&buy_offer, &sell_offer) {
                    // Broker mode: both offers present
                    let buyer = bo.get_account_id(sf("sfOwner"));
                    let seller = so.get_account_id(sf("sfOwner"));
                    let nftoken_id = so.get_field_h256(sf("sfNFTokenID"));
                    let amount = bo.get_field_amount(sf("sfAmount"));
                    (buyer, seller, nftoken_id, amount)
                } else if let Some(ref so) = sell_offer {
                    // Sell offer only: tx_account is buyer
                    let seller = so.get_account_id(sf("sfOwner"));
                    let nftoken_id = so.get_field_h256(sf("sfNFTokenID"));
                    let amount = so.get_field_amount(sf("sfAmount"));
                    (tx_account, seller, nftoken_id, amount)
                } else if let Some(ref bo) = buy_offer {
                    // Buy offer only: tx_account is seller
                    let buyer = bo.get_account_id(sf("sfOwner"));
                    let nftoken_id = bo.get_field_h256(sf("sfNFTokenID"));
                    let amount = bo.get_field_amount(sf("sfAmount"));
                    (buyer, tx_account, nftoken_id, amount)
                } else {
                    return Ter::TEF_INTERNAL;
                };

            // Transfer fee handling: extract transfer fee from NFTokenID.
            // NFTokenID bytes 0-1 = flags, bytes 2-3 = transfer fee (basis points out of 50000).
            // Bytes 4-23 = issuer account (20 bytes).
            // Transfer fee only applies on secondary sales (seller != issuer).
            let id_bytes = nftoken_id.data();
            let nft_flags_from_id = ((id_bytes[0] as u16) << 8) | (id_bytes[1] as u16);
            let transfer_fee_bps = ((id_bytes[2] as u16) << 8) | (id_bytes[3] as u16);
            let has_transfer_fee = (nft_flags_from_id & 0x0008) != 0 && transfer_fee_bps > 0;

            // Extract issuer from NFTokenID bytes 4..24
            let mut issuer_bytes = [0u8; 20];
            issuer_bytes.copy_from_slice(&id_bytes[4..24]);
            let issuer_id = AccountID::from_array(issuer_bytes);

            // Determine if this is a secondary sale (seller is not the issuer)
            let is_secondary_sale = seller != issuer_id;

            if amount.signum() > 0 {
                if amount.native() {
                    if has_transfer_fee && is_secondary_sale {
                        // Calculate transfer fee: fee = amount * transferFee / 50000
                        let total_drops = amount.xrp().drops();
                        let fee_drops = (total_drops as u64 * transfer_fee_bps as u64 / 50000) as i64;
                        let seller_drops = total_drops - fee_drops;

                        // Pay seller (amount minus fee)
                        let seller_amount = STAmount::from_xrp_amount(XRPAmount::from_drops(seller_drops));
                        do_xrp_payment(view, &buyer, &seller, &seller_amount, 0);

                        // Pay issuer the transfer fee
                        if fee_drops > 0 {
                            let fee_amount = STAmount::from_xrp_amount(XRPAmount::from_drops(fee_drops));
                            do_xrp_payment(view, &buyer, &issuer_id, &fee_amount, 0);
                        }
                    } else {
                        do_xrp_payment(view, &buyer, &seller, &amount, 0);
                    }
                } else {
                    // IOU payment via accountSend
                    // TODO: IOU transfer fee would need similar handling
                    ledger::ripple_state_helpers::account_send(view, &buyer, &seller, &amount);
                }
            }

            nft_transfer_token(view, &buyer, &seller, nftoken_id)
        }
        TxType::CLAWBACK => {
            let issuer = sttx.get_account_id(sf("sfAccount"));
            let amount = sttx.get_field_amount(sf("sfAmount"));
            // C++ parity: preflight validation
            if amount.signum() <= 0 {
                return Ter::TEM_BAD_AMOUNT;
            }
            if amount.native() {
                return Ter::TEM_BAD_AMOUNT;
            }
            let holder = sttx.get_account_id(sf("sfHolder"));
            if holder == issuer {
                return Ter::TEM_MALFORMED;
            }
            if amount.holds_mpt_issue() {
                let mpt_issue = match &amount.asset() {
                    protocol::Asset::MPTIssue(i) => *i,
                    _ => return Ter::TEF_INTERNAL,
                };
                let mptid = mpt_issue.mpt_id();
                // C++ parity: preclaim - check lsfMPTCanClawback
                let issuance_keylet = protocol::mpt_issuance_keylet_from_mptid(mptid);
                let Some(iss_sle) = view.peek(issuance_keylet).ok().flatten() else {
                    return Ter::TEC_OBJECT_NOT_FOUND;
                };
                if !iss_sle.is_flag(protocol::lsfMPTCanClawback) {
                    return Ter::TEC_NO_PERMISSION;
                }
                let holder_keylet =
                    protocol::mptoken_keylet_from_mptid(mptid, Uint160::from_void(holder.data()));
                let Some(token_sle) = view.peek(holder_keylet).ok().flatten() else {
                    return Ter::TEC_OBJECT_NOT_FOUND;
                };
                let balance = token_sle.get_field_u64(sf("sfMPTAmount"));
                let clawback_amt = amount.mpt().value().unsigned_abs().min(balance);
                let mut obj = token_sle.clone_as_object();
                obj.set_field_u64(sf("sfMPTAmount"), balance - clawback_amt);
                let _ = view.update(Arc::new(STLedgerEntry::from_stobject(
                    obj,
                    *token_sle.key(),
                )));
                // Update OutstandingAmount on issuance
                let Some(iss) = view.peek(issuance_keylet).ok().flatten() else {
                    return Ter::TEF_INTERNAL;
                };
                let outstanding = iss.get_field_u64(sf("sfOutstandingAmount"));
                let mut iss_obj = iss.clone_as_object();
                iss_obj.set_field_u64(
                    sf("sfOutstandingAmount"),
                    outstanding.saturating_sub(clawback_amt),
                );
                let _ = view.update(Arc::new(STLedgerEntry::from_stobject(iss_obj, *iss.key())));
            } else {
                // IOU clawback — debit specific amount from holder's trust line
                // C++ parity: preclaim - check lsfAllowTrustLineClawback
                let Some(issuer_sle) = view
                    .peek(protocol::account_keylet(Uint160::from_void(issuer.data())))
                    .ok()
                    .flatten()
                else {
                    return Ter::TER_NO_ACCOUNT;
                };
                if !issuer_sle.is_flag(protocol::lsfAllowTrustLineClawback) {
                    return Ter::TEC_NO_PERMISSION;
                }
                let holder = amount.issue().account; // In clawback, the "issuer" field on amount is the holder
                let currency = amount.issue().currency;
                let line_keylet = protocol::line(issuer, holder, currency);
                if let Ok(Some(line)) = view.peek(line_keylet) {
                    let b_high = holder > issuer;
                    let current_balance = line.get_field_amount(sf("sfBalance"));
                    // Determine holder's balance (positive from their perspective)
                    let holder_balance = if b_high {
                        let mut neg = current_balance.clone();
                        neg.negate();
                        neg
                    } else {
                        current_balance.clone()
                    };
                    // Clawback the minimum of requested and available
                    // This makes both amounts have the same issue (issuer's perspective).
                    let normalized_amount = {
                        let mut a = amount.clone();
                        a.set_issue(protocol::Issue {
                            account: issuer,
                            currency,
                        });
                        a
                    };
                    let clawback_actual = if normalized_amount > holder_balance {
                        holder_balance
                    } else {
                        normalized_amount
                    };
                    // Adjust balance: reduce holder's side
                    let new_balance = if b_high {
                        current_balance + clawback_actual
                    } else {
                        current_balance - clawback_actual
                    };
                    let mut obj = line.clone_as_object();
                    obj.set_field_amount(sf("sfBalance"), new_balance);
                    let _ = view.update(Arc::new(STLedgerEntry::from_stobject(obj, *line.key())));
                } else {
                    return Ter::TEC_NO_LINE;
                }
            }
            Ter::TES_SUCCESS
        }

        // --- Tickets ---
        TxType::TICKET_CREATE => {
            let account = sttx.get_account_id(sf("sfAccount"));
            let count = sttx.get_field_u32(sf("sfTicketCount"));
            let mut sink = DispatcherTicketCreateSink {
                view,
                account,
                tx_sequence: sttx.get_field_u32(sf("sfSequence")),
                pre_fee_balance_drops,
            };
            run_ticket_create_do_apply(count, &mut sink)
        }

        // --- DID ---
        TxType::DID_SET => {
            let account = sttx.get_account_id(sf("sfAccount"));
            let did_keylet = protocol::did_keylet(Uint160::from_void(account.data()));
            let existing = view.peek(did_keylet).ok().flatten();
            let is_new = existing.is_none();
            let mut sle = if let Some(e) = existing {
                STLedgerEntry::from_stobject(e.clone_as_object(), *e.key())
            } else {
                let mut new_sle = STLedgerEntry::new(did_keylet);
                new_sle.set_account_id(sf("sfAccount"), account);
                new_sle
            };
            if sttx.is_field_present(sf("sfDIDDocument")) {
                sle.set_stbase(protocol::STBlob::from_buffer(
                    sf("sfDIDDocument"),
                    basics::buffer::Buffer::from(&sttx.get_field_vl(sf("sfDIDDocument"))[..]),
                ));
            }
            if sttx.is_field_present(sf("sfURI")) {
                sle.set_stbase(protocol::STBlob::from_buffer(
                    sf("sfURI"),
                    basics::buffer::Buffer::from(&sttx.get_field_vl(sf("sfURI"))[..]),
                ));
            }
            if is_new {
                let owner_dir = protocol::owner_dir_keylet(Uint160::from_void(account.data()));
                if let Ok(Some(page)) =
                    ledger::dir_append(view, &owner_dir, did_keylet.key, &|_| {})
                {
                    sle.set_field_u64(sf("sfOwnerNode"), page);
                }
                let _ = view.insert(Arc::new(sle));
                if let Ok(Some(acct)) =
                    view.peek(protocol::account_keylet(Uint160::from_void(account.data())))
                {
                    let _ = ledger::adjust_owner_count(view, &acct, 1);
                }
            } else {
                let _ = view.update(Arc::new(sle));
            }
            Ter::TES_SUCCESS
        }
        TxType::DID_DELETE => {
            let account = sttx.get_account_id(sf("sfAccount"));
            let did_keylet = protocol::did_keylet(Uint160::from_void(account.data()));
            let Some(did_sle) = view.peek(did_keylet).ok().flatten() else {
                return Ter::TEC_NO_ENTRY;
            };
            // Remove from owner directory
            let owner_node = did_sle.get_field_u64(sf("sfOwnerNode"));
            let owner_dir = owner_dir_keylet(Uint160::from_void(account.data()));
            let _ = ledger::dir_remove(view, &owner_dir, owner_node, *did_sle.key(), false);
            let _ = view.erase(did_sle);
            if let Ok(Some(acct)) =
                view.peek(protocol::account_keylet(Uint160::from_void(account.data())))
            {
                let _ = ledger::adjust_owner_count(view, &acct, -1);
            }
            Ter::TES_SUCCESS
        }

        // --- Oracle ---
        TxType::ORACLE_SET => {
            let account = sttx.get_account_id(sf("sfAccount"));
            let oracle_doc_id = sttx.get_field_u32(sf("sfOracleDocumentID"));
            let oracle_keylet =
                protocol::oracle_keylet(Uint160::from_void(account.data()), oracle_doc_id);
            let existing = view.peek(oracle_keylet).ok().flatten();
            if let Some(oracle_sle) = existing {
                let mut obj = oracle_sle.clone_as_object();
                if sttx.is_field_present(sf("sfProvider")) {
                    obj.set_stbase(protocol::STBlob::from_buffer(
                        sf("sfProvider"),
                        basics::buffer::Buffer::from(&sttx.get_field_vl(sf("sfProvider"))[..]),
                    ));
                }
                if sttx.is_field_present(sf("sfLastUpdateTime")) {
                    obj.set_field_u32(
                        sf("sfLastUpdateTime"),
                        sttx.get_field_u32(sf("sfLastUpdateTime")),
                    );
                }
                let _ = view.update(Arc::new(STLedgerEntry::from_stobject(
                    obj,
                    *oracle_sle.key(),
                )));
            } else {
                let mut sle = STLedgerEntry::new(oracle_keylet);
                sle.set_account_id(sf("sfOwner"), account);
                sle.set_field_u32(sf("sfOracleDocumentID"), oracle_doc_id);
                let owner_dir = protocol::owner_dir_keylet(Uint160::from_void(account.data()));
                if let Ok(Some(page)) =
                    ledger::dir_append(view, &owner_dir, oracle_keylet.key, &|_| {})
                {
                    sle.set_field_u64(sf("sfOwnerNode"), page);
                }
                let _ = view.insert(Arc::new(sle));
                if let Ok(Some(acct)) =
                    view.peek(protocol::account_keylet(Uint160::from_void(account.data())))
                {
                    let _ = ledger::adjust_owner_count(view, &acct, 1);
                }
            }
            Ter::TES_SUCCESS
        }
        TxType::ORACLE_DELETE => {
            let account = sttx.get_account_id(sf("sfAccount"));
            let oracle_doc_id = sttx.get_field_u32(sf("sfOracleDocumentID"));
            let oracle_keylet =
                protocol::oracle_keylet(Uint160::from_void(account.data()), oracle_doc_id);
            if let Ok(Some(oracle_sle)) = view.peek(oracle_keylet) {
                let owner_node = oracle_sle.get_field_u64(sf("sfOwnerNode"));
                let owner_dir = owner_dir_keylet(Uint160::from_void(account.data()));
                let _ = ledger::dir_remove(view, &owner_dir, owner_node, *oracle_sle.key(), false);
                let _ = view.erase(oracle_sle);
                if let Ok(Some(acct)) =
                    view.peek(protocol::account_keylet(Uint160::from_void(account.data())))
                {
                    let _ = ledger::adjust_owner_count(view, &acct, -1);
                }
            }
            Ter::TES_SUCCESS
        }

        // --- MPToken ---
        TxType::MPTOKEN_ISSUANCE_CREATE => {
            let tx_flags = sttx.get_field_u32(sf("sfFlags"));
            if !mp_token_issuance_create_check_extra_features(
                sttx.is_field_present(sf("sfDomainID")),
                view.rules()
                    .enabled(&protocol::feature_id("PermissionedDomains")),
                view.rules()
                    .enabled(&protocol::feature_id("SingleAssetVault")),
                sttx.is_field_present(sf("sfMutableFlags")),
                view.rules().enabled(&protocol::feature_id("DynamicMPT")),
            ) {
                return Ter::TEM_DISABLED;
            }
            let preflight =
                run_mp_token_issuance_create_preflight(MPTokenIssuanceCreatePreflightFacts {
                    fix_cleanup_3_2_0_enabled: view
                        .rules()
                        .enabled(&protocol::feature_id("fixCleanup3_2_0")),
                    confidential_transfer_enabled: view
                        .rules()
                        .enabled(&protocol::feature_id("ConfidentialTransfer")),
                    reference_holding_present: sttx.is_field_present(sf("sfReferenceHolding")),
                    mutable_flags: sttx
                        .is_field_present(sf("sfMutableFlags"))
                        .then(|| sttx.get_field_u32(sf("sfMutableFlags"))),
                    tx_flags,
                    transfer_fee: sttx
                        .is_field_present(sf("sfTransferFee"))
                        .then(|| sttx.get_field_u16(sf("sfTransferFee"))),
                    domain_id_present: sttx.is_field_present(sf("sfDomainID")),
                    domain_id_is_zero: sttx.is_field_present(sf("sfDomainID"))
                        && sttx.get_field_h256(sf("sfDomainID")).is_zero(),
                    metadata_len: sttx
                        .is_field_present(sf("sfMPTokenMetadata"))
                        .then(|| sttx.get_field_vl(sf("sfMPTokenMetadata")).len()),
                    maximum_amount: sttx
                        .is_field_present(sf("sfMaximumAmount"))
                        .then(|| sttx.get_field_u64(sf("sfMaximumAmount"))),
                });
            if preflight != Ter::TES_SUCCESS {
                return preflight;
            }
            let account = sttx.get_account_id(sf("sfAccount"));
            let sequence = sttx.get_seq_value();
            let account_keylet = protocol::account_keylet(Uint160::from_void(account.data()));
            let Some(account_sle) = view.peek(account_keylet).ok().flatten() else {
                return Ter::TEC_INTERNAL;
            };
            let owner_count = account_sle.get_field_u32(sf("sfOwnerCount"));
            let balance = pre_fee_balance_drops
                .unwrap_or_else(|| account_sle.get_field_amount(sf("sfBalance")).xrp().drops());
            let reserve = view.fees().account_reserve(owner_count as usize + 1) as i64;
            if balance < reserve {
                return Ter::TEC_INSUFFICIENT_RESERVE;
            }
            let issuance_keylet =
                protocol::mpt_issuance_keylet(sequence, Uint160::from_void(account.data()));
            let mut sle = STLedgerEntry::new(issuance_keylet);
            sle.set_account_id(sf("sfIssuer"), account);
            sle.set_field_u32(sf("sfSequence"), sequence);
            sle.set_field_u64(sf("sfOutstandingAmount"), 0);
            if sttx.is_field_present(sf("sfMaximumAmount")) {
                sle.set_field_u64(
                    sf("sfMaximumAmount"),
                    sttx.get_field_u64(sf("sfMaximumAmount")),
                );
            }
            if sttx.is_field_present(sf("sfAssetScale")) {
                sle.set_field_u8(sf("sfAssetScale"), sttx.get_field_u8(sf("sfAssetScale")));
            }
            if sttx.is_field_present(sf("sfTransferFee")) {
                sle.set_field_u16(sf("sfTransferFee"), sttx.get_field_u16(sf("sfTransferFee")));
            }
            if sttx.is_field_present(sf("sfMPTokenMetadata")) {
                sle.set_stbase(protocol::STBlob::from_buffer(
                    sf("sfMPTokenMetadata"),
                    basics::buffer::Buffer::from(&sttx.get_field_vl(sf("sfMPTokenMetadata"))[..]),
                ));
            }
            if sttx.is_field_present(sf("sfDomainID")) {
                sle.set_field_h256(sf("sfDomainID"), sttx.get_field_h256(sf("sfDomainID")));
            }
            if sttx.is_field_present(sf("sfMutableFlags")) {
                sle.set_field_u32(
                    sf("sfMutableFlags"),
                    sttx.get_field_u32(sf("sfMutableFlags")),
                );
            }
            sle.set_field_u32(sf("sfFlags"), tx_flags & !protocol::tfUniversal);
            let owner_dir = protocol::owner_dir_keylet(Uint160::from_void(account.data()));
            let owner_node =
                match ledger::dir_append(view, &owner_dir, issuance_keylet.key, &|_| {}) {
                    Ok(Some(page)) => page,
                    Ok(None) => return Ter::TEC_DIR_FULL,
                    Err(_) => return Ter::TEF_INTERNAL,
                };
            sle.set_field_u64(sf("sfOwnerNode"), owner_node);
            if view.insert(Arc::new(sle)).is_err() {
                return Ter::TEF_INTERNAL;
            }
            if ledger::adjust_owner_count(view, &account_sle, 1).is_err() {
                return Ter::TEF_INTERNAL;
            }
            Ter::TES_SUCCESS
        }
        TxType::MPTOKEN_ISSUANCE_DESTROY => {
            let account = sttx.get_account_id(sf("sfAccount"));
            if !sttx.is_field_present(sf("sfMPTokenIssuanceID")) {
                return Ter::TEM_MALFORMED;
            }
            let mptid = sttx.get_field_h192(sf("sfMPTokenIssuanceID"));
            if mptid.is_zero() {
                return Ter::TEC_OBJECT_NOT_FOUND;
            }
            let issuance_keylet = protocol::mpt_issuance_keylet_from_mptid(mptid);
            let Some(iss_sle) = view.peek(issuance_keylet).ok().flatten() else {
                return Ter::TEC_OBJECT_NOT_FOUND;
            };
            if iss_sle.get_account_id(sf("sfIssuer")) != account {
                return Ter::TEC_NO_PERMISSION;
            }
            let outstanding = iss_sle.get_field_u64(sf("sfOutstandingAmount"));
            let locked = if iss_sle
                .is_field_present(sf("sfLockedAmount")) { iss_sle.get_field_u64(sf("sfLockedAmount")) } else { 0 };
            if outstanding > 0 || locked != 0 {
                return Ter::TEC_HAS_OBLIGATIONS;
            }
            let owner_node = iss_sle.get_field_u64(sf("sfOwnerNode"));
            let owner_dir = owner_dir_keylet(Uint160::from_void(account.data()));
            if !matches!(
                ledger::dir_remove(view, &owner_dir, owner_node, *iss_sle.key(), false),
                Ok(true)
            ) {
                return Ter::TEF_BAD_LEDGER;
            }
            let _ = view.erase(iss_sle.clone());
            if let Ok(Some(acct)) =
                view.peek(protocol::account_keylet(Uint160::from_void(account.data())))
            {
                let _ = ledger::adjust_owner_count(view, &acct, -1);
            }
            Ter::TES_SUCCESS
        }
        TxType::MPTOKEN_ISSUANCE_SET => {
            if !mp_token_issuance_set_check_extra_features(
                sttx.is_field_present(sf("sfDomainID")),
                view.rules()
                    .enabled(&protocol::feature_id("PermissionedDomains")),
                view.rules()
                    .enabled(&protocol::feature_id("SingleAssetVault")),
            ) {
                return Ter::TEM_DISABLED;
            }
            if !sttx.is_field_present(sf("sfMPTokenIssuanceID")) {
                return Ter::TEM_MALFORMED;
            }
            let account = sttx.get_account_id(sf("sfAccount"));
            let holder = sttx
                .is_field_present(sf("sfHolder"))
                .then(|| sttx.get_account_id(sf("sfHolder")));
            let mutable_flags = sttx
                .is_field_present(sf("sfMutableFlags"))
                .then(|| sttx.get_field_u32(sf("sfMutableFlags")));
            let transfer_fee = sttx
                .is_field_present(sf("sfTransferFee"))
                .then(|| sttx.get_field_u16(sf("sfTransferFee")));
            let preflight = run_mp_token_issuance_set_preflight(MPTokenIssuanceSetPreflightFacts {
                dynamic_mpt_enabled: view.rules().enabled(&protocol::feature_id("DynamicMPT")),
                single_asset_vault_enabled: view
                    .rules()
                    .enabled(&protocol::feature_id("SingleAssetVault")),
                domain_id_present: sttx.is_field_present(sf("sfDomainID")),
                holder_present: holder.is_some(),
                account_equals_holder: holder == Some(account),
                tx_flags: sttx.get_flags(),
                mutable_flags,
                metadata_len: sttx
                    .is_field_present(sf("sfMPTokenMetadata"))
                    .then(|| sttx.get_field_vl(sf("sfMPTokenMetadata")).len()),
                transfer_fee,
            });
            if preflight != Ter::TES_SUCCESS {
                return preflight;
            }
            let mptid = sttx.get_field_h192(sf("sfMPTokenIssuanceID"));
            if mptid.is_zero() {
                return Ter::TEC_OBJECT_NOT_FOUND;
            }
            let issuance_keylet = protocol::mpt_issuance_keylet_from_mptid(mptid);
            let Some(iss_sle) = view.peek(issuance_keylet).ok().flatten() else {
                return Ter::TEC_OBJECT_NOT_FOUND;
            };
            let holder_keylet = holder.map(|holder| {
                protocol::mptoken_keylet_from_mptid(mptid, Uint160::from_void(holder.data()))
            });
            let holder_account_exists = holder.is_none_or(|holder| {
                view.peek(protocol::account_keylet(Uint160::from_void(holder.data())))
                    .ok()
                    .flatten()
                    .is_some()
            });
            let holder_token_exists = holder_keylet
                .as_ref()
                .is_none_or(|keylet| view.peek(*keylet).ok().flatten().is_some());
            let domain_id = sttx
                .is_field_present(sf("sfDomainID"))
                .then(|| sttx.get_field_h256(sf("sfDomainID")));
            let domain_exists = domain_id.is_none_or(|domain| {
                domain.is_zero()
                    || view
                        .peek(protocol::permissioned_domain_keylet_from_id(domain))
                        .ok()
                        .flatten()
                        .is_some()
            });
            let preclaim = run_mp_token_issuance_set_preclaim(MPTokenIssuanceSetPreclaimFacts {
                issuance_exists: true,
                issuance_can_lock: iss_sle.is_flag(protocol::lsfMPTCanLock),
                single_asset_vault_enabled: view
                    .rules()
                    .enabled(&protocol::feature_id("SingleAssetVault")),
                dynamic_mpt_enabled: view.rules().enabled(&protocol::feature_id("DynamicMPT")),
                tx_flags: sttx.get_flags(),
                issuer_matches: iss_sle.get_account_id(sf("sfIssuer")) == account,
                holder_present: holder.is_some(),
                holder_account_exists,
                holder_token_exists,
                domain_id_present: domain_id.is_some(),
                domain_id_is_zero: domain_id.is_some_and(|domain| domain.is_zero()),
                issuance_requires_auth: iss_sle.is_flag(protocol::lsfMPTRequireAuth),
                domain_exists,
                issuance_domain_present: iss_sle.is_field_present(sf("sfDomainID")),
                current_mutable_flags: iss_sle.get_field_u32(sf("sfMutableFlags")),
                mutable_flags,
                metadata_present: sttx.is_field_present(sf("sfMPTokenMetadata")),
                transfer_fee,
                issuance_can_transfer: iss_sle.is_flag(protocol::lsfMPTCanTransfer),
            });
            if preclaim != Ter::TES_SUCCESS {
                return preclaim;
            }

            let Some(target_sle) = holder_keylet
                .map(|keylet| view.peek(keylet).ok().flatten())
                .unwrap_or_else(|| Some(iss_sle.clone()))
            else {
                return Ter::TEC_INTERNAL;
            };
            {
                let mut obj = target_sle.clone_as_object();
                let flags_in = obj.get_field_u32(sf("sfFlags"));
                let mut flags_out = flags_in;

                if (sttx.get_flags() & protocol::tfMPTLock) != 0 {
                    flags_out |= protocol::lsfMPTLocked;
                } else if (sttx.get_flags() & protocol::tfMPTUnlock) != 0 {
                    flags_out &= !protocol::lsfMPTLocked;
                }

                if sttx.is_field_present(sf("sfMutableFlags")) {
                    let mutable_flags = sttx.get_field_u32(sf("sfMutableFlags"));
                    let mut apply_mutability = |set_flag: u32, clear_flag: u32, can_mutate: u32| {
                        if (mutable_flags & set_flag) != 0 {
                            flags_out |= can_mutate;
                        } else if (mutable_flags & clear_flag) != 0 {
                            flags_out &= !can_mutate;
                        }
                    };
                    apply_mutability(
                        protocol::tmfMPTSetCanLock,
                        protocol::tmfMPTClearCanLock,
                        protocol::lsmfMPTCanMutateCanLock,
                    );
                    apply_mutability(
                        protocol::tmfMPTSetRequireAuth,
                        protocol::tmfMPTClearRequireAuth,
                        protocol::lsmfMPTCanMutateRequireAuth,
                    );
                    apply_mutability(
                        protocol::tmfMPTSetCanEscrow,
                        protocol::tmfMPTClearCanEscrow,
                        protocol::lsmfMPTCanMutateCanEscrow,
                    );
                    apply_mutability(
                        protocol::tmfMPTSetCanTrade,
                        protocol::tmfMPTClearCanTrade,
                        protocol::lsmfMPTCanMutateCanTrade,
                    );
                    apply_mutability(
                        protocol::tmfMPTSetCanTransfer,
                        protocol::tmfMPTClearCanTransfer,
                        protocol::lsmfMPTCanMutateCanTransfer,
                    );
                    apply_mutability(
                        protocol::tmfMPTSetCanClawback,
                        protocol::tmfMPTClearCanClawback,
                        protocol::lsmfMPTCanMutateCanClawback,
                    );

                    if (mutable_flags & protocol::tmfMPTClearCanTransfer) != 0 {
                        obj.make_field_absent(sf("sfTransferFee"));
                    }
                }

                if flags_in != flags_out {
                    obj.set_field_u32(sf("sfFlags"), flags_out);
                }
                if sttx.is_field_present(sf("sfTransferFee")) {
                    let transfer_fee = sttx.get_field_u16(sf("sfTransferFee"));
                    if transfer_fee == 0 {
                        obj.make_field_absent(sf("sfTransferFee"));
                    } else {
                        obj.set_field_u16(sf("sfTransferFee"), transfer_fee);
                    }
                }
                if sttx.is_field_present(sf("sfMPTokenMetadata")) {
                    let metadata = sttx.get_field_vl(sf("sfMPTokenMetadata"));
                    if metadata.is_empty() {
                        obj.make_field_absent(sf("sfMPTokenMetadata"));
                    } else {
                        obj.set_stbase(protocol::STBlob::from_buffer(
                            sf("sfMPTokenMetadata"),
                            basics::buffer::Buffer::from(&metadata[..]),
                        ));
                    }
                }
                if sttx.is_field_present(sf("sfDomainID")) {
                    let domain_id = sttx.get_field_h256(sf("sfDomainID"));
                    if domain_id.is_zero() {
                        if obj.is_field_present(sf("sfDomainID")) {
                            obj.make_field_absent(sf("sfDomainID"));
                        }
                    } else {
                        obj.set_field_h256(sf("sfDomainID"), domain_id);
                    }
                }
                let _ = view.update(Arc::new(STLedgerEntry::from_stobject(
                    obj,
                    *target_sle.key(),
                )));
            }
            Ter::TES_SUCCESS
        }
        TxType::MPTOKEN_AUTHORIZE => {
            let account = sttx.get_account_id(sf("sfAccount"));
            let holder = sttx
                .is_field_present(sf("sfHolder"))
                .then(|| sttx.get_account_id(sf("sfHolder")));
            let preflight = run_mp_token_authorize_preflight(MPTokenAuthorizePreflightFacts {
                account_equals_holder: holder == Some(account),
            });
            if preflight != Ter::TES_SUCCESS {
                return preflight;
            }
            if !sttx.is_field_present(sf("sfMPTokenIssuanceID")) {
                return Ter::TEM_MALFORMED;
            }
            let mptid = sttx.get_field_h192(sf("sfMPTokenIssuanceID"));
            if mptid.is_zero() {
                return Ter::TEC_OBJECT_NOT_FOUND;
            }

            let flags = sttx.get_field_u32(sf("sfFlags"));
            let unauthorize = (flags & protocol::tfMPTUnauthorize) != 0;

            let issuance_keylet = protocol::mpt_issuance_keylet_from_mptid(mptid);
            if let Some(holder) = holder {
                let Some(holder_root) = view
                    .peek(protocol::account_keylet(Uint160::from_void(holder.data())))
                    .ok()
                    .flatten()
                else {
                    return Ter::TEC_NO_DST;
                };
                let Some(issuance) = view.peek(issuance_keylet).ok().flatten() else {
                    return Ter::TEC_OBJECT_NOT_FOUND;
                };
                let issuer = issuance.get_account_id(sf("sfIssuer"));
                if account != issuer {
                    return Ter::TEC_NO_PERMISSION;
                }
                if !issuance.is_flag(protocol::lsfMPTRequireAuth) {
                    return Ter::TEC_NO_AUTH;
                }
                let holder_keylet =
                    protocol::mptoken_keylet_from_mptid(mptid, Uint160::from_void(holder.data()));
                let Some(holder_token) = view.peek(holder_keylet).ok().flatten() else {
                    return Ter::TEC_OBJECT_NOT_FOUND;
                };
                if holder_root.is_field_present(sf("sfVaultID"))
                    || holder_root.is_field_present(sf("sfLoanBrokerID"))
                    || holder_root.is_field_present(sf("sfAMMID"))
                {
                    return Ter::TEC_NO_PERMISSION;
                }
                let mut obj = holder_token.clone_as_object();
                let mut token_flags = obj.get_field_u32(sf("sfFlags"));
                if unauthorize {
                    token_flags &= !protocol::lsfMPTAuthorized;
                } else {
                    token_flags |= protocol::lsfMPTAuthorized;
                }
                obj.set_field_u32(sf("sfFlags"), token_flags);
                let _ = view.update(Arc::new(STLedgerEntry::from_stobject(
                    obj,
                    *holder_token.key(),
                )));
                return Ter::TES_SUCCESS;
            }

            let token_keylet =
                protocol::mptoken_keylet_from_mptid(mptid, Uint160::from_void(account.data()));
            if unauthorize {
                let Some(token) = view.peek(token_keylet).ok().flatten() else {
                    return Ter::TEC_OBJECT_NOT_FOUND;
                };
                if token.get_field_u64(sf("sfMPTAmount")) != 0
                    || (if token
                        .is_field_present(sf("sfLockedAmount")) { token.get_field_u64(sf("sfLockedAmount")) } else { 0 })
                        != 0
                {
                    if view.peek(issuance_keylet).ok().flatten().is_none() {
                        return Ter::TEF_INTERNAL;
                    }
                    return Ter::TEC_HAS_OBLIGATIONS;
                }
                if view
                    .rules()
                    .enabled(&protocol::feature_id("SingleAssetVault"))
                    && token.is_flag(protocol::lsfMPTLocked)
                {
                    return Ter::TEC_NO_PERMISSION;
                }
                let owner_node = token.get_field_u64(sf("sfOwnerNode"));
                let owner_dir = protocol::owner_dir_keylet(Uint160::from_void(account.data()));
                let _ = ledger::dir_remove(view, &owner_dir, owner_node, *token.key(), false);
                let _ = view.erase(token);
                if let Ok(Some(acct)) =
                    view.peek(protocol::account_keylet(Uint160::from_void(account.data())))
                {
                    let _ = ledger::adjust_owner_count(view, &acct, -1);
                }
                return Ter::TES_SUCCESS;
            }
            let Some(issuance) = view.peek(issuance_keylet).ok().flatten() else {
                return Ter::TEC_OBJECT_NOT_FOUND;
            };
            let issuer = issuance.get_account_id(sf("sfIssuer"));
            if account == issuer {
                return Ter::TEC_NO_PERMISSION;
            }
            if view.peek(token_keylet).ok().flatten().is_some() {
                return Ter::TEC_DUPLICATE;
            }
            if let Ok(Some(acct)) =
                view.peek(protocol::account_keylet(Uint160::from_void(account.data())))
            {
                let owner_count = acct.get_field_u32(sf("sfOwnerCount"));
                let balance = acct.get_field_amount(sf("sfBalance")).xrp().drops();
                let reserve = if owner_count < 2 {
                    0
                } else {
                    view.fees().account_reserve(owner_count as usize + 1) as i64
                };
                if balance < reserve {
                    return Ter::TEC_INSUFFICIENT_RESERVE;
                }
            }
            let mut sle = STLedgerEntry::new(token_keylet);
            sle.set_account_id(sf("sfAccount"), account);
            sle.set_field_h192(sf("sfMPTokenIssuanceID"), mptid);
            sle.set_field_u64(sf("sfMPTAmount"), 0);
            sle.set_field_u32(sf("sfFlags"), 0);
            let owner_dir = protocol::owner_dir_keylet(Uint160::from_void(account.data()));
            let Ok(Some(page)) = ledger::dir_append(view, &owner_dir, token_keylet.key, &|_| {})
            else {
                return Ter::TEC_DIR_FULL;
            };
            sle.set_field_u64(sf("sfOwnerNode"), page);
            let _ = view.insert(Arc::new(sle));
            if let Ok(Some(acct)) =
                view.peek(protocol::account_keylet(Uint160::from_void(account.data())))
            {
                let _ = ledger::adjust_owner_count(view, &acct, 1);
            }
            Ter::TES_SUCCESS
        }

        // --- Permissioned domains ---
        TxType::PERMISSIONED_DOMAIN_SET => {
            let account = sttx.get_account_id(sf("sfAccount"));
            let tx_credentials = sttx
                .get_field_array(sf("sfAcceptedCredentials"))
                .iter()
                .map(|credential| PermissionedDomainCredential {
                    issuer: credential.get_account_id(sf("sfIssuer")),
                    credential_type: credential.get_field_vl(sf("sfCredentialType")),
                })
                .collect();
            let existing_domain_id = sttx
                .is_field_present(sf("sfDomainID"))
                .then(|| sttx.get_field_h256(sf("sfDomainID")));
            // C++ parity: ownership verification on update
            if let Some(domain_id) = existing_domain_id {
                if !domain_id.is_zero() {
                    let domain_keylet = protocol::permissioned_domain_keylet_from_id(domain_id);
                    if let Ok(Some(domain_sle)) = view.peek(domain_keylet) {
                        if domain_sle.get_account_id(sf("sfOwner")) != account {
                            return Ter::TEC_NO_PERMISSION;
                        }
                    } else {
                        return Ter::TEC_NO_ENTRY;
                    }
                }
            }
            let mut sink = ViewBackedPermissionedDomainSetSink::new(
                view,
                account,
                sttx.get_seq_value(),
                existing_domain_id,
            );
            run_permissioned_domain_set_do_apply(
                tx_credentials,
                existing_domain_id.is_some(),
                &mut sink,
            )
        }
        TxType::PERMISSIONED_DOMAIN_DELETE => {
            let account = sttx.get_account_id(sf("sfAccount"));
            let domain_id = sttx.get_field_h256(sf("sfDomainID"));
            // C++ parity: ownership verification
            if !domain_id.is_zero() {
                let domain_keylet = protocol::permissioned_domain_keylet_from_id(domain_id);
                if let Ok(Some(domain_sle)) = view.peek(domain_keylet) {
                    if domain_sle.get_account_id(sf("sfOwner")) != account {
                        return Ter::TEC_NO_PERMISSION;
                    }
                } else {
                    return Ter::TEC_NO_ENTRY;
                }
            }
            let mut sink = ViewBackedPermissionedDomainDeleteSink {
                view,
                account,
                domain_id,
            };
            run_permissioned_domain_delete_do_apply(&mut sink)
        }

        // --- Credentials ---
        TxType::CREDENTIAL_CREATE => {
            let account = sttx.get_account_id(sf("sfAccount"));
            let subject = sttx.get_account_id(sf("sfSubject"));
            let cred_type = if sttx.is_field_present(sf("sfCredentialType")) {
                sttx.get_field_vl(sf("sfCredentialType"))
            } else {
                vec![]
            };
            let cred_keylet = protocol::credential_keylet(
                Uint160::from_void(subject.data()),
                Uint160::from_void(account.data()),
                &cred_type,
            );

            if view.peek(cred_keylet).ok().flatten().is_some() {
                return Ter::TEC_DUPLICATE;
            }
            let Some(issuer_sle) = view
                .peek(protocol::account_keylet(Uint160::from_void(account.data())))
                .ok()
                .flatten()
            else {
                return Ter::TEF_INTERNAL;
            };
            if view
                .peek(protocol::account_keylet(Uint160::from_void(subject.data())))
                .ok()
                .flatten()
                .is_none()
            {
                return Ter::TEC_NO_TARGET;
            }
            if sttx.is_field_present(sf("sfExpiration")) {
                let expiration = sttx.get_field_u32(sf("sfExpiration"));
                if view.header().parent_close_time > expiration {
                    return Ter::TEC_EXPIRED;
                }
            }
            let owner_count = issuer_sle.get_field_u32(sf("sfOwnerCount"));
            let balance = pre_fee_balance_drops
                .unwrap_or_else(|| issuer_sle.get_field_amount(sf("sfBalance")).xrp().drops());
            let reserve = view.fees().account_reserve(owner_count as usize + 1) as i64;
            if balance < reserve {
                return Ter::TEC_INSUFFICIENT_RESERVE;
            }

            let mut sle = STLedgerEntry::new(cred_keylet);
            sle.set_account_id(sf("sfIssuer"), account);
            sle.set_account_id(sf("sfSubject"), subject);
            if sttx.is_field_present(sf("sfCredentialType")) {
                sle.set_stbase(protocol::STBlob::from_buffer(
                    sf("sfCredentialType"),
                    basics::buffer::Buffer::from(&sttx.get_field_vl(sf("sfCredentialType"))[..]),
                ));
            }
            if sttx.is_field_present(sf("sfExpiration")) {
                sle.set_field_u32(sf("sfExpiration"), sttx.get_field_u32(sf("sfExpiration")));
            }
            if sttx.is_field_present(sf("sfURI")) {
                sle.set_stbase(protocol::STBlob::from_buffer(
                    sf("sfURI"),
                    basics::buffer::Buffer::from(&sttx.get_field_vl(sf("sfURI"))[..]),
                ));
            }
            let issuer_dir = protocol::owner_dir_keylet(Uint160::from_void(account.data()));
            let Ok(Some(issuer_page)) =
                ledger::dir_append(view, &issuer_dir, cred_keylet.key, &|_| {})
            else {
                return Ter::TEC_DIR_FULL;
            };
            sle.set_field_u64(sf("sfIssuerNode"), issuer_page);
            if ledger::adjust_owner_count(view, &issuer_sle, 1).is_err() {
                return Ter::TEF_INTERNAL;
            }

            if subject == account {
                sle.set_field_u32(sf("sfFlags"), protocol::lsfAccepted);
            } else {
                let subject_dir = protocol::owner_dir_keylet(Uint160::from_void(subject.data()));
                let Ok(Some(subject_page)) =
                    ledger::dir_append(view, &subject_dir, cred_keylet.key, &|_| {})
                else {
                    return Ter::TEC_DIR_FULL;
                };
                sle.set_field_u64(sf("sfSubjectNode"), subject_page);
            }

            if view.insert(Arc::new(sle)).is_err() {
                return Ter::TEF_INTERNAL;
            }

            Ter::TES_SUCCESS
        }
        TxType::CREDENTIAL_ACCEPT => {
            let subject = sttx.get_account_id(sf("sfAccount"));
            let issuer = sttx.get_account_id(sf("sfIssuer"));
            let cred_type = if sttx.is_field_present(sf("sfCredentialType")) {
                sttx.get_field_vl(sf("sfCredentialType"))
            } else {
                vec![]
            };
            let cred_keylet = protocol::credential_keylet(
                Uint160::from_void(subject.data()),
                Uint160::from_void(issuer.data()),
                &cred_type,
            );

            let Some(cred_sle) = view.peek(cred_keylet).ok().flatten() else {
                return Ter::TEC_NO_ENTRY;
            };
            let Some(subject_sle) = view
                .peek(protocol::account_keylet(Uint160::from_void(subject.data())))
                .ok()
                .flatten()
            else {
                return Ter::TEF_INTERNAL;
            };
            let Some(issuer_sle) = view
                .peek(protocol::account_keylet(Uint160::from_void(issuer.data())))
                .ok()
                .flatten()
            else {
                return Ter::TEF_INTERNAL;
            };
            if ledger::credential_helpers::check_expired(&cred_sle, view.header().parent_close_time)
            {
                let result = ledger::credential_helpers::delete_sle(view, cred_sle)
                    .unwrap_or(Ter::TEF_INTERNAL);
                return if result == Ter::TES_SUCCESS {
                    Ter::TEC_EXPIRED
                } else {
                    result
                };
            }

            let owner_count = subject_sle.get_field_u32(sf("sfOwnerCount"));
            let balance = pre_fee_balance_drops
                .unwrap_or_else(|| subject_sle.get_field_amount(sf("sfBalance")).xrp().drops());
            let reserve = view.fees().account_reserve(owner_count as usize + 1) as i64;
            if balance < reserve {
                return Ter::TEC_INSUFFICIENT_RESERVE;
            }

            // C++ parity: reject duplicate acceptance
            if cred_sle.is_flag(protocol::lsfAccepted) {
                return Ter::TEC_DUPLICATE;
            }

            let mut obj = cred_sle.clone_as_object();
            obj.set_field_u32(sf("sfFlags"), protocol::lsfAccepted);
            if view
                .update(Arc::new(STLedgerEntry::from_stobject(obj, *cred_sle.key())))
                .is_err()
            {
                return Ter::TEF_INTERNAL;
            }
            if ledger::adjust_owner_count(view, &issuer_sle, -1).is_err()
                || ledger::adjust_owner_count(view, &subject_sle, 1).is_err()
            {
                return Ter::TEF_INTERNAL;
            }
            Ter::TES_SUCCESS
        }
        TxType::CREDENTIAL_DELETE => {
            let account = sttx.get_account_id(sf("sfAccount"));
            let subject = if sttx.is_field_present(sf("sfSubject")) {
                sttx.get_account_id(sf("sfSubject"))
            } else {
                account
            };
            let issuer = if sttx.is_field_present(sf("sfIssuer")) {
                sttx.get_account_id(sf("sfIssuer"))
            } else {
                account
            };
            let cred_type = if sttx.is_field_present(sf("sfCredentialType")) {
                sttx.get_field_vl(sf("sfCredentialType"))
            } else {
                vec![]
            };
            let cred_keylet = protocol::credential_keylet(
                Uint160::from_void(subject.data()),
                Uint160::from_void(issuer.data()),
                &cred_type,
            );
            let Some(cred_sle) = view.peek(cred_keylet).ok().flatten() else {
                return Ter::TEC_NO_ENTRY;
            };
            if account != subject
                && account != issuer
                && !ledger::credential_helpers::check_expired(
                    &cred_sle,
                    view.header().parent_close_time,
                )
            {
                return Ter::TEC_NO_PERMISSION;
            }
            ledger::credential_helpers::delete_sle(view, cred_sle).unwrap_or(Ter::TEF_INTERNAL)
        }

        // --- AMM Clawback ---
        TxType::AMM_CLAWBACK => {
            let issuer = sttx.get_account_id(sf("sfAccount"));
            let holder = sttx.get_account_id(sf("sfHolder"));
            let asset1 = tx_amm_asset(sttx, sf("sfAsset"));
            let asset2 = tx_amm_asset(sttx, sf("sfAsset2"));
            if !sttx.is_field_present(sf("sfAsset"))
                || !sttx.is_field_present(sf("sfAsset2"))
                || (asset1.native() && asset2.native() && sttx.is_field_present(sf("sfAmount")))
            {
                return legacy_amm_clawback_direct_dispatch(view, sttx);
            }
            if issuer == holder {
                return Ter::TEM_MALFORMED;
            }
            if asset1.native() || asset1.issuer() != issuer {
                return Ter::TEM_MALFORMED;
            }
            let claw_two_assets = sttx.get_flags() & protocol::AMM_CLAWBACK_TWO_ASSETS_FLAG != 0;
            if claw_two_assets && asset1.issuer() != asset2.issuer() {
                return Ter::TEM_INVALID_FLAG;
            }
            let amount = optional_tx_amount(sttx, sf("sfAmount"));
            if let Some(amount) = &amount {
                if amount.asset() != asset1 {
                    return Ter::TEM_BAD_AMOUNT;
                }
                if amount.signum() <= 0 {
                    return Ter::TEM_BAD_AMOUNT;
                }
            }

            let Some(issuer_sle) = view
                .peek(protocol::account_keylet(Uint160::from_void(issuer.data())))
                .ok()
                .flatten()
            else {
                return Ter::TER_NO_ACCOUNT;
            };
            if view
                .peek(protocol::account_keylet(Uint160::from_void(holder.data())))
                .ok()
                .flatten()
                .is_none()
            {
                return Ter::TER_NO_ACCOUNT;
            }
            let amm_keylet = protocol::keylet::amm(asset1, asset2);
            let Some(amm_sle) = view.peek(amm_keylet).ok().flatten() else {
                return Ter::TER_NO_AMM;
            };
            if !view.rules().enabled(&protocol::feature_id("MPTokensV2"))
                && (!issuer_sle.is_flag(protocol::lsfAllowTrustLineClawback)
                    || issuer_sle.is_flag(protocol::lsfNoFreeze))
            {
                return Ter::TEC_NO_PERMISSION;
            }
            if !amm_clawback_asset_allowed(view, &issuer, &issuer_sle, asset1) {
                return Ter::TEC_NO_PERMISSION;
            }
            if claw_two_assets && !amm_clawback_asset_allowed(view, &issuer, &issuer_sle, asset2) {
                return Ter::TEC_NO_PERMISSION;
            }

            let amm_account = amm_sle.get_account_id(sf("sfAccount"));
            let account_keylet = protocol::account_keylet(Uint160::from_void(amm_account.data()));
            if view.peek(account_keylet).ok().flatten().is_none() {
                return Ter::TEC_INTERNAL;
            }
            let lp_total = amm_sle.get_field_amount(sf("sfLPTokenBalance"));
            if lp_total.signum() == 0 {
                return Ter::TEC_AMM_EMPTY;
            }
            let Some(holder_lp) = amm_lp_holds_in_view(view, &amm_sle, holder).ok().flatten()
            else {
                return Ter::TEC_AMM_BALANCE;
            };
            if holder_lp.signum() == 0 {
                return Ter::TEC_AMM_BALANCE;
            }
            let pool1 = account_holds_amm_asset(view, &amm_account, asset1, sf("sfAmount"))
                .unwrap_or_else(|| zero_amount_for_asset(sf("sfAmount"), asset1));
            let pool2 = account_holds_amm_asset(view, &amm_account, asset2, sf("sfAmount2"))
                .unwrap_or_else(|| zero_amount_for_asset(sf("sfAmount2"), asset2));
            if pool1.signum() <= 0 || pool2.signum() <= 0 {
                return Ter::TEC_INTERNAL;
            }

            let math = match amm_clawback_math(
                amount.as_ref(),
                &pool1,
                &pool2,
                &lp_total,
                &holder_lp,
                view.rules(),
            ) {
                Ok(math) => math,
                Err(ter) => return ter,
            };
            let lp_issue = lp_total.issue();
            let Some(amount1) = math.amount1.as_ref() else {
                return Ter::TEC_INTERNAL;
            };
            let Some(amount2) = math.amount2.as_ref() else {
                return Ter::TEC_INTERNAL;
            };

            let res = amm_withdraw_asset(view, &amm_account, &holder, amount1);
            if res != Ter::TES_SUCCESS {
                return res;
            }
            let res = amm_withdraw_asset(view, &amm_account, &holder, amount2);
            if res != Ter::TES_SUCCESS {
                return res;
            }
            let res = crate::state::amm_bid_apply::redeem_iou_pub(
                view,
                &holder,
                &math.lp_tokens,
                &lp_issue,
            );
            if res != Ter::TES_SUCCESS {
                return res;
            }
            let mut obj = amm_sle.clone_as_object();
            obj.set_field_amount(sf("sfLPTokenBalance"), math.new_lp_token_balance);
            let _ = view.update(Arc::new(STLedgerEntry::from_stobject(obj, *amm_sle.key())));

            let res = amm_clawback_send_amount(view, &holder, &issuer, amount1);
            if res != Ter::TES_SUCCESS {
                return res;
            }
            if claw_two_assets {
                let res = amm_clawback_send_amount(view, &holder, &issuer, amount2);
                if res != Ter::TES_SUCCESS {
                    return res;
                }
            }

            Ter::TES_SUCCESS
        }

        // --- NFToken Modify ---
        TxType::NFTOKEN_MODIFY => {
            if !view.rules().enabled(&protocol::feature_id("DynamicNFT")) {
                return Ter::TEM_DISABLED;
            }
            let account = sttx.get_account_id(sf("sfAccount"));
            let owner = if sttx.is_field_present(sf("sfOwner")) {
                sttx.get_account_id(sf("sfOwner"))
            } else {
                account
            };
            // preflight
            if sttx.is_field_present(sf("sfOwner")) && owner == account {
                return Ter::TEM_MALFORMED;
            }
            if sttx.is_field_present(sf("sfURI")) {
                let uri = sttx.get_field_vl(sf("sfURI"));
                if uri.is_empty() || uri.len() > protocol::MAX_TOKEN_URI_LENGTH {
                    return Ter::TEM_MALFORMED;
                }
            }
            let token_id = sttx.get_field_h256(sf("sfNFTokenID"));
            // preclaim: find token
            let Some((_, page)) = nft_find_token_and_page(view, &owner, token_id)
                .ok()
                .flatten()
            else {
                return Ter::TEC_NO_ENTRY;
            };
            // check mutable flag
            let nft_flags = protocol::get_nft_flags(token_id);
            if (nft_flags & protocol::nft::FLAG_MUTABLE) == 0 {
                return Ter::TEC_NO_PERMISSION;
            }
            // verify issuer permissions
            let issuer = protocol::get_nft_issuer(token_id);
            if issuer != account {
                let issuer_keylet = protocol::account_keylet(Uint160::from_void(issuer.data()));
                let Some(issuer_sle) = view.peek(issuer_keylet).ok().flatten() else {
                    return Ter::TEC_INTERNAL;
                };
                let minter_matches = issuer_sle.is_field_present(sf("sfNFTokenMinter"))
                    && issuer_sle.get_account_id(sf("sfNFTokenMinter")) == account;
                if !minter_matches {
                    return Ter::TEC_NO_PERMISSION;
                }
            }
            // doApply: change URI on the token in the page
            let tokens = page.get_field_array(sf("sfNFTokens"));
            let mut new_tokens = protocol::STArray::new(sf("sfNFTokens"));
            for token in tokens.iter() {
                let tid = token.get_field_h256(sf("sfNFTokenID"));
                if tid == token_id {
                    let mut modified = token.clone();
                    if sttx.is_field_present(sf("sfURI")) {
                        let uri = sttx.get_field_vl(sf("sfURI"));
                        modified.set_field_vl(sf("sfURI"), &uri);
                    } else if modified.is_field_present(sf("sfURI")) {
                        modified.make_field_absent(sf("sfURI"));
                    }
                    new_tokens.push_back(modified);
                } else {
                    new_tokens.push_back(token.clone());
                }
            }
            let mut obj = page.clone_as_object();
            obj.set_field_array(sf("sfNFTokens"), new_tokens);
            let _ = view.update(Arc::new(STLedgerEntry::from_stobject(obj, *page.key())));
            Ter::TES_SUCCESS
        }

        // --- AMMBid: full reference AMMBid::applyBid parity ---
        TxType::AMM_BID => {
            let asset1 = tx_amm_asset(sttx, sf("sfAsset"));
            let asset2 = tx_amm_asset(sttx, sf("sfAsset2"));
            let mpt_gate = check_amm_mptokens_v2_gate(view, &[asset1, asset2]);
            if mpt_gate != Ter::TES_SUCCESS {
                return mpt_gate;
            }
            crate::state::amm_bid_apply::apply_amm_bid(view, sttx)
        }

        // --- Change pseudo-transaction (reference the reference source) ---
        TxType::FEE => {
            let k = protocol::fee_settings_keylet();
            let mut obj = if let Ok(Some(existing)) = view.peek(k) {
                existing.clone_as_object()
            } else {
                protocol::STObject::new(sf("sfGeneric"))
            };
            if sttx.is_field_present(sf("sfBaseFeeDrops")) {
                obj.set_field_amount(
                    sf("sfBaseFeeDrops"),
                    sttx.get_field_amount(sf("sfBaseFeeDrops")),
                );
                obj.set_field_amount(
                    sf("sfReserveBaseDrops"),
                    sttx.get_field_amount(sf("sfReserveBaseDrops")),
                );
                obj.set_field_amount(
                    sf("sfReserveIncrementDrops"),
                    sttx.get_field_amount(sf("sfReserveIncrementDrops")),
                );
            } else {
                if sttx.is_field_present(sf("sfBaseFee")) {
                    obj.set_field_u64(sf("sfBaseFee"), sttx.get_field_u64(sf("sfBaseFee")));
                }
                if sttx.is_field_present(sf("sfReferenceFeeUnits")) {
                    obj.set_field_u32(
                        sf("sfReferenceFeeUnits"),
                        sttx.get_field_u32(sf("sfReferenceFeeUnits")),
                    );
                }
                if sttx.is_field_present(sf("sfReserveBase")) {
                    obj.set_field_u32(sf("sfReserveBase"), sttx.get_field_u32(sf("sfReserveBase")));
                }
                if sttx.is_field_present(sf("sfReserveIncrement")) {
                    obj.set_field_u32(
                        sf("sfReserveIncrement"),
                        sttx.get_field_u32(sf("sfReserveIncrement")),
                    );
                }
            }
            let sle = Arc::new(protocol::STLedgerEntry::from_stobject(obj, k.key));
            let _ = view.update(sle);
            Ter::TES_SUCCESS
        }

        TxType::AMENDMENT => {
            let k = protocol::amendments_keylet();
            let mut obj = if let Ok(Some(existing)) = view.peek(k) {
                existing.clone_as_object()
            } else {
                protocol::STObject::new(sf("sfGeneric"))
            };
            let amendment = sttx.get_field_h256(sf("sfAmendment"));
            let flags = sttx.get_field_u32(sf("sfFlags"));
            let got_majority = (flags & 0x0001_0000) != 0;
            let lost_majority = (flags & 0x0002_0000) != 0;

            if got_majority {
                let mut majorities = if obj.is_field_present(sf("sfMajorities")) {
                    obj.get_field_array(sf("sfMajorities"))
                } else {
                    protocol::STArray::new(sf("sfMajorities"))
                };
                let mut entry = protocol::STObject::new(sf("sfGeneric"));
                entry.set_field_h256(sf("sfAmendment"), amendment);
                entry.set_field_u32(sf("sfCloseTime"), view.parent_close_time().as_seconds());
                majorities.push_back(entry);
                obj.set_field_array(sf("sfMajorities"), majorities);
            } else if lost_majority {
                if obj.is_field_present(sf("sfMajorities")) {
                    let old = obj.get_field_array(sf("sfMajorities"));
                    let mut new_maj = protocol::STArray::new(sf("sfMajorities"));
                    for entry in old.iter() {
                        if entry.get_field_h256(sf("sfAmendment")) != amendment {
                            new_maj.push_back(entry.clone());
                        }
                    }
                    if new_maj.is_empty() {
                        obj.make_field_absent(sf("sfMajorities"));
                    } else {
                        obj.set_field_array(sf("sfMajorities"), new_maj);
                    }
                }
            } else {
                // Enable amendment
                let mut amendments = if obj.is_field_present(sf("sfAmendments")) {
                    obj.get_field_v256(sf("sfAmendments"))
                } else {
                    protocol::STVector256::new()
                };
                amendments.push_back(amendment);
                obj.set_field_v256(sf("sfAmendments"), amendments);
                // Remove from majorities
                if obj.is_field_present(sf("sfMajorities")) {
                    let old = obj.get_field_array(sf("sfMajorities"));
                    let mut new_maj = protocol::STArray::new(sf("sfMajorities"));
                    for entry in old.iter() {
                        if entry.get_field_h256(sf("sfAmendment")) != amendment {
                            new_maj.push_back(entry.clone());
                        }
                    }
                    if new_maj.is_empty() {
                        obj.make_field_absent(sf("sfMajorities"));
                    } else {
                        obj.set_field_array(sf("sfMajorities"), new_maj);
                    }
                }
            }
            let sle = Arc::new(protocol::STLedgerEntry::from_stobject(obj, k.key));
            let _ = view.update(sle);
            Ter::TES_SUCCESS
        }

        TxType::UNL_MODIFY => {
            let k = protocol::negative_unl_keylet();
            let mut obj = if let Ok(Some(existing)) = view.peek(k) {
                existing.clone_as_object()
            } else {
                protocol::STObject::new(sf("sfGeneric"))
            };
            let disabling = sttx.is_field_present(sf("sfUNLModifyDisabling"))
                && sttx.get_field_u8(sf("sfUNLModifyDisabling")) != 0;
            let validator = sttx.get_field_vl(sf("sfUNLModifyValidator"));
            if disabling {
                obj.set_field_vl(sf("sfValidatorToDisable"), &validator);
            } else {
                obj.set_field_vl(sf("sfValidatorToReEnable"), &validator);
            }
            let sle = Arc::new(protocol::STLedgerEntry::from_stobject(obj, k.key));
            let _ = view.update(sle);
            Ter::TES_SUCCESS
        }

        // --- Confidential MPT ---
        TxType::CONFIDENTIAL_MPT_CONVERT => {
            if !view
                .rules()
                .enabled(&protocol::feature_id("ConfidentialTransfer"))
            {
                return Ter::TEM_DISABLED;
            }
            Ter::TES_SUCCESS
        }
        TxType::CONFIDENTIAL_MPT_MERGE_INBOX => {
            if !view
                .rules()
                .enabled(&protocol::feature_id("ConfidentialTransfer"))
            {
                return Ter::TEM_DISABLED;
            }
            Ter::TES_SUCCESS
        }
        TxType::CONFIDENTIAL_MPT_CONVERT_BACK => {
            if !view
                .rules()
                .enabled(&protocol::feature_id("ConfidentialTransfer"))
            {
                return Ter::TEM_DISABLED;
            }
            Ter::TES_SUCCESS
        }
        TxType::CONFIDENTIAL_MPT_SEND => {
            if !view
                .rules()
                .enabled(&protocol::feature_id("ConfidentialTransfer"))
            {
                return Ter::TEM_DISABLED;
            }
            Ter::TES_SUCCESS
        }
        TxType::CONFIDENTIAL_MPT_CLAWBACK => {
            if !view
                .rules()
                .enabled(&protocol::feature_id("ConfidentialTransfer"))
            {
                return Ter::TEM_DISABLED;
            }
            Ter::TES_SUCCESS
        }

        _ => Ter::TEM_UNKNOWN,
    }
}

/// Direct XRP payment — debit source, credit destination.
fn do_xrp_payment<V: ledger::ApplyView>(
    view: &mut V,
    src: &AccountID,
    dst: &AccountID,
    amount: &STAmount,
    _flags: u32,
) -> Ter {
    let xrp = amount.xrp().drops();
    if xrp <= 0 {
        return Ter::TES_SUCCESS;
    }

    let src_keylet = protocol::account_keylet(Uint160::from_void(src.data()));
    let dst_keylet = protocol::account_keylet(Uint160::from_void(dst.data()));

    if let Ok(Some(src_sle)) = view.peek(src_keylet) {
        let bal = src_sle.get_field_amount(sf("sfBalance")).xrp().drops();
        let mut obj = src_sle.clone_as_object();
        obj.set_field_amount(
            sf("sfBalance"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(bal - xrp)),
        );
        let _ = view.update(Arc::new(STLedgerEntry::from_stobject(obj, *src_sle.key())));
    }

    if let Ok(Some(dst_sle)) = view.peek(dst_keylet) {
        let bal = dst_sle.get_field_amount(sf("sfBalance")).xrp().drops();
        let mut obj = dst_sle.clone_as_object();
        obj.set_field_amount(
            sf("sfBalance"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(bal + xrp)),
        );
        let _ = view.update(Arc::new(STLedgerEntry::from_stobject(obj, *dst_sle.key())));
    }

    Ter::TES_SUCCESS
}

fn close_channel<V: ledger::ApplyView>(view: &mut V, chan: &STLedgerEntry, key: Uint256) -> Ter {
    let src = chan.get_account_id(sf("sfAccount"));

    // Remove from source owner directory
    let owner_node = chan.get_field_u64(sf("sfOwnerNode"));
    let src_dir = protocol::owner_dir_keylet(Uint160::from_void(src.data()));
    let _ = ledger::dir_remove(view, &src_dir, owner_node, key, true);

    // Remove from destination owner directory if present
    if chan.is_field_present(sf("sfDestinationNode")) {
        let dst = chan.get_account_id(sf("sfDestination"));
        let dst_node = chan.get_field_u64(sf("sfDestinationNode"));
        let dst_dir = protocol::owner_dir_keylet(Uint160::from_void(dst.data()));
        let _ = ledger::dir_remove(view, &dst_dir, dst_node, key, true);
    }

    // Return remaining funds to source (Amount - Balance)
    let chan_amount = chan.get_field_amount(sf("sfAmount")).xrp().drops();
    let chan_balance = chan.get_field_amount(sf("sfBalance")).xrp().drops();
    let refund = chan_amount - chan_balance;

    let src_keylet = protocol::account_keylet(Uint160::from_void(src.data()));
    if let Ok(Some(src_sle)) = view.peek(src_keylet) {
        let src_bal = src_sle.get_field_amount(sf("sfBalance")).xrp().drops();
        let mut src_obj = src_sle.clone_as_object();
        src_obj.set_field_amount(
            sf("sfBalance"),
            STAmount::from_xrp_amount(XRPAmount::from_drops(src_bal + refund)),
        );
        let _ = view.update(Arc::new(STLedgerEntry::from_stobject(
            src_obj,
            *src_sle.key(),
        )));
        let _ = ledger::adjust_owner_count(view, &src_sle, -1);
    }

    // Erase the channel
    let _ = view.erase(Arc::new(chan.clone()));
    Ter::TES_SUCCESS
}

fn asf_to_lsf(asf: u32) -> u32 {
    match asf {
        1 => 0x0002_0000,  // asfRequireDest → lsfRequireDestTag
        2 => 0x0004_0000,  // asfRequireAuth → lsfRequireAuth
        3 => 0x0008_0000,  // asfDisallowXRP → lsfDisallowXRP
        4 => 0x0010_0000,  // asfDisableMaster → lsfDisableMaster
        5 => 0,            // asfAccountTxnID — handled separately (field, not flag)
        6 => 0x0020_0000,  // asfNoFreeze → lsfNoFreeze
        7 => 0x0040_0000,  // asfGlobalFreeze → lsfGlobalFreeze
        8 => 0x0080_0000,  // asfDefaultRipple → lsfDefaultRipple
        9 => 0x0100_0000,  // asfDepositAuth → lsfDepositAuth
        10 => 0,           // asfAuthorizedNFTokenMinter — handled separately (field, not flag)
        12 => 0x0400_0000, // asfDisallowIncomingNFTokenOffer
        13 => 0x0800_0000, // asfDisallowIncomingCheck
        14 => 0x1000_0000, // asfDisallowIncomingPayChan
        15 => 0x2000_0000, // asfDisallowIncomingTrustline
        16 => 0x8000_0000, // asfAllowTrustLineClawback → lsfAllowTrustLineClawback
        17 => 0x4000_0000, // asfAllowTrustLineLocking → lsfAllowTrustLineLocking
        _ => 0,
    }
}
