//! `AmmWithdraw` transactor port.

use crate::{
    ApplyContext, ApplyResult, PreclaimContext, PreflightContext, TransactorPreflight0Facts,
    TransactorPreflight1Facts, run_transactor_preflight0, run_transactor_preflight1,
};
use basics::number::NumberParts as RuntimeNumber;
use ledger::amm_helpers::{self, IsDeposit};
use protocol::get_field_by_symbol;
use protocol::{NotTec, Ter, feature_batch, is_tes_success};

pub const fn amm_withdraw_check_extra_features(
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
pub struct AMMWithdrawPreflightFacts {
    pub flags: u32,
    pub asset_pair_invalid: Option<NotTec>,
    pub amount: Option<protocol::Asset>,
    pub amount_invalid: Option<NotTec>,
    pub amount2: Option<protocol::Asset>,
    pub amount2_invalid: Option<NotTec>,
    pub e_price: Option<protocol::Asset>,
    pub e_price_invalid: Option<NotTec>,
    pub lp_token_in_signum: Option<i32>,
}

pub fn run_amm_withdraw_preflight_facts(facts: AMMWithdrawPreflightFacts) -> NotTec {
    let has_amount = facts.amount.is_some();
    let has_amount2 = facts.amount2.is_some();
    let has_e_price = facts.e_price.is_some();
    let has_lp_tokens = facts.lp_token_in_signum.is_some();
    let sub_tx_flags = facts.flags & protocol::WITHDRAW_SUB_TX_FLAGS;

    if sub_tx_flags.count_ones() != 1 {
        return Ter::TEM_MALFORMED;
    }

    if (sub_tx_flags & protocol::AMM_LP_TOKEN_FLAG) != 0 {
        if !has_lp_tokens || has_amount || has_amount2 || has_e_price {
            return Ter::TEM_MALFORMED;
        }
    } else if (sub_tx_flags & protocol::AMM_WITHDRAW_ALL_FLAG) != 0 {
        if has_lp_tokens || has_amount || has_amount2 || has_e_price {
            return Ter::TEM_MALFORMED;
        }
    } else if (sub_tx_flags & protocol::AMM_ONE_ASSET_WITHDRAW_ALL_FLAG) != 0 {
        if !has_amount || has_lp_tokens || has_amount2 || has_e_price {
            return Ter::TEM_MALFORMED;
        }
    } else if (sub_tx_flags & protocol::AMM_SINGLE_ASSET_FLAG) != 0 {
        if !has_amount || has_lp_tokens || has_amount2 || has_e_price {
            return Ter::TEM_MALFORMED;
        }
    } else if (sub_tx_flags & protocol::AMM_TWO_ASSET_FLAG) != 0 {
        if !has_amount || !has_amount2 || has_lp_tokens || has_e_price {
            return Ter::TEM_MALFORMED;
        }
    } else if (sub_tx_flags & protocol::AMM_ONE_ASSET_LP_TOKEN_FLAG) != 0 {
        if !has_amount || !has_lp_tokens || has_amount2 || has_e_price {
            return Ter::TEM_MALFORMED;
        }
    } else if (sub_tx_flags & protocol::AMM_LIMIT_LP_TOKEN_FLAG) != 0
        && (!has_amount || !has_e_price || has_lp_tokens || has_amount2)
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

    if facts.lp_token_in_signum.is_some_and(|signum| signum <= 0) {
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

    Ter::TES_SUCCESS
}

pub fn run_amm_withdraw_preflight<Registry, Tx, Journal, ParentBatchId>(
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

pub fn run_amm_withdraw_preclaim<Registry, View, Tx, Journal, ParentBatchId>(
    _ctx: &PreclaimContext<Registry, View, Tx, Journal, ParentBatchId>,
) -> Ter {
    Ter::TES_SUCCESS
}

#[derive(Debug, Clone, Copy)]
pub struct AMMWithdrawPreclaimFacts {
    pub amm_exists: bool,
    pub amm_holds_result: Ter,
    pub amount_balance_signum: i32,
    pub amount2_balance_signum: i32,
    pub lp_token_balance_signum: i32,
    pub amount_check_result: Ter,
    pub amount2_check_result: Ter,
    pub account_lp_tokens_signum: i32,
    pub lp_tokens_withdraw_asset_matches_lp: Option<bool>,
    pub lp_tokens_withdraw_exceeds_balance: bool,
    pub e_price_asset_matches_lp: Option<bool>,
    pub lp_token_or_withdraw_all_mode: bool,
    pub pool_amount_check_result: Ter,
    pub pool_amount2_check_result: Ter,
}

impl Default for AMMWithdrawPreclaimFacts {
    fn default() -> Self {
        Self {
            amm_exists: true,
            amm_holds_result: Ter::TES_SUCCESS,
            amount_balance_signum: 1,
            amount2_balance_signum: 1,
            lp_token_balance_signum: 1,
            amount_check_result: Ter::TES_SUCCESS,
            amount2_check_result: Ter::TES_SUCCESS,
            account_lp_tokens_signum: 1,
            lp_tokens_withdraw_asset_matches_lp: None,
            lp_tokens_withdraw_exceeds_balance: false,
            e_price_asset_matches_lp: None,
            lp_token_or_withdraw_all_mode: false,
            pool_amount_check_result: Ter::TES_SUCCESS,
            pool_amount2_check_result: Ter::TES_SUCCESS,
        }
    }
}

pub fn run_amm_withdraw_preclaim_facts(facts: AMMWithdrawPreclaimFacts) -> Ter {
    if !facts.amm_exists {
        return Ter::TER_NO_AMM;
    }

    if facts.amm_holds_result != Ter::TES_SUCCESS {
        return facts.amm_holds_result;
    }

    if facts.lp_token_balance_signum == 0 {
        return Ter::TEC_AMM_EMPTY;
    }

    if facts.amount_balance_signum <= 0
        || facts.amount2_balance_signum <= 0
        || facts.lp_token_balance_signum < 0
    {
        return Ter::TEC_INTERNAL;
    }

    if facts.amount_check_result != Ter::TES_SUCCESS {
        return facts.amount_check_result;
    }

    if facts.amount2_check_result != Ter::TES_SUCCESS {
        return facts.amount2_check_result;
    }

    if facts.account_lp_tokens_signum <= 0 {
        return Ter::TEC_AMM_BALANCE;
    }

    if facts
        .lp_tokens_withdraw_asset_matches_lp
        .is_some_and(|matches| !matches)
    {
        return Ter::TEM_BAD_AMM_TOKENS;
    }

    if facts.lp_tokens_withdraw_exceeds_balance {
        return Ter::TEC_AMM_INVALID_TOKENS;
    }

    if facts
        .e_price_asset_matches_lp
        .is_some_and(|matches| !matches)
    {
        return Ter::TEM_BAD_AMM_TOKENS;
    }

    if facts.lp_token_or_withdraw_all_mode {
        if facts.pool_amount_check_result != Ter::TES_SUCCESS {
            return facts.pool_amount_check_result;
        }
        if facts.pool_amount2_check_result != Ter::TES_SUCCESS {
            return facts.pool_amount2_check_result;
        }
    }

    Ter::TES_SUCCESS
}

pub struct AMMWithdrawApplyFacts {
    pub account: protocol::AccountID,
    pub asset1: protocol::Asset,
    pub asset2: protocol::Asset,
    pub amount1: Option<protocol::STAmount>,
    pub amount2: Option<protocol::STAmount>,
    pub e_price: Option<protocol::STAmount>,
    pub lp_token_in: Option<protocol::STAmount>,
    pub pool_amount1: protocol::STAmount,
    pub pool_amount2: protocol::STAmount,
    pub lp_token_balance: protocol::STAmount,
    pub account_lp_tokens: protocol::STAmount,
    pub trading_fee: u16,
    pub rules: protocol::Rules,
    pub flags: u32,
}

#[derive(Debug, Clone)]
pub struct AMMWithdrawApplyMathFacts {
    pub amount1: Option<protocol::STAmount>,
    pub amount2: Option<protocol::STAmount>,
    pub e_price: Option<protocol::STAmount>,
    pub lp_token_in: Option<protocol::STAmount>,
    pub pool_amount1: protocol::STAmount,
    pub pool_amount2: protocol::STAmount,
    pub lp_token_balance: protocol::STAmount,
    pub account_lp_tokens: protocol::STAmount,
    pub trading_fee: u16,
    pub rules: protocol::Rules,
    pub flags: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AMMWithdrawApplyMathResult {
    pub amount1: Option<protocol::STAmount>,
    pub amount2: Option<protocol::STAmount>,
    pub lp_tokens: protocol::STAmount,
    pub new_lp_token_balance: protocol::STAmount,
}

fn number_from_i64(value: i64) -> RuntimeNumber {
    RuntimeNumber::try_from_external_parts(value, 0, basics::number::get_mantissa_scale())
        .expect("small integer should stay representable in Number")
}

fn withdraw_all_mode(flags: u32) -> bool {
    (flags & (protocol::AMM_WITHDRAW_ALL_FLAG | protocol::AMM_ONE_ASSET_WITHDRAW_ALL_FLAG)) != 0
}

fn adjusted_lp_tokens_in(
    rules: &protocol::Rules,
    lp_token_balance: &protocol::STAmount,
    lp_tokens: &protocol::STAmount,
    withdraw_all: bool,
) -> protocol::STAmount {
    if !rules.enabled(&protocol::fix_ammv1_3()) || withdraw_all {
        return lp_tokens.clone();
    }
    amm_helpers::adjust_lp_tokens(lp_token_balance, lp_tokens, IsDeposit::No)
}

fn finalize_withdraw_math(
    facts: &AMMWithdrawApplyMathFacts,
    amount1: protocol::STAmount,
    amount2: Option<protocol::STAmount>,
    lp_tokens: protocol::STAmount,
    withdraw_all: bool,
) -> Result<AMMWithdrawApplyMathResult, Ter> {
    let (amount1_actual, amount2_actual, lp_tokens_actual) = if withdraw_all {
        (amount1, amount2, lp_tokens)
    } else {
        amm_helpers::adjust_amounts_by_lp_tokens(
            &facts.pool_amount1,
            &amount1,
            amount2.as_ref(),
            &facts.lp_token_balance,
            &lp_tokens,
            facts.trading_fee,
            IsDeposit::No,
        )
    };

    if lp_tokens_actual.signum() <= 0 || lp_tokens_actual > facts.account_lp_tokens {
        return Err(Ter::TEC_AMM_INVALID_TOKENS);
    }

    if (amount1_actual == facts.pool_amount1
        && amount2_actual.as_ref() != Some(&facts.pool_amount2))
        || (amount2_actual.as_ref() == Some(&facts.pool_amount2)
            && amount1_actual != facts.pool_amount1)
    {
        return Err(Ter::TEC_AMM_BALANCE);
    }

    if lp_tokens_actual == facts.lp_token_balance
        && (amount1_actual != facts.pool_amount1
            || amount2_actual
                .as_ref()
                .is_some_and(|amount2| *amount2 != facts.pool_amount2))
    {
        return Err(Ter::TEC_AMM_BALANCE);
    }

    if amount1_actual > facts.pool_amount1
        || amount2_actual
            .as_ref()
            .is_some_and(|amount2| *amount2 > facts.pool_amount2)
    {
        return Err(Ter::TEC_AMM_BALANCE);
    }

    Ok(AMMWithdrawApplyMathResult {
        amount1: Some(amount1_actual),
        amount2: amount2_actual,
        lp_tokens: lp_tokens_actual.clone(),
        new_lp_token_balance: facts.lp_token_balance.clone() - lp_tokens_actual,
    })
}

pub fn run_amm_withdraw_apply_math_facts(
    facts: &AMMWithdrawApplyMathFacts,
) -> Result<AMMWithdrawApplyMathResult, Ter> {
    let sub_tx_flags = facts.flags & protocol::WITHDRAW_SUB_TX_FLAGS;
    let is_withdraw_all = withdraw_all_mode(facts.flags);
    let lp_tokens_withdraw = if is_withdraw_all {
        Some(facts.account_lp_tokens.clone())
    } else {
        facts.lp_token_in.clone()
    };

    if (sub_tx_flags & (protocol::AMM_LP_TOKEN_FLAG | protocol::AMM_WITHDRAW_ALL_FLAG)) != 0 {
        let Some(lp_tokens_in) = lp_tokens_withdraw else {
            return Err(Ter::TEM_MALFORMED);
        };
        if lp_tokens_in == facts.lp_token_balance {
            return finalize_withdraw_math(
                facts,
                facts.pool_amount1.clone(),
                Some(facts.pool_amount2.clone()),
                lp_tokens_in,
                true,
            );
        }
        let tokens = adjusted_lp_tokens_in(
            &facts.rules,
            &facts.lp_token_balance,
            &lp_tokens_in,
            is_withdraw_all,
        );
        if facts.rules.enabled(&protocol::fix_ammv1_3()) && tokens.signum() == 0 {
            return Err(Ter::TEC_AMM_INVALID_TOKENS);
        }
        let frac = amm_helpers::stamount_as_number(&tokens)
            / amm_helpers::stamount_as_number(&facts.lp_token_balance);
        let amount1 =
            amm_helpers::get_rounded_asset(&facts.rules, &facts.pool_amount1, frac, IsDeposit::No);
        let amount2 =
            amm_helpers::get_rounded_asset(&facts.rules, &facts.pool_amount2, frac, IsDeposit::No);
        if amount1.signum() == 0 || amount2.signum() == 0 {
            return Err(Ter::TEC_AMM_FAILED);
        }
        return finalize_withdraw_math(facts, amount1, Some(amount2), tokens, is_withdraw_all);
    }

    if (sub_tx_flags & protocol::AMM_TWO_ASSET_FLAG) != 0 {
        let (Some(amount1), Some(amount2)) = (&facts.amount1, &facts.amount2) else {
            return Err(Ter::TEM_MALFORMED);
        };
        let mut frac = amm_helpers::stamount_as_number(amount1)
            / amm_helpers::stamount_as_number(&facts.pool_amount1);
        let mut tokens = amm_helpers::get_rounded_lp_tokens(
            &facts.rules,
            &facts.lp_token_balance,
            frac,
            IsDeposit::No,
        );
        if facts.rules.enabled(&protocol::fix_ammv1_3()) && tokens.signum() == 0 {
            return Err(Ter::TEC_AMM_INVALID_TOKENS);
        }
        if facts.rules.enabled(&protocol::fix_ammv1_3()) {
            frac = amm_helpers::stamount_as_number(&tokens)
                / amm_helpers::stamount_as_number(&facts.lp_token_balance);
        }
        let amount2_withdraw =
            amm_helpers::get_rounded_asset(&facts.rules, &facts.pool_amount2, frac, IsDeposit::No);
        if amount2_withdraw <= *amount2 {
            return finalize_withdraw_math(
                facts,
                amount1.clone(),
                Some(amount2_withdraw),
                tokens,
                false,
            );
        }

        frac = amm_helpers::stamount_as_number(amount2)
            / amm_helpers::stamount_as_number(&facts.pool_amount2);
        tokens = amm_helpers::get_rounded_lp_tokens(
            &facts.rules,
            &facts.lp_token_balance,
            frac,
            IsDeposit::No,
        );
        if facts.rules.enabled(&protocol::fix_ammv1_3()) && tokens.signum() == 0 {
            return Err(Ter::TEC_AMM_INVALID_TOKENS);
        }
        if facts.rules.enabled(&protocol::fix_ammv1_3()) {
            frac = amm_helpers::stamount_as_number(&tokens)
                / amm_helpers::stamount_as_number(&facts.lp_token_balance);
        }
        let amount1_withdraw =
            amm_helpers::get_rounded_asset(&facts.rules, &facts.pool_amount1, frac, IsDeposit::No);
        if facts.rules.enabled(&protocol::fix_ammv1_3()) && amount1_withdraw > *amount1 {
            return Err(Ter::TEC_AMM_FAILED);
        }
        return finalize_withdraw_math(
            facts,
            amount1_withdraw,
            Some(amount2.clone()),
            tokens,
            false,
        );
    }

    if (sub_tx_flags & protocol::AMM_SINGLE_ASSET_FLAG) != 0 {
        let Some(amount1) = &facts.amount1 else {
            return Err(Ter::TEM_MALFORMED);
        };
        let tokens = adjusted_lp_tokens_in(
            &facts.rules,
            &facts.lp_token_balance,
            &amm_helpers::lp_tokens_in(
                &facts.pool_amount1,
                amount1,
                &facts.lp_token_balance,
                facts.trading_fee,
            ),
            false,
        );
        if tokens.signum() == 0 {
            if !facts.rules.enabled(&protocol::fix_ammv1_3()) {
                return Err(Ter::TEC_AMM_FAILED);
            }
            return Err(Ter::TEC_AMM_INVALID_TOKENS);
        }
        let (tokens_adj, amount1_adj) = amm_helpers::adjust_asset_out_by_tokens(
            &facts.rules,
            &facts.pool_amount1,
            amount1,
            &facts.lp_token_balance,
            &tokens,
            facts.trading_fee,
        );
        if facts.rules.enabled(&protocol::fix_ammv1_3()) && tokens_adj.signum() == 0 {
            return Err(Ter::TEC_AMM_INVALID_TOKENS);
        }
        return finalize_withdraw_math(facts, amount1_adj, None, tokens_adj, false);
    }

    if (sub_tx_flags
        & (protocol::AMM_ONE_ASSET_LP_TOKEN_FLAG | protocol::AMM_ONE_ASSET_WITHDRAW_ALL_FLAG))
        != 0
    {
        let Some(amount1) = &facts.amount1 else {
            return Err(Ter::TEM_MALFORMED);
        };
        let Some(lp_tokens_in) = lp_tokens_withdraw else {
            return Err(Ter::TEM_MALFORMED);
        };
        let tokens = adjusted_lp_tokens_in(
            &facts.rules,
            &facts.lp_token_balance,
            &lp_tokens_in,
            is_withdraw_all,
        );
        if facts.rules.enabled(&protocol::fix_ammv1_3()) && tokens.signum() == 0 {
            return Err(Ter::TEC_AMM_INVALID_TOKENS);
        }
        let amount1_withdraw = amm_helpers::amm_asset_out(
            &facts.pool_amount1,
            &facts.lp_token_balance,
            &tokens,
            facts.trading_fee,
        );
        if amount1.signum() == 0 || amount1_withdraw >= *amount1 {
            return finalize_withdraw_math(facts, amount1_withdraw, None, tokens, is_withdraw_all);
        }
        return Err(Ter::TEC_AMM_FAILED);
    }

    if (sub_tx_flags & protocol::AMM_LIMIT_LP_TOKEN_FLAG) != 0 {
        let (Some(amount1), Some(e_price)) = (&facts.amount1, &facts.e_price) else {
            return Err(Ter::TEM_MALFORMED);
        };
        let amount_balance = amm_helpers::stamount_as_number(&facts.pool_amount1);
        let e_price_number = amm_helpers::stamount_as_number(e_price);
        let lp_balance = amm_helpers::stamount_as_number(&facts.lp_token_balance);
        let ae = amount_balance * e_price_number;
        let fee = protocol::get_fee(facts.trading_fee);
        let two = number_from_i64(2);
        let tokens = amm_helpers::get_rounded_lp_tokens_with_product(
            &facts.rules,
            || lp_balance * (lp_balance + ae * (fee - two)) / (lp_balance * fee - ae),
            &facts.lp_token_balance,
            || (lp_balance + ae * (fee - two)) / (lp_balance * fee - ae),
            IsDeposit::No,
        );
        if tokens.signum() <= 0 {
            if !facts.rules.enabled(&protocol::fix_ammv1_3()) {
                return Err(Ter::TEC_AMM_FAILED);
            }
            return Err(Ter::TEC_AMM_INVALID_TOKENS);
        }
        let amount1_withdraw = amm_helpers::get_rounded_asset_with_product(
            &facts.rules,
            || amm_helpers::stamount_as_number(&tokens) / e_price_number,
            amount1,
            || amm_helpers::stamount_as_number(&tokens) / e_price_number,
            IsDeposit::No,
        );
        if amount1.signum() == 0 || amount1_withdraw >= *amount1 {
            return finalize_withdraw_math(facts, amount1_withdraw, None, tokens, false);
        }
        return Err(Ter::TEC_AMM_FAILED);
    }

    Err(Ter::TEM_MALFORMED)
}

pub trait AMMWithdrawApplySink {
    fn get_amm_entry(
        &mut self,
        asset1: &protocol::Asset,
        asset2: &protocol::Asset,
    ) -> Option<protocol::STLedgerEntry>;
    fn update_amm_entry(&mut self, sle: protocol::STLedgerEntry);
    fn withdraw_asset(&mut self, account: &protocol::AccountID, amount: &protocol::STAmount)
    -> Ter;
    fn burn_lp_tokens(&mut self, account: &protocol::AccountID, amount: &protocol::STAmount)
    -> Ter;
}

pub fn run_amm_withdraw_do_apply<S: AMMWithdrawApplySink>(
    facts: AMMWithdrawApplyFacts,
    sink: &mut S,
) -> Ter {
    let Some(amm_sle) = sink.get_amm_entry(&facts.asset1, &facts.asset2) else {
        return Ter::TER_NO_AMM;
    };

    let math = match run_amm_withdraw_apply_math_facts(&AMMWithdrawApplyMathFacts {
        amount1: facts.amount1,
        amount2: facts.amount2,
        e_price: facts.e_price,
        lp_token_in: facts.lp_token_in,
        pool_amount1: facts.pool_amount1,
        pool_amount2: facts.pool_amount2,
        lp_token_balance: facts.lp_token_balance,
        account_lp_tokens: facts.account_lp_tokens,
        trading_fee: facts.trading_fee,
        rules: facts.rules,
        flags: facts.flags,
    }) {
        Ok(math) => math,
        Err(ter) => return ter,
    };

    if let Some(amt1) = &math.amount1 {
        let res = sink.withdraw_asset(&facts.account, amt1);
        if !is_tes_success(res) {
            return res;
        }
    }

    if let Some(amt2) = &math.amount2 {
        let res = sink.withdraw_asset(&facts.account, amt2);
        if !is_tes_success(res) {
            return res;
        }
    }

    let res = sink.burn_lp_tokens(&facts.account, &math.lp_tokens);
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

pub fn run_amm_withdraw_apply<Registry, BaseView, View, Tx, Fee, Journal, ParentBatchId>(
    _ctx: &mut ApplyContext<Registry, BaseView, View, Tx, Fee, Journal, ParentBatchId>,
) -> ApplyResult {
    ApplyResult::new(Ter::TES_SUCCESS, true, false)
}
