//! Lock-scope wrapper above the journal-owning `xrpld` `TxQ::apply(...)`
//! boundary.
//!
//! This layer mirrors the accept-side progression:
//! - one owner object carries the landed apply owner shell plus journal sink,
//! - callers still supply a lock token,
//! - callers still supply the live app/runtime, ledger-view, and tx inputs,
//! - the journal-owning apply shell stays explicit at the call boundary.

use std::fmt::Display;

use protocol::Ter;

use crate::{
    ApplyFlags, PreflightResult, QueueApplyAppRuntime, QueueApplyCurrentPreclaimClearRuntime,
    QueueApplyHoldPreflightTxSource, QueueApplyJournalOwner, QueueApplyLedgerViewSource,
    QueueApplyLockScope, QueueApplyObservedTxSource, QueueApplyPreclaimStage,
    QueueApplyPreflightStage, QueueApplyPreparedPreclaimInputs, QueueApplyTopWithLogMessagesResult,
    QueueHoldPreflight, TxConsequences,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueApplyJournalLockScopeOwner<Account, Tx, Journal, ParentBatchId, Sink> {
    owner: QueueApplyJournalOwner<Account, Tx, Journal, ParentBatchId, Sink>,
}

impl<Account, Tx, Journal, ParentBatchId, Sink>
    QueueApplyJournalLockScopeOwner<Account, Tx, Journal, ParentBatchId, Sink>
{
    pub const fn new(
        owner: QueueApplyJournalOwner<Account, Tx, Journal, ParentBatchId, Sink>,
    ) -> Self {
        Self { owner }
    }

    pub fn owner(&self) -> &QueueApplyJournalOwner<Account, Tx, Journal, ParentBatchId, Sink> {
        &self.owner
    }

    pub fn owner_mut(
        &mut self,
    ) -> &mut QueueApplyJournalOwner<Account, Tx, Journal, ParentBatchId, Sink> {
        &mut self.owner
    }
}

impl<Account, Tx, Journal, ParentBatchId, Sink>
    QueueApplyJournalLockScopeOwner<Account, Tx, Journal, ParentBatchId, Sink>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    Sink: crate::QueueApplyJournalSink,
{
    fn with_lock_scope_owner<Lock, R>(
        &mut self,
        _lock: &mut Lock,
        run: impl FnOnce(&mut QueueApplyJournalOwner<Account, Tx, Journal, ParentBatchId, Sink>) -> R,
    ) -> R
    where
        Lock: QueueApplyLockScope,
    {
        run(&mut self.owner)
    }

    fn with_lock_scope_owner_current_app_view<Lock, R>(
        &mut self,
        lock: &mut Lock,
        run: impl FnOnce(&mut QueueApplyJournalOwner<Account, Tx, Journal, ParentBatchId, Sink>) -> R,
    ) -> R
    where
        Lock: QueueApplyLockScope,
    {
        self.with_lock_scope_owner(lock, run)
    }

    fn with_lock_scope_owner_current_app_view_caller_preclaim<Lock, R>(
        &mut self,
        lock: &mut Lock,
        run: impl FnOnce(&mut QueueApplyJournalOwner<Account, Tx, Journal, ParentBatchId, Sink>) -> R,
    ) -> R
    where
        Lock: QueueApplyLockScope,
    {
        self.with_lock_scope_owner_current_app_view(lock, run)
    }

    fn with_lock_scope_owner_current_app_view_caller_preclaim_log_sinks<Lock, R>(
        &mut self,
        lock: &mut Lock,
        run: impl FnOnce(&mut QueueApplyJournalOwner<Account, Tx, Journal, ParentBatchId, Sink>) -> R,
    ) -> R
    where
        Lock: QueueApplyLockScope,
    {
        self.with_lock_scope_owner_current_app_view_caller_preclaim(lock, run)
    }

    fn with_lock_scope_owner_current_app_view_log_sinks<Lock, R>(
        &mut self,
        lock: &mut Lock,
        run: impl FnOnce(&mut QueueApplyJournalOwner<Account, Tx, Journal, ParentBatchId, Sink>) -> R,
    ) -> R
    where
        Lock: QueueApplyLockScope,
    {
        self.with_lock_scope_owner_current_app_view(lock, run)
    }

    pub fn apply<Lock, App, View, TxId, TxSource>(
        &mut self,
        _lock: &mut Lock,
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
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxId: Clone + Display,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_lock_scope_owner_current_app_view_log_sinks(_lock, |owner| {
            owner.apply(
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

    pub fn apply_with_log_messages<Lock, App, View, TxId, TxSource>(
        &mut self,
        _lock: &mut Lock,
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
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxId: Clone + Display,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_lock_scope_owner_current_app_view_log_sinks(_lock, |owner| {
            owner.apply_with_log_messages(
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

    pub fn apply_with_owned_metrics<Lock, App, View, TxId, TxSource>(
        &mut self,
        _lock: &mut Lock,
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
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxId: Clone + Display,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_lock_scope_owner_current_app_view_log_sinks(_lock, |owner| {
            owner.apply_with_owned_metrics(
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

    pub fn apply_after_preflight<Lock, App, View, TxId, TxSource>(
        &mut self,
        _lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
        can_be_held_result: Ter,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxId: Clone + Display,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_lock_scope_owner_current_app_view_log_sinks(_lock, |owner| {
            owner.apply_after_preflight(
                app,
                view,
                tx_source,
                preflight_result,
                hold_preflight,
                can_be_held_result,
            )
        })
    }

    pub fn apply_after_preflight_with_log_messages<Lock, App, View, TxId, TxSource>(
        &mut self,
        _lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
        can_be_held_result: Ter,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxId: Clone + Display,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_lock_scope_owner_current_app_view_log_sinks(_lock, |owner| {
            owner.apply_after_preflight_with_log_messages(
                app,
                view,
                tx_source,
                preflight_result,
                hold_preflight,
                can_be_held_result,
            )
        })
    }

    pub fn apply_after_preflight_with_owned_metrics<Lock, App, View, TxId, TxSource>(
        &mut self,
        _lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
        can_be_held_result: Ter,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxId: Clone + Display,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_lock_scope_owner_current_app_view_log_sinks(_lock, |owner| {
            owner.apply_after_preflight_with_owned_metrics(
                app,
                view,
                tx_source,
                preflight_result,
                hold_preflight,
                can_be_held_result,
            )
        })
    }

    pub fn apply_with_derived_hold_preflight<Lock, App, View, TxId, TxSource>(
        &mut self,
        _lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        flags: ApplyFlags,
        consequences: TxConsequences,
        can_be_held_result: Ter,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxId: Clone + Display,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_lock_scope_owner_current_app_view_log_sinks(_lock, |owner| {
            owner.apply_with_derived_hold_preflight(
                app,
                view,
                tx_source,
                flags,
                consequences,
                can_be_held_result,
            )
        })
    }

    pub fn apply_with_owned_metrics_and_derived_hold_preflight<Lock, App, View, TxId, TxSource>(
        &mut self,
        _lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        flags: ApplyFlags,
        consequences: TxConsequences,
        can_be_held_result: Ter,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxId: Clone + Display,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_lock_scope_owner_current_app_view_log_sinks(_lock, |owner| {
            owner.apply_with_owned_metrics_and_derived_hold_preflight(
                app,
                view,
                tx_source,
                flags,
                consequences,
                can_be_held_result,
            )
        })
    }

    pub fn apply_with_derived_preflight_facts<Lock, App, View, TxId, TxSource>(
        &mut self,
        _lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        can_be_held_result: Ter,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxId: Clone + Display,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_lock_scope_owner_current_app_view_log_sinks(_lock, |owner| {
            owner.apply_with_derived_preflight_facts(app, view, tx_source, can_be_held_result)
        })
    }

    pub fn apply_with_owned_metrics_and_derived_preflight_facts<Lock, App, View, TxId, TxSource>(
        &mut self,
        _lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxId: Clone + Display,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_lock_scope_owner_current_app_view_log_sinks(_lock, |owner| {
            owner.apply_with_owned_metrics_and_derived_preflight_facts(app, view, tx_source)
        })
    }

    pub fn apply_with_derived_preflight_facts_and_hold_admission<Lock, App, View, TxId, TxSource>(
        &mut self,
        _lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxId: Clone + Display,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_lock_scope_owner_current_app_view_log_sinks(_lock, |owner| {
            owner.apply_with_derived_preflight_facts_and_hold_admission(app, view, tx_source)
        })
    }

    pub fn apply_with_owned_metrics_and_derived_preflight_facts_and_hold_admission<
        Lock,
        App,
        View,
        TxId,
        TxSource,
    >(
        &mut self,
        _lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxId: Clone + Display,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_lock_scope_owner_current_app_view_log_sinks(_lock, |owner| {
            owner.apply_with_owned_metrics_and_derived_preflight_facts_and_hold_admission(
                app, view, tx_source,
            )
        })
    }

    pub fn apply_after_preflight_with_derived_hold_preflight<Lock, App, View, TxId, TxSource>(
        &mut self,
        _lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        can_be_held_result: Ter,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxId: Clone + Display,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_lock_scope_owner_current_app_view_log_sinks(_lock, |owner| {
            owner.apply_after_preflight_with_derived_hold_preflight(
                app,
                view,
                tx_source,
                preflight_result,
                can_be_held_result,
            )
        })
    }

    pub fn apply_after_preflight_with_owned_metrics_and_derived_hold_preflight<
        Lock,
        App,
        View,
        TxId,
        TxSource,
    >(
        &mut self,
        _lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        can_be_held_result: Ter,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxId: Clone + Display,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_lock_scope_owner_current_app_view_log_sinks(_lock, |owner| {
            owner.apply_after_preflight_with_owned_metrics_and_derived_hold_preflight(
                app,
                view,
                tx_source,
                preflight_result,
                can_be_held_result,
            )
        })
    }

    pub fn apply_after_preflight_with_derived_hold_admission<Lock, App, View, TxId, TxSource>(
        &mut self,
        _lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxId: Clone + Display,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_lock_scope_owner_current_app_view_log_sinks(_lock, |owner| {
            owner.apply_after_preflight_with_derived_hold_admission(
                app,
                view,
                tx_source,
                preflight_result,
                hold_preflight,
            )
        })
    }

    pub fn apply_after_preflight_with_owned_metrics_and_derived_hold_admission<
        Lock,
        App,
        View,
        TxId,
        TxSource,
    >(
        &mut self,
        _lock: &mut Lock,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Lock: QueueApplyLockScope,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxId: Clone + Display,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_lock_scope_owner_current_app_view_log_sinks(_lock, |owner| {
            owner.apply_after_preflight_with_owned_metrics_and_derived_hold_admission(
                app,
                view,
                tx_source,
                preflight_result,
                hold_preflight,
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_caller_preclaim<Lock, App, View, TxId, TxSource, RunPreclaimStage>(
        &mut self,
        _lock: &mut Lock,
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
        self.with_lock_scope_owner_current_app_view_caller_preclaim_log_sinks(_lock, |owner| {
            owner.apply_with_caller_preclaim(
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
    pub fn apply_with_owned_metrics_and_caller_preclaim<
        Lock,
        App,
        View,
        TxId,
        TxSource,
        RunPreclaimStage,
    >(
        &mut self,
        _lock: &mut Lock,
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
        self.with_lock_scope_owner_current_app_view_caller_preclaim_log_sinks(_lock, |owner| {
            owner.apply_with_owned_metrics_and_caller_preclaim(
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
    pub fn apply_after_preflight_with_caller_preclaim<
        Lock,
        App,
        View,
        TxId,
        TxSource,
        RunPreclaimStage,
    >(
        &mut self,
        _lock: &mut Lock,
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
        self.with_lock_scope_owner_current_app_view_caller_preclaim_log_sinks(_lock, |owner| {
            owner.apply_after_preflight_with_caller_preclaim(
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
    pub fn apply_after_preflight_with_owned_metrics_and_caller_preclaim<
        Lock,
        App,
        View,
        TxId,
        TxSource,
        RunPreclaimStage,
    >(
        &mut self,
        _lock: &mut Lock,
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
        self.with_lock_scope_owner_current_app_view_caller_preclaim_log_sinks(_lock, |owner| {
            owner.apply_after_preflight_with_owned_metrics_and_caller_preclaim(
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
}
