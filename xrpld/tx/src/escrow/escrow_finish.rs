//! `EscrowFinish` transactor port from `xrpld/src/libxrpl/tx/transactors/escrow/the reference source`.

use crate::{
    ApplyContext, ApplyResult, PreclaimContext, PreflightContext, TransactorPreflight0Facts,
    TransactorPreflight1Facts, run_transactor_preflight0, run_transactor_preflight1,
};
use protocol::{NotTec, Ter, feature_batch, is_tes_success};

pub struct EscrowFinishPreflightFacts {
    pub has_condition: bool,
    pub has_fulfillment: bool,
    pub flags: u32,
}

pub fn run_escrow_finish_preflight<Registry, Tx, Journal, ParentBatchId>(
    ctx: &PreflightContext<Registry, Tx, Journal, ParentBatchId>,
    facts: EscrowFinishPreflightFacts,
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
                0, // no special flags for EscrowFinish
            )
        },
        || Ter::TES_SUCCESS,
    );

    if !is_tes_success(ret) {
        return ret;
    }

    if facts.has_condition != facts.has_fulfillment {
        return Ter::TEM_MALFORMED;
    }

    Ter::TES_SUCCESS
}

pub struct EscrowFinishPreclaimFacts {
    pub owner_exists: bool,
    pub escrow_exists: bool,
    pub escrow_has_condition: bool,
    pub fulfillment_valid: bool,
    pub escrow_expired: bool,
    pub finish_time_reached: bool,
}

pub fn run_escrow_finish_preclaim<Registry, View, Tx, Journal, ParentBatchId>(
    _ctx: &PreclaimContext<Registry, View, Tx, Journal, ParentBatchId>,
    facts: EscrowFinishPreclaimFacts,
) -> Ter {
    if !facts.owner_exists {
        return Ter::TER_NO_ACCOUNT;
    }

    if !facts.escrow_exists {
        return Ter::TEC_NO_TARGET;
    }

    if facts.escrow_has_condition && !facts.fulfillment_valid {
        return Ter::TEC_CRYPTOCONDITION_ERROR;
    }

    if facts.escrow_expired {
        return Ter::TEC_NO_PERMISSION;
    }

    if !facts.finish_time_reached {
        return Ter::TEC_NO_PERMISSION;
    }

    Ter::TES_SUCCESS
}

use basics::base_uint::Uint256;

pub struct EscrowFinishApplyFacts {
    pub amount: protocol::STAmount,
    pub destination: protocol::AccountID,
    pub owner: protocol::AccountID,
    pub escrow_key: Uint256,
    pub owner_node: u64,
    pub destination_node: Option<u64>,
}

pub trait EscrowFinishApplySink {
    fn transfer_escrow_amount(&mut self);
    fn remove_escrow_entry(&mut self);
    fn adjust_owner_count(&mut self, account: &protocol::AccountID, delta: i32);
}

pub fn run_escrow_finish_do_apply<S: EscrowFinishApplySink>(
    _facts: EscrowFinishApplyFacts,
    sink: &mut S,
) -> Ter {
    sink.transfer_escrow_amount();
    sink.remove_escrow_entry();
    sink.adjust_owner_count(&_facts.owner, -1);
    Ter::TES_SUCCESS
}

pub fn run_escrow_finish_apply<Registry, BaseView, View, Tx, Fee, Journal, ParentBatchId>(
    _ctx: &mut ApplyContext<Registry, BaseView, View, Tx, Fee, Journal, ParentBatchId>,
) -> ApplyResult {
    ApplyResult::new(Ter::TES_SUCCESS, true, false)
}
