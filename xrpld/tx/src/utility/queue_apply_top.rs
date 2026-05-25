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
mod tests {
    use std::{cell::Cell, cell::RefCell, collections::BTreeMap};

    use basics::base_uint::Uint256;
    use protocol::{Rules, SeqProxy, Ter};

    use super::{
        QueueApplyCallInputs, QueueApplyPreparedQueuedStageInputs,
        QueueApplyTopWithDirectApplyInputs, QueueApplyTopWithFeeContextInputs,
        QueueApplyTopWithQueuedStageInputs,
        run_queue_apply_after_preflight_with_acquired_direct_apply_and_queued_stage,
        run_queue_apply_after_preflight_with_caller_direct_apply_and_caller_queued_stage,
        run_queue_apply_after_preflight_with_caller_direct_apply_and_queued_stage,
        run_queue_apply_after_preflight_with_direct_apply_and_queued_stage,
        run_queue_apply_after_preflight_with_queued_stage, run_queue_apply_top,
        run_queue_apply_top_with_caller_direct_apply_and_caller_queued_stage,
        run_queue_apply_top_with_caller_direct_apply_and_caller_queued_stage_and_log_messages,
        run_queue_apply_top_with_caller_direct_apply_and_queued_stage,
        run_queue_apply_top_with_direct_apply, run_queue_apply_top_with_fee_context,
        run_queue_apply_top_with_queued_stage,
    };
    use crate::{
        ApplyFlags, ApplyResult, BlockerQueueAdmission, DirectApplyAttemptResult,
        DirectApplyExecution, MaybeTx, MaybeTxCore, OrderCandidates, PreclaimResult,
        PreflightResult, PreparedDirectApply, QueueApplyEntryStage, QueueApplyFeeContextInputs,
        QueueApplyPreflightStage, QueueApplyPrerequisite, QueueApplyQueueLogMessages,
        QueueApplyQueuedStage, QueueApplyQueuedStageWithLogMessagesResult,
        QueueApplyQueuedWithFeeContextInputs, QueueApplyTopWithLogMessagesResult,
        QueueFeeMetricsSnapshot, QueueHoldPreflight, QueueViews, TXQ_BASE_LEVEL, TxConsequences,
        TxConsequencesCategory, TxQAccount, evaluate_queue_apply_fee_context,
    };

    #[test]
    fn top_wrapper_forwards_structured_prerequisite_inputs() {
        let rules = Rules::new(std::iter::empty());
        let queued_called = Cell::new(false);

        let stage = run_queue_apply_top::<
            &'static str,
            &'static str,
            &'static str,
            &'static str,
            &'static str,
            &'static str,
            _,
            _,
            _,
        >(
            QueueApplyCallInputs::new(
                &rules,
                false,
                SeqProxy::sequence(8),
                SeqProxy::sequence(8),
                false,
            ),
            || {
                PreflightResult::new(
                    "tx",
                    None::<&str>,
                    rules.clone(),
                    "normal",
                    ApplyFlags::NONE,
                    "journal",
                    Ter::TES_SUCCESS,
                )
            },
            || None,
            || {
                queued_called.set(true);
                unreachable!("missing account should return before queued stage")
            },
        );

        assert_eq!(
            stage,
            QueueApplyPreflightStage::Entry(QueueApplyEntryStage::RejectPrerequisite(
                QueueApplyPrerequisite::MissingAccount
            ))
        );
        assert_eq!(
            stage.apply_result(),
            ApplyResult::new(Ter::TER_NO_ACCOUNT, false, false)
        );
        assert!(!queued_called.get());
    }

    #[test]
    fn top_wrapper_preserves_direct_apply_priority_over_ticket_gate() {
        let rules = Rules::new(std::iter::empty());
        let queued_called = Cell::new(false);
        let direct = DirectApplyExecution {
            transaction_id: "ABC123",
            attempt: DirectApplyAttemptResult {
                apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, true),
                removed_replacement: None::<crate::FeeQueueKey<&'static str>>,
            },
        };

        let stage = run_queue_apply_top(
            QueueApplyCallInputs::new(
                &rules,
                true,
                SeqProxy::sequence(8),
                SeqProxy::ticket(7),
                false,
            ),
            || {
                PreflightResult::new(
                    "tx",
                    None::<&str>,
                    rules.clone(),
                    "normal",
                    ApplyFlags::NONE,
                    "journal",
                    Ter::TES_SUCCESS,
                )
            },
            || Some(direct.clone()),
            || {
                queued_called.set(true);
                unreachable!("direct apply should return before queued stage")
            },
        );

        assert_eq!(
            stage,
            QueueApplyPreflightStage::Entry(QueueApplyEntryStage::DirectApplied(direct))
        );
        assert_eq!(
            stage.apply_result(),
            ApplyResult::new(Ter::TES_SUCCESS, true, true)
        );
        assert!(!queued_called.get());
    }

    #[test]
    fn top_wrapper_with_fee_context_keeps_preflight_ahead_of_fee_derivation() {
        let rules = Rules::new(std::iter::empty());

        let stage = run_queue_apply_top_with_fee_context::<
            &'static str,
            &'static str,
            &'static str,
            &'static str,
            &'static str,
            &'static str,
            _,
            _,
            _,
        >(
            QueueApplyTopWithFeeContextInputs::new(
                QueueApplyCallInputs::new(
                    &rules,
                    true,
                    SeqProxy::sequence(8),
                    SeqProxy::sequence(8),
                    true,
                ),
                crate::QueueApplyFeeContextInputs {
                    calculated_base_fee_drops: -1,
                    fee_paid_drops: 0,
                    default_base_fee_drops: 0,
                    metrics_snapshot: QueueFeeMetricsSnapshot {
                        txns_expected: 32,
                        escalation_multiplier: TXQ_BASE_LEVEL * 500,
                    },
                    open_ledger_tx_count: 0,
                    flags: ApplyFlags::NONE,
                },
            ),
            || {
                PreflightResult::new(
                    "tx",
                    None::<&str>,
                    rules.clone(),
                    "normal",
                    ApplyFlags::NONE,
                    "journal",
                    Ter::TER_RETRY,
                )
            },
            |_| unreachable!("preflight rejection should return before direct apply"),
            |_| unreachable!("preflight rejection should return before queued stage"),
        );

        assert_eq!(
            stage,
            QueueApplyPreflightStage::RejectPreflight(ApplyResult::new(
                Ter::TER_RETRY,
                false,
                false,
            ))
        );
    }

    #[test]
    fn top_wrapper_with_fee_context_feeds_computed_fee_levels_into_direct_apply() {
        let rules = Rules::new(std::iter::empty());
        let queued_called = Cell::new(false);
        let direct = DirectApplyExecution {
            transaction_id: "ABC123",
            attempt: DirectApplyAttemptResult {
                apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, true),
                removed_replacement: None::<crate::FeeQueueKey<&'static str>>,
            },
        };

        let stage = run_queue_apply_top_with_fee_context(
            QueueApplyTopWithFeeContextInputs::new(
                QueueApplyCallInputs::new(
                    &rules,
                    true,
                    SeqProxy::sequence(8),
                    SeqProxy::sequence(8),
                    true,
                ),
                crate::QueueApplyFeeContextInputs {
                    calculated_base_fee_drops: 10,
                    fee_paid_drops: 20,
                    default_base_fee_drops: 10,
                    metrics_snapshot: QueueFeeMetricsSnapshot {
                        txns_expected: 32,
                        escalation_multiplier: TXQ_BASE_LEVEL * 500,
                    },
                    open_ledger_tx_count: 33,
                    flags: ApplyFlags::NONE,
                },
            ),
            || {
                PreflightResult::new(
                    "tx",
                    None::<&str>,
                    rules.clone(),
                    "normal",
                    ApplyFlags::NONE,
                    "journal",
                    Ter::TES_SUCCESS,
                )
            },
            |context| {
                assert_eq!(context.base_level, TXQ_BASE_LEVEL);
                assert_eq!(context.fee_level_paid, 512);
                assert_eq!(context.required_fee_level, 136_125);
                Some(direct.clone())
            },
            |_| {
                queued_called.set(true);
                QueueApplyQueuedStage::MultiTxn(crate::QueueApplyMultiTxnStage::RejectPath(
                    Ter::TER_PRE_SEQ,
                ))
            },
        );

        assert_eq!(
            stage,
            QueueApplyPreflightStage::Entry(QueueApplyEntryStage::DirectApplied(direct))
        );
        assert!(!queued_called.get());
    }

    #[test]
    fn top_wrapper_with_direct_apply_keeps_preflight_ahead_of_entry_and_fee_work() {
        let rules = Rules::new(std::iter::empty());
        let traces = RefCell::new(Vec::new());
        let mut views = QueueViews::<&'static str, ()>::new(BTreeMap::new(), vec![]);

        let stage = run_queue_apply_top_with_direct_apply::<
            &'static str,
            (),
            &'static str,
            &'static str,
            &'static str,
            &'static str,
            &'static str,
            _,
            _,
            _,
            _,
        >(
            &mut views,
            QueueApplyTopWithDirectApplyInputs::new(
                QueueApplyTopWithFeeContextInputs::new(
                    QueueApplyCallInputs::new(
                        &rules,
                        true,
                        SeqProxy::sequence(8),
                        SeqProxy::sequence(8),
                        true,
                    ),
                    crate::QueueApplyFeeContextInputs {
                        calculated_base_fee_drops: -1,
                        fee_paid_drops: 0,
                        default_base_fee_drops: 0,
                        metrics_snapshot: QueueFeeMetricsSnapshot {
                            txns_expected: 32,
                            escalation_multiplier: TXQ_BASE_LEVEL * 500,
                        },
                        open_ledger_tx_count: 0,
                        flags: ApplyFlags::NONE,
                    },
                ),
                "ABC123",
                &"acct",
            ),
            || {
                PreflightResult::new(
                    "tx",
                    None::<&str>,
                    rules.clone(),
                    "normal",
                    ApplyFlags::NONE,
                    "journal",
                    Ter::TER_RETRY,
                )
            },
            |line| traces.borrow_mut().push(line.to_owned()),
            || unreachable!("preflight rejection should return before direct apply"),
            |_, _| unreachable!("preflight rejection should return before queued stage"),
        );

        assert_eq!(
            stage,
            QueueApplyPreflightStage::RejectPreflight(ApplyResult::new(
                Ter::TER_RETRY,
                false,
                false,
            ))
        );
        assert!(traces.borrow().is_empty());
    }

    #[test]
    fn top_wrapper_with_direct_apply_runs_runtime_then_direct_apply_before_queue() {
        let rules = Rules::new(std::iter::empty());
        let traces = RefCell::new(Vec::new());
        let mut views = QueueViews::<&'static str, ()>::new(BTreeMap::new(), vec![]);

        let stage = run_queue_apply_top_with_direct_apply(
            &mut views,
            QueueApplyTopWithDirectApplyInputs::new(
                QueueApplyTopWithFeeContextInputs::new(
                    QueueApplyCallInputs::new(
                        &rules,
                        true,
                        SeqProxy::sequence(8),
                        SeqProxy::sequence(8),
                        true,
                    ),
                    crate::QueueApplyFeeContextInputs {
                        calculated_base_fee_drops: 10,
                        fee_paid_drops: 20,
                        default_base_fee_drops: 10,
                        metrics_snapshot: QueueFeeMetricsSnapshot {
                            txns_expected: 32,
                            escalation_multiplier: TXQ_BASE_LEVEL * 500,
                        },
                        open_ledger_tx_count: 0,
                        flags: ApplyFlags::NONE,
                    },
                ),
                "ABC123",
                &"acct",
            ),
            || {
                PreflightResult::new(
                    "tx",
                    None::<&str>,
                    rules.clone(),
                    "normal",
                    ApplyFlags::NONE,
                    "journal",
                    Ter::TES_SUCCESS,
                )
            },
            |line| traces.borrow_mut().push(line.to_owned()),
            || ApplyResult::new(Ter::TES_SUCCESS, true, true),
            |_, _| {
                QueueApplyQueuedStage::<&'static str, &'static str, &'static str, &'static str>::MultiTxn(
                    crate::QueueApplyMultiTxnStage::RejectPath(Ter::TER_PRE_SEQ),
                )
            },
        );

        assert!(matches!(
            stage,
            QueueApplyPreflightStage::Entry(QueueApplyEntryStage::DirectApplied(_))
        ));
        assert_eq!(
            traces.into_inner(),
            vec![
                "Applying transaction ABC123 to open ledger.".to_owned(),
                "New transaction ABC123 applied successfully with tesSUCCESS".to_owned(),
            ]
        );
    }

    #[test]
    fn top_wrapper_with_caller_direct_apply_and_queued_stage_exposes_prepared_boundary() {
        let rules = Rules::new(std::iter::empty());
        let order = OrderCandidates::new(Uint256::from_u64(0));
        let account = String::from("acct");
        let mut views = QueueViews::<
            String,
            MaybeTx<&'static str, String, &'static str, &'static str>,
        >::new(BTreeMap::new(), vec![]);
        let try_clear_called = Cell::new(false);
        let sandbox_called = Cell::new(false);
        let direct_called = Cell::new(false);

        let stage = run_queue_apply_top_with_caller_direct_apply_and_queued_stage(
            &mut views,
            QueueApplyTopWithQueuedStageInputs::new(
                QueueApplyTopWithDirectApplyInputs::new(
                    QueueApplyTopWithFeeContextInputs::new(
                        QueueApplyCallInputs::new(
                            &rules,
                            true,
                            SeqProxy::sequence(8),
                            SeqProxy::sequence(8),
                            true,
                        ),
                        QueueApplyFeeContextInputs {
                            calculated_base_fee_drops: 10,
                            fee_paid_drops: 20,
                            default_base_fee_drops: 10,
                            metrics_snapshot: QueueFeeMetricsSnapshot {
                                txns_expected: 32,
                                escalation_multiplier: TXQ_BASE_LEVEL * 500,
                            },
                            open_ledger_tx_count: 0,
                            flags: ApplyFlags::NONE,
                        },
                    ),
                    "ABC123",
                    &account,
                ),
                QueueApplyQueuedWithFeeContextInputs {
                    account: account.clone(),
                    preflight: QueueHoldPreflight::new(false, false, ApplyFlags::NONE, None),
                    is_blocker: false,
                    open_ledger_seq: 100,
                    minimum_last_ledger_buffer: 2,
                    maximum_txn_per_account: 10,
                    retry_sequence_percent: 25,
                    queue_is_full: false,
                    balance_drops: 1_000,
                    reserve_drops: 200,
                    base_fee_drops: 10,
                    can_be_held_result: Ter::TES_SUCCESS,
                    open_ledger_tx_count: 4,
                    tx_id: Uint256::from_u64(9),
                    last_valid: Some(250),
                    flags: ApplyFlags::NONE,
                    order: &order,
                },
            ),
            || {
                PreflightResult::new(
                    "tx",
                    None::<&str>,
                    rules.clone(),
                    TxConsequences::new(1, SeqProxy::sequence(8)),
                    ApplyFlags::NONE,
                    "journal",
                    Ter::TES_SUCCESS,
                )
            },
            |_views, prepared| {
                direct_called.set(true);
                assert_eq!(
                    prepared,
                    PreparedDirectApply {
                        transaction_id: "ABC123",
                        applied_account: &account,
                        applied_seq_proxy: SeqProxy::sequence(8),
                    }
                );
                DirectApplyExecution {
                    transaction_id: "ABC123",
                    attempt: DirectApplyAttemptResult {
                        apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, true),
                        removed_replacement: None,
                    },
                }
            },
            |_| true,
            |_| unreachable!("direct apply should return before preclaim"),
            || -> crate::ApplyResult {
                try_clear_called.set(true);
                unreachable!("direct apply should return before try-clear")
            },
            || sandbox_called.set(true),
        );

        assert!(direct_called.get());
        assert!(!try_clear_called.get());
        assert!(!sandbox_called.get());
        assert!(matches!(
            stage,
            QueueApplyPreflightStage::Entry(QueueApplyEntryStage::DirectApplied(_))
        ));
    }

    #[test]
    fn top_wrapper_with_caller_direct_apply_and_caller_queued_stage_exposes_prepared_queue_boundary()
     {
        let rules = Rules::new(std::iter::empty());
        let order = OrderCandidates::new(Uint256::from_u64(0));
        let account = String::from("acct");
        let mut views = QueueViews::<
            String,
            MaybeTx<&'static str, String, &'static str, &'static str>,
        >::new(BTreeMap::new(), vec![]);
        let direct_called = Cell::new(false);
        let queued_called = Cell::new(false);
        let fee_context_inputs = QueueApplyFeeContextInputs {
            calculated_base_fee_drops: 10,
            fee_paid_drops: 20,
            default_base_fee_drops: 10,
            metrics_snapshot: QueueFeeMetricsSnapshot {
                txns_expected: 32,
                escalation_multiplier: TXQ_BASE_LEVEL * 500,
            },
            open_ledger_tx_count: 0,
            flags: ApplyFlags::NONE,
        };
        let expected_fee_context = evaluate_queue_apply_fee_context(fee_context_inputs);
        let expected_consequences = TxConsequences::new(1, SeqProxy::sequence(6));

        let stage = run_queue_apply_top_with_caller_direct_apply_and_caller_queued_stage(
            &mut views,
            QueueApplyTopWithQueuedStageInputs::new(
                QueueApplyTopWithDirectApplyInputs::new(
                    QueueApplyTopWithFeeContextInputs::new(
                        QueueApplyCallInputs::new(
                            &rules,
                            true,
                            SeqProxy::sequence(5),
                            SeqProxy::sequence(6),
                            true,
                        ),
                        fee_context_inputs,
                    ),
                    "ABC123",
                    &account,
                ),
                QueueApplyQueuedWithFeeContextInputs {
                    account: account.clone(),
                    preflight: QueueHoldPreflight::new(false, false, ApplyFlags::NONE, Some(250)),
                    is_blocker: false,
                    open_ledger_seq: 100,
                    minimum_last_ledger_buffer: 2,
                    maximum_txn_per_account: 10,
                    retry_sequence_percent: 25,
                    queue_is_full: false,
                    balance_drops: 1_000,
                    reserve_drops: 200,
                    base_fee_drops: 10,
                    can_be_held_result: Ter::TES_SUCCESS,
                    open_ledger_tx_count: 4,
                    tx_id: Uint256::from_u64(9),
                    last_valid: Some(250),
                    flags: ApplyFlags::NONE,
                    order: &order,
                },
            ),
            || {
                PreflightResult::new(
                    "tx",
                    None::<&str>,
                    rules.clone(),
                    expected_consequences.clone(),
                    ApplyFlags::NONE,
                    "journal",
                    Ter::TES_SUCCESS,
                )
            },
            |_views, _prepared| {
                direct_called.set(true);
                unreachable!("sequence mismatch should skip direct apply")
            },
            |_views, queued| {
                queued_called.set(true);
                let QueueApplyPreparedQueuedStageInputs {
                    account_seq_proxy,
                    tx_seq_proxy,
                    queued,
                    fee_context,
                    preflight_result,
                } = queued;

                assert_eq!(account_seq_proxy, SeqProxy::sequence(5));
                assert_eq!(tx_seq_proxy, SeqProxy::sequence(6));
                assert_eq!(queued.account, account);
                assert_eq!(queued.last_valid, Some(250));
                assert_eq!(fee_context, expected_fee_context);
                assert_eq!(preflight_result.ter, Ter::TES_SUCCESS);
                assert_eq!(preflight_result.flags, ApplyFlags::NONE);
                assert_eq!(preflight_result.consequences, expected_consequences);

                QueueApplyQueuedStage::MultiTxn(crate::QueueApplyMultiTxnStage::RejectPath(
                    Ter::TER_PRE_SEQ,
                ))
            },
        );

        assert!(!direct_called.get());
        assert!(queued_called.get());
        assert_eq!(
            stage,
            QueueApplyPreflightStage::Entry(QueueApplyEntryStage::Queued(
                QueueApplyQueuedStage::MultiTxn(crate::QueueApplyMultiTxnStage::RejectPath(
                    Ter::TER_PRE_SEQ,
                )),
            ))
        );
    }

    #[test]
    fn top_wrapper_with_caller_direct_apply_and_caller_queued_stage_and_log_messages_reuses_prepared_queue_boundary()
     {
        let rules = Rules::new(std::iter::empty());
        let order = OrderCandidates::new(Uint256::from_u64(0));
        let account = String::from("acct");
        let mut views = QueueViews::<
            String,
            MaybeTx<&'static str, String, &'static str, &'static str>,
        >::new(BTreeMap::new(), vec![]);
        let direct_called = Cell::new(false);
        let queued_called = Cell::new(false);
        let fee_context_inputs = QueueApplyFeeContextInputs {
            calculated_base_fee_drops: 10,
            fee_paid_drops: 20,
            default_base_fee_drops: 10,
            metrics_snapshot: QueueFeeMetricsSnapshot {
                txns_expected: 32,
                escalation_multiplier: TXQ_BASE_LEVEL * 500,
            },
            open_ledger_tx_count: 0,
            flags: ApplyFlags::NONE,
        };
        let expected_fee_context = evaluate_queue_apply_fee_context(fee_context_inputs);
        let expected_consequences = TxConsequences::new(1, SeqProxy::sequence(6));

        let stage =
            run_queue_apply_top_with_caller_direct_apply_and_caller_queued_stage_and_log_messages(
                &mut views,
                QueueApplyTopWithQueuedStageInputs::new(
                    QueueApplyTopWithDirectApplyInputs::new(
                        QueueApplyTopWithFeeContextInputs::new(
                            QueueApplyCallInputs::new(
                                &rules,
                                true,
                                SeqProxy::sequence(5),
                                SeqProxy::sequence(6),
                                true,
                            ),
                            fee_context_inputs,
                        ),
                        "ABC123",
                        &account,
                    ),
                    QueueApplyQueuedWithFeeContextInputs {
                        account: account.clone(),
                        preflight: QueueHoldPreflight::new(
                            false,
                            false,
                            ApplyFlags::NONE,
                            Some(250),
                        ),
                        is_blocker: false,
                        open_ledger_seq: 100,
                        minimum_last_ledger_buffer: 2,
                        maximum_txn_per_account: 10,
                        retry_sequence_percent: 25,
                        queue_is_full: false,
                        balance_drops: 1_000,
                        reserve_drops: 200,
                        base_fee_drops: 10,
                        can_be_held_result: Ter::TES_SUCCESS,
                        open_ledger_tx_count: 4,
                        tx_id: Uint256::from_u64(9),
                        last_valid: Some(250),
                        flags: ApplyFlags::NONE,
                        order: &order,
                    },
                ),
                || {
                    PreflightResult::new(
                        "tx",
                        None::<&str>,
                        rules.clone(),
                        expected_consequences.clone(),
                        ApplyFlags::NONE,
                        "journal",
                        Ter::TES_SUCCESS,
                    )
                },
                |_views, _prepared| {
                    direct_called.set(true);
                    unreachable!("sequence mismatch should skip direct apply")
                },
                |_views, queued| {
                    queued_called.set(true);
                    let QueueApplyPreparedQueuedStageInputs {
                        account_seq_proxy,
                        tx_seq_proxy,
                        queued,
                        fee_context,
                        preflight_result,
                    } = queued;

                    assert_eq!(account_seq_proxy, SeqProxy::sequence(5));
                    assert_eq!(tx_seq_proxy, SeqProxy::sequence(6));
                    assert_eq!(queued.account, account);
                    assert_eq!(queued.last_valid, Some(250));
                    assert_eq!(fee_context, expected_fee_context);
                    assert_eq!(preflight_result.ter, Ter::TES_SUCCESS);
                    assert_eq!(preflight_result.flags, ApplyFlags::NONE);
                    assert_eq!(preflight_result.consequences, expected_consequences);

                    QueueApplyQueuedStageWithLogMessagesResult {
                        stage: QueueApplyQueuedStage::MultiTxn(
                            crate::QueueApplyMultiTxnStage::RejectPath(Ter::TER_PRE_SEQ),
                        ),
                        queue_log_messages: QueueApplyQueueLogMessages::default(),
                    }
                },
            );

        assert!(!direct_called.get());
        assert!(queued_called.get());
        assert_eq!(
            stage,
            QueueApplyTopWithLogMessagesResult {
                stage: QueueApplyPreflightStage::Entry(QueueApplyEntryStage::Queued(
                    QueueApplyQueuedStage::MultiTxn(crate::QueueApplyMultiTxnStage::RejectPath(
                        Ter::TER_PRE_SEQ,
                    )),
                )),
                queue_log_messages: QueueApplyQueueLogMessages::default(),
            }
        );
    }

    #[test]
    fn top_wrapper_with_queued_stage_routes_direct_apply_fallthrough_into_landed_queue_stage() {
        let rules = Rules::new(std::iter::empty());
        let mut queued_account = TxQAccount::new("acct");
        queued_account.add(
            SeqProxy::sequence(5),
            crate::MaybeTxCore::new(
                crate::MaybeTx::new(
                    Uint256::from_u64(5),
                    90,
                    "acct",
                    Some(200),
                    SeqProxy::sequence(5),
                    ApplyFlags::NONE,
                    PreflightResult::new(
                        "tx",
                        None::<&str>,
                        rules.clone(),
                        TxConsequences::with_category(
                            1,
                            SeqProxy::sequence(5),
                            TxConsequencesCategory::Blocker,
                        ),
                        ApplyFlags::NONE,
                        "journal",
                        Ter::TES_SUCCESS,
                    ),
                ),
                TxConsequences::with_category(
                    1,
                    SeqProxy::sequence(5),
                    TxConsequencesCategory::Blocker,
                ),
            ),
        );
        let mut views = QueueViews::new(BTreeMap::from([("acct", queued_account)]), vec![]);
        let order = OrderCandidates::new(Uint256::from_u64(0));

        let stage = run_queue_apply_top_with_queued_stage(
            &mut views,
            QueueApplyTopWithQueuedStageInputs::new(
                QueueApplyTopWithDirectApplyInputs::new(
                    QueueApplyTopWithFeeContextInputs::new(
                        QueueApplyCallInputs::new(
                            &rules,
                            true,
                            SeqProxy::sequence(5),
                            SeqProxy::sequence(6),
                            true,
                        ),
                        QueueApplyFeeContextInputs {
                            calculated_base_fee_drops: 10,
                            fee_paid_drops: 1,
                            default_base_fee_drops: 10,
                            metrics_snapshot: QueueFeeMetricsSnapshot {
                                txns_expected: 32,
                                escalation_multiplier: TXQ_BASE_LEVEL * 500,
                            },
                            open_ledger_tx_count: 0,
                            flags: ApplyFlags::NONE,
                        },
                    ),
                    "ABC123",
                    &"acct",
                ),
                QueueApplyQueuedWithFeeContextInputs {
                    account: "acct",
                    preflight: QueueHoldPreflight::new(false, false, ApplyFlags::NONE, None),
                    is_blocker: true,
                    open_ledger_seq: 100,
                    minimum_last_ledger_buffer: 2,
                    maximum_txn_per_account: 10,
                    retry_sequence_percent: 25,
                    queue_is_full: false,
                    balance_drops: 1_000,
                    reserve_drops: 200,
                    base_fee_drops: 10,
                    can_be_held_result: Ter::TES_SUCCESS,
                    open_ledger_tx_count: 4,
                    tx_id: Uint256::from_u64(9),
                    last_valid: Some(250),
                    flags: ApplyFlags::NONE,
                    order: &order,
                },
            ),
            || {
                PreflightResult::new(
                    "tx",
                    None::<&str>,
                    rules.clone(),
                    TxConsequences::with_category(
                        1,
                        SeqProxy::sequence(6),
                        TxConsequencesCategory::Blocker,
                    ),
                    ApplyFlags::NONE,
                    "journal",
                    Ter::TES_SUCCESS,
                )
            },
            |_| unreachable!("direct apply should fall through without tracing"),
            || ApplyResult::new(Ter::TES_SUCCESS, true, true),
            |_| true,
            |view_source| {
                assert!(!view_source.has_multi_txn());
                PreclaimResult::new(
                    100,
                    "tx",
                    None::<&str>,
                    ApplyFlags::NONE,
                    "journal",
                    Ter::TES_SUCCESS,
                )
            },
            || -> crate::ApplyResult {
                unreachable!("queue-stage account rejection should happen before try-clear")
            },
            || unreachable!("queue-stage account rejection should happen before sandbox apply"),
        );

        assert_eq!(
            stage,
            QueueApplyPreflightStage::Entry(QueueApplyEntryStage::Queued(
                QueueApplyQueuedStage::Account(
                    crate::QueueApplyAccountStage::RejectBlockerAdmission(
                        BlockerQueueAdmission::RejectsNonReplacementOfLoneEntry,
                    )
                ),
            ))
        );
    }

    #[test]
    fn after_preflight_wrapper_with_queued_stage_reuses_landed_queue_order() {
        let rules = Rules::new(std::iter::empty());
        let mut queued_account = TxQAccount::new("acct");
        queued_account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                MaybeTx::new(
                    Uint256::from_u64(5),
                    90,
                    "acct",
                    Some(200),
                    SeqProxy::sequence(5),
                    ApplyFlags::NONE,
                    PreflightResult::new(
                        "tx",
                        None::<&str>,
                        rules.clone(),
                        TxConsequences::with_category(
                            1,
                            SeqProxy::sequence(5),
                            TxConsequencesCategory::Blocker,
                        ),
                        ApplyFlags::NONE,
                        "journal",
                        Ter::TES_SUCCESS,
                    ),
                ),
                TxConsequences::with_category(
                    1,
                    SeqProxy::sequence(5),
                    TxConsequencesCategory::Blocker,
                ),
            ),
        );
        let mut views = QueueViews::new(BTreeMap::from([("acct", queued_account)]), vec![]);
        let order = OrderCandidates::new(Uint256::from_u64(0));

        let stage = run_queue_apply_after_preflight_with_queued_stage(
            &mut views,
            QueueApplyTopWithQueuedStageInputs::new(
                QueueApplyTopWithDirectApplyInputs::new(
                    QueueApplyTopWithFeeContextInputs::new(
                        QueueApplyCallInputs::new(
                            &rules,
                            true,
                            SeqProxy::sequence(5),
                            SeqProxy::sequence(6),
                            true,
                        ),
                        QueueApplyFeeContextInputs {
                            calculated_base_fee_drops: 10,
                            fee_paid_drops: 1,
                            default_base_fee_drops: 10,
                            metrics_snapshot: QueueFeeMetricsSnapshot {
                                txns_expected: 32,
                                escalation_multiplier: TXQ_BASE_LEVEL * 500,
                            },
                            open_ledger_tx_count: 0,
                            flags: ApplyFlags::NONE,
                        },
                    ),
                    "ABC123",
                    &"acct",
                ),
                QueueApplyQueuedWithFeeContextInputs {
                    account: "acct",
                    preflight: QueueHoldPreflight::new(false, false, ApplyFlags::NONE, None),
                    is_blocker: true,
                    open_ledger_seq: 100,
                    minimum_last_ledger_buffer: 2,
                    maximum_txn_per_account: 10,
                    retry_sequence_percent: 25,
                    queue_is_full: false,
                    balance_drops: 1_000,
                    reserve_drops: 200,
                    base_fee_drops: 10,
                    can_be_held_result: Ter::TES_SUCCESS,
                    open_ledger_tx_count: 4,
                    tx_id: Uint256::from_u64(9),
                    last_valid: Some(250),
                    flags: ApplyFlags::NONE,
                    order: &order,
                },
            ),
            &PreflightResult::new(
                "tx",
                None::<&str>,
                rules.clone(),
                TxConsequences::with_category(
                    1,
                    SeqProxy::sequence(6),
                    TxConsequencesCategory::Blocker,
                ),
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            ),
            |_| unreachable!("direct apply should fall through without tracing"),
            || ApplyResult::new(Ter::TES_SUCCESS, true, true),
            |_| true,
            |view_source| {
                assert!(!view_source.has_multi_txn());
                PreclaimResult::new(
                    100,
                    "tx",
                    None::<&str>,
                    ApplyFlags::NONE,
                    "journal",
                    Ter::TES_SUCCESS,
                )
            },
            || -> crate::ApplyResult {
                unreachable!("queue-stage account rejection should happen before try-clear")
            },
            || unreachable!("queue-stage account rejection should happen before sandbox apply"),
        );

        assert_eq!(
            stage,
            QueueApplyPreflightStage::Entry(QueueApplyEntryStage::Queued(
                QueueApplyQueuedStage::Account(
                    crate::QueueApplyAccountStage::RejectBlockerAdmission(
                        BlockerQueueAdmission::RejectsNonReplacementOfLoneEntry,
                    )
                ),
            ))
        );
    }

    #[test]
    fn after_preflight_direct_apply_wrapper_with_queued_stage_returns_direct_apply_before_queue_hooks()
     {
        let prepare_called = Cell::new(false);
        let preclaim_called = Cell::new(false);
        let try_clear_called = Cell::new(false);
        let sandbox_called = Cell::new(false);
        let traces = RefCell::new(Vec::new());

        let rules = Rules::new(std::iter::empty());
        let mut views = QueueViews::<
            &'static str,
            MaybeTx<&'static str, &'static str, &'static str, &'static str>,
        >::new(BTreeMap::new(), vec![]);
        let order = OrderCandidates::new(Uint256::from_u64(0));

        let stage = run_queue_apply_after_preflight_with_direct_apply_and_queued_stage(
            &mut views,
            QueueApplyTopWithQueuedStageInputs::new(
                QueueApplyTopWithDirectApplyInputs::new(
                    QueueApplyTopWithFeeContextInputs::new(
                        QueueApplyCallInputs::new(
                            &rules,
                            true,
                            SeqProxy::sequence(8),
                            SeqProxy::sequence(8),
                            true,
                        ),
                        QueueApplyFeeContextInputs {
                            calculated_base_fee_drops: 10,
                            fee_paid_drops: 20,
                            default_base_fee_drops: 10,
                            metrics_snapshot: QueueFeeMetricsSnapshot {
                                txns_expected: 32,
                                escalation_multiplier: TXQ_BASE_LEVEL * 500,
                            },
                            open_ledger_tx_count: 0,
                            flags: ApplyFlags::NONE,
                        },
                    ),
                    "ABC123",
                    &"acct",
                ),
                QueueApplyQueuedWithFeeContextInputs {
                    account: "acct",
                    preflight: QueueHoldPreflight::new(false, false, ApplyFlags::NONE, None),
                    is_blocker: false,
                    open_ledger_seq: 100,
                    minimum_last_ledger_buffer: 2,
                    maximum_txn_per_account: 10,
                    retry_sequence_percent: 25,
                    queue_is_full: false,
                    balance_drops: 1_000,
                    reserve_drops: 200,
                    base_fee_drops: 10,
                    can_be_held_result: Ter::TES_SUCCESS,
                    open_ledger_tx_count: 4,
                    tx_id: Uint256::from_u64(9),
                    last_valid: Some(250),
                    flags: ApplyFlags::NONE,
                    order: &order,
                },
            ),
            &PreflightResult::new(
                "tx",
                None::<&str>,
                rules.clone(),
                TxConsequences::new(1, SeqProxy::sequence(8)),
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            ),
            |line| traces.borrow_mut().push(line.to_owned()),
            || ApplyResult::new(Ter::TES_SUCCESS, true, true),
            |_| {
                prepare_called.set(true);
                true
            },
            |_| {
                preclaim_called.set(true);
                PreclaimResult::new(
                    100,
                    "tx",
                    None::<&str>,
                    ApplyFlags::NONE,
                    "journal",
                    Ter::TES_SUCCESS,
                )
            },
            || {
                try_clear_called.set(true);
                ApplyResult::new(Ter::TES_SUCCESS, true, true)
            },
            || sandbox_called.set(true),
        );

        assert!(matches!(
            stage,
            QueueApplyPreflightStage::Entry(QueueApplyEntryStage::DirectApplied(_))
        ));
        assert!(!prepare_called.get());
        assert!(!preclaim_called.get());
        assert!(!try_clear_called.get());
        assert!(!sandbox_called.get());
        assert_eq!(
            traces.into_inner(),
            vec![
                "Applying transaction ABC123 to open ledger.".to_owned(),
                "New transaction ABC123 applied successfully with tesSUCCESS".to_owned(),
            ]
        );
    }

    #[test]
    fn after_preflight_caller_direct_apply_wrapper_with_queued_stage_exposes_prepared_boundary() {
        let prepare_called = Cell::new(false);
        let preclaim_called = Cell::new(false);
        let try_clear_called = Cell::new(false);
        let sandbox_called = Cell::new(false);
        let direct_called = Cell::new(false);

        let rules = Rules::new(std::iter::empty());
        let account = String::from("acct");
        let mut views = QueueViews::<
            String,
            MaybeTx<&'static str, String, &'static str, &'static str>,
        >::new(BTreeMap::new(), vec![]);
        let order = OrderCandidates::new(Uint256::from_u64(0));

        let stage = run_queue_apply_after_preflight_with_caller_direct_apply_and_queued_stage(
            &mut views,
            QueueApplyTopWithQueuedStageInputs::new(
                QueueApplyTopWithDirectApplyInputs::new(
                    QueueApplyTopWithFeeContextInputs::new(
                        QueueApplyCallInputs::new(
                            &rules,
                            true,
                            SeqProxy::sequence(8),
                            SeqProxy::sequence(8),
                            true,
                        ),
                        QueueApplyFeeContextInputs {
                            calculated_base_fee_drops: 10,
                            fee_paid_drops: 20,
                            default_base_fee_drops: 10,
                            metrics_snapshot: QueueFeeMetricsSnapshot {
                                txns_expected: 32,
                                escalation_multiplier: TXQ_BASE_LEVEL * 500,
                            },
                            open_ledger_tx_count: 0,
                            flags: ApplyFlags::NONE,
                        },
                    ),
                    "ABC123",
                    &account,
                ),
                QueueApplyQueuedWithFeeContextInputs {
                    account: account.clone(),
                    preflight: QueueHoldPreflight::new(false, false, ApplyFlags::NONE, None),
                    is_blocker: false,
                    open_ledger_seq: 100,
                    minimum_last_ledger_buffer: 2,
                    maximum_txn_per_account: 10,
                    retry_sequence_percent: 25,
                    queue_is_full: false,
                    balance_drops: 1_000,
                    reserve_drops: 200,
                    base_fee_drops: 10,
                    can_be_held_result: Ter::TES_SUCCESS,
                    open_ledger_tx_count: 4,
                    tx_id: Uint256::from_u64(9),
                    last_valid: Some(250),
                    flags: ApplyFlags::NONE,
                    order: &order,
                },
            ),
            &PreflightResult::new(
                "tx",
                None::<&str>,
                rules.clone(),
                TxConsequences::new(1, SeqProxy::sequence(8)),
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            ),
            |_views, prepared| {
                direct_called.set(true);
                assert_eq!(
                    prepared,
                    PreparedDirectApply {
                        transaction_id: "ABC123",
                        applied_account: &account,
                        applied_seq_proxy: SeqProxy::sequence(8),
                    }
                );
                DirectApplyExecution {
                    transaction_id: "ABC123",
                    attempt: DirectApplyAttemptResult {
                        apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, true),
                        removed_replacement: None,
                    },
                }
            },
            |_| {
                prepare_called.set(true);
                true
            },
            |_| {
                preclaim_called.set(true);
                PreclaimResult::new(
                    100,
                    "tx",
                    None::<&str>,
                    ApplyFlags::NONE,
                    "journal",
                    Ter::TES_SUCCESS,
                )
            },
            || {
                try_clear_called.set(true);
                ApplyResult::new(Ter::TES_SUCCESS, true, true)
            },
            || sandbox_called.set(true),
        );

        assert!(direct_called.get());
        assert!(!prepare_called.get());
        assert!(!preclaim_called.get());
        assert!(!try_clear_called.get());
        assert!(!sandbox_called.get());
        assert!(matches!(
            stage,
            QueueApplyPreflightStage::Entry(QueueApplyEntryStage::DirectApplied(_))
        ));
    }

    #[test]
    fn after_preflight_caller_direct_apply_and_caller_queued_stage_exposes_prepared_queue_boundary()
    {
        let rules = Rules::new(std::iter::empty());
        let account = String::from("acct");
        let mut views = QueueViews::<
            String,
            MaybeTx<&'static str, String, &'static str, &'static str>,
        >::new(BTreeMap::new(), vec![]);
        let order = OrderCandidates::new(Uint256::from_u64(0));
        let direct_called = Cell::new(false);
        let queued_called = Cell::new(false);
        let fee_context_inputs = QueueApplyFeeContextInputs {
            calculated_base_fee_drops: 10,
            fee_paid_drops: 20,
            default_base_fee_drops: 10,
            metrics_snapshot: QueueFeeMetricsSnapshot {
                txns_expected: 32,
                escalation_multiplier: TXQ_BASE_LEVEL * 500,
            },
            open_ledger_tx_count: 0,
            flags: ApplyFlags::NONE,
        };
        let expected_fee_context = evaluate_queue_apply_fee_context(fee_context_inputs);
        let preflight_result = PreflightResult::new(
            "tx",
            None::<&str>,
            rules.clone(),
            TxConsequences::new(1, SeqProxy::sequence(6)),
            ApplyFlags::NONE,
            "journal",
            Ter::TES_SUCCESS,
        );

        let stage =
            run_queue_apply_after_preflight_with_caller_direct_apply_and_caller_queued_stage(
                &mut views,
                QueueApplyTopWithQueuedStageInputs::new(
                    QueueApplyTopWithDirectApplyInputs::new(
                        QueueApplyTopWithFeeContextInputs::new(
                            QueueApplyCallInputs::new(
                                &rules,
                                true,
                                SeqProxy::sequence(5),
                                SeqProxy::sequence(6),
                                true,
                            ),
                            fee_context_inputs,
                        ),
                        "ABC123",
                        &account,
                    ),
                    QueueApplyQueuedWithFeeContextInputs {
                        account: account.clone(),
                        preflight: QueueHoldPreflight::new(
                            false,
                            false,
                            ApplyFlags::NONE,
                            Some(250),
                        ),
                        is_blocker: false,
                        open_ledger_seq: 100,
                        minimum_last_ledger_buffer: 2,
                        maximum_txn_per_account: 10,
                        retry_sequence_percent: 25,
                        queue_is_full: false,
                        balance_drops: 1_000,
                        reserve_drops: 200,
                        base_fee_drops: 10,
                        can_be_held_result: Ter::TES_SUCCESS,
                        open_ledger_tx_count: 4,
                        tx_id: Uint256::from_u64(9),
                        last_valid: Some(250),
                        flags: ApplyFlags::NONE,
                        order: &order,
                    },
                ),
                &preflight_result,
                |_views, _prepared| {
                    direct_called.set(true);
                    unreachable!("sequence mismatch should skip direct apply")
                },
                |_views, queued| {
                    queued_called.set(true);
                    assert_eq!(queued.account_seq_proxy, SeqProxy::sequence(5));
                    assert_eq!(queued.tx_seq_proxy, SeqProxy::sequence(6));
                    assert_eq!(queued.queued.account, account);
                    assert_eq!(queued.queued.last_valid, Some(250));
                    assert_eq!(queued.fee_context, expected_fee_context);
                    assert_eq!(queued.preflight_result, preflight_result);

                    QueueApplyQueuedStage::MultiTxn(crate::QueueApplyMultiTxnStage::RejectPath(
                        Ter::TER_PRE_SEQ,
                    ))
                },
            );

        assert!(!direct_called.get());
        assert!(queued_called.get());
        assert_eq!(
            stage,
            QueueApplyPreflightStage::Entry(QueueApplyEntryStage::Queued(
                QueueApplyQueuedStage::MultiTxn(crate::QueueApplyMultiTxnStage::RejectPath(
                    Ter::TER_PRE_SEQ,
                )),
            ))
        );
    }

    #[test]
    fn acquired_direct_apply_wrapper_returns_supplied_execution_before_queue_flow() {
        let prepare_called = Cell::new(false);
        let preclaim_called = Cell::new(false);
        let try_clear_called = Cell::new(false);
        let sandbox_called = Cell::new(false);

        let rules = Rules::new(std::iter::empty());
        let mut views = QueueViews::<
            &'static str,
            MaybeTx<&'static str, &'static str, &'static str, &'static str>,
        >::new(BTreeMap::new(), vec![]);
        let order = OrderCandidates::new(Uint256::from_u64(0));
        let direct = DirectApplyExecution {
            transaction_id: "ABC123",
            attempt: DirectApplyAttemptResult {
                apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, true),
                removed_replacement: None::<crate::FeeQueueKey<&'static str>>,
            },
        };

        let stage = run_queue_apply_after_preflight_with_acquired_direct_apply_and_queued_stage(
            &mut views,
            QueueApplyTopWithQueuedStageInputs::new(
                QueueApplyTopWithDirectApplyInputs::new(
                    QueueApplyTopWithFeeContextInputs::new(
                        QueueApplyCallInputs::new(
                            &rules,
                            true,
                            SeqProxy::sequence(8),
                            SeqProxy::sequence(8),
                            true,
                        ),
                        QueueApplyFeeContextInputs {
                            calculated_base_fee_drops: 10,
                            fee_paid_drops: 20,
                            default_base_fee_drops: 10,
                            metrics_snapshot: QueueFeeMetricsSnapshot {
                                txns_expected: 32,
                                escalation_multiplier: TXQ_BASE_LEVEL * 500,
                            },
                            open_ledger_tx_count: 0,
                            flags: ApplyFlags::NONE,
                        },
                    ),
                    "ABC123",
                    &"acct",
                ),
                QueueApplyQueuedWithFeeContextInputs {
                    account: "acct",
                    preflight: QueueHoldPreflight::new(false, false, ApplyFlags::NONE, None),
                    is_blocker: false,
                    open_ledger_seq: 100,
                    minimum_last_ledger_buffer: 2,
                    maximum_txn_per_account: 10,
                    retry_sequence_percent: 25,
                    queue_is_full: false,
                    balance_drops: 1_000,
                    reserve_drops: 200,
                    base_fee_drops: 10,
                    can_be_held_result: Ter::TES_SUCCESS,
                    open_ledger_tx_count: 4,
                    tx_id: Uint256::from_u64(9),
                    last_valid: Some(250),
                    flags: ApplyFlags::NONE,
                    order: &order,
                },
            ),
            &PreflightResult::new(
                "tx",
                None::<&str>,
                rules.clone(),
                TxConsequences::new(1, SeqProxy::sequence(8)),
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            ),
            Some(direct.clone()),
            |_| {
                prepare_called.set(true);
                true
            },
            |_| {
                preclaim_called.set(true);
                PreclaimResult::new(
                    100,
                    "tx",
                    None::<&str>,
                    ApplyFlags::NONE,
                    "journal",
                    Ter::TES_SUCCESS,
                )
            },
            || {
                try_clear_called.set(true);
                ApplyResult::new(Ter::TES_SUCCESS, true, true)
            },
            || sandbox_called.set(true),
        );

        assert_eq!(
            stage,
            QueueApplyPreflightStage::Entry(QueueApplyEntryStage::DirectApplied(direct))
        );
        assert!(!prepare_called.get());
        assert!(!preclaim_called.get());
        assert!(!try_clear_called.get());
        assert!(!sandbox_called.get());
    }
}
