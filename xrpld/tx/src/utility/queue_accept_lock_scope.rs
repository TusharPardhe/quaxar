//! Lock-scope wrapper for the current `xrpld` `TxQ::accept(...)` boundary.
//!
//! This layer is the first explicit step toward the the reference implementation
//! `std::lock_guard lock(mutex_)` call shape without inventing a mutex port:
//! - one owner object carries the landed accept state plus journal sink,
//! - callers still supply the live app/runtime and ledger-view inputs,
//! - callers must also supply a lock-scope token.

use std::fmt::Display;

use crate::{
    QueueAcceptEntryResult, QueueAcceptJournalOwner, QueueAcceptLedgerViewSource,
    QueueAcceptLiveApplyRuntime, QueueAcceptPreparedCallStep,
};

pub trait QueueAcceptLockScope {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueAcceptLockScopeOwner<Account, Tx, Journal, ParentBatchId, Sink> {
    owner: QueueAcceptJournalOwner<Account, Tx, Journal, ParentBatchId, Sink>,
}

impl<Account, Tx, Journal, ParentBatchId, Sink>
    QueueAcceptLockScopeOwner<Account, Tx, Journal, ParentBatchId, Sink>
{
    pub const fn new(
        owner: QueueAcceptJournalOwner<Account, Tx, Journal, ParentBatchId, Sink>,
    ) -> Self {
        Self { owner }
    }

    pub fn owner(&self) -> &QueueAcceptJournalOwner<Account, Tx, Journal, ParentBatchId, Sink> {
        &self.owner
    }

    pub fn owner_mut(
        &mut self,
    ) -> &mut QueueAcceptJournalOwner<Account, Tx, Journal, ParentBatchId, Sink> {
        &mut self.owner
    }
}

impl<Account, Tx, Journal, ParentBatchId, Sink>
    QueueAcceptLockScopeOwner<Account, Tx, Journal, ParentBatchId, Sink>
where
    Account: Clone + Display + Ord + PartialEq,
    Sink: crate::QueueAcceptJournalSink,
{
    pub fn accept<Lock, App, View>(
        &mut self,
        _lock: &mut Lock,
        app: &mut App,
        view: &View,
    ) -> QueueAcceptEntryResult<Account>
    where
        Lock: QueueAcceptLockScope,
        App: QueueAcceptLiveApplyRuntime<Account, Tx, Journal, ParentBatchId>,
        View: QueueAcceptLedgerViewSource,
    {
        self.owner.accept(app, view)
    }

    pub fn prepare_accept<Lock, View>(
        &mut self,
        _lock: &mut Lock,
        view: &View,
    ) -> QueueAcceptPreparedCallStep<Account>
    where
        Lock: QueueAcceptLockScope,
        View: QueueAcceptLedgerViewSource,
    {
        self.owner.prepare_accept(view)
    }
}
