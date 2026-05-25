//! Journal-aware call envelope for the current `xrpld` `TxQ::accept(...)`
//! boundary.
//!
//! This layer adds one more honest wrapper above the landed call envelope:
//! - one apply runtime,
//! - one ledger-view source,
//! - one journal sink surface.
//!
//! It makes the logging surface explicit above the landed
//! `QueueAcceptCallEnvelope`.

use std::{cell::RefCell, fmt::Display};

use crate::{
    QueueAcceptCallEnvelope, QueueAcceptEntryResult, QueueAcceptLogMessages, QueueAcceptOwnerShell,
    QueueAcceptPreparedCallStep,
};

pub trait QueueAcceptJournalSink {
    fn trace(&mut self, message: &str);
    fn debug(&mut self, message: &str);
    fn info(&mut self, message: &str);
    fn warn(&mut self, message: &str);
}

pub fn emit_queue_accept_log_messages<Sink>(sink: &mut Sink, messages: &QueueAcceptLogMessages)
where
    Sink: QueueAcceptJournalSink,
{
    for message in &messages.loop_messages.trace {
        sink.trace(message);
    }
    for message in &messages.loop_messages.debug {
        sink.debug(message);
    }
    for message in &messages.loop_messages.info {
        sink.info(message);
    }
    if let Some(message) = &messages.warning {
        sink.warn(message);
    }
}

pub struct QueueAcceptJournalEnvelope<'a, App, View, Sink> {
    call: QueueAcceptCallEnvelope<'a, App, View>,
    journal: &'a mut Sink,
}

impl<'a, App, View, Sink> QueueAcceptJournalEnvelope<'a, App, View, Sink> {
    pub fn new(app: &'a mut App, view: &'a View, journal: &'a mut Sink) -> Self {
        Self {
            call: QueueAcceptCallEnvelope::new(app, view),
            journal,
        }
    }

    pub fn app(&mut self) -> &mut App {
        self.call.app()
    }

    pub const fn view(&self) -> &View {
        self.call.view()
    }

    pub fn journal(&mut self) -> &mut Sink {
        self.journal
    }
}

impl<'a, App, View, Sink> QueueAcceptJournalEnvelope<'a, App, View, Sink>
where
    Sink: QueueAcceptJournalSink,
{
    pub fn accept<Account, Tx, Journal, ParentBatchId>(
        &mut self,
        owner: &mut QueueAcceptOwnerShell<Account, Tx, Journal, ParentBatchId>,
    ) -> QueueAcceptEntryResult<Account>
    where
        Account: Clone + Display + Ord + PartialEq,
        App: crate::QueueAcceptLiveApplyRuntime<Account, Tx, Journal, ParentBatchId>,
        View: crate::QueueAcceptLedgerViewSource,
    {
        let journal = RefCell::new(&mut *self.journal);
        self.call.accept_with_log_sinks(
            owner,
            |message| journal.borrow_mut().trace(message),
            |message| journal.borrow_mut().debug(message),
            |message| journal.borrow_mut().info(message),
            |message| journal.borrow_mut().warn(message),
        )
    }

    pub fn prepare_accept<Account, Tx, Journal, ParentBatchId>(
        &mut self,
        owner: &mut QueueAcceptOwnerShell<Account, Tx, Journal, ParentBatchId>,
    ) -> QueueAcceptPreparedCallStep<Account>
    where
        Account: Clone + Display + Ord + PartialEq,
        View: crate::QueueAcceptLedgerViewSource,
    {
        self.call.prepare_accept(owner)
    }
}
