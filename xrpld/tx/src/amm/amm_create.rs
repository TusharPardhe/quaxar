//! `AmmCreate` transactor port.

use crate::{
    ApplyContext, ApplyResult, PreclaimContext, PreflightContext, TransactorPreflight0Facts,
    TransactorPreflight1Facts, run_transactor_preflight0, run_transactor_preflight1,
};
use protocol::{NotTec, Ter, feature_batch, is_tes_success};

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
