//! Journal-owning wrapper for the current `xrpld` `TxQ::accept(...)`
//! boundary.
//!
//! This layer moves one more real owner concern into the Rust-side accept
//! object:
//! - one owner object carries the landed TxQ-owned accept state,
//! - the same owner object carries the caller-chosen journal sink surface,
//! - callers still supply the live app/runtime and ledger-view inputs,
//! - callers still own real mutex scope.

use std::fmt::Display;

use crate::{
    QueueAcceptEntryResult, QueueAcceptJournalEnvelope, QueueAcceptJournalSink,
    QueueAcceptLedgerViewSource, QueueAcceptLiveApplyRuntime, QueueAcceptOwnerShell,
    QueueAcceptPreparedCallStep,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueAcceptJournalOwner<Account, Tx, Journal, ParentBatchId, Sink> {
    owner: QueueAcceptOwnerShell<Account, Tx, Journal, ParentBatchId>,
    journal: Sink,
}

impl<Account, Tx, Journal, ParentBatchId, Sink>
    QueueAcceptJournalOwner<Account, Tx, Journal, ParentBatchId, Sink>
{
    pub const fn new(
        owner: QueueAcceptOwnerShell<Account, Tx, Journal, ParentBatchId>,
        journal: Sink,
    ) -> Self {
        Self { owner, journal }
    }

    pub fn owner(&self) -> &QueueAcceptOwnerShell<Account, Tx, Journal, ParentBatchId> {
        &self.owner
    }

    pub fn owner_mut(&mut self) -> &mut QueueAcceptOwnerShell<Account, Tx, Journal, ParentBatchId> {
        &mut self.owner
    }

    pub fn journal(&self) -> &Sink {
        &self.journal
    }

    pub fn journal_mut(&mut self) -> &mut Sink {
        &mut self.journal
    }
}

impl<Account, Tx, Journal, ParentBatchId, Sink>
    QueueAcceptJournalOwner<Account, Tx, Journal, ParentBatchId, Sink>
where
    Account: Clone + Display + Ord + PartialEq,
    Sink: QueueAcceptJournalSink,
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
        let (owner, journal) = (&mut self.owner, &mut self.journal);
        let mut envelope = QueueAcceptJournalEnvelope::new(app, view, journal);
        envelope.accept(owner)
    }

    pub fn prepare_accept<View>(&mut self, view: &View) -> QueueAcceptPreparedCallStep<Account>
    where
        View: QueueAcceptLedgerViewSource,
    {
        self.owner.prepare_accept(view)
    }
}
