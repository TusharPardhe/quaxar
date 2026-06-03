//! `AmmDelete` transactor port.

use crate::{
    ApplyContext, ApplyResult, PreclaimContext, PreflightContext, TransactorPreflight0Facts,
    TransactorPreflight1Facts, run_transactor_preflight0, run_transactor_preflight1,
};
use protocol::{NotTec, Ter, feature_batch, is_tes_success};

pub const fn amm_delete_check_extra_features(
    amm_enabled: bool,
    mptokens_v2_enabled: bool,
    asset_pair_holds_mpt: bool,
) -> bool {
    if !amm_enabled {
        return false;
    }

    if asset_pair_holds_mpt && !mptokens_v2_enabled {
        return false;
    }

    true
}

pub fn run_amm_delete_preflight<Registry, Tx, Journal, ParentBatchId>(
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AMMDeletePreclaimFacts {
    pub amm_exists: bool,
    pub lp_token_balance_is_zero: bool,
}

pub const fn run_amm_delete_preclaim_facts(facts: AMMDeletePreclaimFacts) -> Ter {
    if !facts.amm_exists {
        return Ter::TER_NO_AMM;
    }

    if !facts.lp_token_balance_is_zero {
        return Ter::TEC_AMM_NOT_EMPTY;
    }

    Ter::TES_SUCCESS
}

pub fn run_amm_delete_preclaim<Registry, View, Tx, Journal, ParentBatchId>(
    _ctx: &PreclaimContext<Registry, View, Tx, Journal, ParentBatchId>,
) -> Ter {
    Ter::TES_SUCCESS
}

pub struct AMMDeleteApplyFacts {
    pub account: protocol::AccountID,
    pub asset1: protocol::Asset,
    pub asset2: protocol::Asset,
}

pub trait AMMDeleteApplySink {
    fn get_amm_entry(
        &mut self,
        asset1: &protocol::Asset,
        asset2: &protocol::Asset,
    ) -> Option<protocol::STLedgerEntry>;
    fn delete_amm_entry(&mut self, sle: protocol::STLedgerEntry) -> Ter;
    fn delete_amm_account(&mut self, amm_account: &protocol::AccountID) -> Ter;
}

pub fn run_amm_delete_do_apply<S: AMMDeleteApplySink>(
    facts: AMMDeleteApplyFacts,
    sink: &mut S,
) -> Ter {
    let Some(amm_sle) = sink.get_amm_entry(&facts.asset1, &facts.asset2) else {
        return Ter::TER_NO_AMM;
    };

    let amm_account = amm_sle.get_account_id(protocol::get_field_by_symbol("sfAccount"));

    let res = sink.delete_amm_account(&amm_account);
    if !is_tes_success(res) {
        return res;
    }

    sink.delete_amm_entry(amm_sle)
}

pub fn run_amm_delete_apply<Registry, BaseView, View, Tx, Fee, Journal, ParentBatchId>(
    _ctx: &mut ApplyContext<Registry, BaseView, View, Tx, Fee, Journal, ParentBatchId>,
) -> ApplyResult {
    ApplyResult::new(Ter::TES_SUCCESS, true, false)
}
