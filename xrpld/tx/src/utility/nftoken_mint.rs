//! `NftokenMint` transactor port.

use crate::{
    ApplyContext, ApplyResult, PreclaimContext, PreflightContext, TransactorPreflight0Facts,
    TransactorPreflight1Facts, run_transactor_preflight0, run_transactor_preflight1,
};
use protocol::{NotTec, Ter, feature_batch, is_tes_success};

pub fn run_nftoken_mint_preflight<Registry, Tx, Journal, ParentBatchId>(
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

pub fn run_nftoken_mint_preclaim<Registry, View, Tx, Journal, ParentBatchId>(
    _ctx: &PreclaimContext<Registry, View, Tx, Journal, ParentBatchId>,
) -> Ter {
    Ter::TES_SUCCESS
}

use basics::base_uint::Uint256;

pub struct NFTokenMintApplyFacts {
    pub nftoken_id: Uint256,
    pub issuer: protocol::AccountID,
    pub owner: protocol::AccountID,
    pub transfer_fee: Option<u16>,
    pub uri: Option<protocol::STBlob>,
}

pub trait NFTokenMintApplySink {
    fn mint_nftoken(&mut self, facts: &NFTokenMintApplyFacts);
    fn adjust_owner_count(&mut self, delta: i32);
}

pub fn run_nftoken_mint_do_apply<S: NFTokenMintApplySink>(
    facts: NFTokenMintApplyFacts,
    sink: &mut S,
) -> Ter {
    sink.mint_nftoken(&facts);
    sink.adjust_owner_count(1);
    Ter::TES_SUCCESS
}

pub fn run_nftoken_mint_apply<Registry, BaseView, View, Tx, Fee, Journal, ParentBatchId>(
    _ctx: &mut ApplyContext<Registry, BaseView, View, Tx, Fee, Journal, ParentBatchId>,
) -> ApplyResult {
    ApplyResult::new(Ter::TES_SUCCESS, true, false)
}
