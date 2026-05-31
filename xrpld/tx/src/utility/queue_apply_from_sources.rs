//! Entry helper for callers that already have live tx/view sources and want to
//! enter the landed `TxQ::apply(...)` top wrapper chain without manually
//! rebuilding each intermediate carrier.
//!
//! This module composes the already-landed source lowering, read-side
//! lowering, and top queued-stage wrapper.

use std::fmt::Display;

use crate::{
    ApplyFlags, DirectApplyExecution, MaybeTx, PreclaimResult, PreflightResult,
    QueueApplyObservedQueue, QueueApplyObservedTxSource, QueueApplyObservedViewSource,
    QueueApplyPreclaimStage, QueueApplyPreclaimViewSource, QueueApplyPreflightStage,
    QueueApplyPreparedFlowInputs, QueueApplyPreparedPreclaimInputs,
    QueueApplyPreparedQueuedStageInputs, QueueApplyQueuedStageWithLogMessagesResult,
    QueueApplyTopWithLogMessagesResult, QueueApplyTopWithQueuedStageInputs,
    QueueApplyTryClearResult, QueueApplyViewAdjustment, QueueHoldPreflight, QueueViews,
    TxConsequences, build_queue_apply_top_read_inputs_from_sources,
    build_queue_apply_top_with_queued_stage_inputs, derive_queue_apply_prepared_flow_stage,
    derive_queue_apply_prepared_post_preclaim_inputs, run_prepared_direct_apply_with_trace,
    run_prepared_queue_apply_post_preclaim_stage_with_caller_queue,
    run_prepared_queue_apply_queued_flow_stage,
    run_queue_apply_after_preflight_with_acquired_direct_apply_and_caller_queued_stage,
    run_queue_apply_after_preflight_with_caller_direct_apply_and_caller_queued_stage,
    run_queue_apply_after_preflight_with_direct_apply_and_caller_queued_stage,
    run_queue_apply_after_preflight_with_direct_apply_and_caller_queued_stage_and_log_messages,
    run_queue_apply_flow_stage_with_caller_preclaim, run_queue_apply_queue_stage_with_log_sinks,
    run_queue_apply_queued_stage_with_fee_context,
    run_queue_apply_queued_stage_with_fee_context_and_log_sinks,
    run_queue_apply_queued_stage_with_log_messages,
    run_queue_apply_top_with_caller_direct_apply_and_caller_queued_stage,
    run_queue_apply_top_with_caller_direct_apply_and_caller_queued_stage_and_log_messages,
    run_queue_apply_try_clear_stage,
};

#[derive(Debug, Clone, Copy)]
pub struct QueueApplyTopFromSourcesInputs<'a> {
    pub preflight: QueueHoldPreflight,
    pub flags: ApplyFlags,
    pub consequences: TxConsequences,
    pub queue: QueueApplyObservedQueue<'a>,
}

impl<'a> QueueApplyTopFromSourcesInputs<'a> {
    pub fn new(
        preflight: QueueHoldPreflight,
        flags: ApplyFlags,
        consequences: TxConsequences,
        queue: QueueApplyObservedQueue<'a>,
    ) -> Self {
        Self {
            preflight,
            flags,
            consequences,
            queue,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct QueueApplyAfterPreflightSourceInputs<'a> {
    pub hold_preflight: QueueHoldPreflight,
    pub queue: QueueApplyObservedQueue<'a>,
}

impl<'a> QueueApplyAfterPreflightSourceInputs<'a> {
    pub fn new(hold_preflight: QueueHoldPreflight, queue: QueueApplyObservedQueue<'a>) -> Self {
        Self {
            hold_preflight,
            queue,
        }
    }
}

pub fn build_queue_apply_top_with_queued_stage_inputs_from_sources<'a, TxSource, ViewSource>(
    tx_source: &'a TxSource,
    view_source: &'a ViewSource,
    inputs: QueueApplyTopFromSourcesInputs<'a>,
) -> QueueApplyTopWithQueuedStageInputs<'a, TxSource::Account, TxSource::TransactionId>
where
    TxSource: QueueApplyObservedTxSource,
    TxSource::Account: Clone,
    ViewSource: QueueApplyObservedViewSource<TxSource::Account>,
{
    build_queue_apply_top_with_queued_stage_inputs(build_queue_apply_top_read_inputs_from_sources(
        tx_source,
        view_source,
        inputs.preflight,
        inputs.flags,
        inputs.consequences,
        inputs.queue,
    ))
}

pub fn build_queue_apply_top_with_queued_stage_inputs_from_sources_after_preflight<
    'a,
    TxSource,
    ViewSource,
    Tx,
    Journal,
    ParentBatchId,
>(
    tx_source: &'a TxSource,
    view_source: &'a ViewSource,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    inputs: QueueApplyAfterPreflightSourceInputs<'a>,
) -> QueueApplyTopWithQueuedStageInputs<'a, TxSource::Account, TxSource::TransactionId>
where
    TxSource: QueueApplyObservedTxSource,
    TxSource::Account: Clone,
    ViewSource: QueueApplyObservedViewSource<TxSource::Account>,
{
    build_queue_apply_top_with_queued_stage_inputs_from_sources(
        tx_source,
        view_source,
        QueueApplyTopFromSourcesInputs::new(
            inputs.hold_preflight,
            preflight_result.flags,
            preflight_result.consequences,
            inputs.queue,
        ),
    )
}

fn with_after_preflight_lowered_queue_inputs_from_sources<
    'a,
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TxSource,
    ViewSource,
    R,
    Run,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    tx_source: &'a TxSource,
    view_source: &'a ViewSource,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    inputs: QueueApplyAfterPreflightSourceInputs<'a>,
    run: Run,
) -> R
where
    Account: Clone + Display + Ord + PartialEq + 'a,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    ViewSource: QueueApplyObservedViewSource<Account>,
    Run: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyTopWithQueuedStageInputs<'a, Account, TxId>,
    ) -> R,
{
    run(
        views,
        build_queue_apply_top_with_queued_stage_inputs_from_sources_after_preflight(
            tx_source,
            view_source,
            preflight_result,
            inputs,
        ),
    )
}

fn run_queue_apply_top_with_trace_wrapped_direct_apply_and_caller_queued_stage_from_sources<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TxSource,
    ViewSource,
    RunPreflight,
    TraceFn,
    ApplyFn,
    RunQueuedStage,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    tx_source: &TxSource,
    view_source: &ViewSource,
    inputs: QueueApplyTopFromSourcesInputs<'_>,
    run_preflight: RunPreflight,
    trace: TraceFn,
    apply: ApplyFn,
    run_queued_stage: RunQueuedStage,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    ViewSource: QueueApplyObservedViewSource<Account>,
    RunPreflight: FnOnce() -> PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    TraceFn: FnMut(&str),
    ApplyFn: FnOnce() -> crate::ApplyResult,
    RunQueuedStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedQueuedStageInputs<'_, Account, Tx, Journal, ParentBatchId>,
    )
        -> crate::QueueApplyQueuedStage<Account, Tx, Journal, ParentBatchId>,
{
    run_queue_apply_top_with_caller_direct_apply_and_caller_queued_stage_from_sources(
        views,
        tx_source,
        view_source,
        inputs,
        run_preflight,
        |views, prepared| run_prepared_direct_apply_with_trace(views, prepared, trace, apply),
        run_queued_stage,
    )
}

fn run_queue_apply_top_with_trace_wrapped_direct_apply_and_caller_queued_stage_and_log_messages_from_sources<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TxSource,
    ViewSource,
    RunPreflight,
    TraceFn,
    ApplyFn,
    RunQueuedStage,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    tx_source: &TxSource,
    view_source: &ViewSource,
    inputs: QueueApplyTopFromSourcesInputs<'_>,
    run_preflight: RunPreflight,
    trace: TraceFn,
    apply: ApplyFn,
    run_queued_stage: RunQueuedStage,
) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    ViewSource: QueueApplyObservedViewSource<Account>,
    RunPreflight: FnOnce() -> PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    TraceFn: FnMut(&str),
    ApplyFn: FnOnce() -> crate::ApplyResult,
    RunQueuedStage:
        FnOnce(
            &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
            QueueApplyPreparedQueuedStageInputs<'_, Account, Tx, Journal, ParentBatchId>,
        )
            -> QueueApplyQueuedStageWithLogMessagesResult<Account, Tx, Journal, ParentBatchId>,
{
    run_queue_apply_top_with_caller_direct_apply_and_caller_queued_stage_and_log_messages(
        views,
        build_queue_apply_top_with_queued_stage_inputs_from_sources(tx_source, view_source, inputs),
        run_preflight,
        |views, prepared| run_prepared_direct_apply_with_trace(views, prepared, trace, apply),
        run_queued_stage,
    )
}

pub fn run_queue_apply_top_with_queued_stage_from_sources<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TxSource,
    ViewSource,
    RunPreflight,
    TraceFn,
    ApplyFn,
    PrepareMultiTxn,
    RunPreclaim,
    RunTryClear,
    TryClearResult,
    ApplySandbox,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    tx_source: &TxSource,
    view_source: &ViewSource,
    inputs: QueueApplyTopFromSourcesInputs<'_>,
    run_preflight: RunPreflight,
    trace: TraceFn,
    apply: ApplyFn,
    prepare_multitxn: PrepareMultiTxn,
    run_preclaim: RunPreclaim,
    run_try_clear: RunTryClear,
    apply_sandbox: ApplySandbox,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    ViewSource: QueueApplyObservedViewSource<Account>,
    RunPreflight: FnOnce() -> PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    TraceFn: FnMut(&str),
    ApplyFn: FnOnce() -> crate::ApplyResult,
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
    RunPreclaim: FnOnce(QueueApplyPreclaimViewSource) -> PreclaimResult<Tx, Journal, ParentBatchId>,
    RunTryClear: FnOnce() -> TryClearResult,
    TryClearResult: QueueApplyTryClearResult,
    ApplySandbox: FnOnce(),
{
    run_queue_apply_top_with_lowered_queued_stage_from_sources(
        views,
        tx_source,
        view_source,
        inputs,
        run_preflight,
        trace,
        apply,
        |views, queued| {
            run_queue_apply_standard_queued_stage(
                views,
                queued,
                prepare_multitxn,
                run_preclaim,
                run_try_clear,
                apply_sandbox,
            )
        },
    )
}

fn run_queue_apply_top_with_lowered_queued_stage_from_sources<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TxSource,
    ViewSource,
    RunPreflight,
    TraceFn,
    ApplyFn,
    RunQueuedStage,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    tx_source: &TxSource,
    view_source: &ViewSource,
    inputs: QueueApplyTopFromSourcesInputs<'_>,
    run_preflight: RunPreflight,
    trace: TraceFn,
    apply: ApplyFn,
    run_queued_stage: RunQueuedStage,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    ViewSource: QueueApplyObservedViewSource<Account>,
    RunPreflight: FnOnce() -> PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    TraceFn: FnMut(&str),
    ApplyFn: FnOnce() -> crate::ApplyResult,
    RunQueuedStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedQueuedStageInputs<'_, Account, Tx, Journal, ParentBatchId>,
    )
        -> crate::QueueApplyQueuedStage<Account, Tx, Journal, ParentBatchId>,
{
    run_queue_apply_top_with_trace_wrapped_direct_apply_and_caller_queued_stage_from_sources(
        views,
        tx_source,
        view_source,
        inputs,
        run_preflight,
        trace,
        apply,
        run_queued_stage,
    )
}

pub fn run_queue_apply_top_with_log_sinks_from_sources<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TxSource,
    ViewSource,
    RunPreflight,
    TraceFn,
    DebugFn,
    InfoFn,
    ApplyFn,
    PrepareMultiTxn,
    RunPreclaim,
    RunTryClear,
    TryClearResult,
    ApplySandbox,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    tx_source: &TxSource,
    view_source: &ViewSource,
    inputs: QueueApplyTopFromSourcesInputs<'_>,
    run_preflight: RunPreflight,
    trace: TraceFn,
    mut debug: DebugFn,
    mut info: InfoFn,
    apply: ApplyFn,
    prepare_multitxn: PrepareMultiTxn,
    run_preclaim: RunPreclaim,
    run_try_clear: RunTryClear,
    apply_sandbox: ApplySandbox,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    ViewSource: QueueApplyObservedViewSource<Account>,
    RunPreflight: FnOnce() -> PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    TraceFn: FnMut(&str),
    DebugFn: FnMut(String),
    InfoFn: FnMut(String),
    ApplyFn: FnOnce() -> crate::ApplyResult,
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
    RunPreclaim: FnOnce(QueueApplyPreclaimViewSource) -> PreclaimResult<Tx, Journal, ParentBatchId>,
    RunTryClear: FnOnce() -> TryClearResult,
    TryClearResult: QueueApplyTryClearResult,
    ApplySandbox: FnOnce(),
{
    run_queue_apply_top_with_log_sinks_lowered_queued_stage_from_sources(
        views,
        tx_source,
        view_source,
        inputs,
        run_preflight,
        trace,
        apply,
        |views, queued| {
            run_queue_apply_standard_queued_stage_with_log_sinks(
                views,
                queued,
                prepare_multitxn,
                run_preclaim,
                run_try_clear,
                apply_sandbox,
                |message| debug(message),
                |message| info(message),
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_top_with_log_messages_from_sources<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TxSource,
    ViewSource,
    RunPreflight,
    TraceFn,
    ApplyFn,
    PrepareMultiTxn,
    RunPreclaim,
    RunTryClear,
    TryClearResult,
    ApplySandbox,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    tx_source: &TxSource,
    view_source: &ViewSource,
    inputs: QueueApplyTopFromSourcesInputs<'_>,
    run_preflight: RunPreflight,
    trace: TraceFn,
    apply: ApplyFn,
    prepare_multitxn: PrepareMultiTxn,
    run_preclaim: RunPreclaim,
    run_try_clear: RunTryClear,
    apply_sandbox: ApplySandbox,
) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    ViewSource: QueueApplyObservedViewSource<Account>,
    RunPreflight: FnOnce() -> PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    TraceFn: FnMut(&str),
    ApplyFn: FnOnce() -> crate::ApplyResult,
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
    RunPreclaim: FnOnce(QueueApplyPreclaimViewSource) -> PreclaimResult<Tx, Journal, ParentBatchId>,
    RunTryClear: FnOnce() -> TryClearResult,
    TryClearResult: QueueApplyTryClearResult,
    ApplySandbox: FnOnce(),
{
    run_queue_apply_top_with_trace_wrapped_direct_apply_and_caller_queued_stage_and_log_messages_from_sources(
        views,
        tx_source,
        view_source,
        inputs,
        run_preflight,
        trace,
        apply,
        |views, queued| {
            run_queue_apply_standard_queued_stage_with_log_messages(
                views,
                queued,
                prepare_multitxn,
                run_preclaim,
                run_try_clear,
                apply_sandbox,
            )
        },
    )
}

fn run_queue_apply_top_with_log_sinks_lowered_queued_stage_from_sources<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TxSource,
    ViewSource,
    RunPreflight,
    TraceFn,
    ApplyFn,
    RunQueuedStage,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    tx_source: &TxSource,
    view_source: &ViewSource,
    inputs: QueueApplyTopFromSourcesInputs<'_>,
    run_preflight: RunPreflight,
    trace: TraceFn,
    apply: ApplyFn,
    run_queued_stage: RunQueuedStage,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    ViewSource: QueueApplyObservedViewSource<Account>,
    RunPreflight: FnOnce() -> PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    TraceFn: FnMut(&str),
    ApplyFn: FnOnce() -> crate::ApplyResult,
    RunQueuedStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedQueuedStageInputs<'_, Account, Tx, Journal, ParentBatchId>,
    )
        -> crate::QueueApplyQueuedStage<Account, Tx, Journal, ParentBatchId>,
{
    run_queue_apply_top_with_lowered_queued_stage_from_sources(
        views,
        tx_source,
        view_source,
        inputs,
        run_preflight,
        trace,
        apply,
        run_queued_stage,
    )
}

pub fn run_queue_apply_after_preflight_from_sources<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TxSource,
    ViewSource,
    TraceFn,
    ApplyFn,
    PrepareMultiTxn,
    RunPreclaim,
    RunTryClear,
    TryClearResult,
    ApplySandbox,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    tx_source: &TxSource,
    view_source: &ViewSource,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    inputs: QueueApplyAfterPreflightSourceInputs<'_>,
    trace: TraceFn,
    apply: ApplyFn,
    prepare_multitxn: PrepareMultiTxn,
    run_preclaim: RunPreclaim,
    run_try_clear: RunTryClear,
    apply_sandbox: ApplySandbox,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    ViewSource: QueueApplyObservedViewSource<Account>,
    TraceFn: FnMut(&str),
    ApplyFn: FnOnce() -> crate::ApplyResult,
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
    RunPreclaim: FnOnce(QueueApplyPreclaimViewSource) -> PreclaimResult<Tx, Journal, ParentBatchId>,
    RunTryClear: FnOnce() -> TryClearResult,
    TryClearResult: QueueApplyTryClearResult,
    ApplySandbox: FnOnce(),
{
    run_queue_apply_after_preflight_with_lowered_queued_stage_from_sources(
        views,
        tx_source,
        view_source,
        preflight_result,
        inputs,
        trace,
        apply,
        |views, queued| {
            run_queue_apply_standard_queued_stage(
                views,
                queued,
                prepare_multitxn,
                run_preclaim,
                run_try_clear,
                apply_sandbox,
            )
        },
    )
}

fn run_queue_apply_after_preflight_with_lowered_queued_stage_from_sources<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TxSource,
    ViewSource,
    TraceFn,
    ApplyFn,
    RunQueuedStage,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    tx_source: &TxSource,
    view_source: &ViewSource,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    inputs: QueueApplyAfterPreflightSourceInputs<'_>,
    trace: TraceFn,
    apply: ApplyFn,
    run_queued_stage: RunQueuedStage,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    ViewSource: QueueApplyObservedViewSource<Account>,
    TraceFn: FnMut(&str),
    ApplyFn: FnOnce() -> crate::ApplyResult,
    RunQueuedStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedQueuedStageInputs<'_, Account, Tx, Journal, ParentBatchId>,
    )
        -> crate::QueueApplyQueuedStage<Account, Tx, Journal, ParentBatchId>,
{
    run_queue_apply_after_preflight_with_direct_apply_and_caller_queued_stage_from_sources(
        views,
        tx_source,
        view_source,
        preflight_result,
        inputs,
        trace,
        apply,
        run_queued_stage,
    )
}

pub fn run_queue_apply_after_preflight_with_log_sinks_from_sources<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TxSource,
    ViewSource,
    TraceFn,
    DebugFn,
    InfoFn,
    ApplyFn,
    PrepareMultiTxn,
    RunPreclaim,
    RunTryClear,
    TryClearResult,
    ApplySandbox,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    tx_source: &TxSource,
    view_source: &ViewSource,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    inputs: QueueApplyAfterPreflightSourceInputs<'_>,
    trace: TraceFn,
    mut debug: DebugFn,
    mut info: InfoFn,
    apply: ApplyFn,
    prepare_multitxn: PrepareMultiTxn,
    run_preclaim: RunPreclaim,
    run_try_clear: RunTryClear,
    apply_sandbox: ApplySandbox,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    ViewSource: QueueApplyObservedViewSource<Account>,
    TraceFn: FnMut(&str),
    DebugFn: FnMut(String),
    InfoFn: FnMut(String),
    ApplyFn: FnOnce() -> crate::ApplyResult,
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
    RunPreclaim: FnOnce(QueueApplyPreclaimViewSource) -> PreclaimResult<Tx, Journal, ParentBatchId>,
    RunTryClear: FnOnce() -> TryClearResult,
    TryClearResult: QueueApplyTryClearResult,
    ApplySandbox: FnOnce(),
{
    run_queue_apply_after_preflight_with_log_sinks_lowered_queued_stage_from_sources(
        views,
        tx_source,
        view_source,
        preflight_result,
        inputs,
        trace,
        apply,
        |views, queued| {
            run_queue_apply_standard_queued_stage_with_log_sinks(
                views,
                queued,
                prepare_multitxn,
                run_preclaim,
                run_try_clear,
                apply_sandbox,
                |message| debug(message),
                |message| info(message),
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_after_preflight_with_log_messages_from_sources<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TxSource,
    ViewSource,
    TraceFn,
    ApplyFn,
    PrepareMultiTxn,
    RunPreclaim,
    RunTryClear,
    TryClearResult,
    ApplySandbox,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    tx_source: &TxSource,
    view_source: &ViewSource,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    inputs: QueueApplyAfterPreflightSourceInputs<'_>,
    trace: TraceFn,
    apply: ApplyFn,
    prepare_multitxn: PrepareMultiTxn,
    run_preclaim: RunPreclaim,
    run_try_clear: RunTryClear,
    apply_sandbox: ApplySandbox,
) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    ViewSource: QueueApplyObservedViewSource<Account>,
    TraceFn: FnMut(&str),
    ApplyFn: FnOnce() -> crate::ApplyResult,
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
    RunPreclaim: FnOnce(QueueApplyPreclaimViewSource) -> PreclaimResult<Tx, Journal, ParentBatchId>,
    RunTryClear: FnOnce() -> TryClearResult,
    TryClearResult: QueueApplyTryClearResult,
    ApplySandbox: FnOnce(),
{
    with_after_preflight_lowered_queue_inputs_from_sources(
        views,
        tx_source,
        view_source,
        preflight_result,
        inputs,
        |views, lowered| {
            run_queue_apply_after_preflight_with_direct_apply_and_caller_queued_stage_and_log_messages(
                views,
                lowered,
                preflight_result,
                trace,
                apply,
                |views, queued| {
                    run_queue_apply_standard_queued_stage_with_log_messages(
                        views,
                        queued,
                        prepare_multitxn,
                        run_preclaim,
                        run_try_clear,
                        apply_sandbox,
                    )
                },
            )
        },
    )
}

fn run_queue_apply_after_preflight_with_log_sinks_lowered_queued_stage_from_sources<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TxSource,
    ViewSource,
    TraceFn,
    ApplyFn,
    RunQueuedStage,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    tx_source: &TxSource,
    view_source: &ViewSource,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    inputs: QueueApplyAfterPreflightSourceInputs<'_>,
    trace: TraceFn,
    apply: ApplyFn,
    run_queued_stage: RunQueuedStage,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    ViewSource: QueueApplyObservedViewSource<Account>,
    TraceFn: FnMut(&str),
    ApplyFn: FnOnce() -> crate::ApplyResult,
    RunQueuedStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedQueuedStageInputs<'_, Account, Tx, Journal, ParentBatchId>,
    )
        -> crate::QueueApplyQueuedStage<Account, Tx, Journal, ParentBatchId>,
{
    run_queue_apply_after_preflight_with_lowered_queued_stage_from_sources(
        views,
        tx_source,
        view_source,
        preflight_result,
        inputs,
        trace,
        apply,
        run_queued_stage,
    )
}

fn run_queue_apply_after_preflight_with_trace_wrapped_direct_apply_and_caller_queued_stage_from_sources<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TxSource,
    ViewSource,
    TraceFn,
    ApplyFn,
    RunQueuedStage,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    tx_source: &TxSource,
    view_source: &ViewSource,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    inputs: QueueApplyAfterPreflightSourceInputs<'_>,
    trace: TraceFn,
    apply: ApplyFn,
    run_queued_stage: RunQueuedStage,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    ViewSource: QueueApplyObservedViewSource<Account>,
    TraceFn: FnMut(&str),
    ApplyFn: FnOnce() -> crate::ApplyResult,
    RunQueuedStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedQueuedStageInputs<'_, Account, Tx, Journal, ParentBatchId>,
    )
        -> crate::QueueApplyQueuedStage<Account, Tx, Journal, ParentBatchId>,
{
    run_queue_apply_after_preflight_with_caller_direct_apply_and_caller_queued_stage_from_sources(
        views,
        tx_source,
        view_source,
        preflight_result,
        inputs,
        |views, prepared| run_prepared_direct_apply_with_trace(views, prepared, trace, apply),
        run_queued_stage,
    )
}

fn run_queue_apply_standard_queued_stage<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    PrepareMultiTxn,
    RunPreclaim,
    RunTryClear,
    TryClearResult,
    ApplySandbox,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    queued: QueueApplyPreparedQueuedStageInputs<'_, Account, Tx, Journal, ParentBatchId>,
    prepare_multitxn: PrepareMultiTxn,
    run_preclaim: RunPreclaim,
    run_try_clear: RunTryClear,
    apply_sandbox: ApplySandbox,
) -> crate::QueueApplyQueuedStage<Account, Tx, Journal, ParentBatchId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
    RunPreclaim: FnOnce(QueueApplyPreclaimViewSource) -> PreclaimResult<Tx, Journal, ParentBatchId>,
    RunTryClear: FnOnce() -> TryClearResult,
    TryClearResult: QueueApplyTryClearResult,
    ApplySandbox: FnOnce(),
{
    run_queue_apply_queued_stage_with_fee_context(
        views,
        queued.account_seq_proxy,
        queued.tx_seq_proxy,
        queued.queued,
        queued.fee_context,
        queued.preflight_result,
        prepare_multitxn,
        run_preclaim,
        run_try_clear,
        apply_sandbox,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_queue_apply_standard_queued_stage_with_log_messages<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    PrepareMultiTxn,
    RunPreclaim,
    RunTryClear,
    TryClearResult,
    ApplySandbox,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    queued: QueueApplyPreparedQueuedStageInputs<'_, Account, Tx, Journal, ParentBatchId>,
    prepare_multitxn: PrepareMultiTxn,
    run_preclaim: RunPreclaim,
    run_try_clear: RunTryClear,
    apply_sandbox: ApplySandbox,
) -> QueueApplyQueuedStageWithLogMessagesResult<Account, Tx, Journal, ParentBatchId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
    RunPreclaim: FnOnce(QueueApplyPreclaimViewSource) -> PreclaimResult<Tx, Journal, ParentBatchId>,
    RunTryClear: FnOnce() -> TryClearResult,
    TryClearResult: QueueApplyTryClearResult,
    ApplySandbox: FnOnce(),
{
    run_queue_apply_queued_stage_with_log_messages(
        views,
        queued.queued.account,
        queued.account_seq_proxy,
        queued.tx_seq_proxy,
        queued.queued.preflight,
        queued.queued.is_blocker,
        queued.queued.open_ledger_seq,
        queued.queued.minimum_last_ledger_buffer,
        queued.queued.maximum_txn_per_account,
        queued.queued.retry_sequence_percent,
        queued.queued.queue_is_full,
        queued.fee_context.fee_level_paid,
        queued.fee_context.required_fee_level,
        queued.fee_context.base_level,
        queued.queued.balance_drops,
        queued.queued.reserve_drops,
        queued.queued.base_fee_drops,
        queued.queued.can_be_held_result,
        queued.queued.open_ledger_tx_count,
        queued.queued.tx_id,
        queued.queued.last_valid,
        queued.queued.flags,
        queued.preflight_result,
        queued.queued.order,
        prepare_multitxn,
        run_preclaim,
        run_try_clear,
        apply_sandbox,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_queue_apply_standard_queued_stage_with_log_sinks<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    PrepareMultiTxn,
    DebugFn,
    InfoFn,
    RunPreclaim,
    RunTryClear,
    TryClearResult,
    ApplySandbox,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    queued: QueueApplyPreparedQueuedStageInputs<'_, Account, Tx, Journal, ParentBatchId>,
    prepare_multitxn: PrepareMultiTxn,
    run_preclaim: RunPreclaim,
    run_try_clear: RunTryClear,
    apply_sandbox: ApplySandbox,
    mut debug: DebugFn,
    mut info: InfoFn,
) -> crate::QueueApplyQueuedStage<Account, Tx, Journal, ParentBatchId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
    DebugFn: FnMut(String),
    InfoFn: FnMut(String),
    RunPreclaim: FnOnce(QueueApplyPreclaimViewSource) -> PreclaimResult<Tx, Journal, ParentBatchId>,
    RunTryClear: FnOnce() -> TryClearResult,
    TryClearResult: QueueApplyTryClearResult,
    ApplySandbox: FnOnce(),
{
    run_queue_apply_queued_stage_with_fee_context_and_log_sinks(
        views,
        queued.account_seq_proxy,
        queued.tx_seq_proxy,
        queued.queued,
        queued.fee_context,
        queued.preflight_result,
        prepare_multitxn,
        run_preclaim,
        run_try_clear,
        apply_sandbox,
        |message| debug(message),
        |message| info(message),
    )
}

fn build_queue_apply_prepared_preclaim_inputs_from_flow<Account, Tx, Journal, ParentBatchId>(
    prepared: &QueueApplyPreparedFlowInputs<'_, Account, Tx, Journal, ParentBatchId>,
) -> QueueApplyPreparedPreclaimInputs<Account>
where
    Account: Clone,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
{
    QueueApplyPreparedPreclaimInputs::new(
        prepared.preclaim_view_source,
        prepared.fee_level_paid,
        prepared.base_level,
        prepared.required_fee_level,
        prepared.open_ledger_tx_count,
        prepared.tx_id.clone(),
        prepared.account.clone(),
    )
}

fn run_queue_apply_with_caller_preclaim_prepared_flow_stage<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    PrepareMultiTxn,
    RunFlowStage,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    queued: QueueApplyPreparedQueuedStageInputs<'_, Account, Tx, Journal, ParentBatchId>,
    prepare_multitxn: PrepareMultiTxn,
    run_flow_stage: RunFlowStage,
) -> crate::QueueApplyQueuedStage<Account, Tx, Journal, ParentBatchId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
    RunFlowStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedFlowInputs<'_, Account, Tx, Journal, ParentBatchId>,
    ) -> crate::QueueApplyFlowStage<Account, Tx, Journal, ParentBatchId>,
{
    run_prepared_queue_apply_queued_flow_stage(
        views,
        derive_queue_apply_prepared_flow_stage(
            &*views,
            queued.account_seq_proxy,
            queued.tx_seq_proxy,
            queued.queued,
            queued.fee_context,
            queued.preflight_result,
            prepare_multitxn,
        ),
        run_flow_stage,
    )
}

fn run_queue_apply_caller_preclaim_queued_stage<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    PrepareMultiTxn,
    RunPreclaimStage,
    RunTryClear,
    TryClearResult,
    ApplySandbox,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    queued: QueueApplyPreparedQueuedStageInputs<'_, Account, Tx, Journal, ParentBatchId>,
    prepare_multitxn: PrepareMultiTxn,
    run_preclaim_stage: RunPreclaimStage,
    run_try_clear: RunTryClear,
    apply_sandbox: ApplySandbox,
) -> crate::QueueApplyQueuedStage<Account, Tx, Journal, ParentBatchId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
    RunPreclaimStage:
        FnOnce(
            QueueApplyPreparedPreclaimInputs<Account>,
        )
            -> Result<QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>, crate::ApplyResult>,
    RunTryClear: FnOnce() -> TryClearResult,
    TryClearResult: QueueApplyTryClearResult,
    ApplySandbox: FnOnce(),
{
    run_queue_apply_with_caller_preclaim_prepared_flow_stage(
        views,
        queued,
        prepare_multitxn,
        |views, prepared| {
            let prepared_preclaim = build_queue_apply_prepared_preclaim_inputs_from_flow(&prepared);

            run_queue_apply_flow_stage_with_caller_preclaim(
                views,
                prepared.tx_seq_proxy,
                prepared.first_relevant_retries_remaining,
                prepared.hold_fallback,
                prepared.full_queue_decision,
                prepared.replaced,
                prepared.last_valid,
                prepared.flags,
                prepared.pf_result,
                prepared.order,
                prepared_preclaim,
                run_preclaim_stage,
                run_try_clear,
                apply_sandbox,
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
fn run_queue_apply_caller_preclaim_queued_stage_with_log_sinks<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    PrepareMultiTxn,
    DebugFn,
    InfoFn,
    RunPreclaimStage,
    RunTryClear,
    TryClearResult,
    ApplySandbox,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    queued: QueueApplyPreparedQueuedStageInputs<'_, Account, Tx, Journal, ParentBatchId>,
    prepare_multitxn: PrepareMultiTxn,
    run_preclaim_stage: RunPreclaimStage,
    run_try_clear: RunTryClear,
    apply_sandbox: ApplySandbox,
    mut debug: DebugFn,
    mut info: InfoFn,
) -> crate::QueueApplyQueuedStage<Account, Tx, Journal, ParentBatchId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
    DebugFn: FnMut(String),
    InfoFn: FnMut(String),
    RunPreclaimStage:
        FnOnce(
            QueueApplyPreparedPreclaimInputs<Account>,
        )
            -> Result<QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>, crate::ApplyResult>,
    RunTryClear: FnOnce() -> TryClearResult,
    TryClearResult: QueueApplyTryClearResult,
    ApplySandbox: FnOnce(),
{
    run_queue_apply_with_caller_preclaim_prepared_flow_stage(
        views,
        queued,
        prepare_multitxn,
        |views, prepared| {
            let prepared_preclaim = build_queue_apply_prepared_preclaim_inputs_from_flow(&prepared);
            let QueueApplyPreparedPreclaimInputs {
                view_source: preclaim_view_source,
                fee_level_paid,
                base_level,
                required_fee_level,
                open_ledger_tx_count,
                tx_id,
                account,
            } = prepared_preclaim;

            let preclaim = match run_preclaim_stage(QueueApplyPreparedPreclaimInputs::new(
                preclaim_view_source,
                fee_level_paid,
                base_level,
                required_fee_level,
                open_ledger_tx_count,
                tx_id.clone(),
                account.clone(),
            )) {
                Ok(stage) => stage,
                Err(result) => return crate::QueueApplyFlowStage::RejectPreclaim(result),
            };

            run_prepared_queue_apply_post_preclaim_stage_with_caller_queue(
                views,
                derive_queue_apply_prepared_post_preclaim_inputs(
                    prepared.tx_seq_proxy,
                    prepared.first_relevant_retries_remaining,
                    fee_level_paid,
                    base_level,
                    required_fee_level,
                    prepared.hold_fallback,
                    prepared.full_queue_decision,
                    prepared.replaced,
                    account,
                    tx_id,
                    prepared.last_valid,
                    prepared.flags,
                    prepared.pf_result,
                    prepared.order,
                    preclaim,
                ),
                |prepared| {
                    run_queue_apply_try_clear_stage(prepared.gate, run_try_clear, apply_sandbox)
                },
                |views, prepared| {
                    run_queue_apply_queue_stage_with_log_sinks(
                        views,
                        prepared.hold_fallback,
                        prepared.full_queue_decision,
                        prepared.replaced,
                        prepared.account,
                        prepared.tx_id,
                        prepared.last_valid,
                        prepared.seq_proxy,
                        prepared.fee_level,
                        prepared.flags,
                        prepared.pf_result,
                        prepared.order,
                        |message| debug(message),
                        |message| info(message),
                    )
                },
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
fn run_queue_apply_top_with_caller_preclaim_queued_stage_from_sources<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TxSource,
    ViewSource,
    RunPreflight,
    TraceFn,
    ApplyFn,
    RunQueuedStage,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    tx_source: &TxSource,
    view_source: &ViewSource,
    inputs: QueueApplyTopFromSourcesInputs<'_>,
    run_preflight: RunPreflight,
    trace: TraceFn,
    apply: ApplyFn,
    run_queued_stage: RunQueuedStage,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    ViewSource: QueueApplyObservedViewSource<Account>,
    RunPreflight: FnOnce() -> PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    TraceFn: FnMut(&str),
    ApplyFn: FnOnce() -> crate::ApplyResult,
    RunQueuedStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedQueuedStageInputs<'_, Account, Tx, Journal, ParentBatchId>,
    )
        -> crate::QueueApplyQueuedStage<Account, Tx, Journal, ParentBatchId>,
{
    run_queue_apply_top_with_trace_wrapped_direct_apply_and_caller_queued_stage_from_sources(
        views,
        tx_source,
        view_source,
        inputs,
        run_preflight,
        trace,
        apply,
        run_queued_stage,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_queue_apply_top_with_caller_preclaim_log_sinks_queued_stage_from_sources<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TxSource,
    ViewSource,
    RunPreflight,
    TraceFn,
    ApplyFn,
    RunQueuedStage,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    tx_source: &TxSource,
    view_source: &ViewSource,
    inputs: QueueApplyTopFromSourcesInputs<'_>,
    run_preflight: RunPreflight,
    trace: TraceFn,
    apply: ApplyFn,
    run_queued_stage: RunQueuedStage,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    ViewSource: QueueApplyObservedViewSource<Account>,
    RunPreflight: FnOnce() -> PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    TraceFn: FnMut(&str),
    ApplyFn: FnOnce() -> crate::ApplyResult,
    RunQueuedStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedQueuedStageInputs<'_, Account, Tx, Journal, ParentBatchId>,
    )
        -> crate::QueueApplyQueuedStage<Account, Tx, Journal, ParentBatchId>,
{
    run_queue_apply_top_with_caller_preclaim_queued_stage_from_sources(
        views,
        tx_source,
        view_source,
        inputs,
        run_preflight,
        trace,
        apply,
        run_queued_stage,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_top_with_caller_preclaim_from_sources<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TxSource,
    ViewSource,
    RunPreflight,
    TraceFn,
    ApplyFn,
    PrepareMultiTxn,
    RunPreclaimStage,
    RunTryClear,
    TryClearResult,
    ApplySandbox,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    tx_source: &TxSource,
    view_source: &ViewSource,
    inputs: QueueApplyTopFromSourcesInputs<'_>,
    run_preflight: RunPreflight,
    trace: TraceFn,
    apply: ApplyFn,
    prepare_multitxn: PrepareMultiTxn,
    run_preclaim_stage: RunPreclaimStage,
    run_try_clear: RunTryClear,
    apply_sandbox: ApplySandbox,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    ViewSource: QueueApplyObservedViewSource<Account>,
    RunPreflight: FnOnce() -> PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    TraceFn: FnMut(&str),
    ApplyFn: FnOnce() -> crate::ApplyResult,
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
    RunPreclaimStage:
        FnOnce(
            QueueApplyPreparedPreclaimInputs<Account>,
        )
            -> Result<QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>, crate::ApplyResult>,
    RunTryClear: FnOnce() -> TryClearResult,
    TryClearResult: QueueApplyTryClearResult,
    ApplySandbox: FnOnce(),
{
    run_queue_apply_top_with_caller_preclaim_log_sinks_queued_stage_from_sources(
        views,
        tx_source,
        view_source,
        inputs,
        run_preflight,
        trace,
        apply,
        |views, queued| {
            run_queue_apply_caller_preclaim_queued_stage(
                views,
                queued,
                prepare_multitxn,
                run_preclaim_stage,
                run_try_clear,
                apply_sandbox,
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_top_with_caller_preclaim_and_log_sinks_from_sources<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TxSource,
    ViewSource,
    RunPreflight,
    TraceFn,
    DebugFn,
    InfoFn,
    ApplyFn,
    PrepareMultiTxn,
    RunPreclaimStage,
    RunTryClear,
    TryClearResult,
    ApplySandbox,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    tx_source: &TxSource,
    view_source: &ViewSource,
    inputs: QueueApplyTopFromSourcesInputs<'_>,
    run_preflight: RunPreflight,
    trace: TraceFn,
    debug: DebugFn,
    info: InfoFn,
    apply: ApplyFn,
    prepare_multitxn: PrepareMultiTxn,
    run_preclaim_stage: RunPreclaimStage,
    run_try_clear: RunTryClear,
    apply_sandbox: ApplySandbox,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    ViewSource: QueueApplyObservedViewSource<Account>,
    RunPreflight: FnOnce() -> PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    TraceFn: FnMut(&str),
    DebugFn: FnMut(String),
    InfoFn: FnMut(String),
    ApplyFn: FnOnce() -> crate::ApplyResult,
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
    RunPreclaimStage:
        FnOnce(
            QueueApplyPreparedPreclaimInputs<Account>,
        )
            -> Result<QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>, crate::ApplyResult>,
    RunTryClear: FnOnce() -> TryClearResult,
    TryClearResult: QueueApplyTryClearResult,
    ApplySandbox: FnOnce(),
{
    run_queue_apply_top_with_caller_preclaim_queued_stage_from_sources(
        views,
        tx_source,
        view_source,
        inputs,
        run_preflight,
        trace,
        apply,
        |views, queued| {
            run_queue_apply_caller_preclaim_queued_stage_with_log_sinks(
                views,
                queued,
                prepare_multitxn,
                run_preclaim_stage,
                run_try_clear,
                apply_sandbox,
                debug,
                info,
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
fn run_queue_apply_after_preflight_with_caller_preclaim_queued_stage_from_sources<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TxSource,
    ViewSource,
    TraceFn,
    ApplyFn,
    RunQueuedStage,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    tx_source: &TxSource,
    view_source: &ViewSource,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    inputs: QueueApplyAfterPreflightSourceInputs<'_>,
    trace: TraceFn,
    apply: ApplyFn,
    run_queued_stage: RunQueuedStage,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    ViewSource: QueueApplyObservedViewSource<Account>,
    TraceFn: FnMut(&str),
    ApplyFn: FnOnce() -> crate::ApplyResult,
    RunQueuedStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedQueuedStageInputs<'_, Account, Tx, Journal, ParentBatchId>,
    )
        -> crate::QueueApplyQueuedStage<Account, Tx, Journal, ParentBatchId>,
{
    run_queue_apply_after_preflight_with_trace_wrapped_direct_apply_and_caller_queued_stage_from_sources(
        views,
        tx_source,
        view_source,
        preflight_result,
        inputs,
        trace,
        apply,
        run_queued_stage,
    )
}

#[allow(clippy::too_many_arguments)]
fn run_queue_apply_after_preflight_with_caller_preclaim_log_sinks_queued_stage_from_sources<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TxSource,
    ViewSource,
    TraceFn,
    ApplyFn,
    RunQueuedStage,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    tx_source: &TxSource,
    view_source: &ViewSource,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    inputs: QueueApplyAfterPreflightSourceInputs<'_>,
    trace: TraceFn,
    apply: ApplyFn,
    run_queued_stage: RunQueuedStage,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    ViewSource: QueueApplyObservedViewSource<Account>,
    TraceFn: FnMut(&str),
    ApplyFn: FnOnce() -> crate::ApplyResult,
    RunQueuedStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedQueuedStageInputs<'_, Account, Tx, Journal, ParentBatchId>,
    )
        -> crate::QueueApplyQueuedStage<Account, Tx, Journal, ParentBatchId>,
{
    run_queue_apply_after_preflight_with_caller_preclaim_queued_stage_from_sources(
        views,
        tx_source,
        view_source,
        preflight_result,
        inputs,
        trace,
        apply,
        run_queued_stage,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_after_preflight_with_caller_preclaim_from_sources<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TxSource,
    ViewSource,
    TraceFn,
    ApplyFn,
    PrepareMultiTxn,
    RunPreclaimStage,
    RunTryClear,
    TryClearResult,
    ApplySandbox,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    tx_source: &TxSource,
    view_source: &ViewSource,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    inputs: QueueApplyAfterPreflightSourceInputs<'_>,
    trace: TraceFn,
    apply: ApplyFn,
    prepare_multitxn: PrepareMultiTxn,
    run_preclaim_stage: RunPreclaimStage,
    run_try_clear: RunTryClear,
    apply_sandbox: ApplySandbox,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    ViewSource: QueueApplyObservedViewSource<Account>,
    TraceFn: FnMut(&str),
    ApplyFn: FnOnce() -> crate::ApplyResult,
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
    RunPreclaimStage:
        FnOnce(
            QueueApplyPreparedPreclaimInputs<Account>,
        )
            -> Result<QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>, crate::ApplyResult>,
    RunTryClear: FnOnce() -> TryClearResult,
    TryClearResult: QueueApplyTryClearResult,
    ApplySandbox: FnOnce(),
{
    run_queue_apply_after_preflight_with_caller_preclaim_log_sinks_queued_stage_from_sources(
        views,
        tx_source,
        view_source,
        preflight_result,
        inputs,
        trace,
        apply,
        |views, queued| {
            run_queue_apply_caller_preclaim_queued_stage(
                views,
                queued,
                prepare_multitxn,
                run_preclaim_stage,
                run_try_clear,
                apply_sandbox,
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_after_preflight_with_caller_preclaim_and_log_sinks_from_sources<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TxSource,
    ViewSource,
    TraceFn,
    DebugFn,
    InfoFn,
    ApplyFn,
    PrepareMultiTxn,
    RunPreclaimStage,
    RunTryClear,
    TryClearResult,
    ApplySandbox,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    tx_source: &TxSource,
    view_source: &ViewSource,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    inputs: QueueApplyAfterPreflightSourceInputs<'_>,
    trace: TraceFn,
    debug: DebugFn,
    info: InfoFn,
    apply: ApplyFn,
    prepare_multitxn: PrepareMultiTxn,
    run_preclaim_stage: RunPreclaimStage,
    run_try_clear: RunTryClear,
    apply_sandbox: ApplySandbox,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    ViewSource: QueueApplyObservedViewSource<Account>,
    TraceFn: FnMut(&str),
    DebugFn: FnMut(String),
    InfoFn: FnMut(String),
    ApplyFn: FnOnce() -> crate::ApplyResult,
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
    RunPreclaimStage:
        FnOnce(
            QueueApplyPreparedPreclaimInputs<Account>,
        )
            -> Result<QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>, crate::ApplyResult>,
    RunTryClear: FnOnce() -> TryClearResult,
    TryClearResult: QueueApplyTryClearResult,
    ApplySandbox: FnOnce(),
{
    run_queue_apply_after_preflight_with_caller_preclaim_queued_stage_from_sources(
        views,
        tx_source,
        view_source,
        preflight_result,
        inputs,
        trace,
        apply,
        |views, queued| {
            run_queue_apply_caller_preclaim_queued_stage_with_log_sinks(
                views,
                queued,
                prepare_multitxn,
                run_preclaim_stage,
                run_try_clear,
                apply_sandbox,
                debug,
                info,
            )
        },
    )
}

pub fn run_queue_apply_top_with_caller_direct_apply_and_queued_stage_from_sources<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TxSource,
    ViewSource,
    RunPreflight,
    RunDirectApply,
    PrepareMultiTxn,
    RunPreclaim,
    RunTryClear,
    TryClearResult,
    ApplySandbox,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    tx_source: &TxSource,
    view_source: &ViewSource,
    inputs: QueueApplyTopFromSourcesInputs<'_>,
    run_preflight: RunPreflight,
    run_direct_apply: RunDirectApply,
    prepare_multitxn: PrepareMultiTxn,
    run_preclaim: RunPreclaim,
    run_try_clear: RunTryClear,
    apply_sandbox: ApplySandbox,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    ViewSource: QueueApplyObservedViewSource<Account>,
    RunPreflight: FnOnce() -> PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    RunDirectApply: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        crate::PreparedDirectApply<'_, Account, TxId>,
    ) -> crate::DirectApplyExecution<Account, TxId>,
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
    RunPreclaim: FnOnce(QueueApplyPreclaimViewSource) -> PreclaimResult<Tx, Journal, ParentBatchId>,
    RunTryClear: FnOnce() -> TryClearResult,
    TryClearResult: QueueApplyTryClearResult,
    ApplySandbox: FnOnce(),
{
    run_queue_apply_top_with_caller_direct_apply_and_lowered_queued_stage_from_sources(
        views,
        tx_source,
        view_source,
        inputs,
        run_preflight,
        run_direct_apply,
        |views, queued| {
            run_queue_apply_standard_queued_stage(
                views,
                queued,
                prepare_multitxn,
                run_preclaim,
                run_try_clear,
                apply_sandbox,
            )
        },
    )
}

pub fn run_queue_apply_top_with_caller_direct_apply_and_lowered_queued_stage_from_sources<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TxSource,
    ViewSource,
    RunPreflight,
    RunDirectApply,
    RunQueuedStage,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    tx_source: &TxSource,
    view_source: &ViewSource,
    inputs: QueueApplyTopFromSourcesInputs<'_>,
    run_preflight: RunPreflight,
    run_direct_apply: RunDirectApply,
    run_queued_stage: RunQueuedStage,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    ViewSource: QueueApplyObservedViewSource<Account>,
    RunPreflight: FnOnce() -> PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    RunDirectApply: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        crate::PreparedDirectApply<'_, Account, TxId>,
    ) -> crate::DirectApplyExecution<Account, TxId>,
    RunQueuedStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedQueuedStageInputs<'_, Account, Tx, Journal, ParentBatchId>,
    )
        -> crate::QueueApplyQueuedStage<Account, Tx, Journal, ParentBatchId>,
{
    run_queue_apply_top_with_caller_direct_apply_and_caller_queued_stage_from_sources(
        views,
        tx_source,
        view_source,
        inputs,
        run_preflight,
        run_direct_apply,
        run_queued_stage,
    )
}

pub fn run_queue_apply_top_with_caller_direct_apply_and_caller_queued_stage_from_sources<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TxSource,
    ViewSource,
    RunPreflight,
    RunDirectApply,
    RunQueuedStage,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    tx_source: &TxSource,
    view_source: &ViewSource,
    inputs: QueueApplyTopFromSourcesInputs<'_>,
    run_preflight: RunPreflight,
    run_direct_apply: RunDirectApply,
    run_queued_stage: RunQueuedStage,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    ViewSource: QueueApplyObservedViewSource<Account>,
    RunPreflight: FnOnce() -> PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    RunDirectApply: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        crate::PreparedDirectApply<'_, Account, TxId>,
    ) -> crate::DirectApplyExecution<Account, TxId>,
    RunQueuedStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedQueuedStageInputs<'_, Account, Tx, Journal, ParentBatchId>,
    )
        -> crate::QueueApplyQueuedStage<Account, Tx, Journal, ParentBatchId>,
{
    run_queue_apply_top_with_caller_direct_apply_and_caller_queued_stage(
        views,
        build_queue_apply_top_with_queued_stage_inputs_from_sources(tx_source, view_source, inputs),
        run_preflight,
        run_direct_apply,
        run_queued_stage,
    )
}

pub fn run_queue_apply_after_preflight_with_direct_apply_from_sources<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TxSource,
    ViewSource,
    TraceFn,
    ApplyFn,
    PrepareMultiTxn,
    RunPreclaim,
    RunTryClear,
    TryClearResult,
    ApplySandbox,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    tx_source: &TxSource,
    view_source: &ViewSource,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    inputs: QueueApplyAfterPreflightSourceInputs<'_>,
    trace: TraceFn,
    apply: ApplyFn,
    prepare_multitxn: PrepareMultiTxn,
    run_preclaim: RunPreclaim,
    run_try_clear: RunTryClear,
    apply_sandbox: ApplySandbox,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    ViewSource: QueueApplyObservedViewSource<Account>,
    TraceFn: FnMut(&str),
    ApplyFn: FnOnce() -> crate::ApplyResult,
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
    RunPreclaim: FnOnce(QueueApplyPreclaimViewSource) -> PreclaimResult<Tx, Journal, ParentBatchId>,
    RunTryClear: FnOnce() -> TryClearResult,
    TryClearResult: QueueApplyTryClearResult,
    ApplySandbox: FnOnce(),
{
    run_queue_apply_after_preflight_with_trace_wrapped_direct_apply_and_caller_queued_stage_from_sources(
        views,
        tx_source,
        view_source,
        preflight_result,
        inputs,
        trace,
        apply,
        |views, queued| {
            run_queue_apply_standard_queued_stage(
                views,
                queued,
                prepare_multitxn,
                run_preclaim,
                run_try_clear,
                apply_sandbox,
            )
        },
    )
}

pub fn run_queue_apply_after_preflight_with_caller_direct_apply_from_sources<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TxSource,
    ViewSource,
    RunDirectApply,
    PrepareMultiTxn,
    RunPreclaim,
    RunTryClear,
    TryClearResult,
    ApplySandbox,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    tx_source: &TxSource,
    view_source: &ViewSource,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    inputs: QueueApplyAfterPreflightSourceInputs<'_>,
    run_direct_apply: RunDirectApply,
    prepare_multitxn: PrepareMultiTxn,
    run_preclaim: RunPreclaim,
    run_try_clear: RunTryClear,
    apply_sandbox: ApplySandbox,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    ViewSource: QueueApplyObservedViewSource<Account>,
    RunDirectApply: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        crate::PreparedDirectApply<'_, Account, TxId>,
    ) -> crate::DirectApplyExecution<Account, TxId>,
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
    RunPreclaim: FnOnce(QueueApplyPreclaimViewSource) -> PreclaimResult<Tx, Journal, ParentBatchId>,
    RunTryClear: FnOnce() -> TryClearResult,
    TryClearResult: QueueApplyTryClearResult,
    ApplySandbox: FnOnce(),
{
    run_queue_apply_after_preflight_with_caller_direct_apply_and_lowered_queued_stage_from_sources(
        views,
        tx_source,
        view_source,
        preflight_result,
        inputs,
        run_direct_apply,
        |views, queued| {
            run_queue_apply_standard_queued_stage(
                views,
                queued,
                prepare_multitxn,
                run_preclaim,
                run_try_clear,
                apply_sandbox,
            )
        },
    )
}

pub fn run_queue_apply_after_preflight_with_caller_direct_apply_and_lowered_queued_stage_from_sources<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TxSource,
    ViewSource,
    RunDirectApply,
    RunQueuedStage,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    tx_source: &TxSource,
    view_source: &ViewSource,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    inputs: QueueApplyAfterPreflightSourceInputs<'_>,
    run_direct_apply: RunDirectApply,
    run_queued_stage: RunQueuedStage,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    ViewSource: QueueApplyObservedViewSource<Account>,
    RunDirectApply: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        crate::PreparedDirectApply<'_, Account, TxId>,
    ) -> crate::DirectApplyExecution<Account, TxId>,
    RunQueuedStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedQueuedStageInputs<'_, Account, Tx, Journal, ParentBatchId>,
    )
        -> crate::QueueApplyQueuedStage<Account, Tx, Journal, ParentBatchId>,
{
    run_queue_apply_after_preflight_with_caller_direct_apply_and_caller_queued_stage_from_sources(
        views,
        tx_source,
        view_source,
        preflight_result,
        inputs,
        run_direct_apply,
        run_queued_stage,
    )
}

pub fn run_queue_apply_after_preflight_with_direct_apply_and_caller_queued_stage_from_sources<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TxSource,
    ViewSource,
    TraceFn,
    ApplyFn,
    RunQueuedStage,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    tx_source: &TxSource,
    view_source: &ViewSource,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    inputs: QueueApplyAfterPreflightSourceInputs<'_>,
    trace: TraceFn,
    apply: ApplyFn,
    run_queued_stage: RunQueuedStage,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    ViewSource: QueueApplyObservedViewSource<Account>,
    TraceFn: FnMut(&str),
    ApplyFn: FnOnce() -> crate::ApplyResult,
    RunQueuedStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedQueuedStageInputs<'_, Account, Tx, Journal, ParentBatchId>,
    )
        -> crate::QueueApplyQueuedStage<Account, Tx, Journal, ParentBatchId>,
{
    with_after_preflight_lowered_queue_inputs_from_sources(
        views,
        tx_source,
        view_source,
        preflight_result,
        inputs,
        |views, lowered| {
            run_queue_apply_after_preflight_with_direct_apply_and_caller_queued_stage(
                views,
                lowered,
                preflight_result,
                trace,
                apply,
                run_queued_stage,
            )
        },
    )
}

pub fn run_queue_apply_after_preflight_with_caller_direct_apply_and_caller_queued_stage_from_sources<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TxSource,
    ViewSource,
    RunDirectApply,
    RunQueuedStage,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    tx_source: &TxSource,
    view_source: &ViewSource,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    inputs: QueueApplyAfterPreflightSourceInputs<'_>,
    run_direct_apply: RunDirectApply,
    run_queued_stage: RunQueuedStage,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    ViewSource: QueueApplyObservedViewSource<Account>,
    RunDirectApply: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        crate::PreparedDirectApply<'_, Account, TxId>,
    ) -> crate::DirectApplyExecution<Account, TxId>,
    RunQueuedStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedQueuedStageInputs<'_, Account, Tx, Journal, ParentBatchId>,
    )
        -> crate::QueueApplyQueuedStage<Account, Tx, Journal, ParentBatchId>,
{
    with_after_preflight_lowered_queue_inputs_from_sources(
        views,
        tx_source,
        view_source,
        preflight_result,
        inputs,
        |views, lowered| {
            run_queue_apply_after_preflight_with_caller_direct_apply_and_caller_queued_stage(
                views,
                lowered,
                preflight_result,
                run_direct_apply,
                run_queued_stage,
            )
        },
    )
}

pub fn run_queue_apply_after_preflight_with_acquired_direct_apply_from_sources<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TxSource,
    ViewSource,
    PrepareMultiTxn,
    RunPreclaim,
    RunTryClear,
    TryClearResult,
    ApplySandbox,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    tx_source: &TxSource,
    view_source: &ViewSource,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    inputs: QueueApplyAfterPreflightSourceInputs<'_>,
    direct_applied: Option<DirectApplyExecution<Account, TxId>>,
    prepare_multitxn: PrepareMultiTxn,
    run_preclaim: RunPreclaim,
    run_try_clear: RunTryClear,
    apply_sandbox: ApplySandbox,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    ViewSource: QueueApplyObservedViewSource<Account>,
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
    RunPreclaim: FnOnce(QueueApplyPreclaimViewSource) -> PreclaimResult<Tx, Journal, ParentBatchId>,
    RunTryClear: FnOnce() -> TryClearResult,
    TryClearResult: QueueApplyTryClearResult,
    ApplySandbox: FnOnce(),
{
    run_queue_apply_after_preflight_with_acquired_direct_apply_and_lowered_queued_stage_from_sources(
        views,
        tx_source,
        view_source,
        preflight_result,
        inputs,
        direct_applied,
        |views, queued| {
            run_queue_apply_standard_queued_stage(
                views,
                queued,
                prepare_multitxn,
                run_preclaim,
                run_try_clear,
                apply_sandbox,
            )
        },
    )
}

pub fn run_queue_apply_after_preflight_with_acquired_direct_apply_and_lowered_queued_stage_from_sources<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TxSource,
    ViewSource,
    RunQueuedStage,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    tx_source: &TxSource,
    view_source: &ViewSource,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    inputs: QueueApplyAfterPreflightSourceInputs<'_>,
    direct_applied: Option<DirectApplyExecution<Account, TxId>>,
    run_queued_stage: RunQueuedStage,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    ViewSource: QueueApplyObservedViewSource<Account>,
    RunQueuedStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedQueuedStageInputs<'_, Account, Tx, Journal, ParentBatchId>,
    )
        -> crate::QueueApplyQueuedStage<Account, Tx, Journal, ParentBatchId>,
{
    run_queue_apply_after_preflight_with_acquired_direct_apply_and_caller_queued_stage_from_sources(
        views,
        tx_source,
        view_source,
        preflight_result,
        inputs,
        direct_applied,
        run_queued_stage,
    )
}

pub fn run_queue_apply_after_preflight_with_acquired_direct_apply_and_caller_queued_stage_from_sources<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TxSource,
    ViewSource,
    RunQueuedStage,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    tx_source: &TxSource,
    view_source: &ViewSource,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    inputs: QueueApplyAfterPreflightSourceInputs<'_>,
    direct_applied: Option<DirectApplyExecution<Account, TxId>>,
    run_queued_stage: RunQueuedStage,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    ViewSource: QueueApplyObservedViewSource<Account>,
    RunQueuedStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedQueuedStageInputs<'_, Account, Tx, Journal, ParentBatchId>,
    )
        -> crate::QueueApplyQueuedStage<Account, Tx, Journal, ParentBatchId>,
{
    with_after_preflight_lowered_queue_inputs_from_sources(
        views,
        tx_source,
        view_source,
        preflight_result,
        inputs,
        |views, lowered| {
            run_queue_apply_after_preflight_with_acquired_direct_apply_and_caller_queued_stage(
                views,
                lowered,
                preflight_result,
                direct_applied,
                run_queued_stage,
            )
        },
    )
}

#[cfg(test)]
mod tests;
