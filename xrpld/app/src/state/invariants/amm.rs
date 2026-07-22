use super::common::*;
use basics::number::{NumberParts as RuntimeNumber, get_mantissa_scale, root2};
use ledger::{ApplyView, FlowSandbox, ReadView};
use protocol::{AccountID, Asset, LedgerEntryType, STAmount, STLedgerEntry, Ter};

#[derive(Default)]
pub(super) struct AmmState {
    amm_after: bool,
    amm_deleted: bool,
    amm_account: Option<AccountID>,
    asset: Option<Asset>,
    asset2: Option<Asset>,
    amount: Option<STAmount>,
    amount2: Option<STAmount>,
    lpt_balance_before: Option<STAmount>,
    lpt_balance_before_deletion: Option<STAmount>,
    lpt_balance_after: Option<STAmount>,
    pool_changed: bool,
}
pub(super) fn record_amm_state(
    state: &mut AmmState,
    is_delete: bool,
    before: Option<&STLedgerEntry>,
    after: Option<&STLedgerEntry>,
) {
    if is_delete {
        if let Some(before) = before
            && before.get_type() == LedgerEntryType::AMM
        {
            state.amm_deleted = true;
            state.lpt_balance_before_deletion =
                Some(before.get_field_amount(sf("sfLPTokenBalance")));
        }
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

pub(super) fn amm_invariant_result_applies(result: Ter) -> bool {
    protocol::is_tes_success(result) || result == Ter::TEC_INCOMPLETE
}

pub(super) fn valid_amm_balances(
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

pub(super) fn amm_pool_holds<V: ApplyView>(
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

pub(super) fn validates_amm_create<V: ApplyView>(
    sandbox: &FlowSandbox<V>,
    state: &AmmState,
) -> bool {
    let Some(lp_tokens) = &state.lpt_balance_after else {
        return false;
    };
    let Some((amount, amount2)) = amm_pool_holds(sandbox, state) else {
        return false;
    };

    valid_amm_balances(&amount, &amount2, lp_tokens, false)
        && ledger::amm_helpers::amm_lp_tokens(&amount, &amount2, lp_tokens.issue()) == *lp_tokens
}

pub(super) fn validates_amm_general<V: ApplyView>(
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
        // Pool fully emptied (all LP withdrawn for AMMDelete) — valid state
        return true;
    }
    let distance = RuntimeNumber::try_from_external_parts(1, -11, get_mantissa_scale())
        .expect("relative distance constant");
    ledger::amm_helpers::within_relative_distance_amount(pool_product_mean, lp_number, distance)
}

pub(super) fn validates_amm_state<V: ApplyView>(
    sandbox: &FlowSandbox<V>,
    txn_type: protocol::TxType,
    result: Ter,
    state: &AmmState,
) -> bool {
    if !amm_invariant_result_applies(result) {
        return true;
    }

    let enforce = sandbox.rules().enabled(&protocol::fix_ammv1_3());
    let enforce_amm_delete = sandbox.rules().enabled(&protocol::fix_cleanup_3_3_0());

    if enforce_amm_delete
        && state.amm_deleted
        && !matches!(
            txn_type,
            protocol::TxType::AMM_WITHDRAW
                | protocol::TxType::AMM_CLAWBACK
                | protocol::TxType::AMM_DELETE
        )
    {
        return false;
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
        protocol::TxType::AMM_CREATE => {
            state.amm_after && (validates_amm_create(sandbox, state) || !enforce)
        }
        protocol::TxType::AMM_DEPOSIT => {
            state.amm_after && (validates_amm_general(sandbox, state, false) || !enforce)
        }
        protocol::TxType::AMM_WITHDRAW | protocol::TxType::AMM_CLAWBACK => {
            (enforce_amm_delete && state.amm_deleted)
                || !state.amm_after
                || validates_amm_general(sandbox, state, true)
                || !enforce
        }
        protocol::TxType::AMM_DELETE => {
            if state.amm_after && enforce {
                return false;
            }
            if !enforce_amm_delete {
                return true;
            }
            if protocol::is_tes_success(result) {
                state.amm_deleted
                    && state
                        .lpt_balance_before_deletion
                        .as_ref()
                        .is_some_and(|balance| balance.signum() == 0)
            } else {
                !state.amm_deleted
            }
        }
        protocol::TxType::CHECK_CASH
        | protocol::TxType::OFFER_CREATE
        | protocol::TxType::PAYMENT => {
            // DEX invariant only fails if AMM object was changed
            // AND fixAMMv1_3 (enforce) is enabled. Without fixAMMv1_3 it always passes.
            if state.amm_after {
                let enforce = sandbox.rules().enabled(&protocol::fix_ammv1_3());
                if enforce {
                    return false;
                }
            }
            true
        }
        _ => true,
    }
}
