//! `TxQ`-shaped method facade for the current `xrpld` `TxQ::apply(...)`
//! boundary.
//!
//! This layer is intentionally thin:
//! - one Rust-side owner object carries the landed apply state and explicit
//!   lock-scope boundary,
//! - callers still supply a lock token,
//! - callers still supply the live app/runtime, ledger-view, and tx inputs,
//! - the facade stays focused on the current apply surface.

use std::fmt::Display;

use protocol::Ter;

use crate::{
    ApplyFlags, PreclaimResult, PreflightResult, QueueApplyAppRuntime,
    QueueApplyCurrentPreclaimClearRuntime, QueueApplyHoldPreflightTxSource, QueueApplyJournalSink,
    QueueApplyJournalTxQ, QueueApplyLedgerViewSource, QueueApplyLockScope,
    QueueApplyLockScopeOwner, QueueApplyObservedTxSource, QueueApplyObservedViewSource,
    QueueApplyPreclaimStage, QueueApplyPreclaimViewSource, QueueApplyPreflightStage,
    QueueApplyPreparedPreclaimInputs, QueueApplyTopWithLogMessagesResult, QueueApplyTryClearResult,
    QueueApplyViewAdjustment, QueueHoldPreflight, TxConsequences,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueApplyTxQ<Account, Tx, Journal, ParentBatchId> {
    owner: QueueApplyLockScopeOwner<Account, Tx, Journal, ParentBatchId>,
}

impl<Account, Tx, Journal, ParentBatchId> QueueApplyTxQ<Account, Tx, Journal, ParentBatchId> {
    pub const fn new(owner: QueueApplyLockScopeOwner<Account, Tx, Journal, ParentBatchId>) -> Self {
        Self { owner }
    }

    pub fn lock_scope_owner(
        &self,
    ) -> &QueueApplyLockScopeOwner<Account, Tx, Journal, ParentBatchId> {
        &self.owner
    }

    pub fn lock_scope_owner_mut(
        &mut self,
    ) -> &mut QueueApplyLockScopeOwner<Account, Tx, Journal, ParentBatchId> {
        &mut self.owner
    }

    pub fn into_journal_txq<Sink>(
        self,
        journal: Sink,
    ) -> QueueApplyJournalTxQ<Account, Tx, Journal, ParentBatchId, Sink>
    where
        Sink: QueueApplyJournalSink,
    {
        QueueApplyJournalTxQ::new(self.owner.into_journal_lock_scope_owner(journal))
    }
}

impl<Account, Tx, Journal, ParentBatchId> QueueApplyTxQ<Account, Tx, Journal, ParentBatchId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
{
    fn with_txq_owner<Lock, R>(
        &mut self,
        lock: &mut Lock,
        run: impl FnOnce(
            &mut QueueApplyLockScopeOwner<Account, Tx, Journal, ParentBatchId>,
            &mut Lock,
        ) -> R,
    ) -> R
    where
        Lock: QueueApplyLockScope,
    {
        run(&mut self.owner, lock)
    }

    fn with_txq_owner_current_app_view<Lock, R>(
        &mut self,
        lock: &mut Lock,
        run: impl FnOnce(
            &mut QueueApplyLockScopeOwner<Account, Tx, Journal, ParentBatchId>,
            &mut Lock,
        ) -> R,
    ) -> R
    where
        Lock: QueueApplyLockScope,
    {
        self.with_txq_owner(lock, run)
    }

    fn with_txq_owner_current_app_view_caller_preclaim<Lock, R>(
        &mut self,
        lock: &mut Lock,
        run: impl FnOnce(
            &mut QueueApplyLockScopeOwner<Account, Tx, Journal, ParentBatchId>,
            &mut Lock,
        ) -> R,
    ) -> R
    where
        Lock: QueueApplyLockScope,
    {
        self.with_txq_owner(lock, run)
    }

    fn with_txq_owner_current_app_view_caller_preclaim_log_sinks<Lock, R>(
        &mut self,
        lock: &mut Lock,
        run: impl FnOnce(
            &mut QueueApplyLockScopeOwner<Account, Tx, Journal, ParentBatchId>,
            &mut Lock,
        ) -> R,
    ) -> R
    where
        Lock: QueueApplyLockScope,
    {
        self.with_txq_owner(lock, run)
    }

    fn with_txq_owner_current_app_view_log_sinks<Lock, R>(
        &mut self,
        lock: &mut Lock,
        run: impl FnOnce(
            &mut QueueApplyLockScopeOwner<Account, Tx, Journal, ParentBatchId>,
            &mut Lock,
        ) -> R,
    ) -> R
    where
        Lock: QueueApplyLockScope,
    {
        self.with_txq_owner(lock, run)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply<
        Lock,
        TxId,
        TxSource,
        ViewSource,
        RunPreflight,
        TraceFn,
        ApplyFn,
        PrepareMultiTxn,
        RunPreclaim,
        RunTryClear,
        TryClearResult,
        ApplySandbox,
    >(
        &mut self,
        lock: &mut Lock,
        tx_source: &TxSource,
        view_source: &ViewSource,
        hold_preflight: QueueHoldPreflight,
        flags: ApplyFlags,
        consequences: TxConsequences,
        can_be_held_result: Ter,
        run_preflight: RunPreflight,
        trace: TraceFn,
        apply: ApplyFn,
        prepare_multitxn: PrepareMultiTxn,
        run_preclaim: RunPreclaim,
        run_try_clear: RunTryClear,
        apply_sandbox: ApplySandbox,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: QueueApplyObservedViewSource<Account>,
        RunPreflight: FnOnce() -> PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        TraceFn: FnMut(&str),
        ApplyFn: FnOnce() -> crate::ApplyResult,
        PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
        RunPreclaim:
            FnOnce(QueueApplyPreclaimViewSource) -> PreclaimResult<Tx, Journal, ParentBatchId>,
        RunTryClear: FnOnce() -> TryClearResult,
        TryClearResult: QueueApplyTryClearResult,
        ApplySandbox: FnOnce(),
    {
        self.with_txq_owner(lock, |owner, lock| {
            owner.apply(
                lock,
                tx_source,
                view_source,
                hold_preflight,
                flags,
                consequences,
                can_be_held_result,
                run_preflight,
                trace,
                apply,
                prepare_multitxn,
                run_preclaim,
                run_try_clear,
                apply_sandbox,
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight<
        Lock,
        TxId,
        TxSource,
        ViewSource,
        TraceFn,
        ApplyFn,
        PrepareMultiTxn,
        RunPreclaim,
        RunTryClear,
        TryClearResult,
        ApplySandbox,
    >(
        &mut self,
        lock: &mut Lock,
        tx_source: &TxSource,
        view_source: &ViewSource,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
        can_be_held_result: Ter,
        trace: TraceFn,
        apply: ApplyFn,
        prepare_multitxn: PrepareMultiTxn,
        run_preclaim: RunPreclaim,
        run_try_clear: RunTryClear,
        apply_sandbox: ApplySandbox,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: QueueApplyObservedViewSource<Account>,
        TraceFn: FnMut(&str),
        ApplyFn: FnOnce() -> crate::ApplyResult,
        PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
        RunPreclaim:
            FnOnce(QueueApplyPreclaimViewSource) -> PreclaimResult<Tx, Journal, ParentBatchId>,
        RunTryClear: FnOnce() -> TryClearResult,
        TryClearResult: QueueApplyTryClearResult,
        ApplySandbox: FnOnce(),
    {
        self.with_txq_owner(lock, |owner, lock| {
            owner.apply_after_preflight(
                lock,
                tx_source,
                view_source,
                preflight_result,
                hold_preflight,
                can_be_held_result,
                trace,
                apply,
                prepare_multitxn,
                run_preclaim,
                run_try_clear,
                apply_sandbox,
            )
        })
    }

    pub fn apply_with_app_view<Lock, TxId, App, View, TxSource>(
        &mut self,
        lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        hold_preflight: QueueHoldPreflight,
        flags: ApplyFlags,
        consequences: TxConsequences,
        can_be_held_result: Ter,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_txq_owner_current_app_view(lock, |owner, lock| {
            owner.apply_with_app_view(
                lock,
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

    pub fn apply_with_log_messages<Lock, TxId, App, View, TxSource>(
        &mut self,
        lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        hold_preflight: QueueHoldPreflight,
        flags: ApplyFlags,
        consequences: TxConsequences,
        can_be_held_result: Ter,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_txq_owner_current_app_view(lock, |owner, lock| {
            owner.apply_with_log_messages(
                lock,
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

    pub fn apply_with_app_view_and_owned_metrics<Lock, TxId, App, View, TxSource>(
        &mut self,
        lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        hold_preflight: QueueHoldPreflight,
        flags: ApplyFlags,
        consequences: TxConsequences,
        can_be_held_result: Ter,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_txq_owner_current_app_view(lock, |owner, lock| {
            owner.apply_with_app_view_and_owned_metrics(
                lock,
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

    pub fn apply_with_app_view_and_owned_metrics_and_log_messages<Lock, TxId, App, View, TxSource>(
        &mut self,
        lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        hold_preflight: QueueHoldPreflight,
        flags: ApplyFlags,
        consequences: TxConsequences,
        can_be_held_result: Ter,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_txq_owner_current_app_view(lock, |owner, lock| {
            owner.apply_with_app_view_and_owned_metrics_and_log_messages(
                lock,
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
    pub fn apply_with_app_view_and_caller_preclaim<
        Lock,
        TxId,
        App,
        View,
        TxSource,
        RunPreclaimStage,
    >(
        &mut self,
        lock: &mut Lock,
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
        Lock: QueueApplyLockScope,
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
        self.with_txq_owner_current_app_view_caller_preclaim(lock, |owner, lock| {
            owner.apply_with_app_view_and_caller_preclaim(
                lock,
                app,
                view,
                tx_source,
                hold_preflight,
                flags,
                consequences,
                can_be_held_result,
                run_preclaim_stage,
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_app_view_and_caller_preclaim_and_log_messages<
        Lock,
        TxId,
        App,
        View,
        TxSource,
        RunPreclaimStage,
    >(
        &mut self,
        lock: &mut Lock,
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
        Lock: QueueApplyLockScope,
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
        self.with_txq_owner_current_app_view_caller_preclaim(lock, |owner, lock| {
            owner.apply_with_app_view_and_caller_preclaim_and_log_messages(
                lock,
                app,
                view,
                tx_source,
                hold_preflight,
                flags,
                consequences,
                can_be_held_result,
                run_preclaim_stage,
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_app_view_and_owned_metrics_and_caller_preclaim<
        Lock,
        TxId,
        App,
        View,
        TxSource,
        RunPreclaimStage,
    >(
        &mut self,
        lock: &mut Lock,
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
        Lock: QueueApplyLockScope,
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
        self.with_txq_owner_current_app_view_caller_preclaim(lock, |owner, lock| {
            owner.apply_with_app_view_and_owned_metrics_and_caller_preclaim(
                lock,
                app,
                view,
                tx_source,
                hold_preflight,
                flags,
                consequences,
                can_be_held_result,
                run_preclaim_stage,
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_app_view_and_owned_metrics_and_caller_preclaim_and_log_messages<
        Lock,
        TxId,
        App,
        View,
        TxSource,
        RunPreclaimStage,
    >(
        &mut self,
        lock: &mut Lock,
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
        Lock: QueueApplyLockScope,
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
        self.with_txq_owner_current_app_view_caller_preclaim(lock, |owner, lock| {
            owner.apply_with_app_view_and_owned_metrics_and_caller_preclaim_and_log_messages(
                lock,
                app,
                view,
                tx_source,
                hold_preflight,
                flags,
                consequences,
                can_be_held_result,
                run_preclaim_stage,
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_app_view_and_caller_preclaim_and_log_sinks<
        Lock,
        TxId,
        App,
        View,
        TxSource,
        RunPreclaimStage,
        TraceFn,
        DebugFn,
        InfoFn,
    >(
        &mut self,
        lock: &mut Lock,
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
        Lock: QueueApplyLockScope,
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
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
    {
        self.with_txq_owner_current_app_view_caller_preclaim_log_sinks(lock, |owner, lock| {
            owner.apply_with_app_view_and_caller_preclaim_and_log_sinks(
                lock,
                app,
                view,
                tx_source,
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
    pub fn apply_with_app_view_and_owned_metrics_and_log_sinks<
        Lock,
        TxId,
        App,
        View,
        TxSource,
        TraceFn,
        DebugFn,
        InfoFn,
    >(
        &mut self,
        lock: &mut Lock,
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
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
    {
        self.with_txq_owner_current_app_view_log_sinks(lock, |owner, lock| {
            owner.apply_with_app_view_and_owned_metrics_and_log_sinks(
                lock,
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
    pub fn apply_with_app_view_and_owned_metrics_and_log_sinks_and_derived_hold_admission<
        Lock,
        TxId,
        App,
        View,
        TxSource,
        TraceFn,
        DebugFn,
        InfoFn,
    >(
        &mut self,
        lock: &mut Lock,
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
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
    {
        self.with_txq_owner_current_app_view_log_sinks(lock, |owner, lock| {
            owner.apply_with_app_view_and_owned_metrics_and_log_sinks_and_derived_hold_admission(
                lock,
                app,
                view,
                tx_source,
                hold_preflight,
                flags,
                consequences,
                trace,
                debug,
                info,
            )
        })
    }

    pub fn apply_with_app_view_and_owned_metrics_and_derived_hold_admission_and_log_messages<
        Lock,
        TxId,
        App,
        View,
        TxSource,
    >(
        &mut self,
        lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        hold_preflight: QueueHoldPreflight,
        flags: ApplyFlags,
        consequences: TxConsequences,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_txq_owner_current_app_view(lock, |owner, lock| {
            owner.apply_with_app_view_and_owned_metrics_and_derived_hold_admission_and_log_messages(
                lock,
                app,
                view,
                tx_source,
                hold_preflight,
                flags,
                consequences,
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_app_view_and_owned_metrics_and_log_sinks_and_derived_hold_preflight<
        Lock,
        TxId,
        App,
        View,
        TxSource,
        TraceFn,
        DebugFn,
        InfoFn,
    >(
        &mut self,
        lock: &mut Lock,
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
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
    {
        self.with_txq_owner_current_app_view_log_sinks(lock, |owner, lock| {
            owner.apply_with_app_view_and_owned_metrics_and_log_sinks_and_derived_hold_preflight(
                lock,
                app,
                view,
                tx_source,
                flags,
                consequences,
                can_be_held_result,
                trace,
                debug,
                info,
            )
        })
    }

    pub fn apply_with_app_view_and_owned_metrics_and_derived_hold_preflight_and_log_messages<
        Lock,
        TxId,
        App,
        View,
        TxSource,
    >(
        &mut self,
        lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        flags: ApplyFlags,
        consequences: TxConsequences,
        can_be_held_result: Ter,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_txq_owner_current_app_view(lock, |owner, lock| {
            owner.apply_with_app_view_and_owned_metrics_and_derived_hold_preflight_and_log_messages(
                lock,
                app,
                view,
                tx_source,
                flags,
                consequences,
                can_be_held_result,
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_app_view_and_owned_metrics_and_log_sinks_and_derived_preflight_facts<
        Lock,
        TxId,
        App,
        View,
        TxSource,
        TraceFn,
        DebugFn,
        InfoFn,
    >(
        &mut self,
        lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        trace: TraceFn,
        debug: DebugFn,
        info: InfoFn,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
    {
        self.with_txq_owner_current_app_view_log_sinks(lock, |owner, lock| {
            owner.apply_with_app_view_and_owned_metrics_and_log_sinks_and_derived_preflight_facts(
                lock, app, view, tx_source, trace, debug, info,
            )
        })
    }

    pub fn apply_with_app_view_and_owned_metrics_and_derived_preflight_facts_and_log_messages<
        Lock,
        TxId,
        App,
        View,
        TxSource,
    >(
        &mut self,
        lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_txq_owner_current_app_view(lock, |owner, lock| {
            owner
                .apply_with_app_view_and_owned_metrics_and_derived_preflight_facts_and_log_messages(
                    lock, app, view, tx_source,
                )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_app_view_and_owned_metrics_and_log_sinks_and_derived_preflight_facts_and_hold_admission<
        Lock,
        TxId,
        App,
        View,
        TxSource,
        TraceFn,
        DebugFn,
        InfoFn,
    >(
        &mut self,
        lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        trace: TraceFn,
        debug: DebugFn,
        info: InfoFn,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
    {
        self.with_txq_owner_current_app_view_log_sinks(lock, |owner, lock| {
            owner
                .apply_with_app_view_and_owned_metrics_and_log_sinks_and_derived_preflight_facts_and_hold_admission(
                    lock, app, view, tx_source, trace, debug, info,
                )
        })
    }

    pub fn apply_with_app_view_and_owned_metrics_and_derived_preflight_facts_and_hold_admission_and_log_messages<
        Lock,
        TxId,
        App,
        View,
        TxSource,
    >(
        &mut self,
        lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_txq_owner_current_app_view(lock, |owner, lock| {
            owner
                .apply_with_app_view_and_owned_metrics_and_derived_preflight_facts_and_hold_admission_and_log_messages(
                    lock, app, view, tx_source,
                )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_app_view_and_owned_metrics_and_caller_preclaim_and_log_sinks<
        Lock,
        TxId,
        App,
        View,
        TxSource,
        RunPreclaimStage,
        TraceFn,
        DebugFn,
        InfoFn,
    >(
        &mut self,
        lock: &mut Lock,
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
        Lock: QueueApplyLockScope,
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
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
    {
        self.with_txq_owner_current_app_view_caller_preclaim_log_sinks(lock, |owner, lock| {
            owner.apply_with_app_view_and_owned_metrics_and_caller_preclaim_and_log_sinks(
                lock,
                app,
                view,
                tx_source,
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

    pub fn apply_with_app_view_and_derived_hold_preflight<Lock, TxId, App, View, TxSource>(
        &mut self,
        lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        flags: ApplyFlags,
        consequences: TxConsequences,
        can_be_held_result: Ter,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_txq_owner_current_app_view(lock, |owner, lock| {
            owner.apply_with_app_view_and_derived_hold_preflight(
                lock,
                app,
                view,
                tx_source,
                flags,
                consequences,
                can_be_held_result,
            )
        })
    }

    pub fn apply_with_app_view_and_derived_hold_preflight_and_log_messages<
        Lock,
        TxId,
        App,
        View,
        TxSource,
    >(
        &mut self,
        lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        flags: ApplyFlags,
        consequences: TxConsequences,
        can_be_held_result: Ter,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_txq_owner_current_app_view(lock, |owner, lock| {
            owner.apply_with_app_view_and_derived_hold_preflight_and_log_messages(
                lock,
                app,
                view,
                tx_source,
                flags,
                consequences,
                can_be_held_result,
            )
        })
    }

    pub fn apply_with_app_view_and_derived_hold_admission<Lock, TxId, App, View, TxSource>(
        &mut self,
        lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        hold_preflight: QueueHoldPreflight,
        flags: ApplyFlags,
        consequences: TxConsequences,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_txq_owner_current_app_view(lock, |owner, lock| {
            owner.apply_with_app_view_and_derived_hold_admission(
                lock,
                app,
                view,
                tx_source,
                hold_preflight,
                flags,
                consequences,
            )
        })
    }

    pub fn apply_with_app_view_and_derived_hold_admission_and_log_messages<
        Lock,
        TxId,
        App,
        View,
        TxSource,
    >(
        &mut self,
        lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        hold_preflight: QueueHoldPreflight,
        flags: ApplyFlags,
        consequences: TxConsequences,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_txq_owner_current_app_view(lock, |owner, lock| {
            owner.apply_with_app_view_and_derived_hold_admission_and_log_messages(
                lock,
                app,
                view,
                tx_source,
                hold_preflight,
                flags,
                consequences,
            )
        })
    }

    pub fn apply_with_app_view_and_derived_preflight_facts<Lock, TxId, App, View, TxSource>(
        &mut self,
        lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        can_be_held_result: Ter,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_txq_owner_current_app_view(lock, |owner, lock| {
            owner.apply_with_app_view_and_derived_preflight_facts(
                lock,
                app,
                view,
                tx_source,
                can_be_held_result,
            )
        })
    }

    pub fn apply_with_app_view_and_derived_preflight_facts_and_log_messages<
        Lock,
        TxId,
        App,
        View,
        TxSource,
    >(
        &mut self,
        lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        can_be_held_result: Ter,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_txq_owner_current_app_view(lock, |owner, lock| {
            owner.apply_with_app_view_and_derived_preflight_facts_and_log_messages(
                lock,
                app,
                view,
                tx_source,
                can_be_held_result,
            )
        })
    }

    pub fn apply_with_app_view_and_derived_preflight_facts_and_hold_admission<
        Lock,
        TxId,
        App,
        View,
        TxSource,
    >(
        &mut self,
        lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_txq_owner_current_app_view(lock, |owner, lock| {
            owner.apply_with_app_view_and_derived_preflight_facts_and_hold_admission(
                lock, app, view, tx_source,
            )
        })
    }

    pub fn apply_with_app_view_and_derived_preflight_facts_and_hold_admission_and_log_messages<
        Lock,
        TxId,
        App,
        View,
        TxSource,
    >(
        &mut self,
        lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_txq_owner_current_app_view(lock, |owner, lock| {
            owner.apply_with_app_view_and_derived_preflight_facts_and_hold_admission_and_log_messages(
                lock, app, view, tx_source,
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_app_view_and_log_sinks<
        Lock,
        TxId,
        App,
        View,
        TxSource,
        TraceFn,
        DebugFn,
        InfoFn,
    >(
        &mut self,
        lock: &mut Lock,
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
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
    {
        self.with_txq_owner_current_app_view_log_sinks(lock, |owner, lock| {
            owner.apply_with_app_view_and_log_sinks(
                lock,
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
    pub fn apply_with_app_view_and_log_sinks_and_derived_hold_preflight<
        Lock,
        TxId,
        App,
        View,
        TxSource,
        TraceFn,
        DebugFn,
        InfoFn,
    >(
        &mut self,
        lock: &mut Lock,
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
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
    {
        self.with_txq_owner_current_app_view_log_sinks(lock, |owner, lock| {
            owner.apply_with_app_view_and_log_sinks_and_derived_hold_preflight(
                lock,
                app,
                view,
                tx_source,
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
    pub fn apply_with_app_view_and_log_sinks_and_derived_hold_admission<
        Lock,
        TxId,
        App,
        View,
        TxSource,
        TraceFn,
        DebugFn,
        InfoFn,
    >(
        &mut self,
        lock: &mut Lock,
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
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
    {
        self.with_txq_owner_current_app_view_log_sinks(lock, |owner, lock| {
            owner.apply_with_app_view_and_log_sinks_and_derived_hold_admission(
                lock,
                app,
                view,
                tx_source,
                hold_preflight,
                flags,
                consequences,
                trace,
                debug,
                info,
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_app_view_and_log_sinks_and_derived_preflight_facts<
        Lock,
        TxId,
        App,
        View,
        TxSource,
        TraceFn,
        DebugFn,
        InfoFn,
    >(
        &mut self,
        lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        can_be_held_result: Ter,
        trace: TraceFn,
        debug: DebugFn,
        info: InfoFn,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
    {
        self.with_txq_owner_current_app_view_log_sinks(lock, |owner, lock| {
            owner.apply_with_app_view_and_log_sinks_and_derived_preflight_facts(
                lock,
                app,
                view,
                tx_source,
                can_be_held_result,
                trace,
                debug,
                info,
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_app_view_and_log_sinks_and_derived_preflight_facts_and_hold_admission<
        Lock,
        TxId,
        App,
        View,
        TxSource,
        TraceFn,
        DebugFn,
        InfoFn,
    >(
        &mut self,
        lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        trace: TraceFn,
        debug: DebugFn,
        info: InfoFn,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
    {
        self.with_txq_owner_current_app_view_log_sinks(lock, |owner, lock| {
            owner.apply_with_app_view_and_log_sinks_and_derived_preflight_facts_and_hold_admission(
                lock, app, view, tx_source, trace, debug, info,
            )
        })
    }

    pub fn apply_after_preflight_with_app_view<Lock, TxId, App, View, TxSource>(
        &mut self,
        lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
        can_be_held_result: Ter,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_txq_owner_current_app_view(lock, |owner, lock| {
            owner.apply_after_preflight_with_app_view(
                lock,
                app,
                view,
                tx_source,
                preflight_result,
                hold_preflight,
                can_be_held_result,
            )
        })
    }

    pub fn apply_after_preflight_with_log_messages<Lock, TxId, App, View, TxSource>(
        &mut self,
        lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
        can_be_held_result: Ter,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_txq_owner_current_app_view(lock, |owner, lock| {
            owner.apply_after_preflight_with_log_messages(
                lock,
                app,
                view,
                tx_source,
                preflight_result,
                hold_preflight,
                can_be_held_result,
            )
        })
    }

    pub fn apply_after_preflight_with_app_view_and_owned_metrics<Lock, TxId, App, View, TxSource>(
        &mut self,
        lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
        can_be_held_result: Ter,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_txq_owner_current_app_view(lock, |owner, lock| {
            owner.apply_after_preflight_with_app_view_and_owned_metrics(
                lock,
                app,
                view,
                tx_source,
                preflight_result,
                hold_preflight,
                can_be_held_result,
            )
        })
    }

    pub fn apply_after_preflight_with_app_view_and_owned_metrics_and_log_messages<
        Lock,
        TxId,
        App,
        View,
        TxSource,
    >(
        &mut self,
        lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
        can_be_held_result: Ter,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_txq_owner_current_app_view(lock, |owner, lock| {
            owner.apply_after_preflight_with_app_view_and_owned_metrics_and_log_messages(
                lock,
                app,
                view,
                tx_source,
                preflight_result,
                hold_preflight,
                can_be_held_result,
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_app_view_and_caller_preclaim<
        Lock,
        TxId,
        App,
        View,
        TxSource,
        RunPreclaimStage,
    >(
        &mut self,
        lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
        can_be_held_result: Ter,
        run_preclaim_stage: RunPreclaimStage,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
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
        self.with_txq_owner_current_app_view_caller_preclaim(lock, |owner, lock| {
            owner.apply_after_preflight_with_app_view_and_caller_preclaim(
                lock,
                app,
                view,
                tx_source,
                preflight_result,
                hold_preflight,
                can_be_held_result,
                run_preclaim_stage,
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_app_view_and_caller_preclaim_and_log_messages<
        Lock,
        TxId,
        App,
        View,
        TxSource,
        RunPreclaimStage,
    >(
        &mut self,
        lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
        can_be_held_result: Ter,
        run_preclaim_stage: RunPreclaimStage,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
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
        self.with_txq_owner_current_app_view_caller_preclaim(lock, |owner, lock| {
            owner.apply_after_preflight_with_app_view_and_caller_preclaim_and_log_messages(
                lock,
                app,
                view,
                tx_source,
                preflight_result,
                hold_preflight,
                can_be_held_result,
                run_preclaim_stage,
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_app_view_and_owned_metrics_and_caller_preclaim<
        Lock,
        TxId,
        App,
        View,
        TxSource,
        RunPreclaimStage,
    >(
        &mut self,
        lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
        can_be_held_result: Ter,
        run_preclaim_stage: RunPreclaimStage,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
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
        self.with_txq_owner_current_app_view_caller_preclaim(lock, |owner, lock| {
            owner.apply_after_preflight_with_app_view_and_owned_metrics_and_caller_preclaim(
                lock,
                app,
                view,
                tx_source,
                preflight_result,
                hold_preflight,
                can_be_held_result,
                run_preclaim_stage,
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_app_view_and_owned_metrics_and_caller_preclaim_and_log_messages<
        Lock,
        TxId,
        App,
        View,
        TxSource,
        RunPreclaimStage,
    >(
        &mut self,
        lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
        can_be_held_result: Ter,
        run_preclaim_stage: RunPreclaimStage,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
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
        self.with_txq_owner_current_app_view_caller_preclaim(lock, |owner, lock| {
            owner.apply_after_preflight_with_app_view_and_owned_metrics_and_caller_preclaim_and_log_messages(
                lock,
                app,
                view,
                tx_source,
                preflight_result,
                hold_preflight,
                can_be_held_result,
                run_preclaim_stage,
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_app_view_and_caller_preclaim_and_log_sinks<
        Lock,
        TxId,
        App,
        View,
        TxSource,
        RunPreclaimStage,
        TraceFn,
        DebugFn,
        InfoFn,
    >(
        &mut self,
        lock: &mut Lock,
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
        Lock: QueueApplyLockScope,
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
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
    {
        self.with_txq_owner_current_app_view_caller_preclaim_log_sinks(lock, |owner, lock| {
            owner.apply_after_preflight_with_app_view_and_caller_preclaim_and_log_sinks(
                lock,
                app,
                view,
                tx_source,
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
    pub fn apply_after_preflight_with_app_view_and_owned_metrics_and_log_sinks<
        Lock,
        TxId,
        App,
        View,
        TxSource,
        TraceFn,
        DebugFn,
        InfoFn,
    >(
        &mut self,
        lock: &mut Lock,
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
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
    {
        self.with_txq_owner_current_app_view_log_sinks(lock, |owner, lock| {
            owner.apply_after_preflight_with_app_view_and_owned_metrics_and_log_sinks(
                lock,
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
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_app_view_and_owned_metrics_and_log_sinks_and_derived_hold_admission<
        Lock,
        TxId,
        App,
        View,
        TxSource,
        TraceFn,
        DebugFn,
        InfoFn,
    >(
        &mut self,
        lock: &mut Lock,
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
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
    {
        self.with_txq_owner_current_app_view_log_sinks(lock, |owner, lock| {
            owner.apply_after_preflight_with_app_view_and_owned_metrics_and_log_sinks_and_derived_hold_admission(
                lock,
                app,
                view,
                tx_source,
                preflight_result,
                hold_preflight,
                trace,
                debug,
                info,
            )
        })
    }

    pub fn apply_after_preflight_with_app_view_and_owned_metrics_and_derived_hold_admission_and_log_messages<
        Lock,
        TxId,
        App,
        View,
        TxSource,
    >(
        &mut self,
        lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_txq_owner_current_app_view(lock, |owner, lock| {
            owner.apply_after_preflight_with_app_view_and_owned_metrics_and_derived_hold_admission_and_log_messages(
                lock,
                app,
                view,
                tx_source,
                preflight_result,
                hold_preflight,
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_app_view_and_owned_metrics_and_log_sinks_and_derived_hold_preflight<
        Lock,
        TxId,
        App,
        View,
        TxSource,
        TraceFn,
        DebugFn,
        InfoFn,
    >(
        &mut self,
        lock: &mut Lock,
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
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
    {
        self.with_txq_owner_current_app_view_log_sinks(lock, |owner, lock| {
            owner.apply_after_preflight_with_app_view_and_owned_metrics_and_log_sinks_and_derived_hold_preflight(
                lock,
                app,
                view,
                tx_source,
                preflight_result,
                can_be_held_result,
                trace,
                debug,
                info,
            )
        })
    }

    pub fn apply_after_preflight_with_app_view_and_owned_metrics_and_derived_hold_preflight_and_log_messages<
        Lock,
        TxId,
        App,
        View,
        TxSource,
    >(
        &mut self,
        lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        can_be_held_result: Ter,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_txq_owner_current_app_view(lock, |owner, lock| {
            owner.apply_after_preflight_with_app_view_and_owned_metrics_and_derived_hold_preflight_and_log_messages(
                lock,
                app,
                view,
                tx_source,
                preflight_result,
                can_be_held_result,
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_app_view_and_owned_metrics_and_caller_preclaim_and_log_sinks<
        Lock,
        TxId,
        App,
        View,
        TxSource,
        RunPreclaimStage,
        TraceFn,
        DebugFn,
        InfoFn,
    >(
        &mut self,
        lock: &mut Lock,
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
        Lock: QueueApplyLockScope,
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
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
    {
        self.with_txq_owner_current_app_view_caller_preclaim_log_sinks(lock, |owner, lock| {
            owner.apply_after_preflight_with_app_view_and_owned_metrics_and_caller_preclaim_and_log_sinks(
                lock,
                app,
                view,
                tx_source,
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

    pub fn apply_after_preflight_with_app_view_and_derived_hold_preflight<
        Lock,
        TxId,
        App,
        View,
        TxSource,
    >(
        &mut self,
        lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        can_be_held_result: Ter,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_txq_owner_current_app_view(lock, |owner, lock| {
            owner.apply_after_preflight_with_app_view_and_derived_hold_preflight(
                lock,
                app,
                view,
                tx_source,
                preflight_result,
                can_be_held_result,
            )
        })
    }

    pub fn apply_after_preflight_with_app_view_and_derived_hold_preflight_and_log_messages<
        Lock,
        TxId,
        App,
        View,
        TxSource,
    >(
        &mut self,
        lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        can_be_held_result: Ter,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_txq_owner_current_app_view(lock, |owner, lock| {
            owner.apply_after_preflight_with_app_view_and_derived_hold_preflight_and_log_messages(
                lock,
                app,
                view,
                tx_source,
                preflight_result,
                can_be_held_result,
            )
        })
    }

    pub fn apply_after_preflight_with_app_view_and_derived_hold_admission<
        Lock,
        TxId,
        App,
        View,
        TxSource,
    >(
        &mut self,
        lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_txq_owner_current_app_view(lock, |owner, lock| {
            owner.apply_after_preflight_with_app_view_and_derived_hold_admission(
                lock,
                app,
                view,
                tx_source,
                preflight_result,
                hold_preflight,
            )
        })
    }

    pub fn apply_after_preflight_with_app_view_and_derived_hold_admission_and_log_messages<
        Lock,
        TxId,
        App,
        View,
        TxSource,
    >(
        &mut self,
        lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_txq_owner_current_app_view(lock, |owner, lock| {
            owner.apply_after_preflight_with_app_view_and_derived_hold_admission_and_log_messages(
                lock,
                app,
                view,
                tx_source,
                preflight_result,
                hold_preflight,
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_app_view_and_log_sinks<
        Lock,
        TxId,
        App,
        View,
        TxSource,
        TraceFn,
        DebugFn,
        InfoFn,
    >(
        &mut self,
        lock: &mut Lock,
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
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
    {
        self.with_txq_owner_current_app_view_log_sinks(lock, |owner, lock| {
            owner.apply_after_preflight_with_app_view_and_log_sinks(
                lock,
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
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_app_view_and_log_sinks_and_derived_hold_preflight<
        Lock,
        TxId,
        App,
        View,
        TxSource,
        TraceFn,
        DebugFn,
        InfoFn,
    >(
        &mut self,
        lock: &mut Lock,
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
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
    {
        self.with_txq_owner_current_app_view_log_sinks(lock, |owner, lock| {
            owner.apply_after_preflight_with_app_view_and_log_sinks_and_derived_hold_preflight(
                lock,
                app,
                view,
                tx_source,
                preflight_result,
                can_be_held_result,
                trace,
                debug,
                info,
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_app_view_and_log_sinks_and_derived_hold_admission<
        Lock,
        TxId,
        App,
        View,
        TxSource,
        TraceFn,
        DebugFn,
        InfoFn,
    >(
        &mut self,
        lock: &mut Lock,
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
        Lock: QueueApplyLockScope,
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
    {
        self.with_txq_owner_current_app_view_log_sinks(lock, |owner, lock| {
            owner.apply_after_preflight_with_app_view_and_log_sinks_and_derived_hold_admission(
                lock,
                app,
                view,
                tx_source,
                preflight_result,
                hold_preflight,
                trace,
                debug,
                info,
            )
        })
    }
}
