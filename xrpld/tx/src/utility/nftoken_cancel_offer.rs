//! `NFTokenCancelOffer` transactor port from `xrpld/src/libxrpl/tx/transactors/nft/the reference source`.

use crate::{
    ApplyContext, ApplyResult, PreclaimContext, PreflightContext, TransactorPreflight0Facts,
    TransactorPreflight1Facts, TransactorPreflight2Facts, run_transactor_preflight0,
    run_transactor_preflight1, run_transactor_preflight2,
};
use basics::base_uint::Uint256;
use protocol::{NotTec, Ter, feature_batch, is_tes_success};

pub const NFTOKEN_CANCEL_OFFER_FLAGS_MASK: u32 = 0;

pub struct NFTokenCancelOfferPreflightFacts {
    pub nftoken_offers: Vec<Uint256>,
    pub flags: u32,
    pub has_zero_offer_id: bool,
    pub fix_cleanup_3_2_0_enabled: bool,
}

pub fn run_nftoken_cancel_offer_preflight<Registry, Tx, Journal, ParentBatchId>(
    ctx: &PreflightContext<Registry, Tx, Journal, ParentBatchId>,
    facts: NFTokenCancelOfferPreflightFacts,
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
                NFTOKEN_CANCEL_OFFER_FLAGS_MASK,
            )
        },
        || Ter::TES_SUCCESS,
    );

    if !is_tes_success(ret) {
        return ret;
    }

    if facts.nftoken_offers.is_empty() {
        return Ter::TEM_MALFORMED;
    }

    if facts.fix_cleanup_3_2_0_enabled && facts.has_zero_offer_id {
        return Ter::TEM_MALFORMED;
    }

    run_transactor_preflight2(
        TransactorPreflight2Facts {
            ..Default::default()
        },
        || None,
        || crate::Validity::Valid,
    )
}

pub fn run_nftoken_cancel_offer_preclaim<Registry, View, Tx, Journal, ParentBatchId>(
    _ctx: &PreclaimContext<Registry, View, Tx, Journal, ParentBatchId>,
) -> Ter {
    Ter::TES_SUCCESS
}

pub fn run_nftoken_cancel_offer_do_apply<
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
