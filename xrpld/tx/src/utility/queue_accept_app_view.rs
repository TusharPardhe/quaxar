//! Higher app/view wrapper for the current `xrpld` `TxQ::accept(...)` seam.
//!
//! This layer is intentionally closer to the the reference implementation public signature
//! shape:
//! - one app-side runtime provides metrics, queue sizing, and
//!   queued-apply execution,
//! - one view-side source provides the live ledger facts.
//!
//! It preserves the public app/view split above the landed deterministic
//! accept carriers.

use std::fmt::Display;

use basics::base_uint::Uint256;

use crate::{
    ApplyResult, MaybeTx, QueueAcceptEntryResult, QueueAcceptObservedQueueSource,
    QueueAcceptObservedViewSource, QueueAcceptOwnerState, QueueAcceptPreparedCallStep,
    QueueAcceptRuntimeSource, QueueFeeMetricsState, QueueViews, prepare_queue_accept_with_runtime,
    run_queue_accept_with_runtime, run_queue_accept_with_runtime_and_log_sinks,
};

pub trait QueueAcceptAppSource {
    fn metrics(&self) -> &QueueFeeMetricsState;
    fn current_max_size(&self) -> Option<usize>;
}

pub trait QueueAcceptLedgerViewSource {
    fn open_ledger_tx_count(&self) -> usize;
    fn parent_hash(&self) -> Uint256;
}

pub trait QueueAcceptAppRuntime<Account, Tx, Journal, ParentBatchId>: QueueAcceptAppSource {
    fn apply_queued(
        &mut self,
        queued: &mut MaybeTx<Tx, Account, Journal, ParentBatchId>,
    ) -> ApplyResult;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueAcceptAppViewSnapshot {
    pub metrics: QueueFeeMetricsState,
    pub open_ledger_tx_count: usize,
    pub parent_hash: Uint256,
    pub current_max_size: Option<usize>,
}

impl QueueAcceptAppViewSnapshot {
    pub const fn new(
        metrics: QueueFeeMetricsState,
        open_ledger_tx_count: usize,
        parent_hash: Uint256,
        current_max_size: Option<usize>,
    ) -> Self {
        Self {
            metrics,
            open_ledger_tx_count,
            parent_hash,
            current_max_size,
        }
    }
}

impl QueueAcceptObservedViewSource for QueueAcceptAppViewSnapshot {
    fn open_ledger_tx_count(&self) -> usize {
        self.open_ledger_tx_count
    }

    fn parent_hash(&self) -> Uint256 {
        self.parent_hash
    }
}

impl QueueAcceptObservedQueueSource for QueueAcceptAppViewSnapshot {
    fn current_max_size(&self) -> Option<usize> {
        self.current_max_size
    }
}

impl QueueAcceptRuntimeSource for QueueAcceptAppViewSnapshot {
    fn metrics(&self) -> &QueueFeeMetricsState {
        &self.metrics
    }
}

pub fn snapshot_queue_accept_app_view<App, View>(
    app: &App,
    view: &View,
) -> QueueAcceptAppViewSnapshot
where
    App: QueueAcceptAppSource,
    View: QueueAcceptLedgerViewSource,
{
    QueueAcceptAppViewSnapshot::new(
        app.metrics().clone(),
        view.open_ledger_tx_count(),
        view.parent_hash(),
        app.current_max_size(),
    )
}

pub fn run_queue_accept_with_app_view<Account, Tx, Journal, ParentBatchId, App, View>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    owner_state: &mut QueueAcceptOwnerState,
    app: &mut App,
    view: &View,
) -> QueueAcceptEntryResult<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    App: QueueAcceptAppRuntime<Account, Tx, Journal, ParentBatchId>,
    View: QueueAcceptLedgerViewSource,
{
    let snapshot = snapshot_queue_accept_app_view(&*app, view);
    run_queue_accept_with_runtime(
        views,
        owner_state,
        &mut QueueAcceptAppViewRuntime { app, snapshot },
    )
}

pub fn prepare_queue_accept_with_app_view<Account, Tx, Journal, ParentBatchId, App, View>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    owner_state: &mut QueueAcceptOwnerState,
    app: &App,
    view: &View,
) -> QueueAcceptPreparedCallStep<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    App: QueueAcceptAppSource,
    View: QueueAcceptLedgerViewSource,
{
    let snapshot = snapshot_queue_accept_app_view(app, view);
    prepare_queue_accept_with_runtime(
        views,
        owner_state,
        &QueueAcceptAppViewRuntimeSnapshot(snapshot),
    )
}

pub fn run_queue_accept_with_app_view_and_log_sinks<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    App,
    View,
    TraceFn,
    DebugFn,
    InfoFn,
    WarnFn,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    owner_state: &mut QueueAcceptOwnerState,
    app: &mut App,
    view: &View,
    trace: TraceFn,
    debug: DebugFn,
    info: InfoFn,
    warn: WarnFn,
) -> QueueAcceptEntryResult<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    App: QueueAcceptAppRuntime<Account, Tx, Journal, ParentBatchId>,
    View: QueueAcceptLedgerViewSource,
    TraceFn: FnMut(&str),
    DebugFn: FnMut(&str),
    InfoFn: FnMut(&str),
    WarnFn: FnMut(&str),
{
    let snapshot = snapshot_queue_accept_app_view(&*app, view);
    run_queue_accept_with_runtime_and_log_sinks(
        views,
        owner_state,
        &mut QueueAcceptAppViewRuntime { app, snapshot },
        trace,
        debug,
        info,
        warn,
    )
}

struct QueueAcceptAppViewRuntime<'a, App> {
    app: &'a mut App,
    snapshot: QueueAcceptAppViewSnapshot,
}

impl<App> QueueAcceptObservedViewSource for QueueAcceptAppViewRuntime<'_, App> {
    fn open_ledger_tx_count(&self) -> usize {
        self.snapshot.open_ledger_tx_count
    }

    fn parent_hash(&self) -> Uint256 {
        self.snapshot.parent_hash
    }
}

impl<App> QueueAcceptObservedQueueSource for QueueAcceptAppViewRuntime<'_, App> {
    fn current_max_size(&self) -> Option<usize> {
        self.snapshot.current_max_size
    }
}

impl<App> QueueAcceptRuntimeSource for QueueAcceptAppViewRuntime<'_, App> {
    fn metrics(&self) -> &QueueFeeMetricsState {
        &self.snapshot.metrics
    }
}

impl<Account, Tx, Journal, ParentBatchId, App>
    crate::QueueAcceptApplyRuntime<Account, Tx, Journal, ParentBatchId>
    for QueueAcceptAppViewRuntime<'_, App>
where
    App: QueueAcceptAppRuntime<Account, Tx, Journal, ParentBatchId>,
{
    fn apply_queued(
        &mut self,
        queued: &mut MaybeTx<Tx, Account, Journal, ParentBatchId>,
    ) -> ApplyResult {
        self.app.apply_queued(queued)
    }
}

struct QueueAcceptAppViewRuntimeSnapshot(QueueAcceptAppViewSnapshot);

impl QueueAcceptObservedViewSource for QueueAcceptAppViewRuntimeSnapshot {
    fn open_ledger_tx_count(&self) -> usize {
        self.0.open_ledger_tx_count
    }

    fn parent_hash(&self) -> Uint256 {
        self.0.parent_hash
    }
}

impl QueueAcceptObservedQueueSource for QueueAcceptAppViewRuntimeSnapshot {
    fn current_max_size(&self) -> Option<usize> {
        self.0.current_max_size
    }
}

impl QueueAcceptRuntimeSource for QueueAcceptAppViewRuntimeSnapshot {
    fn metrics(&self) -> &QueueFeeMetricsState {
        &self.0.metrics
    }
}
