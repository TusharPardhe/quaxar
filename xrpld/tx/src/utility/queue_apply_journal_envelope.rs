//! Journal-aware app/view wrapper for the current `xrpld` `TxQ::apply(...)`
//! boundary.
//!
//! This layer adds one honest logging surface above the landed app/view split:
//! - one apply runtime,
//! - one ledger-view source,
//! - one journal sink for `trace`, `debug`, and `info`.
//!
//! It preserves live sink ordering for the currently landed apply parity
//! slices.

use std::{cell::RefCell, fmt::Display};

use protocol::Ter;

use crate::{
    ApplyFlags, PreflightResult, QueueApplyAppRuntime, QueueApplyCurrentPreclaimClearRuntime,
    QueueApplyHoldPreflightTxSource, QueueApplyLedgerViewSource, QueueApplyObservedTxSource,
    QueueApplyOwnerShell, QueueApplyPreclaimStage, QueueApplyPreflightStage,
    QueueApplyPreparedPreclaimInputs, QueueApplyQueueLogMessages,
    QueueApplyTopWithLogMessagesResult, QueueHoldPreflight, TxConsequences,
    run_queue_apply_after_preflight_with_app_view_and_log_messages,
    run_queue_apply_with_app_view_and_log_messages,
};

pub trait QueueApplyJournalSink {
    fn trace(&mut self, message: &str);
    fn debug(&mut self, message: &str);
    fn info(&mut self, message: &str);
}

pub fn emit_queue_apply_log_messages<Sink>(sink: &mut Sink, messages: &QueueApplyQueueLogMessages)
where
    Sink: QueueApplyJournalSink,
{
    for message in &messages.trace {
        sink.trace(message);
    }
    for message in &messages.info {
        sink.info(message);
    }
    for message in &messages.debug {
        sink.debug(message);
    }
}

pub struct QueueApplyJournalEnvelope<'a, App, View, Sink> {
    app: &'a mut App,
    view: &'a View,
    journal: &'a mut Sink,
}

impl<'a, App, View, Sink> QueueApplyJournalEnvelope<'a, App, View, Sink> {
    pub fn new(app: &'a mut App, view: &'a View, journal: &'a mut Sink) -> Self {
        Self { app, view, journal }
    }

    pub fn app(&mut self) -> &mut App {
        self.app
    }

    pub const fn view(&self) -> &View {
        self.view
    }

    pub fn journal(&mut self) -> &mut Sink {
        self.journal
    }
}

impl<'a, App, View, Sink> QueueApplyJournalEnvelope<'a, App, View, Sink>
where
    Sink: QueueApplyJournalSink,
{
    fn with_journal_sinks<Account, Tx, Journal, ParentBatchId, R>(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        run: impl FnOnce(
            &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
            &mut App,
            &'a View,
            &RefCell<&mut Sink>,
        ) -> R,
    ) -> R
    where
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
    {
        let journal = RefCell::new(&mut *self.journal);
        run(owner, self.app, self.view, &journal)
    }

    fn with_journal_sinks_current_app_view_caller_preclaim<Account, Tx, Journal, ParentBatchId, R>(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        run: impl FnOnce(
            &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
            &mut App,
            &'a View,
            &RefCell<&mut Sink>,
        ) -> R,
    ) -> R
    where
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
    {
        self.with_journal_sinks(owner, run)
    }

    fn with_journal_sinks_current_app_view_caller_preclaim_log_sinks<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        R,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        run: impl FnOnce(
            &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
            &mut App,
            &'a View,
            &RefCell<&mut Sink>,
        ) -> R,
    ) -> R
    where
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
    {
        self.with_journal_sinks_current_app_view_caller_preclaim(owner, run)
    }

    fn with_journal_sinks_current_app_view_log_sinks<Account, Tx, Journal, ParentBatchId, R>(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        run: impl FnOnce(
            &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
            &mut App,
            &'a View,
            &RefCell<&mut Sink>,
        ) -> R,
    ) -> R
    where
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
    {
        self.with_journal_sinks(owner, run)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply<Account, Tx, Journal, ParentBatchId, TxId, TxSource>(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
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
        self.with_journal_sinks_current_app_view_log_sinks(owner, |owner, app, view, journal| {
            owner.apply_with_app_view_and_log_sinks(
                app,
                view,
                tx_source,
                hold_preflight,
                flags,
                consequences,
                can_be_held_result,
                |message| journal.borrow_mut().trace(message),
                |message| journal.borrow_mut().debug(&message),
                |message| journal.borrow_mut().info(&message),
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_log_messages<Account, Tx, Journal, ParentBatchId, TxId, TxSource>(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
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
        self.with_journal_sinks_current_app_view_log_sinks(owner, |owner, app, view, journal| {
            let result = run_queue_apply_with_app_view_and_log_messages(
                owner,
                app,
                view,
                tx_source,
                hold_preflight,
                flags,
                consequences,
                can_be_held_result,
            );
            let mut sink = journal.borrow_mut();
            emit_queue_apply_log_messages(&mut **sink, &result.queue_log_messages);
            result
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_owned_metrics<Account, Tx, Journal, ParentBatchId, TxId, TxSource>(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
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
        self.with_journal_sinks_current_app_view_log_sinks(owner, |owner, app, view, journal| {
            owner.apply_with_app_view_and_owned_metrics_and_log_sinks(
                app,
                view,
                tx_source,
                hold_preflight,
                flags,
                consequences,
                can_be_held_result,
                |message| journal.borrow_mut().trace(message),
                |message| journal.borrow_mut().debug(&message),
                |message| journal.borrow_mut().info(&message),
            )
        })
    }

    pub fn apply_after_preflight<Account, Tx, Journal, ParentBatchId, TxId, TxSource>(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
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
        self.with_journal_sinks_current_app_view_log_sinks(owner, |owner, app, view, journal| {
            owner.apply_after_preflight_with_app_view_and_log_sinks(
                app,
                view,
                tx_source,
                preflight_result,
                hold_preflight,
                can_be_held_result,
                |message| journal.borrow_mut().trace(message),
                |message| journal.borrow_mut().debug(&message),
                |message| journal.borrow_mut().info(&message),
            )
        })
    }

    pub fn apply_after_preflight_with_log_messages<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
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
        self.with_journal_sinks_current_app_view_log_sinks(owner, |owner, app, view, journal| {
            let result = run_queue_apply_after_preflight_with_app_view_and_log_messages(
                owner,
                app,
                view,
                tx_source,
                preflight_result,
                hold_preflight,
                can_be_held_result,
            );
            let mut sink = journal.borrow_mut();
            emit_queue_apply_log_messages(&mut **sink, &result.queue_log_messages);
            result
        })
    }

    pub fn apply_after_preflight_with_owned_metrics<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
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
        self.with_journal_sinks_current_app_view_log_sinks(owner, |owner, app, view, journal| {
            owner.apply_after_preflight_with_app_view_and_owned_metrics_and_log_sinks(
                app,
                view,
                tx_source,
                preflight_result,
                hold_preflight,
                can_be_held_result,
                |message| journal.borrow_mut().trace(message),
                |message| journal.borrow_mut().debug(&message),
                |message| journal.borrow_mut().info(&message),
            )
        })
    }

    pub fn apply_with_derived_hold_preflight<Account, Tx, Journal, ParentBatchId, TxId, TxSource>(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
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
        self.with_journal_sinks_current_app_view_log_sinks(owner, |owner, app, view, journal| {
            owner.apply_with_app_view_and_log_sinks_and_derived_hold_preflight(
                app,
                view,
                tx_source,
                flags,
                consequences,
                can_be_held_result,
                |message| journal.borrow_mut().trace(message),
                |message| journal.borrow_mut().debug(&message),
                |message| journal.borrow_mut().info(&message),
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_owned_metrics_and_derived_hold_preflight<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
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
        self.with_journal_sinks_current_app_view_log_sinks(owner, |owner, app, view, journal| {
            owner.apply_with_app_view_and_owned_metrics_and_log_sinks_and_derived_hold_preflight(
                app,
                view,
                tx_source,
                flags,
                consequences,
                can_be_held_result,
                |message| journal.borrow_mut().trace(message),
                |message| journal.borrow_mut().debug(&message),
                |message| journal.borrow_mut().info(&message),
            )
        })
    }

    pub fn apply_with_derived_preflight_facts<Account, Tx, Journal, ParentBatchId, TxId, TxSource>(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
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
        self.with_journal_sinks_current_app_view_log_sinks(owner, |owner, app, view, journal| {
            owner.apply_with_app_view_and_log_sinks_and_derived_preflight_facts(
                app,
                view,
                tx_source,
                can_be_held_result,
                |message| journal.borrow_mut().trace(message),
                |message| journal.borrow_mut().debug(&message),
                |message| journal.borrow_mut().info(&message),
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_owned_metrics_and_derived_preflight_facts<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
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
        self.with_journal_sinks_current_app_view_log_sinks(owner, |owner, app, view, journal| {
            owner.apply_with_app_view_and_owned_metrics_and_log_sinks_and_derived_preflight_facts(
                app,
                view,
                tx_source,
                |message| journal.borrow_mut().trace(message),
                |message| journal.borrow_mut().debug(&message),
                |message| journal.borrow_mut().info(&message),
            )
        })
    }

    pub fn apply_with_derived_preflight_facts_and_hold_admission<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
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
        self.with_journal_sinks_current_app_view_log_sinks(owner, |owner, app, view, journal| {
            owner.apply_with_app_view_and_log_sinks_and_derived_preflight_facts_and_hold_admission(
                app,
                view,
                tx_source,
                |message| journal.borrow_mut().trace(message),
                |message| journal.borrow_mut().debug(&message),
                |message| journal.borrow_mut().info(&message),
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_owned_metrics_and_derived_preflight_facts_and_hold_admission<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
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
        self.with_journal_sinks_current_app_view_log_sinks(owner, |owner, app, view, journal| {
            owner.apply_with_app_view_and_owned_metrics_and_log_sinks_and_derived_preflight_facts_and_hold_admission(
                app,
                view,
                tx_source,
                |message| journal.borrow_mut().trace(message),
                |message| journal.borrow_mut().debug(&message),
                |message| journal.borrow_mut().info(&message),
            )
        })
    }

    pub fn apply_after_preflight_with_derived_hold_admission<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
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
        self.with_journal_sinks_current_app_view_log_sinks(owner, |owner, app, view, journal| {
            owner.apply_after_preflight_with_app_view_and_log_sinks_and_derived_hold_admission(
                app,
                view,
                tx_source,
                preflight_result,
                hold_preflight,
                |message| journal.borrow_mut().trace(message),
                |message| journal.borrow_mut().debug(&message),
                |message| journal.borrow_mut().info(&message),
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_owned_metrics_and_derived_hold_admission<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
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
        self.with_journal_sinks_current_app_view_log_sinks(owner, |owner, app, view, journal| {
            owner.apply_after_preflight_with_app_view_and_owned_metrics_and_log_sinks_and_derived_hold_admission(
                app,
                view,
                tx_source,
                preflight_result,
                hold_preflight,
                |message| journal.borrow_mut().trace(message),
                |message| journal.borrow_mut().debug(&message),
                |message| journal.borrow_mut().info(&message),
            )
        })
    }

    pub fn apply_after_preflight_with_derived_hold_preflight<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
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
        self.with_journal_sinks_current_app_view_log_sinks(owner, |owner, app, view, journal| {
            owner.apply_after_preflight_with_app_view_and_log_sinks_and_derived_hold_preflight(
                app,
                view,
                tx_source,
                preflight_result,
                can_be_held_result,
                |message| journal.borrow_mut().trace(message),
                |message| journal.borrow_mut().debug(&message),
                |message| journal.borrow_mut().info(&message),
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_owned_metrics_and_derived_hold_preflight<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
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
        self.with_journal_sinks_current_app_view_log_sinks(owner, |owner, app, view, journal| {
            owner.apply_after_preflight_with_app_view_and_owned_metrics_and_log_sinks_and_derived_hold_preflight(
                app,
                view,
                tx_source,
                preflight_result,
                can_be_held_result,
                |message| journal.borrow_mut().trace(message),
                |message| journal.borrow_mut().debug(&message),
                |message| journal.borrow_mut().info(&message),
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_caller_preclaim<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
        RunPreclaimStage,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
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
        RunPreclaimStage: FnOnce(
            QueueApplyPreparedPreclaimInputs<Account>,
        ) -> Result<
            QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>,
            crate::ApplyResult,
        >,
    {
        self.with_journal_sinks_current_app_view_caller_preclaim_log_sinks(
            owner,
            |owner, app, view, journal| {
                owner.apply_with_app_view_and_caller_preclaim_and_log_sinks(
                    app,
                    view,
                    tx_source,
                    hold_preflight,
                    flags,
                    consequences,
                    can_be_held_result,
                    run_preclaim_stage,
                    |message| journal.borrow_mut().trace(message),
                    |message| journal.borrow_mut().debug(&message),
                    |message| journal.borrow_mut().info(&message),
                )
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_owned_metrics_and_caller_preclaim<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
        RunPreclaimStage,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
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
        RunPreclaimStage: FnOnce(
            QueueApplyPreparedPreclaimInputs<Account>,
        ) -> Result<
            QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>,
            crate::ApplyResult,
        >,
    {
        self.with_journal_sinks_current_app_view_caller_preclaim_log_sinks(
            owner,
            |owner, app, view, journal| {
                owner.apply_with_app_view_and_owned_metrics_and_caller_preclaim_and_log_sinks(
                    app,
                    view,
                    tx_source,
                    hold_preflight,
                    flags,
                    consequences,
                    can_be_held_result,
                    run_preclaim_stage,
                    |message| journal.borrow_mut().trace(message),
                    |message| journal.borrow_mut().debug(&message),
                    |message| journal.borrow_mut().info(&message),
                )
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_caller_preclaim<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
        RunPreclaimStage,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
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
        RunPreclaimStage: FnOnce(
            QueueApplyPreparedPreclaimInputs<Account>,
        ) -> Result<
            QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>,
            crate::ApplyResult,
        >,
    {
        self.with_journal_sinks_current_app_view_caller_preclaim_log_sinks(
            owner,
            |owner, app, view, journal| {
                owner.apply_after_preflight_with_app_view_and_caller_preclaim_and_log_sinks(
                    app,
                    view,
                    tx_source,
                    preflight_result,
                    hold_preflight,
                    can_be_held_result,
                    run_preclaim_stage,
                    |message| journal.borrow_mut().trace(message),
                    |message| journal.borrow_mut().debug(&message),
                    |message| journal.borrow_mut().info(&message),
                )
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_owned_metrics_and_caller_preclaim<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
        RunPreclaimStage,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
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
        RunPreclaimStage: FnOnce(
            QueueApplyPreparedPreclaimInputs<Account>,
        ) -> Result<
            QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>,
            crate::ApplyResult,
        >,
    {
        self.with_journal_sinks_current_app_view_caller_preclaim_log_sinks(
            owner,
            |owner, app, view, journal| {
                owner.apply_after_preflight_with_app_view_and_owned_metrics_and_caller_preclaim_and_log_sinks(
                    app,
                    view,
                    tx_source,
                    preflight_result,
                    hold_preflight,
                    can_be_held_result,
                    run_preclaim_stage,
                    |message| journal.borrow_mut().trace(message),
                    |message| journal.borrow_mut().debug(&message),
                    |message| journal.borrow_mut().info(&message),
                )
            },
        )
    }
}
