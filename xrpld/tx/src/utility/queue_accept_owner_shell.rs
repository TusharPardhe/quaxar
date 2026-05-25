//! Method-style owner shell for the current `xrpld` `TxQ::accept(...)`
//! boundary.
//!
//! This layer wraps the landed live-owner state in a single method-oriented
//! facade so higher Rust callers can move closer to the the reference implementation call
//! shape:
//! - one owner object carries the TxQ-owned accept state,
//! - callers still supply the live ledger-view facts,
//! - callers still own queued-apply execution,
//! - callers still own real `Application`, real `OpenView`, real journal,
//!   and mutex scope.

use std::fmt::Display;

use crate::{
    QueueAcceptEntryResult, QueueAcceptLedgerViewSource, QueueAcceptLiveApplyRuntime,
    QueueAcceptLiveOwner, QueueAcceptPreparedCallStep, run_queue_accept_with_live_owner,
    run_queue_accept_with_live_owner_and_log_sinks,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueAcceptOwnerShell<Account, Tx, Journal, ParentBatchId> {
    owner: QueueAcceptLiveOwner<Account, Tx, Journal, ParentBatchId>,
}

impl<Account, Tx, Journal, ParentBatchId>
    QueueAcceptOwnerShell<Account, Tx, Journal, ParentBatchId>
{
    pub const fn new(owner: QueueAcceptLiveOwner<Account, Tx, Journal, ParentBatchId>) -> Self {
        Self { owner }
    }

    pub fn owner(&self) -> &QueueAcceptLiveOwner<Account, Tx, Journal, ParentBatchId> {
        &self.owner
    }

    pub fn owner_mut(&mut self) -> &mut QueueAcceptLiveOwner<Account, Tx, Journal, ParentBatchId> {
        &mut self.owner
    }
}

impl<Account, Tx, Journal, ParentBatchId> QueueAcceptOwnerShell<Account, Tx, Journal, ParentBatchId>
where
    Account: Clone + Display + Ord + PartialEq,
{
    pub fn accept<App, View>(
        &mut self,
        app: &mut App,
        view: &View,
    ) -> QueueAcceptEntryResult<Account>
    where
        App: QueueAcceptLiveApplyRuntime<Account, Tx, Journal, ParentBatchId>,
        View: QueueAcceptLedgerViewSource,
    {
        run_queue_accept_with_live_owner(&mut self.owner, app, view)
    }

    pub fn prepare_accept<View>(&mut self, view: &View) -> QueueAcceptPreparedCallStep<Account>
    where
        View: QueueAcceptLedgerViewSource,
    {
        crate::prepare_queue_accept_with_live_owner(&mut self.owner, view)
    }

    pub fn accept_with_log_sinks<App, View, TraceFn, DebugFn, InfoFn, WarnFn>(
        &mut self,
        app: &mut App,
        view: &View,
        trace: TraceFn,
        debug: DebugFn,
        info: InfoFn,
        warn: WarnFn,
    ) -> QueueAcceptEntryResult<Account>
    where
        App: QueueAcceptLiveApplyRuntime<Account, Tx, Journal, ParentBatchId>,
        View: QueueAcceptLedgerViewSource,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(&str),
        InfoFn: FnMut(&str),
        WarnFn: FnMut(&str),
    {
        run_queue_accept_with_live_owner_and_log_sinks(
            &mut self.owner,
            app,
            view,
            trace,
            debug,
            info,
            warn,
        )
    }
}
