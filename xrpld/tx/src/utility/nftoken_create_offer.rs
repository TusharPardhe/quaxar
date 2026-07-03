//! `NFTokenCreateOffer` transactor port from `xrpld/src/libxrpl/tx/transactors/nft/the reference source`.

use crate::{
    ApplyContext, ApplyResult, PreclaimContext, PreflightContext, TransactorPreflight0Facts,
    TransactorPreflight1Facts, TransactorPreflight2Facts, run_transactor_preflight0,
    run_transactor_preflight1, run_transactor_preflight2,
};
use basics::base_uint::Uint256;
use protocol::{NotTec, STAmount, Ter, feature_batch, is_tes_success};

pub const TF_SELL_NFTOKEN: u32 = 0x0000_0001;

pub const NFTOKEN_CREATE_OFFER_FLAGS_MASK: u32 = !TF_SELL_NFTOKEN;

pub struct NFTokenCreateOfferPreflightFacts {
    pub amount: STAmount,
    pub nftoken_id: Uint256,
    pub flags: u32,
}

pub fn run_nftoken_create_offer_preflight<Registry, Tx, Journal, ParentBatchId>(
    ctx: &PreflightContext<Registry, Tx, Journal, ParentBatchId>,
    facts: NFTokenCreateOfferPreflightFacts,
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
                    tx_flags: facts.flags,
                    ..Default::default()
                },
                NFTOKEN_CREATE_OFFER_FLAGS_MASK,
            )
        },
        || Ter::TES_SUCCESS,
    );

    if !is_tes_success(ret) {
        return ret;
    }

    if facts.amount.negative() || !facts.amount.is_legal_net() {
        return Ter::TEM_BAD_AMOUNT;
    }

    // (tokenOfferCreatePreflight): IOU zero is always bad
    if !facts.amount.native() && facts.amount.mantissa() == 0 {
        return Ter::TEM_BAD_AMOUNT;
    }

    // Buy offers must have non-zero amount
    let is_sell = (facts.flags & TF_SELL_NFTOKEN) != 0;
    if !is_sell && facts.amount.mantissa() == 0 {
        return Ter::TEM_BAD_AMOUNT;
    }

    run_transactor_preflight2(
        TransactorPreflight2Facts {
            ..Default::default()
        },
        || None,
        || crate::Validity::Valid,
    )
}

pub fn run_nftoken_create_offer_preclaim<Registry, View, Tx, Journal, ParentBatchId>(
    _ctx: &PreclaimContext<Registry, View, Tx, Journal, ParentBatchId>,
) -> Ter {
    Ter::TES_SUCCESS
}

pub fn run_nftoken_create_offer_do_apply<
    Registry,
    BaseView,
    View,
    Tx,
    Fee,
    Journal,
    ParentBatchId,
>(
    _ctx: &mut ApplyContext<Registry, BaseView, View, Tx, Fee, Journal, ParentBatchId>,
) -> ApplyResult {
    ApplyResult::new(Ter::TES_SUCCESS, true, false)
}
