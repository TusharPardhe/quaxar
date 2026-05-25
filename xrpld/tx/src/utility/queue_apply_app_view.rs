//! Higher app/view wrapper for the current `xrpld` `TxQ::apply(...)` seam.
//!
//! This layer moves one step closer to the the reference implementation signature shape:
//! - one app-side runtime owns execution hooks like
//!   `preflight(...)`, direct apply, preclaim, try-clear, and trace emission,
//! - one view-side source provides the tx-specific live ledger
//!   facts needed for queue admission,
//! - the transaction source still stays explicit at the call boundary.
//!
//! It preserves the app/view split above the landed apply runtime envelope.

use std::{cell::RefCell, fmt::Display};

use protocol::{Rules, SeqProxy, Ter};

use crate::{
    ApplyFlags, PreflightResult, QueueApplyCallEnvelope, QueueApplyCurrentPreclaimClearRuntime,
    QueueApplyExecutionRuntime, QueueApplyHoldPreflightTxSource, QueueApplyObservedAccountLookup,
    QueueApplyObservedTicketLookup, QueueApplyObservedTxSource, QueueApplyObservedViewSource,
    QueueApplyOwnerShell, QueueApplyPreclaimStage, QueueApplyPreflightStage,
    QueueApplyPreparedPreclaimInputs, QueueApplyQueueLogMessages, QueueApplyRuntimeEnvelope,
    QueueApplyTopWithLogMessagesResult, QueueFeeMetricsSnapshot, QueueHoldPreflight,
    TxConsequences, derive_queue_hold_preflight_from_tx_source,
};

pub trait QueueApplyAppRuntime<Tx, Journal, ParentBatchId>:
    QueueApplyExecutionRuntime<Tx, Journal, ParentBatchId>
{
}

impl<Tx, Journal, ParentBatchId, Runtime> QueueApplyAppRuntime<Tx, Journal, ParentBatchId>
    for Runtime
where
    Runtime: QueueApplyExecutionRuntime<Tx, Journal, ParentBatchId>,
{
}

pub trait QueueApplyLedgerViewSource<Account>: QueueApplyObservedViewSource<Account> {}

impl<Account, View> QueueApplyLedgerViewSource<Account> for View where
    View: QueueApplyObservedViewSource<Account>
{
}

#[derive(Debug, Clone)]
pub struct QueueApplyAppViewSnapshot {
    pub rules: Rules,
    pub account_lookup: QueueApplyObservedAccountLookup,
    pub ticket_lookup: QueueApplyObservedTicketLookup,
    pub calculated_base_fee_drops: i64,
    pub fee_paid_drops: i64,
    pub default_base_fee_drops: i64,
    pub metrics_snapshot: QueueFeeMetricsSnapshot,
    pub open_ledger_tx_count: usize,
    pub open_ledger_seq: u32,
    pub reserve_drops: u64,
    pub base_fee_drops: u64,
}

impl<Account> QueueApplyObservedViewSource<Account> for QueueApplyAppViewSnapshot {
    fn rules(&self) -> &Rules {
        &self.rules
    }

    fn account_lookup(&self, _account: &Account) -> QueueApplyObservedAccountLookup {
        self.account_lookup
    }

    fn ticket_lookup(
        &self,
        _account: &Account,
        _tx_seq_proxy: SeqProxy,
    ) -> QueueApplyObservedTicketLookup {
        self.ticket_lookup
    }

    fn calculated_base_fee_drops(&self) -> i64 {
        self.calculated_base_fee_drops
    }

    fn fee_paid_drops(&self) -> i64 {
        self.fee_paid_drops
    }

    fn default_base_fee_drops(&self) -> i64 {
        self.default_base_fee_drops
    }

    fn metrics_snapshot(&self) -> QueueFeeMetricsSnapshot {
        self.metrics_snapshot
    }

    fn open_ledger_tx_count(&self) -> usize {
        self.open_ledger_tx_count
    }

    fn open_ledger_seq(&self) -> u32 {
        self.open_ledger_seq
    }

    fn reserve_drops(&self) -> u64 {
        self.reserve_drops
    }

    fn base_fee_drops(&self) -> u64 {
        self.base_fee_drops
    }
}

pub fn snapshot_queue_apply_app_view<Account, TxSource, View>(
    tx_source: &TxSource,
    view: &View,
) -> QueueApplyAppViewSnapshot
where
    TxSource: QueueApplyObservedTxSource<Account = Account>,
    View: QueueApplyLedgerViewSource<Account>,
{
    snapshot_queue_apply_app_view_with_metrics(tx_source, view, view.metrics_snapshot())
}

pub fn snapshot_queue_apply_app_view_with_metrics<Account, TxSource, View>(
    tx_source: &TxSource,
    view: &View,
    metrics_snapshot: QueueFeeMetricsSnapshot,
) -> QueueApplyAppViewSnapshot
where
    TxSource: QueueApplyObservedTxSource<Account = Account>,
    View: QueueApplyLedgerViewSource<Account>,
{
    QueueApplyAppViewSnapshot {
        rules: view.rules().clone(),
        account_lookup: view.account_lookup(tx_source.account()),
        ticket_lookup: view.ticket_lookup(tx_source.account(), tx_source.tx_seq_proxy()),
        calculated_base_fee_drops: view.calculated_base_fee_drops(),
        fee_paid_drops: view.fee_paid_drops(),
        default_base_fee_drops: view.default_base_fee_drops(),
        metrics_snapshot,
        open_ledger_tx_count: view.open_ledger_tx_count(),
        open_ledger_seq: view.open_ledger_seq(),
        reserve_drops: view.reserve_drops(),
        base_fee_drops: view.base_fee_drops(),
    }
}

fn with_queue_apply_app_view_call<Account, TxSource, View, R>(
    tx_source: &TxSource,
    view: &View,
    metrics_snapshot: QueueFeeMetricsSnapshot,
    run: impl FnOnce(QueueApplyCallEnvelope<'_, TxSource, QueueApplyAppViewSnapshot>) -> R,
) -> R
where
    TxSource: QueueApplyObservedTxSource<Account = Account>,
    View: QueueApplyLedgerViewSource<Account>,
{
    let snapshot = snapshot_queue_apply_app_view_with_metrics(tx_source, view, metrics_snapshot);
    let call = QueueApplyCallEnvelope::new(tx_source, &snapshot);
    run(call)
}

fn with_queue_apply_app_view_metrics_snapshot<Account, View, R>(
    view: &View,
    run: impl FnOnce(QueueFeeMetricsSnapshot) -> R,
) -> R
where
    View: QueueApplyLedgerViewSource<Account>,
{
    run(view.metrics_snapshot())
}

fn with_current_queue_apply_app_view_caller_preclaim<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    View,
    TxSource,
    R,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    view: &View,
    tx_source: &TxSource,
    run: impl FnOnce(
        &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        QueueApplyCallEnvelope<'_, TxSource, QueueApplyAppViewSnapshot>,
    ) -> R,
) -> R
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
{
    with_queue_apply_app_view_metrics_snapshot(view, |metrics_snapshot| {
        with_queue_apply_app_view_call(tx_source, view, metrics_snapshot, |call| run(owner, call))
    })
}

fn with_current_queue_apply_app_view<Account, Tx, Journal, ParentBatchId, TxId, View, TxSource, R>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    view: &View,
    tx_source: &TxSource,
    run: impl FnOnce(
        &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        QueueApplyCallEnvelope<'_, TxSource, QueueApplyAppViewSnapshot>,
    ) -> R,
) -> R
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
{
    with_queue_apply_app_view_metrics_snapshot(view, |metrics_snapshot| {
        with_queue_apply_app_view_call(tx_source, view, metrics_snapshot, |call| run(owner, call))
    })
}

fn with_current_queue_apply_app_view_log_sinks<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    View,
    TxSource,
    R,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    view: &View,
    tx_source: &TxSource,
    run: impl FnOnce(
        &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        QueueApplyCallEnvelope<'_, TxSource, QueueApplyAppViewSnapshot>,
    ) -> R,
) -> R
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
{
    with_queue_apply_app_view_metrics_snapshot(view, |metrics_snapshot| {
        with_queue_apply_app_view_call(tx_source, view, metrics_snapshot, |call| run(owner, call))
    })
}

fn with_current_queue_apply_app_view_caller_preclaim_log_sinks<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    View,
    TxSource,
    R,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    view: &View,
    tx_source: &TxSource,
    run: impl FnOnce(
        &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        QueueApplyCallEnvelope<'_, TxSource, QueueApplyAppViewSnapshot>,
    ) -> R,
) -> R
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
{
    with_queue_apply_app_view_metrics_snapshot(view, |metrics_snapshot| {
        with_queue_apply_app_view_call(tx_source, view, metrics_snapshot, |call| run(owner, call))
    })
}

fn with_queue_apply_app_view_metrics_snapshot_current_app_view<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    View,
    TxSource,
    R,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    view: &View,
    tx_source: &TxSource,
    metrics_snapshot: QueueFeeMetricsSnapshot,
    run: impl FnOnce(
        &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        QueueApplyCallEnvelope<'_, TxSource, QueueApplyAppViewSnapshot>,
    ) -> R,
) -> R
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
{
    with_queue_apply_app_view_call(tx_source, view, metrics_snapshot, |call| run(owner, call))
}

fn with_queue_apply_app_view_metrics_snapshot_current_app_view_caller_preclaim<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    View,
    TxSource,
    R,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    view: &View,
    tx_source: &TxSource,
    metrics_snapshot: QueueFeeMetricsSnapshot,
    run: impl FnOnce(
        &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        QueueApplyCallEnvelope<'_, TxSource, QueueApplyAppViewSnapshot>,
    ) -> R,
) -> R
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
{
    with_queue_apply_app_view_call(tx_source, view, metrics_snapshot, |call| run(owner, call))
}

fn with_queue_apply_app_view_metrics_snapshot_current_app_view_log_sinks<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    View,
    TxSource,
    R,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    view: &View,
    tx_source: &TxSource,
    metrics_snapshot: QueueFeeMetricsSnapshot,
    run: impl FnOnce(
        &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        QueueApplyCallEnvelope<'_, TxSource, QueueApplyAppViewSnapshot>,
    ) -> R,
) -> R
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
{
    with_queue_apply_app_view_call(tx_source, view, metrics_snapshot, |call| run(owner, call))
}

fn with_queue_apply_app_view_metrics_snapshot_current_app_view_caller_preclaim_log_sinks<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    View,
    TxSource,
    R,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    view: &View,
    tx_source: &TxSource,
    metrics_snapshot: QueueFeeMetricsSnapshot,
    run: impl FnOnce(
        &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        QueueApplyCallEnvelope<'_, TxSource, QueueApplyAppViewSnapshot>,
    ) -> R,
) -> R
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
{
    with_queue_apply_app_view_call(tx_source, view, metrics_snapshot, |call| run(owner, call))
}

fn with_runtime_envelope_app_view_call<
    'app,
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxSource,
    App,
    R,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &'app mut App,
    call: QueueApplyCallEnvelope<'_, TxSource, QueueApplyAppViewSnapshot>,
    run: impl FnOnce(
        &mut QueueApplyRuntimeEnvelope<'app, App>,
        &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        QueueApplyCallEnvelope<'_, TxSource, QueueApplyAppViewSnapshot>,
    ) -> R,
) -> R {
    let mut runtime = QueueApplyRuntimeEnvelope::new(app);
    run(&mut runtime, owner, call)
}

fn with_runtime_envelope_current_app_view_caller_preclaim_log_sinks<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
    R,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    run: impl FnOnce(
        &mut QueueApplyRuntimeEnvelope<'_, App>,
        &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        QueueApplyCallEnvelope<'_, TxSource, QueueApplyAppViewSnapshot>,
    ) -> R,
) -> R
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
{
    with_current_queue_apply_app_view_caller_preclaim_log_sinks(
        owner,
        view,
        tx_source,
        |owner, call| with_runtime_envelope_app_view_call(owner, app, call, run),
    )
}

fn with_runtime_envelope_metrics_snapshot_current_app_view_caller_preclaim_log_sinks<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
    R,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    metrics_snapshot: QueueFeeMetricsSnapshot,
    run: impl FnOnce(
        &mut QueueApplyRuntimeEnvelope<'_, App>,
        &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        QueueApplyCallEnvelope<'_, TxSource, QueueApplyAppViewSnapshot>,
    ) -> R,
) -> R
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
{
    with_queue_apply_app_view_metrics_snapshot_current_app_view_caller_preclaim_log_sinks(
        owner,
        view,
        tx_source,
        metrics_snapshot,
        |owner, call| with_runtime_envelope_app_view_call(owner, app, call, run),
    )
}

fn with_runtime_envelope_current_app_view_log_sinks<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
    R,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    run: impl FnOnce(
        &mut QueueApplyRuntimeEnvelope<'_, App>,
        &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        QueueApplyCallEnvelope<'_, TxSource, QueueApplyAppViewSnapshot>,
    ) -> R,
) -> R
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
{
    with_current_queue_apply_app_view_log_sinks(owner, view, tx_source, |owner, call| {
        with_runtime_envelope_app_view_call(owner, app, call, run)
    })
}

fn with_runtime_envelope_metrics_snapshot_current_app_view_log_sinks<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
    R,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    metrics_snapshot: QueueFeeMetricsSnapshot,
    run: impl FnOnce(
        &mut QueueApplyRuntimeEnvelope<'_, App>,
        &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        QueueApplyCallEnvelope<'_, TxSource, QueueApplyAppViewSnapshot>,
    ) -> R,
) -> R
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
{
    with_queue_apply_app_view_metrics_snapshot_current_app_view_log_sinks(
        owner,
        view,
        tx_source,
        metrics_snapshot,
        |owner, call| with_runtime_envelope_app_view_call(owner, app, call, run),
    )
}

fn with_runtime_envelope_current_app_view<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
    R,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    run: impl FnOnce(
        &mut QueueApplyRuntimeEnvelope<'_, App>,
        &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        QueueApplyCallEnvelope<'_, TxSource, QueueApplyAppViewSnapshot>,
    ) -> R,
) -> R
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
{
    with_current_queue_apply_app_view(owner, view, tx_source, |owner, call| {
        with_runtime_envelope_app_view_call(owner, app, call, run)
    })
}

fn with_runtime_envelope_metrics_snapshot_current_app_view<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
    R,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    metrics_snapshot: QueueFeeMetricsSnapshot,
    run: impl FnOnce(
        &mut QueueApplyRuntimeEnvelope<'_, App>,
        &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        QueueApplyCallEnvelope<'_, TxSource, QueueApplyAppViewSnapshot>,
    ) -> R,
) -> R
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
{
    with_queue_apply_app_view_metrics_snapshot_current_app_view(
        owner,
        view,
        tx_source,
        metrics_snapshot,
        |owner, call| with_runtime_envelope_app_view_call(owner, app, call, run),
    )
}

fn with_runtime_envelope_current_app_view_caller_preclaim<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
    R,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    run: impl FnOnce(
        &mut QueueApplyRuntimeEnvelope<'_, App>,
        &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        QueueApplyCallEnvelope<'_, TxSource, QueueApplyAppViewSnapshot>,
    ) -> R,
) -> R
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
{
    with_current_queue_apply_app_view_caller_preclaim(owner, view, tx_source, |owner, call| {
        with_runtime_envelope_app_view_call(owner, app, call, run)
    })
}

fn with_runtime_envelope_metrics_snapshot_current_app_view_caller_preclaim<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
    R,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    metrics_snapshot: QueueFeeMetricsSnapshot,
    run: impl FnOnce(
        &mut QueueApplyRuntimeEnvelope<'_, App>,
        &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        QueueApplyCallEnvelope<'_, TxSource, QueueApplyAppViewSnapshot>,
    ) -> R,
) -> R
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
{
    with_queue_apply_app_view_metrics_snapshot_current_app_view_caller_preclaim(
        owner,
        view,
        tx_source,
        metrics_snapshot,
        |owner, call| with_runtime_envelope_app_view_call(owner, app, call, run),
    )
}

fn with_queue_apply_app_view_metrics_snapshot_current_app_view_log_sinks_hold_admission<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    View,
    TxSource,
    R,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    view: &View,
    tx_source: &TxSource,
    metrics_snapshot: QueueFeeMetricsSnapshot,
    hold_preflight: QueueHoldPreflight,
    run: impl FnOnce(
        &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        QueueFeeMetricsSnapshot,
        Ter,
    ) -> R,
) -> R
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
{
    with_queue_apply_app_view_hold_admission::<Account, Tx, Journal, ParentBatchId, TxId, _, _, _>(
        owner,
        tx_source,
        view,
        metrics_snapshot,
        hold_preflight,
        run,
    )
}

fn with_queue_apply_app_view_metrics_snapshot_current_app_view_log_sinks_derived_hold_preflight<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    View,
    TxSource,
    R,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    _view: &View,
    tx_source: &TxSource,
    metrics_snapshot: QueueFeeMetricsSnapshot,
    flags: ApplyFlags,
    run: impl FnOnce(
        &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        QueueFeeMetricsSnapshot,
        QueueHoldPreflight,
    ) -> R,
) -> R
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
{
    with_queue_apply_app_view_derived_hold_preflight(tx_source, flags, |hold_preflight| {
        run(owner, metrics_snapshot, hold_preflight)
    })
}

fn with_queue_apply_app_view_hold_admission<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    View,
    TxSource,
    R,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    tx_source: &TxSource,
    view: &View,
    metrics_snapshot: QueueFeeMetricsSnapshot,
    hold_preflight: QueueHoldPreflight,
    run: impl FnOnce(
        &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        QueueFeeMetricsSnapshot,
        Ter,
    ) -> R,
) -> R
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
{
    let snapshot = snapshot_queue_apply_app_view_with_metrics(tx_source, view, metrics_snapshot);
    let can_be_held_result =
        owner
            .owner()
            .derive_can_be_held_result(tx_source, &snapshot, hold_preflight);
    run(owner, snapshot.metrics_snapshot, can_be_held_result)
}

fn with_current_queue_apply_app_view_hold_admission<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    View,
    TxSource,
    R,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    tx_source: &TxSource,
    view: &View,
    hold_preflight: QueueHoldPreflight,
    run: impl FnOnce(
        &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        QueueFeeMetricsSnapshot,
        Ter,
    ) -> R,
) -> R
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
{
    with_queue_apply_app_view_metrics_snapshot(view, |metrics_snapshot| {
        with_queue_apply_app_view_hold_admission::<Account, Tx, Journal, ParentBatchId, TxId, _, _, _>(
            owner,
            tx_source,
            view,
            metrics_snapshot,
            hold_preflight,
            run,
        )
    })
}

fn with_queue_apply_app_view_derived_hold_preflight<Account, TxSource, R>(
    tx_source: &TxSource,
    flags: ApplyFlags,
    run: impl FnOnce(QueueHoldPreflight) -> R,
) -> R
where
    TxSource: QueueApplyHoldPreflightTxSource<Account = Account>,
{
    run(derive_queue_hold_preflight_from_tx_source(tx_source, flags))
}

pub fn run_queue_apply_with_app_view<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    hold_preflight: QueueHoldPreflight,
    flags: ApplyFlags,
    consequences: TxConsequences,
    can_be_held_result: Ter,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
{
    with_runtime_envelope_current_app_view::<Account, Tx, Journal, ParentBatchId, TxId, App, _, _, _>(
        owner,
        app,
        view,
        tx_source,
        |runtime, owner, call| {
            runtime.apply(
                owner,
                &call,
                hold_preflight,
                flags,
                consequences,
                can_be_held_result,
            )
        },
    )
}

pub fn run_queue_apply_with_app_view_and_log_messages<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    hold_preflight: QueueHoldPreflight,
    flags: ApplyFlags,
    consequences: TxConsequences,
    can_be_held_result: Ter,
) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
{
    with_runtime_envelope_current_app_view::<Account, Tx, Journal, ParentBatchId, TxId, App, _, _, _>(
        owner,
        app,
        view,
        tx_source,
        |runtime, owner, call| {
            runtime.apply_with_log_messages(
                owner,
                &call,
                hold_preflight,
                flags,
                consequences,
                can_be_held_result,
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_with_app_view_and_metrics_snapshot<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    metrics_snapshot: QueueFeeMetricsSnapshot,
    hold_preflight: QueueHoldPreflight,
    flags: ApplyFlags,
    consequences: TxConsequences,
    can_be_held_result: Ter,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
{
    with_runtime_envelope_metrics_snapshot_current_app_view::<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        App,
        _,
        _,
        _,
    >(
        owner,
        app,
        view,
        tx_source,
        metrics_snapshot,
        |runtime, owner, call| {
            runtime.apply(
                owner,
                &call,
                hold_preflight,
                flags,
                consequences,
                can_be_held_result,
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_with_app_view_and_metrics_snapshot_and_log_messages<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    metrics_snapshot: QueueFeeMetricsSnapshot,
    hold_preflight: QueueHoldPreflight,
    flags: ApplyFlags,
    consequences: TxConsequences,
    can_be_held_result: Ter,
) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
{
    with_runtime_envelope_metrics_snapshot_current_app_view::<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        App,
        _,
        _,
        _,
    >(
        owner,
        app,
        view,
        tx_source,
        metrics_snapshot,
        |runtime, owner, call| {
            runtime.apply_with_log_messages(
                owner,
                &call,
                hold_preflight,
                flags,
                consequences,
                can_be_held_result,
            )
        },
    )
}

pub fn run_queue_apply_with_app_view_and_derived_hold_admission<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    hold_preflight: QueueHoldPreflight,
    flags: ApplyFlags,
    consequences: TxConsequences,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
{
    with_current_queue_apply_app_view_hold_admission::<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        _,
        _,
        _,
    >(
        owner,
        tx_source,
        view,
        hold_preflight,
        |owner, metrics_snapshot, can_be_held_result| {
            run_queue_apply_with_app_view_and_metrics_snapshot(
                owner,
                app,
                view,
                tx_source,
                metrics_snapshot,
                hold_preflight,
                flags,
                consequences,
                can_be_held_result,
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_with_app_view_and_derived_hold_admission_and_log_messages<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    hold_preflight: QueueHoldPreflight,
    flags: ApplyFlags,
    consequences: TxConsequences,
) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
{
    let queue_log_messages = RefCell::new(QueueApplyQueueLogMessages::default());
    let stage = run_queue_apply_with_app_view_and_log_sinks_and_derived_hold_admission(
        owner,
        app,
        view,
        tx_source,
        hold_preflight,
        flags,
        consequences,
        |_message| {},
        |message| queue_log_messages.borrow_mut().debug.push(message),
        |message| queue_log_messages.borrow_mut().info.push(message),
    );
    QueueApplyTopWithLogMessagesResult {
        stage,
        queue_log_messages: queue_log_messages.into_inner(),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_with_app_view_and_metrics_snapshot_and_derived_hold_admission_and_log_messages<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    metrics_snapshot: QueueFeeMetricsSnapshot,
    hold_preflight: QueueHoldPreflight,
    flags: ApplyFlags,
    consequences: TxConsequences,
) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
{
    let queue_log_messages = RefCell::new(QueueApplyQueueLogMessages::default());
    let stage =
        run_queue_apply_with_app_view_and_metrics_snapshot_and_log_sinks_and_derived_hold_admission(
            owner,
            app,
            view,
            tx_source,
            metrics_snapshot,
            hold_preflight,
            flags,
            consequences,
            |_message| {},
            |message| queue_log_messages.borrow_mut().debug.push(message),
            |message| queue_log_messages.borrow_mut().info.push(message),
        );
    QueueApplyTopWithLogMessagesResult {
        stage,
        queue_log_messages: queue_log_messages.into_inner(),
    }
}

pub fn run_queue_apply_with_app_view_and_derived_preflight_facts<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    can_be_held_result: Ter,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
{
    with_runtime_envelope_current_app_view::<Account, Tx, Journal, ParentBatchId, TxId, App, _, _, _>(
        owner,
        app,
        view,
        tx_source,
        |runtime, owner, call| {
            runtime.apply_with_derived_preflight_facts(owner, &call, can_be_held_result)
        },
    )
}

pub fn run_queue_apply_with_app_view_and_derived_preflight_facts_and_log_messages<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    can_be_held_result: Ter,
) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
{
    let queue_log_messages = RefCell::new(QueueApplyQueueLogMessages::default());
    let stage = run_queue_apply_with_app_view_and_log_sinks_and_derived_preflight_facts(
        owner,
        app,
        view,
        tx_source,
        can_be_held_result,
        |_message| {},
        |message| queue_log_messages.borrow_mut().debug.push(message),
        |message| queue_log_messages.borrow_mut().info.push(message),
    );
    QueueApplyTopWithLogMessagesResult {
        stage,
        queue_log_messages: queue_log_messages.into_inner(),
    }
}

pub fn run_queue_apply_with_app_view_and_derived_preflight_facts_and_hold_admission<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
{
    with_runtime_envelope_current_app_view::<Account, Tx, Journal, ParentBatchId, TxId, App, _, _, _>(
        owner,
        app,
        view,
        tx_source,
        |runtime, owner, call| {
            runtime.apply_with_derived_preflight_facts_and_hold_admission(owner, &call)
        },
    )
}

pub fn run_queue_apply_with_app_view_and_derived_preflight_facts_and_hold_admission_and_log_messages<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
{
    let queue_log_messages = RefCell::new(QueueApplyQueueLogMessages::default());
    let stage =
        run_queue_apply_with_app_view_and_log_sinks_and_derived_preflight_facts_and_hold_admission(
            owner,
            app,
            view,
            tx_source,
            |_message| {},
            |message| queue_log_messages.borrow_mut().debug.push(message),
            |message| queue_log_messages.borrow_mut().info.push(message),
        );
    QueueApplyTopWithLogMessagesResult {
        stage,
        queue_log_messages: queue_log_messages.into_inner(),
    }
}

pub fn run_queue_apply_with_app_view_and_derived_hold_preflight<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    flags: ApplyFlags,
    consequences: TxConsequences,
    can_be_held_result: Ter,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
{
    with_queue_apply_app_view_derived_hold_preflight(tx_source, flags, |hold_preflight| {
        run_queue_apply_with_app_view(
            owner,
            app,
            view,
            tx_source,
            hold_preflight,
            flags,
            consequences,
            can_be_held_result,
        )
    })
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_with_app_view_and_derived_hold_preflight_and_log_messages<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    flags: ApplyFlags,
    consequences: TxConsequences,
    can_be_held_result: Ter,
) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
{
    let queue_log_messages = RefCell::new(QueueApplyQueueLogMessages::default());
    let stage = run_queue_apply_with_app_view_and_log_sinks_and_derived_hold_preflight(
        owner,
        app,
        view,
        tx_source,
        flags,
        consequences,
        can_be_held_result,
        |_message| {},
        |message| queue_log_messages.borrow_mut().debug.push(message),
        |message| queue_log_messages.borrow_mut().info.push(message),
    );
    QueueApplyTopWithLogMessagesResult {
        stage,
        queue_log_messages: queue_log_messages.into_inner(),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_with_app_view_and_metrics_snapshot_and_derived_hold_preflight_and_log_messages<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    metrics_snapshot: QueueFeeMetricsSnapshot,
    flags: ApplyFlags,
    consequences: TxConsequences,
    can_be_held_result: Ter,
) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
{
    let queue_log_messages = RefCell::new(QueueApplyQueueLogMessages::default());
    let stage =
        run_queue_apply_with_app_view_and_metrics_snapshot_and_log_sinks_and_derived_hold_preflight(
            owner,
            app,
            view,
            tx_source,
            metrics_snapshot,
            flags,
            consequences,
            can_be_held_result,
            |_message| {},
            |message| queue_log_messages.borrow_mut().debug.push(message),
            |message| queue_log_messages.borrow_mut().info.push(message),
        );
    QueueApplyTopWithLogMessagesResult {
        stage,
        queue_log_messages: queue_log_messages.into_inner(),
    }
}

pub fn run_queue_apply_after_preflight_with_app_view<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    hold_preflight: QueueHoldPreflight,
    can_be_held_result: Ter,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
{
    with_runtime_envelope_current_app_view::<Account, Tx, Journal, ParentBatchId, TxId, App, _, _, _>(
        owner,
        app,
        view,
        tx_source,
        |runtime, owner, call| {
            runtime.apply_after_preflight(
                owner,
                &call,
                preflight_result,
                hold_preflight,
                can_be_held_result,
            )
        },
    )
}

pub fn run_queue_apply_after_preflight_with_app_view_and_log_messages<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    hold_preflight: QueueHoldPreflight,
    can_be_held_result: Ter,
) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
{
    with_runtime_envelope_current_app_view::<Account, Tx, Journal, ParentBatchId, TxId, App, _, _, _>(
        owner,
        app,
        view,
        tx_source,
        |runtime, owner, call| {
            runtime.apply_after_preflight_with_log_messages(
                owner,
                &call,
                preflight_result,
                hold_preflight,
                can_be_held_result,
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    metrics_snapshot: QueueFeeMetricsSnapshot,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    hold_preflight: QueueHoldPreflight,
    can_be_held_result: Ter,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
{
    with_runtime_envelope_metrics_snapshot_current_app_view::<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        App,
        _,
        _,
        _,
    >(
        owner,
        app,
        view,
        tx_source,
        metrics_snapshot,
        |runtime, owner, call| {
            runtime.apply_after_preflight(
                owner,
                &call,
                preflight_result,
                hold_preflight,
                can_be_held_result,
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot_and_log_messages<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    metrics_snapshot: QueueFeeMetricsSnapshot,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    hold_preflight: QueueHoldPreflight,
    can_be_held_result: Ter,
) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
{
    with_runtime_envelope_metrics_snapshot_current_app_view::<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        App,
        _,
        _,
        _,
    >(
        owner,
        app,
        view,
        tx_source,
        metrics_snapshot,
        |runtime, owner, call| {
            runtime.apply_after_preflight_with_log_messages(
                owner,
                &call,
                preflight_result,
                hold_preflight,
                can_be_held_result,
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_with_app_view_and_caller_preclaim<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
    RunPreclaimStage,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    hold_preflight: QueueHoldPreflight,
    flags: ApplyFlags,
    consequences: TxConsequences,
    can_be_held_result: Ter,
    run_preclaim_stage: RunPreclaimStage,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>
        + QueueApplyCurrentPreclaimClearRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    RunPreclaimStage:
        FnOnce(
            QueueApplyPreparedPreclaimInputs<Account>,
        )
            -> Result<QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>, crate::ApplyResult>,
{
    with_runtime_envelope_current_app_view_caller_preclaim::<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        App,
        _,
        _,
        _,
    >(owner, app, view, tx_source, |runtime, owner, call| {
        runtime.apply_with_caller_preclaim(
            owner,
            &call,
            hold_preflight,
            flags,
            consequences,
            can_be_held_result,
            run_preclaim_stage,
        )
    })
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_with_app_view_and_caller_preclaim_and_log_messages<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
    RunPreclaimStage,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    hold_preflight: QueueHoldPreflight,
    flags: ApplyFlags,
    consequences: TxConsequences,
    can_be_held_result: Ter,
    run_preclaim_stage: RunPreclaimStage,
) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>
        + QueueApplyCurrentPreclaimClearRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    RunPreclaimStage:
        FnOnce(
            QueueApplyPreparedPreclaimInputs<Account>,
        )
            -> Result<QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>, crate::ApplyResult>,
{
    with_runtime_envelope_current_app_view_caller_preclaim::<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        App,
        _,
        _,
        _,
    >(owner, app, view, tx_source, |runtime, owner, call| {
        runtime.apply_with_caller_preclaim_and_log_messages(
            owner,
            &call,
            hold_preflight,
            flags,
            consequences,
            can_be_held_result,
            run_preclaim_stage,
        )
    })
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_with_app_view_and_metrics_snapshot_and_caller_preclaim<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
    RunPreclaimStage,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    metrics_snapshot: QueueFeeMetricsSnapshot,
    hold_preflight: QueueHoldPreflight,
    flags: ApplyFlags,
    consequences: TxConsequences,
    can_be_held_result: Ter,
    run_preclaim_stage: RunPreclaimStage,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>
        + QueueApplyCurrentPreclaimClearRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    RunPreclaimStage:
        FnOnce(
            QueueApplyPreparedPreclaimInputs<Account>,
        )
            -> Result<QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>, crate::ApplyResult>,
{
    with_runtime_envelope_metrics_snapshot_current_app_view_caller_preclaim::<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        App,
        _,
        _,
        _,
    >(
        owner,
        app,
        view,
        tx_source,
        metrics_snapshot,
        |runtime, owner, call| {
            runtime.apply_with_caller_preclaim(
                owner,
                &call,
                hold_preflight,
                flags,
                consequences,
                can_be_held_result,
                run_preclaim_stage,
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_with_app_view_and_metrics_snapshot_and_caller_preclaim_and_log_messages<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
    RunPreclaimStage,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    metrics_snapshot: QueueFeeMetricsSnapshot,
    hold_preflight: QueueHoldPreflight,
    flags: ApplyFlags,
    consequences: TxConsequences,
    can_be_held_result: Ter,
    run_preclaim_stage: RunPreclaimStage,
) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>
        + QueueApplyCurrentPreclaimClearRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    RunPreclaimStage:
        FnOnce(
            QueueApplyPreparedPreclaimInputs<Account>,
        )
            -> Result<QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>, crate::ApplyResult>,
{
    with_runtime_envelope_metrics_snapshot_current_app_view_caller_preclaim::<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        App,
        _,
        _,
        _,
    >(
        owner,
        app,
        view,
        tx_source,
        metrics_snapshot,
        |runtime, owner, call| {
            runtime.apply_with_caller_preclaim_and_log_messages(
                owner,
                &call,
                hold_preflight,
                flags,
                consequences,
                can_be_held_result,
                run_preclaim_stage,
            )
        },
    )
}

pub fn run_queue_apply_after_preflight_with_app_view_and_derived_hold_admission<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    hold_preflight: QueueHoldPreflight,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
{
    with_current_queue_apply_app_view_hold_admission::<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        _,
        _,
        _,
    >(
        owner,
        tx_source,
        view,
        hold_preflight,
        |owner, metrics_snapshot, can_be_held_result| {
            run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot(
                owner,
                app,
                view,
                tx_source,
                metrics_snapshot,
                preflight_result,
                hold_preflight,
                can_be_held_result,
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_after_preflight_with_app_view_and_derived_hold_admission_and_log_messages<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    hold_preflight: QueueHoldPreflight,
) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
{
    let queue_log_messages = RefCell::new(QueueApplyQueueLogMessages::default());
    let stage =
        run_queue_apply_after_preflight_with_app_view_and_log_sinks_and_derived_hold_admission(
            owner,
            app,
            view,
            tx_source,
            preflight_result,
            hold_preflight,
            |_message| {},
            |message| queue_log_messages.borrow_mut().debug.push(message),
            |message| queue_log_messages.borrow_mut().info.push(message),
        );
    QueueApplyTopWithLogMessagesResult {
        stage,
        queue_log_messages: queue_log_messages.into_inner(),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot_and_derived_hold_admission_and_log_messages<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    metrics_snapshot: QueueFeeMetricsSnapshot,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    hold_preflight: QueueHoldPreflight,
) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
{
    let queue_log_messages = RefCell::new(QueueApplyQueueLogMessages::default());
    let stage = run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot_and_log_sinks_and_derived_hold_admission(
        owner,
        app,
        view,
        tx_source,
        metrics_snapshot,
        preflight_result,
        hold_preflight,
        |_message| {},
        |message| queue_log_messages.borrow_mut().debug.push(message),
        |message| queue_log_messages.borrow_mut().info.push(message),
    );
    QueueApplyTopWithLogMessagesResult {
        stage,
        queue_log_messages: queue_log_messages.into_inner(),
    }
}

pub fn run_queue_apply_after_preflight_with_app_view_and_derived_hold_preflight<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    can_be_held_result: Ter,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
{
    with_queue_apply_app_view_derived_hold_preflight(
        tx_source,
        preflight_result.flags,
        |hold_preflight| {
            run_queue_apply_after_preflight_with_app_view(
                owner,
                app,
                view,
                tx_source,
                preflight_result,
                hold_preflight,
                can_be_held_result,
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_after_preflight_with_app_view_and_derived_hold_preflight_and_log_messages<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    can_be_held_result: Ter,
) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
{
    let queue_log_messages = RefCell::new(QueueApplyQueueLogMessages::default());
    let stage =
        run_queue_apply_after_preflight_with_app_view_and_log_sinks_and_derived_hold_preflight(
            owner,
            app,
            view,
            tx_source,
            preflight_result,
            can_be_held_result,
            |_message| {},
            |message| queue_log_messages.borrow_mut().debug.push(message),
            |message| queue_log_messages.borrow_mut().info.push(message),
        );
    QueueApplyTopWithLogMessagesResult {
        stage,
        queue_log_messages: queue_log_messages.into_inner(),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot_and_derived_hold_preflight_and_log_messages<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    metrics_snapshot: QueueFeeMetricsSnapshot,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    can_be_held_result: Ter,
) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
{
    let queue_log_messages = RefCell::new(QueueApplyQueueLogMessages::default());
    let stage = run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot_and_log_sinks_and_derived_hold_preflight(
        owner,
        app,
        view,
        tx_source,
        metrics_snapshot,
        preflight_result,
        can_be_held_result,
        |_message| {},
        |message| queue_log_messages.borrow_mut().debug.push(message),
        |message| queue_log_messages.borrow_mut().info.push(message),
    );
    QueueApplyTopWithLogMessagesResult {
        stage,
        queue_log_messages: queue_log_messages.into_inner(),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_after_preflight_with_app_view_and_caller_preclaim<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
    RunPreclaimStage,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    hold_preflight: QueueHoldPreflight,
    can_be_held_result: Ter,
    run_preclaim_stage: RunPreclaimStage,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>
        + QueueApplyCurrentPreclaimClearRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    RunPreclaimStage:
        FnOnce(
            QueueApplyPreparedPreclaimInputs<Account>,
        )
            -> Result<QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>, crate::ApplyResult>,
{
    with_runtime_envelope_current_app_view_caller_preclaim::<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        App,
        _,
        _,
        _,
    >(owner, app, view, tx_source, |runtime, owner, call| {
        runtime.apply_after_preflight_with_caller_preclaim(
            owner,
            &call,
            preflight_result,
            hold_preflight,
            can_be_held_result,
            run_preclaim_stage,
        )
    })
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_after_preflight_with_app_view_and_caller_preclaim_and_log_messages<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
    RunPreclaimStage,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    hold_preflight: QueueHoldPreflight,
    can_be_held_result: Ter,
    run_preclaim_stage: RunPreclaimStage,
) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>
        + QueueApplyCurrentPreclaimClearRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    RunPreclaimStage:
        FnOnce(
            QueueApplyPreparedPreclaimInputs<Account>,
        )
            -> Result<QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>, crate::ApplyResult>,
{
    with_runtime_envelope_current_app_view_caller_preclaim::<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        App,
        _,
        _,
        _,
    >(owner, app, view, tx_source, |runtime, owner, call| {
        runtime.apply_after_preflight_with_caller_preclaim_and_log_messages(
            owner,
            &call,
            preflight_result,
            hold_preflight,
            can_be_held_result,
            run_preclaim_stage,
        )
    })
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot_and_caller_preclaim<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
    RunPreclaimStage,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    metrics_snapshot: QueueFeeMetricsSnapshot,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    hold_preflight: QueueHoldPreflight,
    can_be_held_result: Ter,
    run_preclaim_stage: RunPreclaimStage,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>
        + QueueApplyCurrentPreclaimClearRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    RunPreclaimStage:
        FnOnce(
            QueueApplyPreparedPreclaimInputs<Account>,
        )
            -> Result<QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>, crate::ApplyResult>,
{
    with_runtime_envelope_metrics_snapshot_current_app_view_caller_preclaim::<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        App,
        _,
        _,
        _,
    >(
        owner,
        app,
        view,
        tx_source,
        metrics_snapshot,
        |runtime, owner, call| {
            runtime.apply_after_preflight_with_caller_preclaim(
                owner,
                &call,
                preflight_result,
                hold_preflight,
                can_be_held_result,
                run_preclaim_stage,
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot_and_caller_preclaim_and_log_messages<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
    RunPreclaimStage,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    metrics_snapshot: QueueFeeMetricsSnapshot,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    hold_preflight: QueueHoldPreflight,
    can_be_held_result: Ter,
    run_preclaim_stage: RunPreclaimStage,
) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>
        + QueueApplyCurrentPreclaimClearRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    RunPreclaimStage:
        FnOnce(
            QueueApplyPreparedPreclaimInputs<Account>,
        )
            -> Result<QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>, crate::ApplyResult>,
{
    with_runtime_envelope_metrics_snapshot_current_app_view_caller_preclaim::<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        App,
        _,
        _,
        _,
    >(
        owner,
        app,
        view,
        tx_source,
        metrics_snapshot,
        |runtime, owner, call| {
            runtime.apply_after_preflight_with_caller_preclaim_and_log_messages(
                owner,
                &call,
                preflight_result,
                hold_preflight,
                can_be_held_result,
                run_preclaim_stage,
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_with_app_view_and_caller_preclaim_and_log_sinks<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
    RunPreclaimStage,
    TraceFn,
    DebugFn,
    InfoFn,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    hold_preflight: QueueHoldPreflight,
    flags: ApplyFlags,
    consequences: TxConsequences,
    can_be_held_result: Ter,
    run_preclaim_stage: RunPreclaimStage,
    trace: TraceFn,
    debug: DebugFn,
    info: InfoFn,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>
        + QueueApplyCurrentPreclaimClearRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    RunPreclaimStage:
        FnOnce(
            QueueApplyPreparedPreclaimInputs<Account>,
        )
            -> Result<QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>, crate::ApplyResult>,
    TraceFn: FnMut(&str),
    DebugFn: FnMut(String),
    InfoFn: FnMut(String),
{
    with_runtime_envelope_current_app_view_caller_preclaim_log_sinks::<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        App,
        _,
        _,
        _,
    >(owner, app, view, tx_source, |runtime, owner, call| {
        runtime.apply_with_caller_preclaim_and_log_sinks(
            owner,
            &call,
            hold_preflight,
            flags,
            consequences,
            can_be_held_result,
            run_preclaim_stage,
            trace,
            debug,
            info,
        )
    })
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_with_app_view_and_metrics_snapshot_and_caller_preclaim_and_log_sinks<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
    RunPreclaimStage,
    TraceFn,
    DebugFn,
    InfoFn,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    metrics_snapshot: QueueFeeMetricsSnapshot,
    hold_preflight: QueueHoldPreflight,
    flags: ApplyFlags,
    consequences: TxConsequences,
    can_be_held_result: Ter,
    run_preclaim_stage: RunPreclaimStage,
    trace: TraceFn,
    debug: DebugFn,
    info: InfoFn,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>
        + QueueApplyCurrentPreclaimClearRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    RunPreclaimStage:
        FnOnce(
            QueueApplyPreparedPreclaimInputs<Account>,
        )
            -> Result<QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>, crate::ApplyResult>,
    TraceFn: FnMut(&str),
    DebugFn: FnMut(String),
    InfoFn: FnMut(String),
{
    with_runtime_envelope_metrics_snapshot_current_app_view_caller_preclaim_log_sinks::<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        App,
        _,
        _,
        _,
    >(
        owner,
        app,
        view,
        tx_source,
        metrics_snapshot,
        |runtime, owner, call| {
            runtime.apply_with_caller_preclaim_and_log_sinks(
                owner,
                &call,
                hold_preflight,
                flags,
                consequences,
                can_be_held_result,
                run_preclaim_stage,
                trace,
                debug,
                info,
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_with_app_view_and_log_sinks<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
    TraceFn,
    DebugFn,
    InfoFn,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    hold_preflight: QueueHoldPreflight,
    flags: ApplyFlags,
    consequences: TxConsequences,
    can_be_held_result: Ter,
    trace: TraceFn,
    debug: DebugFn,
    info: InfoFn,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    TraceFn: FnMut(&str),
    DebugFn: FnMut(String),
    InfoFn: FnMut(String),
{
    with_runtime_envelope_current_app_view_log_sinks::<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        App,
        _,
        _,
        _,
    >(owner, app, view, tx_source, |runtime, owner, call| {
        runtime.apply_with_log_sinks(
            owner,
            &call,
            hold_preflight,
            flags,
            consequences,
            can_be_held_result,
            trace,
            debug,
            info,
        )
    })
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_with_app_view_and_metrics_snapshot_and_log_sinks<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
    TraceFn,
    DebugFn,
    InfoFn,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    metrics_snapshot: QueueFeeMetricsSnapshot,
    hold_preflight: QueueHoldPreflight,
    flags: ApplyFlags,
    consequences: TxConsequences,
    can_be_held_result: Ter,
    trace: TraceFn,
    debug: DebugFn,
    info: InfoFn,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    TraceFn: FnMut(&str),
    DebugFn: FnMut(String),
    InfoFn: FnMut(String),
{
    with_runtime_envelope_metrics_snapshot_current_app_view_log_sinks::<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        App,
        _,
        _,
        _,
    >(
        owner,
        app,
        view,
        tx_source,
        metrics_snapshot,
        |runtime, owner, call| {
            runtime.apply_with_log_sinks(
                owner,
                &call,
                hold_preflight,
                flags,
                consequences,
                can_be_held_result,
                trace,
                debug,
                info,
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_with_app_view_and_log_sinks_and_derived_hold_admission<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
    TraceFn,
    DebugFn,
    InfoFn,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    hold_preflight: QueueHoldPreflight,
    flags: ApplyFlags,
    consequences: TxConsequences,
    trace: TraceFn,
    debug: DebugFn,
    info: InfoFn,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    TraceFn: FnMut(&str),
    DebugFn: FnMut(String),
    InfoFn: FnMut(String),
{
    with_current_queue_apply_app_view_hold_admission::<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        _,
        _,
        _,
    >(
        owner,
        tx_source,
        view,
        hold_preflight,
        |owner, metrics_snapshot, can_be_held_result| {
            run_queue_apply_with_app_view_and_metrics_snapshot_and_log_sinks(
                owner,
                app,
                view,
                tx_source,
                metrics_snapshot,
                hold_preflight,
                flags,
                consequences,
                can_be_held_result,
                trace,
                debug,
                info,
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_with_app_view_and_metrics_snapshot_and_log_sinks_and_derived_hold_admission<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
    TraceFn,
    DebugFn,
    InfoFn,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    metrics_snapshot: QueueFeeMetricsSnapshot,
    hold_preflight: QueueHoldPreflight,
    flags: ApplyFlags,
    consequences: TxConsequences,
    trace: TraceFn,
    debug: DebugFn,
    info: InfoFn,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    TraceFn: FnMut(&str),
    DebugFn: FnMut(String),
    InfoFn: FnMut(String),
{
    with_queue_apply_app_view_metrics_snapshot_current_app_view_log_sinks_hold_admission::<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        _,
        _,
        _,
    >(
        owner,
        view,
        tx_source,
        metrics_snapshot,
        hold_preflight,
        |owner, metrics_snapshot, can_be_held_result| {
            run_queue_apply_with_app_view_and_metrics_snapshot_and_log_sinks(
                owner,
                app,
                view,
                tx_source,
                metrics_snapshot,
                hold_preflight,
                flags,
                consequences,
                can_be_held_result,
                trace,
                debug,
                info,
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_with_app_view_and_log_sinks_and_derived_preflight_facts<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
    TraceFn,
    DebugFn,
    InfoFn,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    can_be_held_result: Ter,
    trace: TraceFn,
    debug: DebugFn,
    info: InfoFn,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    TraceFn: FnMut(&str),
    DebugFn: FnMut(String),
    InfoFn: FnMut(String),
{
    with_runtime_envelope_current_app_view_log_sinks::<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        App,
        _,
        _,
        _,
    >(owner, app, view, tx_source, |runtime, owner, call| {
        runtime.apply_with_log_sinks_and_derived_preflight_facts(
            owner,
            &call,
            can_be_held_result,
            trace,
            debug,
            info,
        )
    })
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_with_app_view_and_metrics_snapshot_and_log_sinks_and_derived_preflight_facts<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
    TraceFn,
    DebugFn,
    InfoFn,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    metrics_snapshot: QueueFeeMetricsSnapshot,
    trace: TraceFn,
    debug: DebugFn,
    info: InfoFn,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    TraceFn: FnMut(&str),
    DebugFn: FnMut(String),
    InfoFn: FnMut(String),
{
    with_runtime_envelope_metrics_snapshot_current_app_view_log_sinks::<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        App,
        _,
        _,
        _,
    >(
        owner,
        app,
        view,
        tx_source,
        metrics_snapshot,
        |runtime, owner, call| {
            runtime.apply_with_log_sinks_and_derived_preflight_facts_and_hold_admission(
                owner, &call, trace, debug, info,
            )
        },
    )
}

pub fn run_queue_apply_with_app_view_and_metrics_snapshot_and_derived_preflight_facts_and_log_messages<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    metrics_snapshot: QueueFeeMetricsSnapshot,
) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
{
    let queue_log_messages = RefCell::new(QueueApplyQueueLogMessages::default());
    let stage =
        run_queue_apply_with_app_view_and_metrics_snapshot_and_log_sinks_and_derived_preflight_facts(
            owner,
            app,
            view,
            tx_source,
            metrics_snapshot,
            |_message| {},
            |message| queue_log_messages.borrow_mut().debug.push(message),
            |message| queue_log_messages.borrow_mut().info.push(message),
        );
    QueueApplyTopWithLogMessagesResult {
        stage,
        queue_log_messages: queue_log_messages.into_inner(),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_with_app_view_and_log_sinks_and_derived_preflight_facts_and_hold_admission<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
    TraceFn,
    DebugFn,
    InfoFn,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    trace: TraceFn,
    debug: DebugFn,
    info: InfoFn,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    TraceFn: FnMut(&str),
    DebugFn: FnMut(String),
    InfoFn: FnMut(String),
{
    with_runtime_envelope_current_app_view_log_sinks::<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        App,
        _,
        _,
        _,
    >(owner, app, view, tx_source, |runtime, owner, call| {
        runtime.apply_with_log_sinks_and_derived_preflight_facts_and_hold_admission(
            owner, &call, trace, debug, info,
        )
    })
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_with_app_view_and_metrics_snapshot_and_log_sinks_and_derived_preflight_facts_and_hold_admission<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
    TraceFn,
    DebugFn,
    InfoFn,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    metrics_snapshot: QueueFeeMetricsSnapshot,
    trace: TraceFn,
    debug: DebugFn,
    info: InfoFn,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    TraceFn: FnMut(&str),
    DebugFn: FnMut(String),
    InfoFn: FnMut(String),
{
    with_runtime_envelope_metrics_snapshot_current_app_view_log_sinks::<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        App,
        _,
        _,
        _,
    >(
        owner,
        app,
        view,
        tx_source,
        metrics_snapshot,
        |runtime, owner, call| {
            runtime.apply_with_log_sinks_and_derived_preflight_facts_and_hold_admission(
                owner, &call, trace, debug, info,
            )
        },
    )
}

pub fn run_queue_apply_with_app_view_and_metrics_snapshot_and_derived_preflight_facts_and_hold_admission_and_log_messages<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    metrics_snapshot: QueueFeeMetricsSnapshot,
) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
{
    let queue_log_messages = RefCell::new(QueueApplyQueueLogMessages::default());
    let stage = run_queue_apply_with_app_view_and_metrics_snapshot_and_log_sinks_and_derived_preflight_facts_and_hold_admission(
        owner,
        app,
        view,
        tx_source,
        metrics_snapshot,
        |_message| {},
        |message| queue_log_messages.borrow_mut().debug.push(message),
        |message| queue_log_messages.borrow_mut().info.push(message),
    );
    QueueApplyTopWithLogMessagesResult {
        stage,
        queue_log_messages: queue_log_messages.into_inner(),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_with_app_view_and_log_sinks_and_derived_hold_preflight<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
    TraceFn,
    DebugFn,
    InfoFn,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    flags: ApplyFlags,
    consequences: TxConsequences,
    can_be_held_result: Ter,
    trace: TraceFn,
    debug: DebugFn,
    info: InfoFn,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    TraceFn: FnMut(&str),
    DebugFn: FnMut(String),
    InfoFn: FnMut(String),
{
    with_queue_apply_app_view_derived_hold_preflight(tx_source, flags, |hold_preflight| {
        run_queue_apply_with_app_view_and_log_sinks(
            owner,
            app,
            view,
            tx_source,
            hold_preflight,
            flags,
            consequences,
            can_be_held_result,
            trace,
            debug,
            info,
        )
    })
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_with_app_view_and_metrics_snapshot_and_log_sinks_and_derived_hold_preflight<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
    TraceFn,
    DebugFn,
    InfoFn,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    metrics_snapshot: QueueFeeMetricsSnapshot,
    flags: ApplyFlags,
    consequences: TxConsequences,
    can_be_held_result: Ter,
    trace: TraceFn,
    debug: DebugFn,
    info: InfoFn,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    TraceFn: FnMut(&str),
    DebugFn: FnMut(String),
    InfoFn: FnMut(String),
{
    with_queue_apply_app_view_metrics_snapshot_current_app_view_log_sinks_derived_hold_preflight::<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        _,
        _,
        _,
    >(
        owner,
        view,
        tx_source,
        metrics_snapshot,
        flags,
        |owner, metrics_snapshot, hold_preflight| {
            run_queue_apply_with_app_view_and_metrics_snapshot_and_log_sinks(
                owner,
                app,
                view,
                tx_source,
                metrics_snapshot,
                hold_preflight,
                flags,
                consequences,
                can_be_held_result,
                trace,
                debug,
                info,
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_after_preflight_with_app_view_and_log_sinks<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
    TraceFn,
    DebugFn,
    InfoFn,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    hold_preflight: QueueHoldPreflight,
    can_be_held_result: Ter,
    trace: TraceFn,
    debug: DebugFn,
    info: InfoFn,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    TraceFn: FnMut(&str),
    DebugFn: FnMut(String),
    InfoFn: FnMut(String),
{
    with_runtime_envelope_current_app_view_log_sinks::<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        App,
        _,
        _,
        _,
    >(owner, app, view, tx_source, |runtime, owner, call| {
        runtime.apply_after_preflight_with_log_sinks(
            owner,
            &call,
            preflight_result,
            hold_preflight,
            can_be_held_result,
            trace,
            debug,
            info,
        )
    })
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot_and_log_sinks<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
    TraceFn,
    DebugFn,
    InfoFn,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    metrics_snapshot: QueueFeeMetricsSnapshot,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    hold_preflight: QueueHoldPreflight,
    can_be_held_result: Ter,
    trace: TraceFn,
    debug: DebugFn,
    info: InfoFn,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    TraceFn: FnMut(&str),
    DebugFn: FnMut(String),
    InfoFn: FnMut(String),
{
    with_runtime_envelope_metrics_snapshot_current_app_view_log_sinks::<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        App,
        _,
        _,
        _,
    >(
        owner,
        app,
        view,
        tx_source,
        metrics_snapshot,
        |runtime, owner, call| {
            runtime.apply_after_preflight_with_log_sinks(
                owner,
                &call,
                preflight_result,
                hold_preflight,
                can_be_held_result,
                trace,
                debug,
                info,
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_after_preflight_with_app_view_and_caller_preclaim_and_log_sinks<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
    RunPreclaimStage,
    TraceFn,
    DebugFn,
    InfoFn,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    hold_preflight: QueueHoldPreflight,
    can_be_held_result: Ter,
    run_preclaim_stage: RunPreclaimStage,
    trace: TraceFn,
    debug: DebugFn,
    info: InfoFn,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>
        + QueueApplyCurrentPreclaimClearRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    RunPreclaimStage:
        FnOnce(
            QueueApplyPreparedPreclaimInputs<Account>,
        )
            -> Result<QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>, crate::ApplyResult>,
    TraceFn: FnMut(&str),
    DebugFn: FnMut(String),
    InfoFn: FnMut(String),
{
    with_runtime_envelope_current_app_view_caller_preclaim_log_sinks::<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        App,
        _,
        _,
        _,
    >(owner, app, view, tx_source, |runtime, owner, call| {
        runtime.apply_after_preflight_with_caller_preclaim_and_log_sinks(
            owner,
            &call,
            preflight_result,
            hold_preflight,
            can_be_held_result,
            run_preclaim_stage,
            trace,
            debug,
            info,
        )
    })
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot_and_caller_preclaim_and_log_sinks<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
    RunPreclaimStage,
    TraceFn,
    DebugFn,
    InfoFn,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    metrics_snapshot: QueueFeeMetricsSnapshot,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    hold_preflight: QueueHoldPreflight,
    can_be_held_result: Ter,
    run_preclaim_stage: RunPreclaimStage,
    trace: TraceFn,
    debug: DebugFn,
    info: InfoFn,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>
        + QueueApplyCurrentPreclaimClearRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    RunPreclaimStage:
        FnOnce(
            QueueApplyPreparedPreclaimInputs<Account>,
        )
            -> Result<QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>, crate::ApplyResult>,
    TraceFn: FnMut(&str),
    DebugFn: FnMut(String),
    InfoFn: FnMut(String),
{
    with_runtime_envelope_metrics_snapshot_current_app_view_caller_preclaim_log_sinks::<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        App,
        _,
        _,
        _,
    >(
        owner,
        app,
        view,
        tx_source,
        metrics_snapshot,
        |runtime, owner, call| {
            runtime.apply_after_preflight_with_caller_preclaim_and_log_sinks(
                owner,
                &call,
                preflight_result,
                hold_preflight,
                can_be_held_result,
                run_preclaim_stage,
                trace,
                debug,
                info,
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_after_preflight_with_app_view_and_log_sinks_and_derived_hold_admission<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
    TraceFn,
    DebugFn,
    InfoFn,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    hold_preflight: QueueHoldPreflight,
    trace: TraceFn,
    debug: DebugFn,
    info: InfoFn,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    TraceFn: FnMut(&str),
    DebugFn: FnMut(String),
    InfoFn: FnMut(String),
{
    with_current_queue_apply_app_view_hold_admission::<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        _,
        _,
        _,
    >(
        owner,
        tx_source,
        view,
        hold_preflight,
        |owner, metrics_snapshot, can_be_held_result| {
            run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot_and_log_sinks(
                owner,
                app,
                view,
                tx_source,
                metrics_snapshot,
                preflight_result,
                hold_preflight,
                can_be_held_result,
                trace,
                debug,
                info,
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot_and_log_sinks_and_derived_hold_admission<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
    TraceFn,
    DebugFn,
    InfoFn,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    metrics_snapshot: QueueFeeMetricsSnapshot,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    hold_preflight: QueueHoldPreflight,
    trace: TraceFn,
    debug: DebugFn,
    info: InfoFn,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    TraceFn: FnMut(&str),
    DebugFn: FnMut(String),
    InfoFn: FnMut(String),
{
    with_queue_apply_app_view_metrics_snapshot_current_app_view_log_sinks_hold_admission::<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        _,
        _,
        _,
    >(
        owner,
        view,
        tx_source,
        metrics_snapshot,
        hold_preflight,
        |owner, metrics_snapshot, can_be_held_result| {
            run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot_and_log_sinks(
                owner,
                app,
                view,
                tx_source,
                metrics_snapshot,
                preflight_result,
                hold_preflight,
                can_be_held_result,
                trace,
                debug,
                info,
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_after_preflight_with_app_view_and_log_sinks_and_derived_hold_preflight<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
    TraceFn,
    DebugFn,
    InfoFn,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    can_be_held_result: Ter,
    trace: TraceFn,
    debug: DebugFn,
    info: InfoFn,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    TraceFn: FnMut(&str),
    DebugFn: FnMut(String),
    InfoFn: FnMut(String),
{
    with_queue_apply_app_view_derived_hold_preflight(
        tx_source,
        preflight_result.flags,
        |hold_preflight| {
            run_queue_apply_after_preflight_with_app_view_and_log_sinks(
                owner,
                app,
                view,
                tx_source,
                preflight_result,
                hold_preflight,
                can_be_held_result,
                trace,
                debug,
                info,
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot_and_log_sinks_and_derived_hold_preflight<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    App,
    View,
    TxSource,
    TraceFn,
    DebugFn,
    InfoFn,
>(
    owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    app: &mut App,
    view: &View,
    tx_source: &TxSource,
    metrics_snapshot: QueueFeeMetricsSnapshot,
    preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    can_be_held_result: Ter,
    trace: TraceFn,
    debug: DebugFn,
    info: InfoFn,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    TxId: Clone + Display,
    App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
    View: QueueApplyLedgerViewSource<Account>,
    TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    TraceFn: FnMut(&str),
    DebugFn: FnMut(String),
    InfoFn: FnMut(String),
{
    with_queue_apply_app_view_metrics_snapshot_current_app_view_log_sinks_derived_hold_preflight::<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        _,
        _,
        _,
    >(
        owner,
        view,
        tx_source,
        metrics_snapshot,
        preflight_result.flags,
        |owner, metrics_snapshot, hold_preflight| {
            run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot_and_log_sinks(
                owner,
                app,
                view,
                tx_source,
                metrics_snapshot,
                preflight_result,
                hold_preflight,
                can_be_held_result,
                trace,
                debug,
                info,
            )
        },
    )
}
