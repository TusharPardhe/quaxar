//! Higher runtime wrapper for the current `xrpld` `TxQ::accept(...)` seam.
//!
//! This layer reuses the widened top-level runtime-source seam directly by
//! letting one runtime object:
//! 1. provide the live accept facts,
//! 2. own the real queued-apply execution,
//! 3. call the landed deterministic top carrier without the extra public-entry
//!    wrapper hop.

use std::fmt::Display;

use basics::base_uint::Uint256;

use crate::{
    ApplyResult, MaybeTx, QueueAcceptEntryResult, QueueAcceptObservedQueueSource,
    QueueAcceptObservedViewSource, QueueAcceptOwnerState, QueueAcceptPreparedCallStep,
    QueueAcceptRuntimeSource, QueueFeeMetricsState, QueueViews,
    prepare_queue_accept_top_with_runtime_source, run_queue_accept_top_with_runtime_source,
    run_queue_accept_top_with_runtime_source_with_log_sinks,
};

pub trait QueueAcceptApplyRuntime<Account, Tx, Journal, ParentBatchId>:
    QueueAcceptRuntimeSource
{
    fn apply_queued(
        &mut self,
        queued: &mut MaybeTx<Tx, Account, Journal, ParentBatchId>,
    ) -> ApplyResult;
}

#[derive(Debug)]
pub struct QueueAcceptRuntimeEnvelope<'a, Runtime> {
    runtime: &'a mut Runtime,
}

impl<'a, Runtime> QueueAcceptRuntimeEnvelope<'a, Runtime> {
    pub fn new(runtime: &'a mut Runtime) -> Self {
        Self { runtime }
    }

    pub fn runtime(&mut self) -> &mut Runtime {
        self.runtime
    }

    fn snapshot(&self) -> QueueAcceptRuntimeSnapshot
    where
        Runtime: QueueAcceptRuntimeSource,
    {
        snapshot_queue_accept_runtime(&*self.runtime)
    }

    pub fn prepare_accept<Account, Tx, Journal, ParentBatchId>(
        &self,
        views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        owner_state: &mut QueueAcceptOwnerState,
    ) -> QueueAcceptPreparedCallStep<Account>
    where
        Account: Clone + Display + Ord + PartialEq,
        Runtime: QueueAcceptRuntimeSource,
    {
        let snapshot = self.snapshot();
        prepare_queue_accept_top_with_runtime_source(views, owner_state, &snapshot)
    }

    pub fn accept<Account, Tx, Journal, ParentBatchId>(
        &mut self,
        views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        owner_state: &mut QueueAcceptOwnerState,
    ) -> QueueAcceptEntryResult<Account>
    where
        Account: Clone + Display + Ord + PartialEq,
        Runtime: QueueAcceptApplyRuntime<Account, Tx, Journal, ParentBatchId>,
    {
        let snapshot = self.snapshot();
        let accept =
            run_queue_accept_top_with_runtime_source(views, owner_state, &snapshot, |queued| {
                self.runtime.apply_queued(queued)
            });

        QueueAcceptEntryResult {
            ledger_changed: accept.ledger_changed(),
            accept,
        }
    }

    pub fn accept_with_log_sinks<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TraceFn,
        DebugFn,
        InfoFn,
        WarnFn,
    >(
        &mut self,
        views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        owner_state: &mut QueueAcceptOwnerState,
        trace: TraceFn,
        debug: DebugFn,
        info: InfoFn,
        warn: WarnFn,
    ) -> QueueAcceptEntryResult<Account>
    where
        Account: Clone + Display + Ord + PartialEq,
        Runtime: QueueAcceptApplyRuntime<Account, Tx, Journal, ParentBatchId>,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(&str),
        InfoFn: FnMut(&str),
        WarnFn: FnMut(&str),
    {
        let snapshot = self.snapshot();
        let accept = run_queue_accept_top_with_runtime_source_with_log_sinks(
            views,
            owner_state,
            &snapshot,
            trace,
            debug,
            info,
            warn,
            |queued| self.runtime.apply_queued(queued),
        );

        QueueAcceptEntryResult {
            ledger_changed: accept.ledger_changed(),
            accept,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueAcceptRuntimeSnapshot {
    pub metrics: QueueFeeMetricsState,
    pub open_ledger_tx_count: usize,
    pub parent_hash: Uint256,
    pub current_max_size: Option<usize>,
}

impl QueueAcceptRuntimeSnapshot {
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

impl QueueAcceptObservedViewSource for QueueAcceptRuntimeSnapshot {
    fn open_ledger_tx_count(&self) -> usize {
        self.open_ledger_tx_count
    }

    fn parent_hash(&self) -> Uint256 {
        self.parent_hash
    }
}

impl QueueAcceptObservedQueueSource for QueueAcceptRuntimeSnapshot {
    fn current_max_size(&self) -> Option<usize> {
        self.current_max_size
    }
}

impl QueueAcceptRuntimeSource for QueueAcceptRuntimeSnapshot {
    fn metrics(&self) -> &QueueFeeMetricsState {
        &self.metrics
    }
}

pub fn snapshot_queue_accept_runtime<Source>(runtime: &Source) -> QueueAcceptRuntimeSnapshot
where
    Source: QueueAcceptRuntimeSource,
{
    QueueAcceptRuntimeSnapshot::new(
        runtime.metrics().clone(),
        runtime.open_ledger_tx_count(),
        runtime.parent_hash(),
        runtime.current_max_size(),
    )
}

pub fn run_queue_accept_with_runtime<Account, Tx, Journal, ParentBatchId, Runtime>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    owner_state: &mut QueueAcceptOwnerState,
    runtime: &mut Runtime,
) -> QueueAcceptEntryResult<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    Runtime: QueueAcceptApplyRuntime<Account, Tx, Journal, ParentBatchId>,
{
    QueueAcceptRuntimeEnvelope::new(runtime).accept(views, owner_state)
}

pub fn prepare_queue_accept_with_runtime<Account, Tx, Journal, ParentBatchId, Runtime>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    owner_state: &mut QueueAcceptOwnerState,
    runtime: &Runtime,
) -> QueueAcceptPreparedCallStep<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    Runtime: QueueAcceptRuntimeSource,
{
    prepare_queue_accept_top_with_runtime_source(views, owner_state, runtime)
}

pub fn run_queue_accept_with_runtime_and_log_sinks<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    Runtime,
    TraceFn,
    DebugFn,
    InfoFn,
    WarnFn,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    owner_state: &mut QueueAcceptOwnerState,
    runtime: &mut Runtime,
    trace: TraceFn,
    debug: DebugFn,
    info: InfoFn,
    warn: WarnFn,
) -> QueueAcceptEntryResult<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    Runtime: QueueAcceptApplyRuntime<Account, Tx, Journal, ParentBatchId>,
    TraceFn: FnMut(&str),
    DebugFn: FnMut(&str),
    InfoFn: FnMut(&str),
    WarnFn: FnMut(&str),
{
    QueueAcceptRuntimeEnvelope::new(runtime).accept_with_log_sinks(
        views,
        owner_state,
        trace,
        debug,
        info,
        warn,
    )
}
