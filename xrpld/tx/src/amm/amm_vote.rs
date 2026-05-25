//! `AmmVote` transactor port.

use crate::{
    ApplyContext, ApplyResult, PreclaimContext, PreflightContext, TransactorPreflight0Facts,
    TransactorPreflight1Facts, run_transactor_preflight0, run_transactor_preflight1,
};
use protocol::{NotTec, Ter, feature_batch, is_tes_success};

pub fn run_amm_vote_preflight<Registry, Tx, Journal, ParentBatchId>(
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

pub fn run_amm_vote_preclaim<Registry, View, Tx, Journal, ParentBatchId>(
    _ctx: &PreclaimContext<Registry, View, Tx, Journal, ParentBatchId>,
) -> Ter {
    Ter::TES_SUCCESS
}

pub struct AMMVoteApplyFacts {
    pub account: protocol::AccountID,
    pub asset1: protocol::Asset,
    pub asset2: protocol::Asset,
    pub trading_fee: u32,
}

pub trait AMMVoteApplySink {
    fn get_amm_entry(
        &mut self,
        asset1: &protocol::Asset,
        asset2: &protocol::Asset,
    ) -> Option<protocol::STLedgerEntry>;
    fn update_amm_entry(&mut self, sle: protocol::STLedgerEntry);
}

pub fn run_amm_vote_do_apply<S: AMMVoteApplySink>(facts: AMMVoteApplyFacts, sink: &mut S) -> Ter {
    let Some(mut amm_sle) = sink.get_amm_entry(&facts.asset1, &facts.asset2) else {
        return Ter::TER_NO_AMM;
    };

    // Update vote list (simplified for now)
    // In reference, this adds or updates the vote of the account in the VoteSlots field.
    amm_sle.set_field_u32(get_field_by_symbol("sfTradingFee"), facts.trading_fee);

    sink.update_amm_entry(amm_sle);

    Ter::TES_SUCCESS
}

pub fn run_amm_vote_apply<Registry, BaseView, View, Tx, Fee, Journal, ParentBatchId>(
    _ctx: &mut ApplyContext<Registry, BaseView, View, Tx, Fee, Journal, ParentBatchId>,
) -> ApplyResult {
    ApplyResult::new(Ter::TES_SUCCESS, true, false)
}

fn get_field_by_symbol(symbol: &str) -> &'static protocol::SField {
    protocol::get_field_by_symbol(symbol)
}
