//! Public-entry wrapper for the current `xrpld` `TxQ::accept(...)`
//! seam.
//!
//! This wrapper now stays focused on one public-entry job:
//! returning the public `ledgerChanged`-style bool alongside the richer
//! deterministic parity carrier after the widened top-level runtime-source
//! helpers have already built the current accept call state.

use std::fmt::Display;

use crate::{
    ApplyResult, MaybeTx, PreparedQueueAcceptCall, QueueAcceptObservedQueueSource,
    QueueAcceptObservedViewSource, QueueAcceptOwnerState, QueueAcceptPreparedCallStep,
    QueueAcceptTopInputs, QueueAcceptWithMetricsResult, QueueFeeMetricsState, QueueViews,
    build_queue_accept_top_inputs_from_runtime_source,
    prepare_queue_accept_top_with_runtime_source, run_queue_accept_top_with_runtime_source,
    run_queue_accept_top_with_runtime_source_with_caller_prepared_apply,
    run_queue_accept_top_with_runtime_source_with_caller_prepared_apply_and_log_sinks,
    run_queue_accept_top_with_runtime_source_with_log_sinks,
};

pub trait QueueAcceptRuntimeSource:
    QueueAcceptObservedViewSource + QueueAcceptObservedQueueSource
{
    fn metrics(&self) -> &QueueFeeMetricsState;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueAcceptEntryResult<Account> {
    pub ledger_changed: bool,
    pub accept: QueueAcceptWithMetricsResult<Account>,
}

impl<Account> QueueAcceptEntryResult<Account> {
    fn from_accept(accept: QueueAcceptWithMetricsResult<Account>) -> Self {
        Self {
            ledger_changed: accept.ledger_changed(),
            accept,
        }
    }
}

pub fn build_queue_accept_top_inputs_from_runtime<Source>(
    source: &Source,
) -> QueueAcceptTopInputs<'_, Source, Source>
where
    Source: QueueAcceptRuntimeSource,
{
    build_queue_accept_top_inputs_from_runtime_source(source)
}

pub fn run_queue_accept_entry<Account, Tx, Journal, ParentBatchId, Source, ApplyFn>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    owner_state: &mut QueueAcceptOwnerState,
    runtime: &Source,
    apply: ApplyFn,
) -> QueueAcceptEntryResult<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    Source: QueueAcceptRuntimeSource,
    ApplyFn: FnMut(&mut MaybeTx<Tx, Account, Journal, ParentBatchId>) -> ApplyResult,
{
    QueueAcceptEntryResult::from_accept(run_queue_accept_top_with_runtime_source(
        views,
        owner_state,
        runtime,
        apply,
    ))
}

pub fn prepare_queue_accept_entry<Account, Tx, Journal, ParentBatchId, Source>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    owner_state: &mut QueueAcceptOwnerState,
    runtime: &Source,
) -> QueueAcceptPreparedCallStep<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    Source: QueueAcceptRuntimeSource,
{
    prepare_queue_accept_top_with_runtime_source(views, owner_state, runtime)
}

pub fn run_queue_accept_entry_with_caller_prepared_apply<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    Source,
    RunPreparedApply,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    owner_state: &mut QueueAcceptOwnerState,
    runtime: &Source,
    run_prepared_apply: RunPreparedApply,
) -> QueueAcceptEntryResult<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    Source: QueueAcceptRuntimeSource,
    RunPreparedApply: FnMut(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        &mut QueueAcceptOwnerState,
        PreparedQueueAcceptCall<Account>,
    ) -> QueueAcceptPreparedCallStep<Account>,
{
    QueueAcceptEntryResult::from_accept(
        run_queue_accept_top_with_runtime_source_with_caller_prepared_apply(
            views,
            owner_state,
            runtime,
            run_prepared_apply,
        ),
    )
}

pub fn run_queue_accept_entry_with_log_sinks<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    Source,
    ApplyFn,
    TraceFn,
    DebugFn,
    InfoFn,
    WarnFn,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    owner_state: &mut QueueAcceptOwnerState,
    runtime: &Source,
    trace: TraceFn,
    debug: DebugFn,
    info: InfoFn,
    warn: WarnFn,
    apply: ApplyFn,
) -> QueueAcceptEntryResult<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    Source: QueueAcceptRuntimeSource,
    ApplyFn: FnMut(&mut MaybeTx<Tx, Account, Journal, ParentBatchId>) -> ApplyResult,
    TraceFn: FnMut(&str),
    DebugFn: FnMut(&str),
    InfoFn: FnMut(&str),
    WarnFn: FnMut(&str),
{
    QueueAcceptEntryResult::from_accept(run_queue_accept_top_with_runtime_source_with_log_sinks(
        views,
        owner_state,
        runtime,
        trace,
        debug,
        info,
        warn,
        apply,
    ))
}

pub fn run_queue_accept_entry_with_caller_prepared_apply_and_log_sinks<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    Source,
    RunPreparedApply,
    TraceFn,
    DebugFn,
    InfoFn,
    WarnFn,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    owner_state: &mut QueueAcceptOwnerState,
    runtime: &Source,
    trace: TraceFn,
    debug: DebugFn,
    info: InfoFn,
    warn: WarnFn,
    run_prepared_apply: RunPreparedApply,
) -> QueueAcceptEntryResult<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    Source: QueueAcceptRuntimeSource,
    RunPreparedApply: FnMut(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        &mut QueueAcceptOwnerState,
        PreparedQueueAcceptCall<Account>,
    ) -> QueueAcceptPreparedCallStep<Account>,
    TraceFn: FnMut(&str),
    DebugFn: FnMut(&str),
    InfoFn: FnMut(&str),
    WarnFn: FnMut(&str),
{
    QueueAcceptEntryResult::from_accept(
        run_queue_accept_top_with_runtime_source_with_caller_prepared_apply_and_log_sinks(
            views,
            owner_state,
            runtime,
            trace,
            debug,
            info,
            warn,
            run_prepared_apply,
        ),
    )
}
