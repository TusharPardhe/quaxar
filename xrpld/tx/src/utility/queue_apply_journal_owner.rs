//! Journal-owning wrapper for the current `xrpld` `TxQ::apply(...)`
//! boundary.
//!
//! This layer moves one more real owner concern into the Rust-side apply
//! object:
//! - one owner object carries the landed TxQ-owned apply state,
//! - the same owner object carries the caller-chosen journal sink surface,
//! - callers still supply the live app/runtime, ledger-view, and tx inputs,
//! - callers still own real mutex scope.

use std::fmt::Display;

use protocol::Ter;

use crate::{
    ApplyFlags, PreflightResult, QueueApplyAppRuntime, QueueApplyCurrentPreclaimClearRuntime,
    QueueApplyHoldPreflightTxSource, QueueApplyJournalEnvelope, QueueApplyJournalSink,
    QueueApplyLedgerViewSource, QueueApplyObservedTxSource, QueueApplyOwnerShell,
    QueueApplyPreclaimStage, QueueApplyPreflightStage, QueueApplyPreparedPreclaimInputs,
    QueueApplyTopWithLogMessagesResult, QueueHoldPreflight, TxConsequences,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueApplyJournalOwner<Account, Tx, Journal, ParentBatchId, Sink> {
    owner: QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
    journal: Sink,
}

impl<Account, Tx, Journal, ParentBatchId, Sink>
    QueueApplyJournalOwner<Account, Tx, Journal, ParentBatchId, Sink>
{
    pub const fn new(
        owner: QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        journal: Sink,
    ) -> Self {
        Self { owner, journal }
    }

    pub fn owner(&self) -> &QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId> {
        &self.owner
    }

    pub fn owner_mut(&mut self) -> &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId> {
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
    QueueApplyJournalOwner<Account, Tx, Journal, ParentBatchId, Sink>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    Sink: QueueApplyJournalSink,
{
    fn with_journal_envelope<App, View, R>(
        &mut self,
        app: &mut App,
        view: &View,
        run: impl FnOnce(
            &mut QueueApplyJournalEnvelope<'_, App, View, Sink>,
            &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        ) -> R,
    ) -> R
    where
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
    {
        let (owner, journal) = (&mut self.owner, &mut self.journal);
        let mut envelope = QueueApplyJournalEnvelope::new(app, view, journal);
        run(&mut envelope, owner)
    }

    fn with_journal_envelope_current_app_view_caller_preclaim<App, View, R>(
        &mut self,
        app: &mut App,
        view: &View,
        run: impl FnOnce(
            &mut QueueApplyJournalEnvelope<'_, App, View, Sink>,
            &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        ) -> R,
    ) -> R
    where
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
    {
        self.with_journal_envelope(app, view, run)
    }

    fn with_journal_envelope_current_app_view_caller_preclaim_log_sinks<App, View, R>(
        &mut self,
        app: &mut App,
        view: &View,
        run: impl FnOnce(
            &mut QueueApplyJournalEnvelope<'_, App, View, Sink>,
            &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        ) -> R,
    ) -> R
    where
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
    {
        self.with_journal_envelope_current_app_view_caller_preclaim(app, view, run)
    }

    fn with_journal_envelope_current_app_view_log_sinks<App, View, R>(
        &mut self,
        app: &mut App,
        view: &View,
        run: impl FnOnce(
            &mut QueueApplyJournalEnvelope<'_, App, View, Sink>,
            &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        ) -> R,
    ) -> R
    where
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
    {
        self.with_journal_envelope(app, view, run)
    }

    pub fn apply<App, View, TxId, TxSource>(
        &mut self,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        hold_preflight: QueueHoldPreflight,
        flags: ApplyFlags,
        consequences: TxConsequences,
        can_be_held_result: Ter,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxId: Clone + Display,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_journal_envelope_current_app_view_log_sinks(app, view, |envelope, owner| {
            envelope.apply(
                owner,
                tx_source,
                hold_preflight,
                flags,
                consequences,
                can_be_held_result,
            )
        })
    }

    pub fn apply_with_log_messages<App, View, TxId, TxSource>(
        &mut self,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        hold_preflight: QueueHoldPreflight,
        flags: ApplyFlags,
        consequences: TxConsequences,
        can_be_held_result: Ter,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxId: Clone + Display,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_journal_envelope_current_app_view_log_sinks(app, view, |envelope, owner| {
            envelope.apply_with_log_messages(
                owner,
                tx_source,
                hold_preflight,
                flags,
                consequences,
                can_be_held_result,
            )
        })
    }

    pub fn apply_with_owned_metrics<App, View, TxId, TxSource>(
        &mut self,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        hold_preflight: QueueHoldPreflight,
        flags: ApplyFlags,
        consequences: TxConsequences,
        can_be_held_result: Ter,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxId: Clone + Display,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_journal_envelope_current_app_view_log_sinks(app, view, |envelope, owner| {
            envelope.apply_with_owned_metrics(
                owner,
                tx_source,
                hold_preflight,
                flags,
                consequences,
                can_be_held_result,
            )
        })
    }

    pub fn apply_after_preflight<App, View, TxId, TxSource>(
        &mut self,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
        can_be_held_result: Ter,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxId: Clone + Display,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_journal_envelope_current_app_view_log_sinks(app, view, |envelope, owner| {
            envelope.apply_after_preflight(
                owner,
                tx_source,
                preflight_result,
                hold_preflight,
                can_be_held_result,
            )
        })
    }

    pub fn apply_after_preflight_with_log_messages<App, View, TxId, TxSource>(
        &mut self,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
        can_be_held_result: Ter,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxId: Clone + Display,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_journal_envelope_current_app_view_log_sinks(app, view, |envelope, owner| {
            envelope.apply_after_preflight_with_log_messages(
                owner,
                tx_source,
                preflight_result,
                hold_preflight,
                can_be_held_result,
            )
        })
    }

    pub fn apply_after_preflight_with_owned_metrics<App, View, TxId, TxSource>(
        &mut self,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
        can_be_held_result: Ter,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxId: Clone + Display,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_journal_envelope_current_app_view_log_sinks(app, view, |envelope, owner| {
            envelope.apply_after_preflight_with_owned_metrics(
                owner,
                tx_source,
                preflight_result,
                hold_preflight,
                can_be_held_result,
            )
        })
    }

    pub fn apply_with_derived_hold_preflight<App, View, TxId, TxSource>(
        &mut self,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        flags: ApplyFlags,
        consequences: TxConsequences,
        can_be_held_result: Ter,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxId: Clone + Display,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_journal_envelope_current_app_view_log_sinks(app, view, |envelope, owner| {
            envelope.apply_with_derived_hold_preflight(
                owner,
                tx_source,
                flags,
                consequences,
                can_be_held_result,
            )
        })
    }

    pub fn apply_with_owned_metrics_and_derived_hold_preflight<App, View, TxId, TxSource>(
        &mut self,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        flags: ApplyFlags,
        consequences: TxConsequences,
        can_be_held_result: Ter,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxId: Clone + Display,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_journal_envelope_current_app_view_log_sinks(app, view, |envelope, owner| {
            envelope.apply_with_owned_metrics_and_derived_hold_preflight(
                owner,
                tx_source,
                flags,
                consequences,
                can_be_held_result,
            )
        })
    }

    pub fn apply_with_derived_preflight_facts<App, View, TxId, TxSource>(
        &mut self,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        can_be_held_result: Ter,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxId: Clone + Display,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_journal_envelope_current_app_view_log_sinks(app, view, |envelope, owner| {
            envelope.apply_with_derived_preflight_facts(owner, tx_source, can_be_held_result)
        })
    }

    pub fn apply_with_owned_metrics_and_derived_preflight_facts<App, View, TxId, TxSource>(
        &mut self,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxId: Clone + Display,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_journal_envelope_current_app_view_log_sinks(app, view, |envelope, owner| {
            envelope.apply_with_owned_metrics_and_derived_preflight_facts(owner, tx_source)
        })
    }

    pub fn apply_with_derived_preflight_facts_and_hold_admission<App, View, TxId, TxSource>(
        &mut self,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxId: Clone + Display,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_journal_envelope_current_app_view_log_sinks(app, view, |envelope, owner| {
            envelope.apply_with_derived_preflight_facts_and_hold_admission(owner, tx_source)
        })
    }

    pub fn apply_with_owned_metrics_and_derived_preflight_facts_and_hold_admission<
        App,
        View,
        TxId,
        TxSource,
    >(
        &mut self,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxId: Clone + Display,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_journal_envelope_current_app_view_log_sinks(app, view, |envelope, owner| {
            envelope.apply_with_owned_metrics_and_derived_preflight_facts_and_hold_admission(
                owner, tx_source,
            )
        })
    }

    pub fn apply_after_preflight_with_derived_hold_preflight<App, View, TxId, TxSource>(
        &mut self,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        can_be_held_result: Ter,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxId: Clone + Display,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_journal_envelope_current_app_view_log_sinks(app, view, |envelope, owner| {
            envelope.apply_after_preflight_with_derived_hold_preflight(
                owner,
                tx_source,
                preflight_result,
                can_be_held_result,
            )
        })
    }

    pub fn apply_after_preflight_with_owned_metrics_and_derived_hold_preflight<
        App,
        View,
        TxId,
        TxSource,
    >(
        &mut self,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        can_be_held_result: Ter,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxId: Clone + Display,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_journal_envelope_current_app_view_log_sinks(app, view, |envelope, owner| {
            envelope.apply_after_preflight_with_owned_metrics_and_derived_hold_preflight(
                owner,
                tx_source,
                preflight_result,
                can_be_held_result,
            )
        })
    }

    pub fn apply_after_preflight_with_derived_hold_admission<App, View, TxId, TxSource>(
        &mut self,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxId: Clone + Display,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_journal_envelope_current_app_view_log_sinks(app, view, |envelope, owner| {
            envelope.apply_after_preflight_with_derived_hold_admission(
                owner,
                tx_source,
                preflight_result,
                hold_preflight,
            )
        })
    }

    pub fn apply_after_preflight_with_owned_metrics_and_derived_hold_admission<
        App,
        View,
        TxId,
        TxSource,
    >(
        &mut self,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxId: Clone + Display,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_journal_envelope_current_app_view_log_sinks(app, view, |envelope, owner| {
            envelope.apply_after_preflight_with_owned_metrics_and_derived_hold_admission(
                owner,
                tx_source,
                preflight_result,
                hold_preflight,
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_caller_preclaim<App, View, TxId, TxSource, RunPreclaimStage>(
        &mut self,
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
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>
            + QueueApplyCurrentPreclaimClearRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxId: Clone + Display,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        RunPreclaimStage: FnOnce(
            QueueApplyPreparedPreclaimInputs<Account>,
        ) -> Result<
            QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>,
            crate::ApplyResult,
        >,
    {
        self.with_journal_envelope_current_app_view_caller_preclaim_log_sinks(
            app,
            view,
            |envelope, owner| {
                envelope.apply_with_caller_preclaim(
                    owner,
                    tx_source,
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
    pub fn apply_with_owned_metrics_and_caller_preclaim<
        App,
        View,
        TxId,
        TxSource,
        RunPreclaimStage,
    >(
        &mut self,
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
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>
            + QueueApplyCurrentPreclaimClearRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxId: Clone + Display,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        RunPreclaimStage: FnOnce(
            QueueApplyPreparedPreclaimInputs<Account>,
        ) -> Result<
            QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>,
            crate::ApplyResult,
        >,
    {
        self.with_journal_envelope_current_app_view_caller_preclaim_log_sinks(
            app,
            view,
            |envelope, owner| {
                envelope.apply_with_owned_metrics_and_caller_preclaim(
                    owner,
                    tx_source,
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
    pub fn apply_after_preflight_with_caller_preclaim<App, View, TxId, TxSource, RunPreclaimStage>(
        &mut self,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
        can_be_held_result: Ter,
        run_preclaim_stage: RunPreclaimStage,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>
            + QueueApplyCurrentPreclaimClearRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxId: Clone + Display,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        RunPreclaimStage: FnOnce(
            QueueApplyPreparedPreclaimInputs<Account>,
        ) -> Result<
            QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>,
            crate::ApplyResult,
        >,
    {
        self.with_journal_envelope_current_app_view_caller_preclaim_log_sinks(
            app,
            view,
            |envelope, owner| {
                envelope.apply_after_preflight_with_caller_preclaim(
                    owner,
                    tx_source,
                    preflight_result,
                    hold_preflight,
                    can_be_held_result,
                    run_preclaim_stage,
                )
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_owned_metrics_and_caller_preclaim<
        App,
        View,
        TxId,
        TxSource,
        RunPreclaimStage,
    >(
        &mut self,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
        can_be_held_result: Ter,
        run_preclaim_stage: RunPreclaimStage,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>
            + QueueApplyCurrentPreclaimClearRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxId: Clone + Display,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        RunPreclaimStage: FnOnce(
            QueueApplyPreparedPreclaimInputs<Account>,
        ) -> Result<
            QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>,
            crate::ApplyResult,
        >,
    {
        self.with_journal_envelope_current_app_view_caller_preclaim_log_sinks(
            app,
            view,
            |envelope, owner| {
                envelope.apply_after_preflight_with_owned_metrics_and_caller_preclaim(
                    owner,
                    tx_source,
                    preflight_result,
                    hold_preflight,
                    can_be_held_result,
                    run_preclaim_stage,
                )
            },
        )
    }
}
