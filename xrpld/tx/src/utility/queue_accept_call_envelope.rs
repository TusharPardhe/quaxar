//! Explicit call envelope for the current `xrpld` `TxQ::accept(...)`
//! boundary.
//!
//! This layer bundles the two live inputs above the landed owner shell:
//! - one apply runtime,
//! - one ledger-view source.
//!
//! It makes the live call boundary explicit above the landed
//! `QueueAcceptOwnerShell`.

use std::fmt::Display;

use crate::{
    QueueAcceptEntryResult, QueueAcceptLedgerViewSource, QueueAcceptLiveApplyRuntime,
    QueueAcceptOwnerShell, QueueAcceptPreparedCallStep,
};

#[derive(Debug)]
pub struct QueueAcceptCallEnvelope<'a, App, View> {
    app: &'a mut App,
    view: &'a View,
}

impl<'a, App, View> QueueAcceptCallEnvelope<'a, App, View> {
    pub fn new(app: &'a mut App, view: &'a View) -> Self {
        Self { app, view }
    }

    pub fn app(&mut self) -> &mut App {
        self.app
    }

    pub const fn view(&self) -> &View {
        self.view
    }
}

impl<'a, App, View> QueueAcceptCallEnvelope<'a, App, View> {
    pub fn accept<Account, Tx, Journal, ParentBatchId>(
        &mut self,
        owner: &mut QueueAcceptOwnerShell<Account, Tx, Journal, ParentBatchId>,
    ) -> QueueAcceptEntryResult<Account>
    where
        Account: Clone + Display + Ord + PartialEq,
        App: QueueAcceptLiveApplyRuntime<Account, Tx, Journal, ParentBatchId>,
        View: QueueAcceptLedgerViewSource,
    {
        owner.accept(self.app, self.view)
    }

    pub fn prepare_accept<Account, Tx, Journal, ParentBatchId>(
        &mut self,
        owner: &mut QueueAcceptOwnerShell<Account, Tx, Journal, ParentBatchId>,
    ) -> QueueAcceptPreparedCallStep<Account>
    where
        Account: Clone + Display + Ord + PartialEq,
        View: QueueAcceptLedgerViewSource,
    {
        owner.prepare_accept(self.view)
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
        owner: &mut QueueAcceptOwnerShell<Account, Tx, Journal, ParentBatchId>,
        trace: TraceFn,
        debug: DebugFn,
        info: InfoFn,
        warn: WarnFn,
    ) -> QueueAcceptEntryResult<Account>
    where
        Account: Clone + Display + Ord + PartialEq,
        App: QueueAcceptLiveApplyRuntime<Account, Tx, Journal, ParentBatchId>,
        View: QueueAcceptLedgerViewSource,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(&str),
        InfoFn: FnMut(&str),
        WarnFn: FnMut(&str),
    {
        owner.accept_with_log_sinks(self.app, self.view, trace, debug, info, warn)
    }
}
