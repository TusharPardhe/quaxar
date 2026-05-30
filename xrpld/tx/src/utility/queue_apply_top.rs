//! Public top-level Rust carrier for the current `TxQ::apply(...)` seam.
//!
//! This shapes the already-landed Rust seam so the top call site can pass one
//! structured input bundle while preserving the the reference implementation branch order:
//! 1. enter the transaction-apply runtime,
//! 2. run `preflight(...)`,
//! 3. lazily derive the shared fee context when direct apply or queueing
//!    actually needs it,
//! 4. attempt direct apply,
//! 5. enforce account and ticket prerequisites,
//! 6. delegate into the queued-path carrier.

use std::cell::OnceCell;
use std::fmt::Display;

use protocol::{Rules, SeqProxy};

use crate::{
    DirectApplyExecution, MaybeTx, PreclaimResult, PreflightResult, QueueApplyFeeContext,
    QueueApplyFeeContextInputs, QueueApplyPreclaimViewSource, QueueApplyPreflightStage,
    QueueApplyPreflightStageWithLogMessagesResult, QueueApplyPreflightWithDirectApplyInputs,
    QueueApplyQueueLogMessages, QueueApplyQueuedStage, QueueApplyQueuedStageWithLogMessagesResult,
    QueueApplyQueuedWithFeeContextInputs, QueueApplyTryClearResult, QueueApplyViewAdjustment,
    QueueViews, TxConsequences, evaluate_queue_apply_fee_context,
    run_prepared_direct_apply_with_trace,
    run_queue_apply_after_preflight_with_acquired_direct_apply,
    run_queue_apply_after_preflight_with_acquired_direct_apply_and_log_messages,
    run_queue_apply_after_preflight_with_caller_direct_apply,
    run_queue_apply_after_preflight_with_caller_direct_apply_and_log_messages,
    run_queue_apply_preflight_stage_with_log_messages, run_queue_apply_preflight_with_direct_apply,
    run_queue_apply_preflight_with_direct_apply_and_log_messages,
    run_queue_apply_queued_stage_with_fee_context, run_queue_apply_queued_stage_with_log_messages,
    run_queue_apply_with_runtime, with_transaction_apply_runtime,
};

#[derive(Debug, Clone, Copy)]
pub struct QueueApplyCallInputs<'a> {
    pub rules: &'a Rules,
    pub account_exists: bool,
    pub account_seq_proxy: SeqProxy,
    pub tx_seq_proxy: SeqProxy,
    pub ticket_exists: bool,
}

impl<'a> QueueApplyCallInputs<'a> {
    pub fn new(
        rules: &'a Rules,
        account_exists: bool,
        account_seq_proxy: SeqProxy,
        tx_seq_proxy: SeqProxy,
        ticket_exists: bool,
    ) -> Self {
        Self {
            rules,
            account_exists,
            account_seq_proxy,
            tx_seq_proxy,
            ticket_exists,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct QueueApplyTopWithFeeContextInputs<'a> {
    pub call: QueueApplyCallInputs<'a>,
    pub fee_context_inputs: QueueApplyFeeContextInputs,
}

impl<'a> QueueApplyTopWithFeeContextInputs<'a> {
    pub fn new(
        call: QueueApplyCallInputs<'a>,
        fee_context_inputs: QueueApplyFeeContextInputs,
    ) -> Self {
        Self {
            call,
            fee_context_inputs,
        }
    }
}

#[derive(Debug, Clone)]
pub struct QueueApplyTopWithDirectApplyInputs<'a, Account, TxId> {
    pub top: QueueApplyTopWithFeeContextInputs<'a>,
    pub transaction_id: TxId,
    pub applied_account: &'a Account,
}

impl<'a, Account, TxId> QueueApplyTopWithDirectApplyInputs<'a, Account, TxId> {
    pub fn new(
        top: QueueApplyTopWithFeeContextInputs<'a>,
        transaction_id: TxId,
        applied_account: &'a Account,
    ) -> Self {
        Self {
            top,
            transaction_id,
            applied_account,
        }
    }
}

#[derive(Debug, Clone)]
pub struct QueueApplyTopWithQueuedStageInputs<'a, Account, TxId> {
    pub direct_apply: QueueApplyTopWithDirectApplyInputs<'a, Account, TxId>,
    pub queued: QueueApplyQueuedWithFeeContextInputs<'a, Account>,
}

impl<'a, Account, TxId> QueueApplyTopWithQueuedStageInputs<'a, Account, TxId> {
    pub fn new(
        direct_apply: QueueApplyTopWithDirectApplyInputs<'a, Account, TxId>,
        queued: QueueApplyQueuedWithFeeContextInputs<'a, Account>,
    ) -> Self {
        Self {
            direct_apply,
            queued,
        }
    }
}

#[derive(Debug, Clone)]
pub struct QueueApplyPreparedQueuedStageInputs<'a, Account, Tx, Journal, ParentBatchId> {
    pub account_seq_proxy: SeqProxy,
    pub tx_seq_proxy: SeqProxy,
    pub queued: QueueApplyQueuedWithFeeContextInputs<'a, Account>,
    pub fee_context: QueueApplyFeeContext,
    pub preflight_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
}

impl<'a, Account, Tx, Journal, ParentBatchId>
    QueueApplyPreparedQueuedStageInputs<'a, Account, Tx, Journal, ParentBatchId>
{
    pub fn new(
        account_seq_proxy: SeqProxy,
        tx_seq_proxy: SeqProxy,
        queued: QueueApplyQueuedWithFeeContextInputs<'a, Account>,
        fee_context: QueueApplyFeeContext,
        preflight_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    ) -> Self {
        Self {
            account_seq_proxy,
            tx_seq_proxy,
            queued,
            fee_context,
            preflight_result,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId> {
    pub stage: QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>,
    pub queue_log_messages: QueueApplyQueueLogMessages,
}

fn into_queue_apply_top_with_log_messages_result<Account, Tx, Journal, ParentBatchId, TxId>(
    result: QueueApplyPreflightStageWithLogMessagesResult<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
    >,
) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId> {
    QueueApplyTopWithLogMessagesResult {
        stage: result.stage,
        queue_log_messages: result.queue_log_messages,
    }
}

fn into_queue_apply_prepared_queued_stage_inputs<'a, Account, Tx, Journal, ParentBatchId>(
    queued: QueueApplyQueuedWithFeeContextInputs<'a, Account>,
    account_seq_proxy: SeqProxy,
    tx_seq_proxy: SeqProxy,
    fee_context: QueueApplyFeeContext,
    preflight_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
) -> QueueApplyPreparedQueuedStageInputs<'a, Account, Tx, Journal, ParentBatchId> {
    QueueApplyPreparedQueuedStageInputs::new(
        account_seq_proxy,
        tx_seq_proxy,
        queued,
        fee_context,
        preflight_result,
    )
}

pub fn run_queue_apply_top<
    Account,
    Tx,
    Consequences,
    Journal,
    ParentBatchId,
    TxId,
    RunPreflight,
    RunDirectApply,
    RunQueuedStage,
>(
    inputs: QueueApplyCallInputs<'_>,
    run_preflight: RunPreflight,
    run_direct_apply: RunDirectApply,
    run_queued_stage: RunQueuedStage,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    RunPreflight: FnOnce() -> PreflightResult<Tx, Consequences, Journal, ParentBatchId>,
    RunDirectApply: FnOnce() -> Option<DirectApplyExecution<Account, TxId>>,
    RunQueuedStage: FnOnce() -> QueueApplyQueuedStage<Account, Tx, Journal, ParentBatchId>,
{
    run_queue_apply_with_runtime(
        inputs.rules,
        run_preflight,
        inputs.account_exists,
        inputs.account_seq_proxy,
        inputs.tx_seq_proxy,
        inputs.ticket_exists,
        run_direct_apply,
        run_queued_stage,
    )
}

pub fn run_queue_apply_top_with_log_messages<
    Account,
    Tx,
    Consequences,
    Journal,
    ParentBatchId,
    TxId,
    RunPreflight,
    RunDirectApply,
    RunQueuedStage,
>(
    inputs: QueueApplyCallInputs<'_>,
    run_preflight: RunPreflight,
    run_direct_apply: RunDirectApply,
    run_queued_stage: RunQueuedStage,
) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
where
    RunPreflight: FnOnce() -> PreflightResult<Tx, Consequences, Journal, ParentBatchId>,
    RunDirectApply: FnOnce() -> Option<DirectApplyExecution<Account, TxId>>,
    RunQueuedStage:
        FnOnce() -> QueueApplyQueuedStageWithLogMessagesResult<Account, Tx, Journal, ParentBatchId>,
{
    into_queue_apply_top_with_log_messages_result(with_transaction_apply_runtime(
        inputs.rules,
        || {
            let preflight_result = run_preflight();
            run_queue_apply_preflight_stage_with_log_messages(
                &preflight_result,
                inputs.account_exists,
                inputs.account_seq_proxy,
                inputs.tx_seq_proxy,
                inputs.ticket_exists,
                run_direct_apply,
                run_queued_stage,
            )
        },
    ))
}

pub fn run_queue_apply_top_with_fee_context<
    Account,
    Tx,
    Consequences,
    Journal,
    ParentBatchId,
    TxId,
    RunPreflight,
    RunDirectApply,
    RunQueuedStage,
>(
    inputs: QueueApplyTopWithFeeContextInputs<'_>,
    run_preflight: RunPreflight,
    run_direct_apply: RunDirectApply,
    run_queued_stage: RunQueuedStage,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    RunPreflight: FnOnce() -> PreflightResult<Tx, Consequences, Journal, ParentBatchId>,
    RunDirectApply: FnOnce(QueueApplyFeeContext) -> Option<DirectApplyExecution<Account, TxId>>,
    RunQueuedStage:
        FnOnce(QueueApplyFeeContext) -> QueueApplyQueuedStage<Account, Tx, Journal, ParentBatchId>,
{
    let fee_context = OnceCell::new();

    run_queue_apply_top(
        inputs.call,
        run_preflight,
        || {
            run_direct_apply(
                *fee_context
                    .get_or_init(|| evaluate_queue_apply_fee_context(inputs.fee_context_inputs)),
            )
        },
        || {
            run_queued_stage(
                *fee_context
                    .get_or_init(|| evaluate_queue_apply_fee_context(inputs.fee_context_inputs)),
            )
        },
    )
}

pub fn run_queue_apply_top_with_fee_context_and_log_messages<
    Account,
    Tx,
    Consequences,
    Journal,
    ParentBatchId,
    TxId,
    RunPreflight,
    RunDirectApply,
    RunQueuedStage,
>(
    inputs: QueueApplyTopWithFeeContextInputs<'_>,
    run_preflight: RunPreflight,
    run_direct_apply: RunDirectApply,
    run_queued_stage: RunQueuedStage,
) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
where
    RunPreflight: FnOnce() -> PreflightResult<Tx, Consequences, Journal, ParentBatchId>,
    RunDirectApply: FnOnce(QueueApplyFeeContext) -> Option<DirectApplyExecution<Account, TxId>>,
    RunQueuedStage:
        FnOnce(
            QueueApplyFeeContext,
        )
            -> QueueApplyQueuedStageWithLogMessagesResult<Account, Tx, Journal, ParentBatchId>,
{
    let fee_context = OnceCell::new();

    run_queue_apply_top_with_log_messages(
        inputs.call,
        run_preflight,
        || {
            run_direct_apply(
                *fee_context
                    .get_or_init(|| evaluate_queue_apply_fee_context(inputs.fee_context_inputs)),
            )
        },
        || {
            run_queued_stage(
                *fee_context
                    .get_or_init(|| evaluate_queue_apply_fee_context(inputs.fee_context_inputs)),
            )
        },
    )
}

pub fn run_queue_apply_top_with_direct_apply<
    Account,
    T,
    Tx,
    Consequences,
    Journal,
    ParentBatchId,
    TxId,
    RunPreflight,
    TraceFn,
    ApplyFn,
    RunQueuedStage,
>(
    views: &mut QueueViews<Account, T>,
    inputs: QueueApplyTopWithDirectApplyInputs<'_, Account, TxId>,
    run_preflight: RunPreflight,
    trace: TraceFn,
    apply: ApplyFn,
    run_queued_stage: RunQueuedStage,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    TxId: Clone + Display,
    RunPreflight: FnOnce() -> PreflightResult<Tx, Consequences, Journal, ParentBatchId>,
    TraceFn: FnMut(&str),
    ApplyFn: FnOnce() -> crate::ApplyResult,
    RunQueuedStage: FnOnce(
        &mut QueueViews<Account, T>,
        QueueApplyFeeContext,
    ) -> QueueApplyQueuedStage<Account, Tx, Journal, ParentBatchId>,
{
    with_transaction_apply_runtime(inputs.top.call.rules, || {
        let preflight_result = run_preflight();
        run_queue_apply_preflight_with_direct_apply(
            &preflight_result,
            views,
            QueueApplyPreflightWithDirectApplyInputs::new(
                inputs.transaction_id,
                inputs.top.call.account_exists,
                inputs.top.call.account_seq_proxy,
                inputs.top.call.tx_seq_proxy,
                inputs.top.call.ticket_exists,
                inputs.top.fee_context_inputs,
                inputs.applied_account,
            ),
            trace,
            apply,
            run_queued_stage,
        )
    })
}

pub fn run_queue_apply_top_with_direct_apply_and_log_messages<
    Account,
    T,
    Tx,
    Consequences,
    Journal,
    ParentBatchId,
    TxId,
    RunPreflight,
    TraceFn,
    ApplyFn,
    RunQueuedStage,
>(
    views: &mut QueueViews<Account, T>,
    inputs: QueueApplyTopWithDirectApplyInputs<'_, Account, TxId>,
    run_preflight: RunPreflight,
    trace: TraceFn,
    apply: ApplyFn,
    run_queued_stage: RunQueuedStage,
) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    TxId: Clone + Display,
    RunPreflight: FnOnce() -> PreflightResult<Tx, Consequences, Journal, ParentBatchId>,
    TraceFn: FnMut(&str),
    ApplyFn: FnOnce() -> crate::ApplyResult,
    RunQueuedStage:
        FnOnce(
            &mut QueueViews<Account, T>,
            QueueApplyFeeContext,
        )
            -> QueueApplyQueuedStageWithLogMessagesResult<Account, Tx, Journal, ParentBatchId>,
{
    into_queue_apply_top_with_log_messages_result(with_transaction_apply_runtime(
        inputs.top.call.rules,
        || {
            let preflight_result = run_preflight();
            run_queue_apply_preflight_with_direct_apply_and_log_messages(
                &preflight_result,
                views,
                QueueApplyPreflightWithDirectApplyInputs::new(
                    inputs.transaction_id,
                    inputs.top.call.account_exists,
                    inputs.top.call.account_seq_proxy,
                    inputs.top.call.tx_seq_proxy,
                    inputs.top.call.ticket_exists,
                    inputs.top.fee_context_inputs,
                    inputs.applied_account,
                ),
                trace,
                apply,
                run_queued_stage,
            )
        },
    ))
}

pub fn run_queue_apply_top_with_queued_stage<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
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
    inputs: QueueApplyTopWithQueuedStageInputs<'_, Account, TxId>,
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
    RunPreflight: FnOnce() -> PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    TraceFn: FnMut(&str),
    ApplyFn: FnOnce() -> crate::ApplyResult,
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
    RunPreclaim: FnOnce(QueueApplyPreclaimViewSource) -> PreclaimResult<Tx, Journal, ParentBatchId>,
    RunTryClear: FnOnce() -> TryClearResult,
    TryClearResult: QueueApplyTryClearResult,
    ApplySandbox: FnOnce(),
{
    run_queue_apply_top_with_caller_direct_apply_and_caller_queued_stage(
        views,
        inputs,
        run_preflight,
        |views, prepared| run_prepared_direct_apply_with_trace(views, prepared, trace, apply),
        |views, queued| {
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
        },
    )
}

pub fn run_queue_apply_top_with_queued_stage_and_log_messages<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
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
    inputs: QueueApplyTopWithQueuedStageInputs<'_, Account, TxId>,
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
    RunPreflight: FnOnce() -> PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    TraceFn: FnMut(&str),
    ApplyFn: FnOnce() -> crate::ApplyResult,
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
    RunPreclaim: FnOnce(QueueApplyPreclaimViewSource) -> PreclaimResult<Tx, Journal, ParentBatchId>,
    RunTryClear: FnOnce() -> TryClearResult,
    TryClearResult: QueueApplyTryClearResult,
    ApplySandbox: FnOnce(),
{
    run_queue_apply_top_with_caller_direct_apply_and_queued_stage_and_log_messages(
        views,
        inputs,
        run_preflight,
        |views, prepared| run_prepared_direct_apply_with_trace(views, prepared, trace, apply),
        prepare_multitxn,
        run_preclaim,
        run_try_clear,
        apply_sandbox,
    )
}

pub fn run_queue_apply_after_preflight_with_queued_stage<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TraceFn,
    ApplyFn,
    PrepareMultiTxn,
    RunPreclaim,
    RunTryClear,
    TryClearResult,
    ApplySandbox,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    inputs: QueueApplyTopWithQueuedStageInputs<'_, Account, TxId>,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
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
    TraceFn: FnMut(&str),
    ApplyFn: FnOnce() -> crate::ApplyResult,
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
    RunPreclaim: FnOnce(QueueApplyPreclaimViewSource) -> PreclaimResult<Tx, Journal, ParentBatchId>,
    RunTryClear: FnOnce() -> TryClearResult,
    TryClearResult: QueueApplyTryClearResult,
    ApplySandbox: FnOnce(),
{
    run_queue_apply_after_preflight_with_direct_apply_and_caller_queued_stage(
        views,
        inputs,
        preflight_result,
        trace,
        apply,
        |views, queued| {
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
        },
    )
}

pub fn run_queue_apply_after_preflight_with_queued_stage_and_log_messages<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TraceFn,
    ApplyFn,
    PrepareMultiTxn,
    RunPreclaim,
    RunTryClear,
    TryClearResult,
    ApplySandbox,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    inputs: QueueApplyTopWithQueuedStageInputs<'_, Account, TxId>,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
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
    TraceFn: FnMut(&str),
    ApplyFn: FnOnce() -> crate::ApplyResult,
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
    RunPreclaim: FnOnce(QueueApplyPreclaimViewSource) -> PreclaimResult<Tx, Journal, ParentBatchId>,
    RunTryClear: FnOnce() -> TryClearResult,
    TryClearResult: QueueApplyTryClearResult,
    ApplySandbox: FnOnce(),
{
    run_queue_apply_after_preflight_with_direct_apply_and_caller_queued_stage_and_log_messages(
        views,
        inputs,
        preflight_result,
        trace,
        apply,
        |views, queued| {
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
        },
    )
}

pub fn run_queue_apply_after_preflight_with_direct_apply_and_queued_stage<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TraceFn,
    ApplyFn,
    PrepareMultiTxn,
    RunPreclaim,
    RunTryClear,
    TryClearResult,
    ApplySandbox,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    inputs: QueueApplyTopWithQueuedStageInputs<'_, Account, TxId>,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
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
    TraceFn: FnMut(&str),
    ApplyFn: FnOnce() -> crate::ApplyResult,
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
    RunPreclaim: FnOnce(QueueApplyPreclaimViewSource) -> PreclaimResult<Tx, Journal, ParentBatchId>,
    RunTryClear: FnOnce() -> TryClearResult,
    TryClearResult: QueueApplyTryClearResult,
    ApplySandbox: FnOnce(),
{
    run_queue_apply_after_preflight_with_direct_apply_and_caller_queued_stage(
        views,
        inputs,
        preflight_result,
        trace,
        apply,
        |views, queued| {
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
        },
    )
}

pub fn run_queue_apply_after_preflight_with_direct_apply_and_queued_stage_and_log_messages<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TraceFn,
    ApplyFn,
    PrepareMultiTxn,
    RunPreclaim,
    RunTryClear,
    TryClearResult,
    ApplySandbox,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    inputs: QueueApplyTopWithQueuedStageInputs<'_, Account, TxId>,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
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
    TraceFn: FnMut(&str),
    ApplyFn: FnOnce() -> crate::ApplyResult,
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
    RunPreclaim: FnOnce(QueueApplyPreclaimViewSource) -> PreclaimResult<Tx, Journal, ParentBatchId>,
    RunTryClear: FnOnce() -> TryClearResult,
    TryClearResult: QueueApplyTryClearResult,
    ApplySandbox: FnOnce(),
{
    run_queue_apply_after_preflight_with_direct_apply_and_caller_queued_stage_and_log_messages(
        views,
        inputs,
        preflight_result,
        trace,
        apply,
        |views, queued| {
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
        },
    )
}

pub fn run_queue_apply_top_with_caller_direct_apply_and_queued_stage<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    RunPreflight,
    RunDirectApply,
    PrepareMultiTxn,
    RunPreclaim,
    RunTryClear,
    TryClearResult,
    ApplySandbox,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    inputs: QueueApplyTopWithQueuedStageInputs<'_, Account, TxId>,
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
    run_queue_apply_top_with_caller_direct_apply_and_caller_queued_stage(
        views,
        inputs,
        run_preflight,
        run_direct_apply,
        |views, queued| {
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
        },
    )
}

pub fn run_queue_apply_top_with_caller_direct_apply_and_queued_stage_and_log_messages<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    RunPreflight,
    RunDirectApply,
    PrepareMultiTxn,
    RunPreclaim,
    RunTryClear,
    TryClearResult,
    ApplySandbox,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    inputs: QueueApplyTopWithQueuedStageInputs<'_, Account, TxId>,
    run_preflight: RunPreflight,
    run_direct_apply: RunDirectApply,
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
    run_queue_apply_top_with_caller_direct_apply_and_caller_queued_stage_and_log_messages(
        views,
        inputs,
        run_preflight,
        run_direct_apply,
        |views, queued| {
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
        },
    )
}

pub fn run_queue_apply_top_with_caller_direct_apply_and_caller_queued_stage<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    RunPreflight,
    RunDirectApply,
    RunQueuedStage,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    inputs: QueueApplyTopWithQueuedStageInputs<'_, Account, TxId>,
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
    RunPreflight: FnOnce() -> PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    RunDirectApply: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        crate::PreparedDirectApply<'_, Account, TxId>,
    ) -> crate::DirectApplyExecution<Account, TxId>,
    RunQueuedStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedQueuedStageInputs<'_, Account, Tx, Journal, ParentBatchId>,
    ) -> QueueApplyQueuedStage<Account, Tx, Journal, ParentBatchId>,
{
    let rules = inputs.direct_apply.top.call.rules;

    with_transaction_apply_runtime(rules, || {
        let preflight_result = run_preflight();
        run_queue_apply_after_preflight_with_caller_direct_apply_and_caller_queued_stage(
            views,
            inputs,
            &preflight_result,
            run_direct_apply,
            run_queued_stage,
        )
    })
}

pub fn run_queue_apply_top_with_caller_direct_apply_and_caller_queued_stage_and_log_messages<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    RunPreflight,
    RunDirectApply,
    RunQueuedStage,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    inputs: QueueApplyTopWithQueuedStageInputs<'_, Account, TxId>,
    run_preflight: RunPreflight,
    run_direct_apply: RunDirectApply,
    run_queued_stage: RunQueuedStage,
) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    RunPreflight: FnOnce() -> PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    RunDirectApply: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        crate::PreparedDirectApply<'_, Account, TxId>,
    ) -> crate::DirectApplyExecution<Account, TxId>,
    RunQueuedStage:
        FnOnce(
            &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
            QueueApplyPreparedQueuedStageInputs<'_, Account, Tx, Journal, ParentBatchId>,
        )
            -> QueueApplyQueuedStageWithLogMessagesResult<Account, Tx, Journal, ParentBatchId>,
{
    let rules = inputs.direct_apply.top.call.rules;

    with_transaction_apply_runtime(rules, || {
        let preflight_result = run_preflight();
        run_queue_apply_after_preflight_with_caller_direct_apply_and_caller_queued_stage_and_log_messages(
            views,
            inputs,
            &preflight_result,
            run_direct_apply,
            run_queued_stage,
        )
    })
}

pub fn run_queue_apply_after_preflight_with_direct_apply_and_caller_queued_stage<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TraceFn,
    ApplyFn,
    RunQueuedStage,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    inputs: QueueApplyTopWithQueuedStageInputs<'_, Account, TxId>,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
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
    TraceFn: FnMut(&str),
    ApplyFn: FnOnce() -> crate::ApplyResult,
    RunQueuedStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedQueuedStageInputs<'_, Account, Tx, Journal, ParentBatchId>,
    ) -> QueueApplyQueuedStage<Account, Tx, Journal, ParentBatchId>,
{
    run_queue_apply_after_preflight_with_caller_direct_apply_and_caller_queued_stage(
        views,
        inputs,
        preflight_result,
        |views, prepared| run_prepared_direct_apply_with_trace(views, prepared, trace, apply),
        run_queued_stage,
    )
}

pub fn run_queue_apply_after_preflight_with_direct_apply_and_caller_queued_stage_and_log_messages<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TraceFn,
    ApplyFn,
    RunQueuedStage,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    inputs: QueueApplyTopWithQueuedStageInputs<'_, Account, TxId>,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
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
    TraceFn: FnMut(&str),
    ApplyFn: FnOnce() -> crate::ApplyResult,
    RunQueuedStage:
        FnOnce(
            &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
            QueueApplyPreparedQueuedStageInputs<'_, Account, Tx, Journal, ParentBatchId>,
        )
            -> QueueApplyQueuedStageWithLogMessagesResult<Account, Tx, Journal, ParentBatchId>,
{
    run_queue_apply_after_preflight_with_caller_direct_apply_and_caller_queued_stage_and_log_messages(
        views,
        inputs,
        preflight_result,
        |views, prepared| run_prepared_direct_apply_with_trace(views, prepared, trace, apply),
        run_queued_stage,
    )
}

pub fn run_queue_apply_after_preflight_with_caller_direct_apply_and_queued_stage<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    RunDirectApply,
    PrepareMultiTxn,
    RunPreclaim,
    RunTryClear,
    TryClearResult,
    ApplySandbox,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    inputs: QueueApplyTopWithQueuedStageInputs<'_, Account, TxId>,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
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
    run_queue_apply_after_preflight_with_caller_direct_apply_and_caller_queued_stage(
        views,
        inputs,
        preflight_result,
        run_direct_apply,
        |views, queued| {
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
        },
    )
}

pub fn run_queue_apply_after_preflight_with_caller_direct_apply_and_queued_stage_and_log_messages<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    RunDirectApply,
    PrepareMultiTxn,
    RunPreclaim,
    RunTryClear,
    TryClearResult,
    ApplySandbox,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    inputs: QueueApplyTopWithQueuedStageInputs<'_, Account, TxId>,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    run_direct_apply: RunDirectApply,
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
    run_queue_apply_after_preflight_with_caller_direct_apply_and_caller_queued_stage_and_log_messages(
        views,
        inputs,
        preflight_result,
        run_direct_apply,
        |views, queued| {
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
        },
    )
}

pub fn run_queue_apply_after_preflight_with_caller_direct_apply_and_caller_queued_stage<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    RunDirectApply,
    RunQueuedStage,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    inputs: QueueApplyTopWithQueuedStageInputs<'_, Account, TxId>,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    run_direct_apply: RunDirectApply,
    run_queued_stage: RunQueuedStage,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    RunDirectApply: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        crate::PreparedDirectApply<'_, Account, TxId>,
    ) -> crate::DirectApplyExecution<Account, TxId>,
    RunQueuedStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedQueuedStageInputs<'_, Account, Tx, Journal, ParentBatchId>,
    ) -> QueueApplyQueuedStage<Account, Tx, Journal, ParentBatchId>,
{
    let QueueApplyTopWithQueuedStageInputs {
        direct_apply,
        queued,
    } = inputs;
    let call = direct_apply.top.call;
    let fee_context_inputs = direct_apply.top.fee_context_inputs;
    let queued_preflight = preflight_result.clone();

    run_queue_apply_after_preflight_with_caller_direct_apply(
        preflight_result,
        views,
        QueueApplyPreflightWithDirectApplyInputs::new(
            direct_apply.transaction_id,
            call.account_exists,
            call.account_seq_proxy,
            call.tx_seq_proxy,
            call.ticket_exists,
            fee_context_inputs,
            direct_apply.applied_account,
        ),
        run_direct_apply,
        |views, fee_context| {
            run_queued_stage(
                views,
                into_queue_apply_prepared_queued_stage_inputs(
                    queued,
                    call.account_seq_proxy,
                    call.tx_seq_proxy,
                    fee_context,
                    queued_preflight,
                ),
            )
        },
    )
}

pub fn run_queue_apply_after_preflight_with_caller_direct_apply_and_caller_queued_stage_and_log_messages<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    RunDirectApply,
    RunQueuedStage,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    inputs: QueueApplyTopWithQueuedStageInputs<'_, Account, TxId>,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    run_direct_apply: RunDirectApply,
    run_queued_stage: RunQueuedStage,
) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    RunDirectApply: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        crate::PreparedDirectApply<'_, Account, TxId>,
    ) -> crate::DirectApplyExecution<Account, TxId>,
    RunQueuedStage:
        FnOnce(
            &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
            QueueApplyPreparedQueuedStageInputs<'_, Account, Tx, Journal, ParentBatchId>,
        )
            -> QueueApplyQueuedStageWithLogMessagesResult<Account, Tx, Journal, ParentBatchId>,
{
    let QueueApplyTopWithQueuedStageInputs {
        direct_apply,
        queued,
    } = inputs;
    let call = direct_apply.top.call;
    let fee_context_inputs = direct_apply.top.fee_context_inputs;
    let queued_preflight = preflight_result.clone();

    into_queue_apply_top_with_log_messages_result(
        run_queue_apply_after_preflight_with_caller_direct_apply_and_log_messages(
            preflight_result,
            views,
            QueueApplyPreflightWithDirectApplyInputs::new(
                direct_apply.transaction_id,
                call.account_exists,
                call.account_seq_proxy,
                call.tx_seq_proxy,
                call.ticket_exists,
                fee_context_inputs,
                direct_apply.applied_account,
            ),
            run_direct_apply,
            |views, fee_context| {
                run_queued_stage(
                    views,
                    into_queue_apply_prepared_queued_stage_inputs(
                        queued,
                        call.account_seq_proxy,
                        call.tx_seq_proxy,
                        fee_context,
                        queued_preflight,
                    ),
                )
            },
        ),
    )
}

pub fn run_queue_apply_after_preflight_with_acquired_direct_apply_and_caller_queued_stage<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    RunQueuedStage,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    inputs: QueueApplyTopWithQueuedStageInputs<'_, Account, TxId>,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    direct_applied: Option<DirectApplyExecution<Account, TxId>>,
    run_queued_stage: RunQueuedStage,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    RunQueuedStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedQueuedStageInputs<'_, Account, Tx, Journal, ParentBatchId>,
    ) -> QueueApplyQueuedStage<Account, Tx, Journal, ParentBatchId>,
{
    let QueueApplyTopWithQueuedStageInputs {
        direct_apply,
        queued,
    } = inputs;
    let call = direct_apply.top.call;
    let fee_context_inputs = direct_apply.top.fee_context_inputs;

    run_queue_apply_after_preflight_with_acquired_direct_apply(
        preflight_result,
        call.account_exists,
        call.account_seq_proxy,
        call.tx_seq_proxy,
        call.ticket_exists,
        direct_applied,
        || {
            run_queued_stage(
                views,
                into_queue_apply_prepared_queued_stage_inputs(
                    queued,
                    call.account_seq_proxy,
                    call.tx_seq_proxy,
                    evaluate_queue_apply_fee_context(fee_context_inputs),
                    preflight_result.clone(),
                ),
            )
        },
    )
}

pub fn run_queue_apply_after_preflight_with_acquired_direct_apply_and_caller_queued_stage_and_log_messages<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    RunQueuedStage,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    inputs: QueueApplyTopWithQueuedStageInputs<'_, Account, TxId>,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    direct_applied: Option<DirectApplyExecution<Account, TxId>>,
    run_queued_stage: RunQueuedStage,
) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    RunQueuedStage:
        FnOnce(
            &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
            QueueApplyPreparedQueuedStageInputs<'_, Account, Tx, Journal, ParentBatchId>,
        )
            -> QueueApplyQueuedStageWithLogMessagesResult<Account, Tx, Journal, ParentBatchId>,
{
    let QueueApplyTopWithQueuedStageInputs {
        direct_apply,
        queued,
    } = inputs;
    let call = direct_apply.top.call;
    let fee_context_inputs = direct_apply.top.fee_context_inputs;

    into_queue_apply_top_with_log_messages_result(
        run_queue_apply_after_preflight_with_acquired_direct_apply_and_log_messages(
            preflight_result,
            call.account_exists,
            call.account_seq_proxy,
            call.tx_seq_proxy,
            call.ticket_exists,
            direct_applied,
            || {
                run_queued_stage(
                    views,
                    into_queue_apply_prepared_queued_stage_inputs(
                        queued,
                        call.account_seq_proxy,
                        call.tx_seq_proxy,
                        evaluate_queue_apply_fee_context(fee_context_inputs),
                        preflight_result.clone(),
                    ),
                )
            },
        ),
    )
}

pub fn run_queue_apply_after_preflight_with_acquired_direct_apply_and_queued_stage<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    PrepareMultiTxn,
    RunPreclaim,
    RunTryClear,
    TryClearResult,
    ApplySandbox,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    inputs: QueueApplyTopWithQueuedStageInputs<'_, Account, TxId>,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
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
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
    RunPreclaim: FnOnce(QueueApplyPreclaimViewSource) -> PreclaimResult<Tx, Journal, ParentBatchId>,
    RunTryClear: FnOnce() -> TryClearResult,
    TryClearResult: QueueApplyTryClearResult,
    ApplySandbox: FnOnce(),
{
    run_queue_apply_after_preflight_with_acquired_direct_apply_and_caller_queued_stage(
        views,
        inputs,
        preflight_result,
        direct_applied,
        |views, queued| {
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
        },
    )
}

pub fn run_queue_apply_after_preflight_with_acquired_direct_apply_and_queued_stage_and_log_messages<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    PrepareMultiTxn,
    RunPreclaim,
    RunTryClear,
    TryClearResult,
    ApplySandbox,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    inputs: QueueApplyTopWithQueuedStageInputs<'_, Account, TxId>,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    direct_applied: Option<DirectApplyExecution<Account, TxId>>,
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
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
    RunPreclaim: FnOnce(QueueApplyPreclaimViewSource) -> PreclaimResult<Tx, Journal, ParentBatchId>,
    RunTryClear: FnOnce() -> TryClearResult,
    TryClearResult: QueueApplyTryClearResult,
    ApplySandbox: FnOnce(),
{
    run_queue_apply_after_preflight_with_acquired_direct_apply_and_caller_queued_stage_and_log_messages(
        views,
        inputs,
        preflight_result,
        direct_applied,
        |views, queued| {
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
        },
    )
}

#[cfg(test)]
mod tests;
