//! `AmmDeposit` transactor port.

use crate::{
    ApplyContext, ApplyResult, PreclaimContext, PreflightContext, TransactorPreflight0Facts,
    TransactorPreflight1Facts, run_transactor_preflight0, run_transactor_preflight1,
};
use basics::number::{NumberParts as RuntimeNumber, root2};
use ledger::amm_helpers::{self, IsDeposit};
use protocol::get_field_by_symbol;
use protocol::{NotTec, Ter, feature_batch, fix_ammv1_3, is_tes_success};

pub const fn amm_deposit_check_extra_features(
    amm_enabled: bool,
    mptokens_v2_enabled: bool,
    asset_holds_mpt: bool,
    asset2_holds_mpt: bool,
    amount_holds_mpt: bool,
    amount2_holds_mpt: bool,
) -> bool {
    if !amm_enabled {
        return false;
    }

    if (asset_holds_mpt || asset2_holds_mpt || amount_holds_mpt || amount2_holds_mpt)
        && !mptokens_v2_enabled
    {
        return false;
    }

    true
}

#[derive(Debug, Clone)]
pub struct AMMDepositPreflightFacts {
    pub flags: u32,
    pub asset_pair_invalid: Option<NotTec>,
    pub amount: Option<protocol::Asset>,
    pub amount_invalid: Option<NotTec>,
    pub amount2: Option<protocol::Asset>,
    pub amount2_invalid: Option<NotTec>,
    pub e_price: Option<protocol::Asset>,
    pub e_price_invalid: Option<NotTec>,
    pub lp_token_out_signum: Option<i32>,
    pub trading_fee: Option<u16>,
}

pub fn run_amm_deposit_preflight_facts(facts: AMMDepositPreflightFacts) -> NotTec {
    let has_amount = facts.amount.is_some();
    let has_amount2 = facts.amount2.is_some();
    let has_e_price = facts.e_price.is_some();
    let has_lp_tokens = facts.lp_token_out_signum.is_some();
    let has_trading_fee = facts.trading_fee.is_some();
    let sub_tx_flags = facts.flags & protocol::DEPOSIT_SUB_TX_FLAGS;

    if sub_tx_flags.count_ones() != 1 {
        return Ter::TEM_MALFORMED;
    }

    if (sub_tx_flags & protocol::AMM_LP_TOKEN_FLAG) != 0 {
        if !has_lp_tokens
            || has_e_price
            || (has_amount && !has_amount2)
            || (!has_amount && has_amount2)
            || has_trading_fee
        {
            return Ter::TEM_MALFORMED;
        }
    } else if (sub_tx_flags & protocol::AMM_SINGLE_ASSET_FLAG) != 0 {
        if !has_amount || has_amount2 || has_e_price || has_trading_fee {
            return Ter::TEM_MALFORMED;
        }
    } else if (sub_tx_flags & protocol::AMM_TWO_ASSET_FLAG) != 0 {
        if !has_amount || !has_amount2 || has_e_price || has_trading_fee {
            return Ter::TEM_MALFORMED;
        }
    } else if (sub_tx_flags & protocol::AMM_ONE_ASSET_LP_TOKEN_FLAG) != 0 {
        if !has_amount || !has_lp_tokens || has_amount2 || has_e_price || has_trading_fee {
            return Ter::TEM_MALFORMED;
        }
    } else if (sub_tx_flags & protocol::AMM_LIMIT_LP_TOKEN_FLAG) != 0 {
        if !has_amount || !has_e_price || has_lp_tokens || has_amount2 || has_trading_fee {
            return Ter::TEM_MALFORMED;
        }
    } else if (sub_tx_flags & protocol::AMM_TWO_ASSET_IF_EMPTY_FLAG) != 0
        && (!has_amount || !has_amount2 || has_e_price || has_lp_tokens)
    {
        return Ter::TEM_MALFORMED;
    }

    if let Some(err) = facts.asset_pair_invalid {
        return err;
    }

    if let (Some(amount), Some(amount2)) = (facts.amount, facts.amount2)
        && amount == amount2
    {
        return Ter::TEM_BAD_AMM_TOKENS;
    }

    if facts.lp_token_out_signum.is_some_and(|signum| signum <= 0) {
        return Ter::TEM_BAD_AMM_TOKENS;
    }

    if let Some(err) = facts.amount_invalid {
        return err;
    }
    if let Some(err) = facts.amount2_invalid {
        return err;
    }
    if let Some(err) = facts.e_price_invalid {
        return err;
    }

    if facts
        .trading_fee
        .is_some_and(|fee| fee > protocol::TRADING_FEE_THRESHOLD)
    {
        return Ter::TEM_BAD_FEE;
    }

    Ter::TES_SUCCESS
}

pub fn run_amm_deposit_preflight<Registry, Tx, Journal, ParentBatchId>(
    ctx: &PreflightContext<Registry, Tx, Journal, ParentBatchId>,
    flags: u32,
) -> NotTec {
    let ret = run_transactor_preflight1(
        TransactorPreflight1Facts {
            inner_batch_flag_set: (ctx.flags.bits() & crate::ApplyFlags::BATCH.bits()) != 0,
            batch_enabled: ctx.rules.enabled(&feature_batch()),
            ..Default::default()
        },
        || {
            run_transactor_preflight0(
                TransactorPreflight0Facts {
                    tx_flags: flags,
                    ..Default::default()
                },
                0,
            )
        },
        || Ter::TES_SUCCESS,
    );
    if !is_tes_success(ret) {
        return ret;
    }
    Ter::TES_SUCCESS
}

pub fn run_amm_deposit_preclaim<Registry, View, Tx, Journal, ParentBatchId>(
    _ctx: &PreclaimContext<Registry, View, Tx, Journal, ParentBatchId>,
) -> Ter {
    Ter::TES_SUCCESS
}

#[derive(Debug, Clone, Copy)]
pub struct AMMDepositPreclaimFacts {
    pub amm_exists: bool,
    pub amm_holds_result: Ter,
    pub two_asset_if_empty: bool,
    pub amount_balance_signum: i32,
    pub amount2_balance_signum: i32,
    pub lp_token_balance_signum: i32,
    pub amm_clawback_enabled: bool,
    pub asset_auth_result: Ter,
    pub asset_frozen_result: Ter,
    pub asset2_auth_result: Ter,
    pub asset2_frozen_result: Ter,
    pub lp_token_mode: bool,
    pub amount_check_result: Ter,
    pub amount2_check_result: Ter,
    pub pool_amount_check_result: Ter,
    pub pool_amount2_check_result: Ter,
    pub lp_token_out_asset_matches_lpt: Option<bool>,
    pub account_lp_holds_signum: i32,
    pub xrp_reserve_positive: bool,
    pub asset_mpt_trade_transfer_result: Ter,
    pub asset2_mpt_trade_transfer_result: Ter,
}

impl Default for AMMDepositPreclaimFacts {
    fn default() -> Self {
        Self {
            amm_exists: true,
            amm_holds_result: Ter::TES_SUCCESS,
            two_asset_if_empty: false,
            amount_balance_signum: 1,
            amount2_balance_signum: 1,
            lp_token_balance_signum: 1,
            amm_clawback_enabled: false,
            asset_auth_result: Ter::TES_SUCCESS,
            asset_frozen_result: Ter::TES_SUCCESS,
            asset2_auth_result: Ter::TES_SUCCESS,
            asset2_frozen_result: Ter::TES_SUCCESS,
            lp_token_mode: false,
            amount_check_result: Ter::TES_SUCCESS,
            amount2_check_result: Ter::TES_SUCCESS,
            pool_amount_check_result: Ter::TES_SUCCESS,
            pool_amount2_check_result: Ter::TES_SUCCESS,
            lp_token_out_asset_matches_lpt: None,
            account_lp_holds_signum: 1,
            xrp_reserve_positive: true,
            asset_mpt_trade_transfer_result: Ter::TES_SUCCESS,
            asset2_mpt_trade_transfer_result: Ter::TES_SUCCESS,
        }
    }
}

pub fn run_amm_deposit_preclaim_facts(facts: AMMDepositPreclaimFacts) -> Ter {
    if !facts.amm_exists {
        return Ter::TER_NO_AMM;
    }

    if facts.amm_holds_result != Ter::TES_SUCCESS {
        return facts.amm_holds_result;
    }

    if facts.two_asset_if_empty {
        if facts.lp_token_balance_signum != 0 {
            return Ter::TEC_AMM_NOT_EMPTY;
        }
        if facts.amount_balance_signum != 0 || facts.amount2_balance_signum != 0 {
            return Ter::TEC_INTERNAL;
        }
    } else {
        if facts.lp_token_balance_signum == 0 {
            return Ter::TEC_AMM_EMPTY;
        }
        if facts.amount_balance_signum <= 0
            || facts.amount2_balance_signum <= 0
            || facts.lp_token_balance_signum < 0
        {
            return Ter::TEC_INTERNAL;
        }
    }

    if facts.amm_clawback_enabled {
        if facts.asset_auth_result != Ter::TES_SUCCESS {
            return facts.asset_auth_result;
        }
        if facts.asset_frozen_result != Ter::TES_SUCCESS {
            return facts.asset_frozen_result;
        }
        if facts.asset2_auth_result != Ter::TES_SUCCESS {
            return facts.asset2_auth_result;
        }
        if facts.asset2_frozen_result != Ter::TES_SUCCESS {
            return facts.asset2_frozen_result;
        }
    }

    if !facts.lp_token_mode {
        if facts.amount_check_result != Ter::TES_SUCCESS {
            return facts.amount_check_result;
        }
        if facts.amount2_check_result != Ter::TES_SUCCESS {
            return facts.amount2_check_result;
        }
    } else {
        if facts.pool_amount_check_result != Ter::TES_SUCCESS {
            return facts.pool_amount_check_result;
        }
        if facts.pool_amount2_check_result != Ter::TES_SUCCESS {
            return facts.pool_amount2_check_result;
        }
    }

    if facts
        .lp_token_out_asset_matches_lpt
        .is_some_and(|matches| !matches)
    {
        return Ter::TEM_BAD_AMM_TOKENS;
    }

    if facts.account_lp_holds_signum == 0 && !facts.xrp_reserve_positive {
        return Ter::TEC_INSUF_RESERVE_LINE;
    }

    if facts.asset_mpt_trade_transfer_result != Ter::TES_SUCCESS {
        return facts.asset_mpt_trade_transfer_result;
    }
    if facts.asset2_mpt_trade_transfer_result != Ter::TES_SUCCESS {
        return facts.asset2_mpt_trade_transfer_result;
    }

    Ter::TES_SUCCESS
}

pub struct AMMDepositApplyFacts {
    pub account: protocol::AccountID,
    pub asset1: protocol::Asset,
    pub asset2: protocol::Asset,
    pub amount1: Option<protocol::STAmount>,
    pub amount2: Option<protocol::STAmount>,
    pub e_price: Option<protocol::STAmount>,
    pub lp_token_out: Option<protocol::STAmount>,
    pub pool_amount1: protocol::STAmount,
    pub pool_amount2: protocol::STAmount,
    pub lp_token_balance: protocol::STAmount,
    pub trading_fee: u16,
    pub rules: protocol::Rules,
    pub flags: u32,
}

#[derive(Debug, Clone)]
pub struct AMMDepositApplyMathFacts {
    pub amount1: Option<protocol::STAmount>,
    pub amount2: Option<protocol::STAmount>,
    pub e_price: Option<protocol::STAmount>,
    pub lp_token_out: Option<protocol::STAmount>,
    pub pool_amount1: protocol::STAmount,
    pub pool_amount2: protocol::STAmount,
    pub lp_token_balance: protocol::STAmount,
    pub trading_fee: u16,
    pub rules: protocol::Rules,
    pub flags: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AMMDepositApplyMathResult {
    pub amount1: Option<protocol::STAmount>,
    pub amount2: Option<protocol::STAmount>,
    pub lp_tokens: protocol::STAmount,
    pub new_lp_token_balance: protocol::STAmount,
    pub empty_pool_reinit: bool,
}

fn number_from_i64(value: i64) -> RuntimeNumber {
    RuntimeNumber::try_from_external_parts(value, 0, basics::number::get_mantissa_scale())
        .expect("small integer should stay representable in Number")
}

fn solve_quadratic_eq(a: RuntimeNumber, b: RuntimeNumber, c: RuntimeNumber) -> RuntimeNumber {
    (-b + root2(b * b - number_from_i64(4) * a * c).expect("discriminant should be nonnegative"))
        / (number_from_i64(2) * a)
}

fn adjusted_lp_tokens_out(
    rules: &protocol::Rules,
    lp_token_balance: &protocol::STAmount,
    lp_tokens: &protocol::STAmount,
) -> protocol::STAmount {
    if !rules.enabled(&fix_ammv1_3()) {
        return lp_tokens.clone();
    }
    amm_helpers::adjust_lp_tokens(lp_token_balance, lp_tokens, IsDeposit::Yes)
}

fn finalize_deposit_math(
    facts: &AMMDepositApplyMathFacts,
    amount1: protocol::STAmount,
    amount2: Option<protocol::STAmount>,
    lp_tokens: protocol::STAmount,
    amount1_min: Option<&protocol::STAmount>,
    amount2_min: Option<&protocol::STAmount>,
    lp_tokens_min: Option<&protocol::STAmount>,
    empty_pool_reinit: bool,
) -> Result<AMMDepositApplyMathResult, Ter> {
    let (amount1_actual, amount2_actual, lp_tokens_actual) =
        amm_helpers::adjust_amounts_by_lp_tokens(
            &facts.pool_amount1,
            &amount1,
            amount2.as_ref(),
            &facts.lp_token_balance,
            &lp_tokens,
            facts.trading_fee,
            IsDeposit::Yes,
        );

    if lp_tokens_actual.signum() <= 0 {
        return Err(Ter::TEC_AMM_INVALID_TOKENS);
    }

    if amount1_min.is_some_and(|min| amount1_actual < *min)
        || match (&amount2_actual, amount2_min) {
            (Some(actual), Some(min)) => actual < min,
            (None, Some(_)) => true,
            _ => false,
        }
        || lp_tokens_min.is_some_and(|min| lp_tokens_actual < *min)
    {
        return Err(Ter::TEC_AMM_FAILED);
    }

    Ok(AMMDepositApplyMathResult {
        amount1: Some(amount1_actual),
        amount2: amount2_actual,
        lp_tokens: lp_tokens_actual.clone(),
        new_lp_token_balance: facts.lp_token_balance.clone() + lp_tokens_actual,
        empty_pool_reinit,
    })
}

pub fn run_amm_deposit_apply_math_facts(
    facts: &AMMDepositApplyMathFacts,
) -> Result<AMMDepositApplyMathResult, Ter> {
    let sub_tx_flags = facts.flags & protocol::DEPOSIT_SUB_TX_FLAGS;

    if (sub_tx_flags & protocol::AMM_LP_TOKEN_FLAG) != 0 {
        let Some(lp_token_out) = &facts.lp_token_out else {
            return Err(Ter::TEM_MALFORMED);
        };
        let lp_tokens = adjusted_lp_tokens_out(&facts.rules, &facts.lp_token_balance, lp_token_out);
        if facts.rules.enabled(&fix_ammv1_3()) && lp_tokens.signum() == 0 {
            return Err(Ter::TEC_AMM_INVALID_TOKENS);
        }
        let frac = amm_helpers::stamount_as_number(&lp_tokens)
            / amm_helpers::stamount_as_number(&facts.lp_token_balance);
        let amount1 =
            amm_helpers::get_rounded_asset(&facts.rules, &facts.pool_amount1, frac, IsDeposit::Yes);
        let amount2 =
            amm_helpers::get_rounded_asset(&facts.rules, &facts.pool_amount2, frac, IsDeposit::Yes);
        return finalize_deposit_math(
            facts,
            amount1,
            Some(amount2),
            lp_tokens,
            facts.amount1.as_ref(),
            facts.amount2.as_ref(),
            None,
            false,
        );
    }

    if (sub_tx_flags & protocol::AMM_TWO_ASSET_IF_EMPTY_FLAG) != 0 {
        let (Some(amount1), Some(amount2)) = (&facts.amount1, &facts.amount2) else {
            return Err(Ter::TEM_MALFORMED);
        };
        if facts.lp_token_balance.signum() != 0 {
            return Err(Ter::TEC_AMM_FAILED);
        }

        let lp_minted =
            amm_helpers::amm_lp_tokens(amount1, amount2, facts.lp_token_balance.issue());
        if lp_minted.signum() == 0 {
            return Err(Ter::TEC_AMM_INVALID_TOKENS);
        }

        return finalize_deposit_math(
            facts,
            amount1.clone(),
            Some(amount2.clone()),
            lp_minted,
            None,
            None,
            None,
            true,
        );
    }

    if (sub_tx_flags & protocol::AMM_TWO_ASSET_FLAG) != 0 {
        let (Some(amount1), Some(amount2)) = (&facts.amount1, &facts.amount2) else {
            return Err(Ter::TEM_MALFORMED);
        };
        if facts.pool_amount1.signum() <= 0
            || facts.pool_amount2.signum() <= 0
            || facts.lp_token_balance.signum() <= 0
        {
            return Err(Ter::TEC_AMM_FAILED);
        }

        let mut frac = amm_helpers::stamount_as_number(amount1)
            / amm_helpers::stamount_as_number(&facts.pool_amount1);
        let mut lp_minted = amm_helpers::get_rounded_lp_tokens(
            &facts.rules,
            &facts.lp_token_balance,
            frac,
            IsDeposit::Yes,
        );
        if lp_minted.signum() == 0 {
            if !facts.rules.enabled(&fix_ammv1_3()) {
                return Err(Ter::TEC_AMM_FAILED);
            }
            return Err(Ter::TEC_AMM_INVALID_TOKENS);
        }
        if facts.rules.enabled(&fix_ammv1_3()) {
            frac = amm_helpers::stamount_as_number(&lp_minted)
                / amm_helpers::stamount_as_number(&facts.lp_token_balance);
        }
        let amount2_deposit =
            amm_helpers::get_rounded_asset(&facts.rules, &facts.pool_amount2, frac, IsDeposit::Yes);
        if amount2_deposit <= *amount2 {
            return finalize_deposit_math(
                facts,
                amount1.clone(),
                Some(amount2_deposit),
                lp_minted,
                None,
                None,
                facts.lp_token_out.as_ref(),
                false,
            );
        }

        frac = amm_helpers::stamount_as_number(amount2)
            / amm_helpers::stamount_as_number(&facts.pool_amount2);
        lp_minted = amm_helpers::get_rounded_lp_tokens(
            &facts.rules,
            &facts.lp_token_balance,
            frac,
            IsDeposit::Yes,
        );
        if lp_minted.signum() == 0 {
            if !facts.rules.enabled(&fix_ammv1_3()) {
                return Err(Ter::TEC_AMM_FAILED);
            }
            return Err(Ter::TEC_AMM_INVALID_TOKENS);
        }
        if facts.rules.enabled(&fix_ammv1_3()) {
            frac = amm_helpers::stamount_as_number(&lp_minted)
                / amm_helpers::stamount_as_number(&facts.lp_token_balance);
        }
        let amount1_deposit =
            amm_helpers::get_rounded_asset(&facts.rules, &facts.pool_amount1, frac, IsDeposit::Yes);
        if amount1_deposit <= *amount1 {
            return finalize_deposit_math(
                facts,
                amount1_deposit,
                Some(amount2.clone()),
                lp_minted,
                None,
                None,
                facts.lp_token_out.as_ref(),
                false,
            );
        }
        return Err(Ter::TEC_AMM_FAILED);
    }

    if (sub_tx_flags & protocol::AMM_ONE_ASSET_LP_TOKEN_FLAG) != 0 {
        let (Some(amount1), Some(lp_token_out)) = (&facts.amount1, &facts.lp_token_out) else {
            return Err(Ter::TEM_MALFORMED);
        };
        let lp_tokens = adjusted_lp_tokens_out(&facts.rules, &facts.lp_token_balance, lp_token_out);
        if facts.rules.enabled(&fix_ammv1_3()) && lp_tokens.signum() == 0 {
            return Err(Ter::TEC_AMM_INVALID_TOKENS);
        }
        let amount1_deposit = amm_helpers::amm_asset_in(
            &facts.pool_amount1,
            &facts.lp_token_balance,
            &lp_tokens,
            facts.trading_fee,
        );
        if amount1_deposit > *amount1 {
            return Err(Ter::TEC_AMM_FAILED);
        }
        return finalize_deposit_math(
            facts,
            amount1_deposit,
            None,
            lp_tokens,
            None,
            None,
            None,
            false,
        );
    }

    if (sub_tx_flags & protocol::AMM_LIMIT_LP_TOKEN_FLAG) != 0 {
        let (Some(amount1), Some(e_price)) = (&facts.amount1, &facts.e_price) else {
            return Err(Ter::TEM_MALFORMED);
        };
        if amount1.signum() != 0 {
            let tokens = adjusted_lp_tokens_out(
                &facts.rules,
                &facts.lp_token_balance,
                &amm_helpers::lp_tokens_out(
                    &facts.pool_amount1,
                    amount1,
                    &facts.lp_token_balance,
                    facts.trading_fee,
                ),
            );
            if tokens.signum() <= 0 {
                if !facts.rules.enabled(&fix_ammv1_3()) {
                    return Err(Ter::TEC_AMM_FAILED);
                }
                return Err(Ter::TEC_AMM_INVALID_TOKENS);
            }
            let (tokens_adj, amount1_deposit) = amm_helpers::adjust_asset_in_by_tokens(
                &facts.rules,
                &facts.pool_amount1,
                amount1,
                &facts.lp_token_balance,
                &tokens,
                facts.trading_fee,
            );
            if facts.rules.enabled(&fix_ammv1_3()) && tokens_adj.signum() == 0 {
                return Err(Ter::TEC_AMM_INVALID_TOKENS);
            }
            let effective_price = amm_helpers::stamount_as_number(&amount1_deposit)
                / amm_helpers::stamount_as_number(&tokens_adj);
            if effective_price <= amm_helpers::stamount_as_number(e_price) {
                return finalize_deposit_math(
                    facts,
                    amount1_deposit,
                    None,
                    tokens_adj,
                    None,
                    None,
                    None,
                    false,
                );
            }
        }

        let f1 = protocol::fee_mult(facts.trading_fee);
        let f2 = protocol::fee_mult_half(facts.trading_fee) / f1;
        let amount_balance = amm_helpers::stamount_as_number(&facts.pool_amount1);
        let e_price_number = amm_helpers::stamount_as_number(e_price);
        let lp_balance = amm_helpers::stamount_as_number(&facts.lp_token_balance);
        let c = f1 * amount_balance / (e_price_number * lp_balance);
        let d = f1 + c * f2 - c;
        let a1 = c * c;
        let b1 = c * c * f2 * f2 + number_from_i64(2) * c - d * d;
        let c1 =
            number_from_i64(2) * c * f2 * f2 + number_from_i64(1) - number_from_i64(2) * d * f2;
        let solved = solve_quadratic_eq(a1, b1, c1);
        let amount1_deposit = amm_helpers::get_rounded_asset_with_product(
            &facts.rules,
            || f1 * amount_balance * solved,
            &facts.pool_amount1,
            || f1 * solved,
            IsDeposit::Yes,
        );
        if amount1_deposit.signum() <= 0 {
            return Err(Ter::TEC_AMM_FAILED);
        }
        let tokens = amm_helpers::get_rounded_lp_tokens_with_product(
            &facts.rules,
            || amm_helpers::stamount_as_number(&amount1_deposit) / e_price_number,
            &facts.lp_token_balance,
            || amm_helpers::stamount_as_number(&amount1_deposit) / e_price_number,
            IsDeposit::Yes,
        );
        let (tokens_adj, amount1_deposit_adj) = amm_helpers::adjust_asset_in_by_tokens(
            &facts.rules,
            &facts.pool_amount1,
            &amount1_deposit,
            &facts.lp_token_balance,
            &tokens,
            facts.trading_fee,
        );
        if facts.rules.enabled(&fix_ammv1_3()) && tokens_adj.signum() == 0 {
            return Err(Ter::TEC_AMM_INVALID_TOKENS);
        }
        return finalize_deposit_math(
            facts,
            amount1_deposit_adj,
            None,
            tokens_adj,
            None,
            None,
            None,
            false,
        );
    }

    if (sub_tx_flags & protocol::AMM_SINGLE_ASSET_FLAG) != 0 {
        let Some(amount1) = &facts.amount1 else {
            return Err(Ter::TEM_MALFORMED);
        };
        let lp_adjusted = adjusted_lp_tokens_out(
            &facts.rules,
            &facts.lp_token_balance,
            &amm_helpers::lp_tokens_out(
                &facts.pool_amount1,
                amount1,
                &facts.lp_token_balance,
                facts.trading_fee,
            ),
        );
        if lp_adjusted.signum() == 0 {
            if !facts.rules.enabled(&fix_ammv1_3()) {
                return Err(Ter::TEC_AMM_FAILED);
            }
            return Err(Ter::TEC_AMM_INVALID_TOKENS);
        }
        let (lp_minted, amount1_actual) = amm_helpers::adjust_asset_in_by_tokens(
            &facts.rules,
            &facts.pool_amount1,
            amount1,
            &facts.lp_token_balance,
            &lp_adjusted,
            facts.trading_fee,
        );
        if lp_minted.signum() == 0 {
            return Err(Ter::TEC_AMM_INVALID_TOKENS);
        }

        let empty_pool_reinit = facts.lp_token_balance.signum() == 0;
        return finalize_deposit_math(
            facts,
            amount1_actual,
            None,
            lp_minted,
            None,
            None,
            facts.lp_token_out.as_ref(),
            empty_pool_reinit,
        );
    }

    Err(Ter::TEM_MALFORMED)
}

pub trait AMMDepositApplySink {
    fn get_amm_entry(
        &mut self,
        asset1: &protocol::Asset,
        asset2: &protocol::Asset,
    ) -> Option<protocol::STLedgerEntry>;
    fn update_amm_entry(&mut self, sle: protocol::STLedgerEntry);
    fn deposit_asset(&mut self, account: &protocol::AccountID, amount: &protocol::STAmount) -> Ter;
    fn mint_lp_tokens(&mut self, account: &protocol::AccountID, amount: &protocol::STAmount)
    -> Ter;
}

pub fn run_amm_deposit_do_apply<S: AMMDepositApplySink>(
    facts: AMMDepositApplyFacts,
    sink: &mut S,
) -> Ter {
    let Some(amm_sle) = sink.get_amm_entry(&facts.asset1, &facts.asset2) else {
        return Ter::TER_NO_AMM;
    };

    let math = match run_amm_deposit_apply_math_facts(&AMMDepositApplyMathFacts {
        amount1: facts.amount1,
        amount2: facts.amount2,
        e_price: facts.e_price,
        lp_token_out: facts.lp_token_out,
        pool_amount1: facts.pool_amount1,
        pool_amount2: facts.pool_amount2,
        lp_token_balance: facts.lp_token_balance,
        trading_fee: facts.trading_fee,
        rules: facts.rules,
        flags: facts.flags,
    }) {
        Ok(math) => math,
        Err(ter) => return ter,
    };

    if let Some(amt1) = &math.amount1 {
        let res = sink.deposit_asset(&facts.account, amt1);
        if !is_tes_success(res) {
            return res;
        }
    }

    if let Some(amt2) = &math.amount2 {
        let res = sink.deposit_asset(&facts.account, amt2);
        if !is_tes_success(res) {
            return res;
        }
    }

    let res = sink.mint_lp_tokens(&facts.account, &math.lp_tokens);
    if !is_tes_success(res) {
        return res;
    }

    let mut obj = amm_sle.clone_as_object();
    obj.set_field_amount(
        get_field_by_symbol("sfLPTokenBalance"),
        math.new_lp_token_balance,
    );
    sink.update_amm_entry(protocol::STLedgerEntry::from_stobject(obj, *amm_sle.key()));

    Ter::TES_SUCCESS
}

pub fn run_amm_deposit_apply<Registry, BaseView, View, Tx, Fee, Journal, ParentBatchId>(
    _ctx: &mut ApplyContext<Registry, BaseView, View, Tx, Fee, Journal, ParentBatchId>,
) -> ApplyResult {
    ApplyResult::new(Ter::TES_SUCCESS, true, false)
}
