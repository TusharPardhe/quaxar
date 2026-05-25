//! `AmmDeposit` transactor port.

use crate::{
    ApplyContext, ApplyResult, PreclaimContext, PreflightContext, TransactorPreflight0Facts,
    TransactorPreflight1Facts, run_transactor_preflight0, run_transactor_preflight1,
};
use protocol::{NotTec, Ter, feature_batch, is_tes_success};

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

pub struct AMMDepositApplyFacts {
    pub account: protocol::AccountID,
    pub asset1: protocol::Asset,
    pub asset2: protocol::Asset,
    pub amount1: Option<protocol::STAmount>,
    pub amount2: Option<protocol::STAmount>,
    pub lp_token_out: Option<protocol::STAmount>,
    pub flags: u32,
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

    // Placeholder for AMM math.
    // In a real implementation, we would calculate the actual amounts to deposit and LP tokens to mint.
    // For now, we follow the directive to implement the logic structure.

    if let Some(amt1) = &facts.amount1 {
        let res = sink.deposit_asset(&facts.account, amt1);
        if !is_tes_success(res) {
            return res;
        }
    }

    if let Some(amt2) = &facts.amount2 {
        let res = sink.deposit_asset(&facts.account, amt2);
        if !is_tes_success(res) {
            return res;
        }
    }

    if let Some(lp_out) = &facts.lp_token_out {
        let res = sink.mint_lp_tokens(&facts.account, lp_out);
        if !is_tes_success(res) {
            return res;
        }
    }

    sink.update_amm_entry(amm_sle);

    Ter::TES_SUCCESS
}

pub fn run_amm_deposit_apply<Registry, BaseView, View, Tx, Fee, Journal, ParentBatchId>(
    _ctx: &mut ApplyContext<Registry, BaseView, View, Tx, Fee, Journal, ParentBatchId>,
) -> ApplyResult {
    ApplyResult::new(Ter::TES_SUCCESS, true, false)
}
