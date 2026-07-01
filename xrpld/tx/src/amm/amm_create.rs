//! `AmmCreate` transactor port.

use crate::{
    ApplyContext, ApplyResult, PreclaimContext, PreflightContext, TransactorPreflight0Facts,
    TransactorPreflight1Facts, run_transactor_preflight0, run_transactor_preflight1,
};
use protocol::{NotTec, Ter, feature_batch, is_tes_success};

pub const fn amm_create_check_extra_features(
    amm_enabled: bool,
    mptokens_v2_enabled: bool,
    amount_holds_mpt: bool,
    amount2_holds_mpt: bool,
) -> bool {
    if !amm_enabled {
        return false;
    }

    if (amount_holds_mpt || amount2_holds_mpt) && !mptokens_v2_enabled {
        return false;
    }

    true
}

#[derive(Debug, Clone)]
pub struct AMMCreatePreflightFacts {
    pub amount_asset: protocol::Asset,
    pub amount_invalid: Option<NotTec>,
    pub amount2_asset: protocol::Asset,
    pub amount2_invalid: Option<NotTec>,
    pub trading_fee: u16,
}

pub fn run_amm_create_preflight_facts(facts: AMMCreatePreflightFacts) -> NotTec {
    if facts.amount_asset == facts.amount2_asset {
        return Ter::TEM_BAD_AMM_TOKENS;
    }

    if let Some(err) = facts.amount_invalid {
        return err;
    }

    if let Some(err) = facts.amount2_invalid {
        return err;
    }

    if facts.trading_fee > protocol::TRADING_FEE_THRESHOLD {
        return Ter::TEM_BAD_FEE;
    }

    Ter::TES_SUCCESS
}

#[derive(Debug, Clone, Copy)]
pub struct AMMCreatePreclaimFacts {
    pub amm_exists: bool,
    pub amount_auth_result: Ter,
    pub amount2_auth_result: Ter,
    pub amount_frozen_result: Ter,
    pub amount2_frozen_result: Ter,
    pub amount_no_default_ripple: bool,
    pub amount2_no_default_ripple: bool,
    pub xrp_reserve_positive: bool,
    pub amount_insufficient_balance: bool,
    pub amount2_insufficient_balance: bool,
    pub amount_is_lp_token: bool,
    pub amount2_is_lp_token: bool,
    pub address_collision: bool,
    pub amount_mpt_trade_transfer_result: Ter,
    pub amount2_mpt_trade_transfer_result: Ter,
    pub amm_clawback_enabled: bool,
    pub amount_clawback_disabled_result: Ter,
    pub amount2_clawback_disabled_result: Ter,
    pub amount_is_vault_share: bool,
    pub amount2_is_vault_share: bool,
    pub single_asset_vault_enabled: bool,
}

impl Default for AMMCreatePreclaimFacts {
    fn default() -> Self {
        Self {
            amm_exists: false,
            amount_auth_result: Ter::TES_SUCCESS,
            amount2_auth_result: Ter::TES_SUCCESS,
            amount_frozen_result: Ter::TES_SUCCESS,
            amount2_frozen_result: Ter::TES_SUCCESS,
            amount_no_default_ripple: false,
            amount2_no_default_ripple: false,
            xrp_reserve_positive: true,
            amount_insufficient_balance: false,
            amount2_insufficient_balance: false,
            amount_is_lp_token: false,
            amount2_is_lp_token: false,
            address_collision: false,
            amount_mpt_trade_transfer_result: Ter::TES_SUCCESS,
            amount2_mpt_trade_transfer_result: Ter::TES_SUCCESS,
            amm_clawback_enabled: false,
            amount_clawback_disabled_result: Ter::TES_SUCCESS,
            amount2_clawback_disabled_result: Ter::TES_SUCCESS,
            amount_is_vault_share: false,
            amount2_is_vault_share: false,
            single_asset_vault_enabled: false,
        }
    }
}

pub fn run_amm_create_preclaim_facts(facts: AMMCreatePreclaimFacts) -> Ter {
    if facts.amm_exists {
        return Ter::TEC_DUPLICATE;
    }

    if facts.amount_auth_result != Ter::TES_SUCCESS {
        return facts.amount_auth_result;
    }

    if facts.amount2_auth_result != Ter::TES_SUCCESS {
        return facts.amount2_auth_result;
    }

    if facts.amount_frozen_result != Ter::TES_SUCCESS {
        return facts.amount_frozen_result;
    }

    if facts.amount2_frozen_result != Ter::TES_SUCCESS {
        return facts.amount2_frozen_result;
    }

    if facts.amount_no_default_ripple || facts.amount2_no_default_ripple {
        return Ter::TER_NO_RIPPLE;
    }

    if !facts.xrp_reserve_positive {
        return Ter::TEC_INSUF_RESERVE_LINE;
    }

    if facts.amount_insufficient_balance || facts.amount2_insufficient_balance {
        return Ter::TEC_UNFUNDED_AMM;
    }

    if facts.amount_is_lp_token || facts.amount2_is_lp_token {
        return Ter::TEC_AMM_INVALID_TOKENS;
    }

    if facts.address_collision {
        return Ter::TER_ADDRESS_COLLISION;
    }

    if facts.single_asset_vault_enabled
        && (facts.amount_is_vault_share || facts.amount2_is_vault_share)
    {
        return Ter::TEC_WRONG_ASSET;
    }

    if facts.amount_mpt_trade_transfer_result != Ter::TES_SUCCESS {
        return facts.amount_mpt_trade_transfer_result;
    }

    if facts.amount2_mpt_trade_transfer_result != Ter::TES_SUCCESS {
        return facts.amount2_mpt_trade_transfer_result;
    }

    if facts.amm_clawback_enabled {
        return Ter::TES_SUCCESS;
    }

    if facts.amount_clawback_disabled_result != Ter::TES_SUCCESS {
        return facts.amount_clawback_disabled_result;
    }

    if facts.amount2_clawback_disabled_result != Ter::TES_SUCCESS {
        return facts.amount2_clawback_disabled_result;
    }

    Ter::TES_SUCCESS
}

pub fn run_amm_create_preflight<Registry, Tx, Journal, ParentBatchId>(
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

pub fn run_amm_create_preclaim<Registry, View, Tx, Journal, ParentBatchId>(
    _ctx: &PreclaimContext<Registry, View, Tx, Journal, ParentBatchId>,
) -> Ter {
    Ter::TES_SUCCESS
}

pub struct AMMCreateApplyFacts {
    pub amount1: protocol::STAmount,
    pub amount2: protocol::STAmount,
    pub trading_fee: u16,
    pub account: protocol::AccountID,
    pub amm_account: protocol::AccountID,
}

pub trait AMMCreateApplySink {
    fn create_amm_account(&mut self) -> Ter;
    fn create_amm_entry(&mut self) -> Ter;
    fn deposit_initial_liquidity(&mut self) -> Ter;
    fn mint_lp_tokens(&mut self) -> Ter;
    fn adjust_owner_count(&mut self, delta: i32) -> Ter;
}

pub fn run_amm_create_do_apply<S: AMMCreateApplySink>(
    _facts: AMMCreateApplyFacts,
    sink: &mut S,
) -> Ter {
    let result = sink.create_amm_account();
    if result != Ter::TES_SUCCESS {
        return result;
    }
    let result = sink.create_amm_entry();
    if result != Ter::TES_SUCCESS {
        return result;
    }
    let result = sink.deposit_initial_liquidity();
    if result != Ter::TES_SUCCESS {
        return result;
    }
    let result = sink.mint_lp_tokens();
    if result != Ter::TES_SUCCESS {
        return result;
    }
    let result = sink.adjust_owner_count(1);
    if result != Ter::TES_SUCCESS {
        return result;
    }
    Ter::TES_SUCCESS
}

pub fn run_amm_create_apply<Registry, BaseView, View, Tx, Fee, Journal, ParentBatchId>(
    _ctx: &mut ApplyContext<Registry, BaseView, View, Tx, Fee, Journal, ParentBatchId>,
) -> ApplyResult {
    ApplyResult::new(Ter::TES_SUCCESS, true, false)
}
