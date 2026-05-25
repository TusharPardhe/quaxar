//! Method-style owner shell for the current `xrpld` `TxQ::apply(...)`
//! boundary.
//!
//! This layer wraps the landed apply live-owner state in a single owner facade
//! so higher Rust callers can move closer to the the reference implementation member shape:
//! - one owner object carries the TxQ-owned apply state,
//! - callers still supply the live tx and ledger-view facts,
//! - callers still own real preflight, direct apply, preclaim, try-clear, and
//!   sandbox execution,
//! - callers still own real application, journal, and lock scope.

use std::fmt::Display;

use protocol::Ter;

use crate::{
    ApplyFlags, PreclaimResult, PreflightResult, QueueApplyAppRuntime, QueueApplyAppViewSnapshot,
    QueueApplyCurrentPreclaimClearRuntime, QueueApplyHoldPreflightTxSource, QueueApplyJournalOwner,
    QueueApplyLedgerViewSource, QueueApplyLiveOwner, QueueApplyObservedTxSource,
    QueueApplyObservedViewSource, QueueApplyPreclaimStage, QueueApplyPreclaimViewSource,
    QueueApplyPreflightStage, QueueApplyPreparedPreclaimInputs, QueueApplyTopWithLogMessagesResult,
    QueueApplyTryClearResult, QueueApplyViewAdjustment, QueueFeeMetricsSnapshot,
    QueueHoldPreflight, TxConsequences, run_queue_apply_after_preflight_with_app_view,
    run_queue_apply_after_preflight_with_app_view_and_caller_preclaim,
    run_queue_apply_after_preflight_with_app_view_and_caller_preclaim_and_log_messages,
    run_queue_apply_after_preflight_with_app_view_and_caller_preclaim_and_log_sinks,
    run_queue_apply_after_preflight_with_app_view_and_derived_hold_admission,
    run_queue_apply_after_preflight_with_app_view_and_derived_hold_admission_and_log_messages,
    run_queue_apply_after_preflight_with_app_view_and_derived_hold_preflight,
    run_queue_apply_after_preflight_with_app_view_and_derived_hold_preflight_and_log_messages,
    run_queue_apply_after_preflight_with_app_view_and_log_sinks,
    run_queue_apply_after_preflight_with_app_view_and_log_sinks_and_derived_hold_admission,
    run_queue_apply_after_preflight_with_app_view_and_log_sinks_and_derived_hold_preflight,
    run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot,
    run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot_and_caller_preclaim,
    run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot_and_caller_preclaim_and_log_messages,
    run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot_and_caller_preclaim_and_log_sinks,
    run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot_and_derived_hold_admission_and_log_messages,
    run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot_and_derived_hold_preflight_and_log_messages,
    run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot_and_log_messages,
    run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot_and_log_sinks,
    run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot_and_log_sinks_and_derived_hold_admission,
    run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot_and_log_sinks_and_derived_hold_preflight,
    run_queue_apply_after_preflight_with_live_owner_from_sources,
    run_queue_apply_after_preflight_with_live_owner_from_sources_and_caller_preclaim,
    run_queue_apply_after_preflight_with_live_owner_from_sources_and_caller_preclaim_and_log_sinks,
    run_queue_apply_after_preflight_with_live_owner_from_sources_and_log_messages,
    run_queue_apply_after_preflight_with_live_owner_from_sources_and_log_sinks,
    run_queue_apply_with_app_view, run_queue_apply_with_app_view_and_caller_preclaim,
    run_queue_apply_with_app_view_and_caller_preclaim_and_log_messages,
    run_queue_apply_with_app_view_and_caller_preclaim_and_log_sinks,
    run_queue_apply_with_app_view_and_derived_hold_admission,
    run_queue_apply_with_app_view_and_derived_hold_admission_and_log_messages,
    run_queue_apply_with_app_view_and_derived_hold_preflight,
    run_queue_apply_with_app_view_and_derived_hold_preflight_and_log_messages,
    run_queue_apply_with_app_view_and_derived_preflight_facts,
    run_queue_apply_with_app_view_and_derived_preflight_facts_and_hold_admission,
    run_queue_apply_with_app_view_and_derived_preflight_facts_and_hold_admission_and_log_messages,
    run_queue_apply_with_app_view_and_derived_preflight_facts_and_log_messages,
    run_queue_apply_with_app_view_and_log_sinks,
    run_queue_apply_with_app_view_and_log_sinks_and_derived_hold_admission,
    run_queue_apply_with_app_view_and_log_sinks_and_derived_hold_preflight,
    run_queue_apply_with_app_view_and_log_sinks_and_derived_preflight_facts,
    run_queue_apply_with_app_view_and_log_sinks_and_derived_preflight_facts_and_hold_admission,
    run_queue_apply_with_app_view_and_metrics_snapshot,
    run_queue_apply_with_app_view_and_metrics_snapshot_and_caller_preclaim,
    run_queue_apply_with_app_view_and_metrics_snapshot_and_caller_preclaim_and_log_messages,
    run_queue_apply_with_app_view_and_metrics_snapshot_and_caller_preclaim_and_log_sinks,
    run_queue_apply_with_app_view_and_metrics_snapshot_and_derived_hold_admission_and_log_messages,
    run_queue_apply_with_app_view_and_metrics_snapshot_and_derived_hold_preflight_and_log_messages,
    run_queue_apply_with_app_view_and_metrics_snapshot_and_derived_preflight_facts_and_hold_admission_and_log_messages,
    run_queue_apply_with_app_view_and_metrics_snapshot_and_derived_preflight_facts_and_log_messages,
    run_queue_apply_with_app_view_and_metrics_snapshot_and_log_messages,
    run_queue_apply_with_app_view_and_metrics_snapshot_and_log_sinks,
    run_queue_apply_with_app_view_and_metrics_snapshot_and_log_sinks_and_derived_hold_admission,
    run_queue_apply_with_app_view_and_metrics_snapshot_and_log_sinks_and_derived_hold_preflight,
    run_queue_apply_with_app_view_and_metrics_snapshot_and_log_sinks_and_derived_preflight_facts,
    run_queue_apply_with_app_view_and_metrics_snapshot_and_log_sinks_and_derived_preflight_facts_and_hold_admission,
    run_queue_apply_with_live_owner_from_sources,
    run_queue_apply_with_live_owner_from_sources_and_caller_preclaim,
    run_queue_apply_with_live_owner_from_sources_and_caller_preclaim_and_log_sinks,
    run_queue_apply_with_live_owner_from_sources_and_log_messages,
    run_queue_apply_with_live_owner_from_sources_and_log_sinks,
    snapshot_queue_apply_app_view_with_metrics,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId> {
    owner: QueueApplyLiveOwner<Account, Tx, Journal, ParentBatchId>,
}

impl<Account, Tx, Journal, ParentBatchId>
    QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>
{
    pub const fn new(owner: QueueApplyLiveOwner<Account, Tx, Journal, ParentBatchId>) -> Self {
        Self { owner }
    }

    pub fn owner(&self) -> &QueueApplyLiveOwner<Account, Tx, Journal, ParentBatchId> {
        &self.owner
    }

    pub fn owner_mut(&mut self) -> &mut QueueApplyLiveOwner<Account, Tx, Journal, ParentBatchId> {
        &mut self.owner
    }

    pub fn into_journal_owner<Sink>(
        self,
        journal: Sink,
    ) -> QueueApplyJournalOwner<Account, Tx, Journal, ParentBatchId, Sink> {
        QueueApplyJournalOwner::new(self, journal)
    }

    fn snapshot_source_view_with_owned_metrics<TxId, TxSource, ViewSource>(
        &self,
        tx_source: &TxSource,
        view_source: &ViewSource,
    ) -> QueueApplyAppViewSnapshot
    where
        TxId: Clone + Display,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: QueueApplyObservedViewSource<Account>,
    {
        snapshot_queue_apply_app_view_with_metrics(
            tx_source,
            view_source,
            self.owner.metrics().snapshot(),
        )
    }

    fn with_owned_metrics_snapshot<R>(
        &mut self,
        run: impl FnOnce(&mut Self, QueueFeeMetricsSnapshot) -> R,
    ) -> R {
        let metrics_snapshot = self.owner.metrics().snapshot();
        run(self, metrics_snapshot)
    }

    fn with_owned_metrics_current_app_view_log_sinks<R>(
        &mut self,
        run: impl FnOnce(&mut Self, QueueFeeMetricsSnapshot) -> R,
    ) -> R {
        self.with_owned_metrics_snapshot(run)
    }

    fn with_owned_metrics_current_app_view_caller_preclaim<R>(
        &mut self,
        run: impl FnOnce(&mut Self, QueueFeeMetricsSnapshot) -> R,
    ) -> R {
        self.with_owned_metrics_snapshot(run)
    }

    fn with_owned_metrics_current_app_view_caller_preclaim_log_sinks<R>(
        &mut self,
        run: impl FnOnce(&mut Self, QueueFeeMetricsSnapshot) -> R,
    ) -> R {
        self.with_owned_metrics_current_app_view_caller_preclaim(run)
    }

    fn with_owned_metrics_current_app_view<R>(
        &mut self,
        run: impl FnOnce(&mut Self, QueueFeeMetricsSnapshot) -> R,
    ) -> R {
        self.with_owned_metrics_snapshot(run)
    }

    fn with_current_app_view<R>(&mut self, run: impl FnOnce(&mut Self) -> R) -> R {
        run(self)
    }

    fn with_current_app_view_caller_preclaim_log_sinks<R>(
        &mut self,
        run: impl FnOnce(&mut Self) -> R,
    ) -> R {
        self.with_current_app_view(run)
    }

    fn with_current_app_view_caller_preclaim<R>(&mut self, run: impl FnOnce(&mut Self) -> R) -> R {
        self.with_current_app_view(run)
    }

    fn with_owned_metrics_source_view_snapshot<TxId, TxSource, ViewSource, R>(
        &mut self,
        tx_source: &TxSource,
        view_source: &ViewSource,
        run: impl FnOnce(
            &mut QueueApplyLiveOwner<Account, Tx, Journal, ParentBatchId>,
            &TxSource,
            &QueueApplyAppViewSnapshot,
        ) -> R,
    ) -> R
    where
        TxId: Clone + Display,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: QueueApplyObservedViewSource<Account>,
    {
        let view_snapshot =
            self.snapshot_source_view_with_owned_metrics::<TxId, _, _>(tx_source, view_source);
        run(&mut self.owner, tx_source, &view_snapshot)
    }

    fn with_owned_metrics_source_view_caller_preclaim<TxId, TxSource, ViewSource, R>(
        &mut self,
        tx_source: &TxSource,
        view_source: &ViewSource,
        run: impl FnOnce(
            &mut QueueApplyLiveOwner<Account, Tx, Journal, ParentBatchId>,
            &TxSource,
            &QueueApplyAppViewSnapshot,
        ) -> R,
    ) -> R
    where
        TxId: Clone + Display,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: QueueApplyObservedViewSource<Account>,
    {
        self.with_owned_metrics_source_view_snapshot::<TxId, _, _, _>(tx_source, view_source, run)
    }

    fn with_owned_metrics_source_view_caller_preclaim_log_sinks<TxId, TxSource, ViewSource, R>(
        &mut self,
        tx_source: &TxSource,
        view_source: &ViewSource,
        run: impl FnOnce(
            &mut QueueApplyLiveOwner<Account, Tx, Journal, ParentBatchId>,
            &TxSource,
            &QueueApplyAppViewSnapshot,
        ) -> R,
    ) -> R
    where
        TxId: Clone + Display,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: QueueApplyObservedViewSource<Account>,
    {
        self.with_owned_metrics_source_view_caller_preclaim::<TxId, _, _, _>(
            tx_source,
            view_source,
            run,
        )
    }

    fn with_owned_metrics_source_view_log_sinks<TxId, TxSource, ViewSource, R>(
        &mut self,
        tx_source: &TxSource,
        view_source: &ViewSource,
        run: impl FnOnce(
            &mut QueueApplyLiveOwner<Account, Tx, Journal, ParentBatchId>,
            &TxSource,
            &QueueApplyAppViewSnapshot,
        ) -> R,
    ) -> R
    where
        TxId: Clone + Display,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: QueueApplyObservedViewSource<Account>,
    {
        self.with_owned_metrics_source_view_snapshot::<TxId, _, _, _>(tx_source, view_source, run)
    }

    fn with_source_view<TxId, TxSource, ViewSource, R>(
        &mut self,
        tx_source: &TxSource,
        view_source: &ViewSource,
        run: impl FnOnce(
            &mut QueueApplyLiveOwner<Account, Tx, Journal, ParentBatchId>,
            &TxSource,
            &ViewSource,
        ) -> R,
    ) -> R
    where
        TxId: Clone + Display,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: QueueApplyObservedViewSource<Account>,
    {
        run(&mut self.owner, tx_source, view_source)
    }

    fn with_source_view_caller_preclaim<TxId, TxSource, ViewSource, R>(
        &mut self,
        tx_source: &TxSource,
        view_source: &ViewSource,
        run: impl FnOnce(
            &mut QueueApplyLiveOwner<Account, Tx, Journal, ParentBatchId>,
            &TxSource,
            &ViewSource,
        ) -> R,
    ) -> R
    where
        TxId: Clone + Display,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: QueueApplyObservedViewSource<Account>,
    {
        run(&mut self.owner, tx_source, view_source)
    }

    fn with_source_view_caller_preclaim_log_sinks<TxId, TxSource, ViewSource, R>(
        &mut self,
        tx_source: &TxSource,
        view_source: &ViewSource,
        run: impl FnOnce(
            &mut QueueApplyLiveOwner<Account, Tx, Journal, ParentBatchId>,
            &TxSource,
            &ViewSource,
        ) -> R,
    ) -> R
    where
        TxId: Clone + Display,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: QueueApplyObservedViewSource<Account>,
    {
        self.with_source_view_caller_preclaim::<TxId, _, _, _>(tx_source, view_source, run)
    }
}

impl<Account, Tx, Journal, ParentBatchId> QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
{
    #[allow(clippy::too_many_arguments)]
    pub fn apply<
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
        self.with_source_view::<TxId, _, _, _>(
            tx_source,
            view_source,
            |owner, tx_source, view_source| {
                run_queue_apply_with_live_owner_from_sources(
                    owner,
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
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight<
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
        self.with_source_view::<TxId, _, _, _>(
            tx_source,
            view_source,
            |owner, tx_source, view_source| {
                run_queue_apply_after_preflight_with_live_owner_from_sources(
                    owner,
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
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_log_messages<
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
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
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
        self.with_source_view::<TxId, _, _, _>(
            tx_source,
            view_source,
            |owner, tx_source, view_source| {
                run_queue_apply_with_live_owner_from_sources_and_log_messages(
                    owner,
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
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_log_messages<
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
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
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
        self.with_source_view::<TxId, _, _, _>(
            tx_source,
            view_source,
            |owner, tx_source, view_source| {
                run_queue_apply_after_preflight_with_live_owner_from_sources_and_log_messages(
                    owner,
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
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_caller_preclaim<
        TxId,
        TxSource,
        ViewSource,
        RunPreflight,
        TraceFn,
        ApplyFn,
        PrepareMultiTxn,
        RunPreclaimStage,
        RunTryClear,
        TryClearResult,
        ApplySandbox,
    >(
        &mut self,
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
        run_preclaim_stage: RunPreclaimStage,
        run_try_clear: RunTryClear,
        apply_sandbox: ApplySandbox,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        TxId: Clone + Display,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: QueueApplyObservedViewSource<Account>,
        RunPreflight: FnOnce() -> PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        TraceFn: FnMut(&str),
        ApplyFn: FnOnce() -> crate::ApplyResult,
        PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
        RunPreclaimStage: FnOnce(
            QueueApplyPreparedPreclaimInputs<Account>,
        ) -> Result<
            QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>,
            crate::ApplyResult,
        >,
        RunTryClear: FnOnce() -> TryClearResult,
        TryClearResult: QueueApplyTryClearResult,
        ApplySandbox: FnOnce(),
    {
        self.with_source_view_caller_preclaim::<TxId, _, _, _>(
            tx_source,
            view_source,
            |owner, tx_source, view_source| {
                run_queue_apply_with_live_owner_from_sources_and_caller_preclaim(
                    owner,
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
                    run_preclaim_stage,
                    run_try_clear,
                    apply_sandbox,
                )
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_caller_preclaim<
        TxId,
        TxSource,
        ViewSource,
        TraceFn,
        ApplyFn,
        PrepareMultiTxn,
        RunPreclaimStage,
        RunTryClear,
        TryClearResult,
        ApplySandbox,
    >(
        &mut self,
        tx_source: &TxSource,
        view_source: &ViewSource,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
        can_be_held_result: Ter,
        trace: TraceFn,
        apply: ApplyFn,
        prepare_multitxn: PrepareMultiTxn,
        run_preclaim_stage: RunPreclaimStage,
        run_try_clear: RunTryClear,
        apply_sandbox: ApplySandbox,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        TxId: Clone + Display,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: QueueApplyObservedViewSource<Account>,
        TraceFn: FnMut(&str),
        ApplyFn: FnOnce() -> crate::ApplyResult,
        PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
        RunPreclaimStage: FnOnce(
            QueueApplyPreparedPreclaimInputs<Account>,
        ) -> Result<
            QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>,
            crate::ApplyResult,
        >,
        RunTryClear: FnOnce() -> TryClearResult,
        TryClearResult: QueueApplyTryClearResult,
        ApplySandbox: FnOnce(),
    {
        self.with_source_view_caller_preclaim::<TxId, _, _, _>(
            tx_source,
            view_source,
            |owner, tx_source, view_source| {
                run_queue_apply_after_preflight_with_live_owner_from_sources_and_caller_preclaim(
                    owner,
                    tx_source,
                    view_source,
                    preflight_result,
                    hold_preflight,
                    can_be_held_result,
                    trace,
                    apply,
                    prepare_multitxn,
                    run_preclaim_stage,
                    run_try_clear,
                    apply_sandbox,
                )
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_caller_preclaim_and_log_sinks<
        TxId,
        TxSource,
        ViewSource,
        RunPreflight,
        TraceFn,
        DebugFn,
        InfoFn,
        ApplyFn,
        PrepareMultiTxn,
        RunPreclaimStage,
        RunTryClear,
        TryClearResult,
        ApplySandbox,
    >(
        &mut self,
        tx_source: &TxSource,
        view_source: &ViewSource,
        hold_preflight: QueueHoldPreflight,
        flags: ApplyFlags,
        consequences: TxConsequences,
        can_be_held_result: Ter,
        run_preflight: RunPreflight,
        trace: TraceFn,
        debug: DebugFn,
        info: InfoFn,
        apply: ApplyFn,
        prepare_multitxn: PrepareMultiTxn,
        run_preclaim_stage: RunPreclaimStage,
        run_try_clear: RunTryClear,
        apply_sandbox: ApplySandbox,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        TxId: Clone + Display,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: QueueApplyObservedViewSource<Account>,
        RunPreflight: FnOnce() -> PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
        ApplyFn: FnOnce() -> crate::ApplyResult,
        PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
        RunPreclaimStage: FnOnce(
            QueueApplyPreparedPreclaimInputs<Account>,
        ) -> Result<
            QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>,
            crate::ApplyResult,
        >,
        RunTryClear: FnOnce() -> TryClearResult,
        TryClearResult: QueueApplyTryClearResult,
        ApplySandbox: FnOnce(),
    {
        self.with_source_view_caller_preclaim_log_sinks::<TxId, _, _, _>(
            tx_source,
            view_source,
            |owner, tx_source, view_source| {
                run_queue_apply_with_live_owner_from_sources_and_caller_preclaim_and_log_sinks(
                    owner,
                    tx_source,
                    view_source,
                    hold_preflight,
                    flags,
                    consequences,
                    can_be_held_result,
                    run_preflight,
                    trace,
                    debug,
                    info,
                    apply,
                    prepare_multitxn,
                    run_preclaim_stage,
                    run_try_clear,
                    apply_sandbox,
                )
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_log_sinks<
        TxId,
        TxSource,
        ViewSource,
        RunPreflight,
        TraceFn,
        DebugFn,
        InfoFn,
        ApplyFn,
        PrepareMultiTxn,
        RunPreclaim,
        RunTryClear,
        TryClearResult,
        ApplySandbox,
    >(
        &mut self,
        tx_source: &TxSource,
        view_source: &ViewSource,
        hold_preflight: QueueHoldPreflight,
        flags: ApplyFlags,
        consequences: TxConsequences,
        can_be_held_result: Ter,
        run_preflight: RunPreflight,
        trace: TraceFn,
        debug: DebugFn,
        info: InfoFn,
        apply: ApplyFn,
        prepare_multitxn: PrepareMultiTxn,
        run_preclaim: RunPreclaim,
        run_try_clear: RunTryClear,
        apply_sandbox: ApplySandbox,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        TxId: Clone + Display,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: QueueApplyObservedViewSource<Account>,
        RunPreflight: FnOnce() -> PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
        ApplyFn: FnOnce() -> crate::ApplyResult,
        PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
        RunPreclaim:
            FnOnce(QueueApplyPreclaimViewSource) -> PreclaimResult<Tx, Journal, ParentBatchId>,
        RunTryClear: FnOnce() -> TryClearResult,
        TryClearResult: QueueApplyTryClearResult,
        ApplySandbox: FnOnce(),
    {
        self.with_source_view::<TxId, _, _, _>(
            tx_source,
            view_source,
            |owner, tx_source, view_source| {
                run_queue_apply_with_live_owner_from_sources_and_log_sinks(
                    owner,
                    tx_source,
                    view_source,
                    hold_preflight,
                    flags,
                    consequences,
                    can_be_held_result,
                    run_preflight,
                    trace,
                    debug,
                    info,
                    apply,
                    prepare_multitxn,
                    run_preclaim,
                    run_try_clear,
                    apply_sandbox,
                )
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_log_sinks<
        TxId,
        TxSource,
        ViewSource,
        TraceFn,
        DebugFn,
        InfoFn,
        ApplyFn,
        PrepareMultiTxn,
        RunPreclaim,
        RunTryClear,
        TryClearResult,
        ApplySandbox,
    >(
        &mut self,
        tx_source: &TxSource,
        view_source: &ViewSource,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
        can_be_held_result: Ter,
        trace: TraceFn,
        debug: DebugFn,
        info: InfoFn,
        apply: ApplyFn,
        prepare_multitxn: PrepareMultiTxn,
        run_preclaim: RunPreclaim,
        run_try_clear: RunTryClear,
        apply_sandbox: ApplySandbox,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        TxId: Clone + Display,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: QueueApplyObservedViewSource<Account>,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
        ApplyFn: FnOnce() -> crate::ApplyResult,
        PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
        RunPreclaim:
            FnOnce(QueueApplyPreclaimViewSource) -> PreclaimResult<Tx, Journal, ParentBatchId>,
        RunTryClear: FnOnce() -> TryClearResult,
        TryClearResult: QueueApplyTryClearResult,
        ApplySandbox: FnOnce(),
    {
        self.with_source_view::<TxId, _, _, _>(
            tx_source,
            view_source,
            |owner, tx_source, view_source| {
                run_queue_apply_after_preflight_with_live_owner_from_sources_and_log_sinks(
                    owner,
                    tx_source,
                    view_source,
                    preflight_result,
                    hold_preflight,
                    can_be_held_result,
                    trace,
                    debug,
                    info,
                    apply,
                    prepare_multitxn,
                    run_preclaim,
                    run_try_clear,
                    apply_sandbox,
                )
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_caller_preclaim_and_log_sinks<
        TxId,
        TxSource,
        ViewSource,
        TraceFn,
        DebugFn,
        InfoFn,
        ApplyFn,
        PrepareMultiTxn,
        RunPreclaimStage,
        RunTryClear,
        TryClearResult,
        ApplySandbox,
    >(
        &mut self,
        tx_source: &TxSource,
        view_source: &ViewSource,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
        can_be_held_result: Ter,
        trace: TraceFn,
        debug: DebugFn,
        info: InfoFn,
        apply: ApplyFn,
        prepare_multitxn: PrepareMultiTxn,
        run_preclaim_stage: RunPreclaimStage,
        run_try_clear: RunTryClear,
        apply_sandbox: ApplySandbox,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        TxId: Clone + Display,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: QueueApplyObservedViewSource<Account>,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
        ApplyFn: FnOnce() -> crate::ApplyResult,
        PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
        RunPreclaimStage: FnOnce(
            QueueApplyPreparedPreclaimInputs<Account>,
        ) -> Result<
            QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>,
            crate::ApplyResult,
        >,
        RunTryClear: FnOnce() -> TryClearResult,
        TryClearResult: QueueApplyTryClearResult,
        ApplySandbox: FnOnce(),
    {
        self.with_source_view_caller_preclaim_log_sinks::<TxId, _, _, _>(
            tx_source,
            view_source,
            |owner, tx_source, view_source| {
                run_queue_apply_after_preflight_with_live_owner_from_sources_and_caller_preclaim_and_log_sinks(
                    owner,
                    tx_source,
                    view_source,
                    preflight_result,
                    hold_preflight,
                    can_be_held_result,
                    trace,
                    debug,
                    info,
                    apply,
                    prepare_multitxn,
                    run_preclaim_stage,
                    run_try_clear,
                    apply_sandbox,
                )
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_owned_metrics<
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
        self.with_owned_metrics_source_view_log_sinks::<TxId, _, _, _>(
            tx_source,
            view_source,
            |owner, tx_source, view_snapshot| {
                run_queue_apply_with_live_owner_from_sources(
                    owner,
                    tx_source,
                    view_snapshot,
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
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_owned_metrics<
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
        self.with_owned_metrics_source_view_log_sinks::<TxId, _, _, _>(
            tx_source,
            view_source,
            |owner, tx_source, view_snapshot| {
                run_queue_apply_after_preflight_with_live_owner_from_sources(
                    owner,
                    tx_source,
                    view_snapshot,
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
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_owned_metrics_and_caller_preclaim<
        TxId,
        TxSource,
        ViewSource,
        RunPreflight,
        TraceFn,
        ApplyFn,
        PrepareMultiTxn,
        RunPreclaimStage,
        RunTryClear,
        TryClearResult,
        ApplySandbox,
    >(
        &mut self,
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
        run_preclaim_stage: RunPreclaimStage,
        run_try_clear: RunTryClear,
        apply_sandbox: ApplySandbox,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        TxId: Clone + Display,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: QueueApplyObservedViewSource<Account>,
        RunPreflight: FnOnce() -> PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        TraceFn: FnMut(&str),
        ApplyFn: FnOnce() -> crate::ApplyResult,
        PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
        RunPreclaimStage: FnOnce(
            QueueApplyPreparedPreclaimInputs<Account>,
        ) -> Result<
            QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>,
            crate::ApplyResult,
        >,
        RunTryClear: FnOnce() -> TryClearResult,
        TryClearResult: QueueApplyTryClearResult,
        ApplySandbox: FnOnce(),
    {
        self.with_owned_metrics_source_view_caller_preclaim::<TxId, _, _, _>(
            tx_source,
            view_source,
            |owner, tx_source, view_snapshot| {
                run_queue_apply_with_live_owner_from_sources_and_caller_preclaim(
                    owner,
                    tx_source,
                    view_snapshot,
                    hold_preflight,
                    flags,
                    consequences,
                    can_be_held_result,
                    run_preflight,
                    trace,
                    apply,
                    prepare_multitxn,
                    run_preclaim_stage,
                    run_try_clear,
                    apply_sandbox,
                )
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_owned_metrics_and_caller_preclaim<
        TxId,
        TxSource,
        ViewSource,
        TraceFn,
        ApplyFn,
        PrepareMultiTxn,
        RunPreclaimStage,
        RunTryClear,
        TryClearResult,
        ApplySandbox,
    >(
        &mut self,
        tx_source: &TxSource,
        view_source: &ViewSource,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
        can_be_held_result: Ter,
        trace: TraceFn,
        apply: ApplyFn,
        prepare_multitxn: PrepareMultiTxn,
        run_preclaim_stage: RunPreclaimStage,
        run_try_clear: RunTryClear,
        apply_sandbox: ApplySandbox,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        TxId: Clone + Display,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: QueueApplyObservedViewSource<Account>,
        TraceFn: FnMut(&str),
        ApplyFn: FnOnce() -> crate::ApplyResult,
        PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
        RunPreclaimStage: FnOnce(
            QueueApplyPreparedPreclaimInputs<Account>,
        ) -> Result<
            QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>,
            crate::ApplyResult,
        >,
        RunTryClear: FnOnce() -> TryClearResult,
        TryClearResult: QueueApplyTryClearResult,
        ApplySandbox: FnOnce(),
    {
        self.with_owned_metrics_source_view_caller_preclaim::<TxId, _, _, _>(
            tx_source,
            view_source,
            |owner, tx_source, view_snapshot| {
                run_queue_apply_after_preflight_with_live_owner_from_sources_and_caller_preclaim(
                    owner,
                    tx_source,
                    view_snapshot,
                    preflight_result,
                    hold_preflight,
                    can_be_held_result,
                    trace,
                    apply,
                    prepare_multitxn,
                    run_preclaim_stage,
                    run_try_clear,
                    apply_sandbox,
                )
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_owned_metrics_and_log_sinks<
        TxId,
        TxSource,
        ViewSource,
        RunPreflight,
        TraceFn,
        DebugFn,
        InfoFn,
        ApplyFn,
        PrepareMultiTxn,
        RunPreclaim,
        RunTryClear,
        TryClearResult,
        ApplySandbox,
    >(
        &mut self,
        tx_source: &TxSource,
        view_source: &ViewSource,
        hold_preflight: QueueHoldPreflight,
        flags: ApplyFlags,
        consequences: TxConsequences,
        can_be_held_result: Ter,
        run_preflight: RunPreflight,
        trace: TraceFn,
        debug: DebugFn,
        info: InfoFn,
        apply: ApplyFn,
        prepare_multitxn: PrepareMultiTxn,
        run_preclaim: RunPreclaim,
        run_try_clear: RunTryClear,
        apply_sandbox: ApplySandbox,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        TxId: Clone + Display,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: QueueApplyObservedViewSource<Account>,
        RunPreflight: FnOnce() -> PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
        ApplyFn: FnOnce() -> crate::ApplyResult,
        PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
        RunPreclaim:
            FnOnce(QueueApplyPreclaimViewSource) -> PreclaimResult<Tx, Journal, ParentBatchId>,
        RunTryClear: FnOnce() -> TryClearResult,
        TryClearResult: QueueApplyTryClearResult,
        ApplySandbox: FnOnce(),
    {
        self.with_owned_metrics_source_view_log_sinks::<TxId, _, _, _>(
            tx_source,
            view_source,
            |owner, tx_source, view_snapshot| {
                run_queue_apply_with_live_owner_from_sources_and_log_sinks(
                    owner,
                    tx_source,
                    view_snapshot,
                    hold_preflight,
                    flags,
                    consequences,
                    can_be_held_result,
                    run_preflight,
                    trace,
                    debug,
                    info,
                    apply,
                    prepare_multitxn,
                    run_preclaim,
                    run_try_clear,
                    apply_sandbox,
                )
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_owned_metrics_and_log_sinks<
        TxId,
        TxSource,
        ViewSource,
        TraceFn,
        DebugFn,
        InfoFn,
        ApplyFn,
        PrepareMultiTxn,
        RunPreclaim,
        RunTryClear,
        TryClearResult,
        ApplySandbox,
    >(
        &mut self,
        tx_source: &TxSource,
        view_source: &ViewSource,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
        can_be_held_result: Ter,
        trace: TraceFn,
        debug: DebugFn,
        info: InfoFn,
        apply: ApplyFn,
        prepare_multitxn: PrepareMultiTxn,
        run_preclaim: RunPreclaim,
        run_try_clear: RunTryClear,
        apply_sandbox: ApplySandbox,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        TxId: Clone + Display,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: QueueApplyObservedViewSource<Account>,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
        ApplyFn: FnOnce() -> crate::ApplyResult,
        PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
        RunPreclaim:
            FnOnce(QueueApplyPreclaimViewSource) -> PreclaimResult<Tx, Journal, ParentBatchId>,
        RunTryClear: FnOnce() -> TryClearResult,
        TryClearResult: QueueApplyTryClearResult,
        ApplySandbox: FnOnce(),
    {
        self.with_owned_metrics_source_view_log_sinks::<TxId, _, _, _>(
            tx_source,
            view_source,
            |owner, tx_source, view_snapshot| {
                run_queue_apply_after_preflight_with_live_owner_from_sources_and_log_sinks(
                    owner,
                    tx_source,
                    view_snapshot,
                    preflight_result,
                    hold_preflight,
                    can_be_held_result,
                    trace,
                    debug,
                    info,
                    apply,
                    prepare_multitxn,
                    run_preclaim,
                    run_try_clear,
                    apply_sandbox,
                )
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_owned_metrics_and_caller_preclaim_and_log_sinks<
        TxId,
        TxSource,
        ViewSource,
        RunPreflight,
        TraceFn,
        DebugFn,
        InfoFn,
        ApplyFn,
        PrepareMultiTxn,
        RunPreclaimStage,
        RunTryClear,
        TryClearResult,
        ApplySandbox,
    >(
        &mut self,
        tx_source: &TxSource,
        view_source: &ViewSource,
        hold_preflight: QueueHoldPreflight,
        flags: ApplyFlags,
        consequences: TxConsequences,
        can_be_held_result: Ter,
        run_preflight: RunPreflight,
        trace: TraceFn,
        debug: DebugFn,
        info: InfoFn,
        apply: ApplyFn,
        prepare_multitxn: PrepareMultiTxn,
        run_preclaim_stage: RunPreclaimStage,
        run_try_clear: RunTryClear,
        apply_sandbox: ApplySandbox,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        TxId: Clone + Display,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: QueueApplyObservedViewSource<Account>,
        RunPreflight: FnOnce() -> PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
        ApplyFn: FnOnce() -> crate::ApplyResult,
        PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
        RunPreclaimStage: FnOnce(
            QueueApplyPreparedPreclaimInputs<Account>,
        ) -> Result<
            QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>,
            crate::ApplyResult,
        >,
        RunTryClear: FnOnce() -> TryClearResult,
        TryClearResult: QueueApplyTryClearResult,
        ApplySandbox: FnOnce(),
    {
        self.with_owned_metrics_source_view_caller_preclaim_log_sinks::<TxId, _, _, _>(
            tx_source,
            view_source,
            |owner, tx_source, view_snapshot| {
                run_queue_apply_with_live_owner_from_sources_and_caller_preclaim_and_log_sinks(
                    owner,
                    tx_source,
                    view_snapshot,
                    hold_preflight,
                    flags,
                    consequences,
                    can_be_held_result,
                    run_preflight,
                    trace,
                    debug,
                    info,
                    apply,
                    prepare_multitxn,
                    run_preclaim_stage,
                    run_try_clear,
                    apply_sandbox,
                )
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_owned_metrics_and_caller_preclaim_and_log_sinks<
        TxId,
        TxSource,
        ViewSource,
        TraceFn,
        DebugFn,
        InfoFn,
        ApplyFn,
        PrepareMultiTxn,
        RunPreclaimStage,
        RunTryClear,
        TryClearResult,
        ApplySandbox,
    >(
        &mut self,
        tx_source: &TxSource,
        view_source: &ViewSource,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
        can_be_held_result: Ter,
        trace: TraceFn,
        debug: DebugFn,
        info: InfoFn,
        apply: ApplyFn,
        prepare_multitxn: PrepareMultiTxn,
        run_preclaim_stage: RunPreclaimStage,
        run_try_clear: RunTryClear,
        apply_sandbox: ApplySandbox,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        TxId: Clone + Display,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: QueueApplyObservedViewSource<Account>,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
        ApplyFn: FnOnce() -> crate::ApplyResult,
        PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
        RunPreclaimStage: FnOnce(
            QueueApplyPreparedPreclaimInputs<Account>,
        ) -> Result<
            QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>,
            crate::ApplyResult,
        >,
        RunTryClear: FnOnce() -> TryClearResult,
        TryClearResult: QueueApplyTryClearResult,
        ApplySandbox: FnOnce(),
    {
        self.with_owned_metrics_source_view_caller_preclaim_log_sinks::<TxId, _, _, _>(
            tx_source,
            view_source,
            |owner, tx_source, view_snapshot| {
                run_queue_apply_after_preflight_with_live_owner_from_sources_and_caller_preclaim_and_log_sinks(
                    owner,
                    tx_source,
                    view_snapshot,
                    preflight_result,
                    hold_preflight,
                    can_be_held_result,
                    trace,
                    debug,
                    info,
                    apply,
                    prepare_multitxn,
                    run_preclaim_stage,
                    run_try_clear,
                    apply_sandbox,
                )
            },
        )
    }

    pub fn apply_with_app_view<TxId, App, View, TxSource>(
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
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_current_app_view(|owner| {
            run_queue_apply_with_app_view(
                owner,
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

    pub fn apply_with_app_view_and_owned_metrics<TxId, App, View, TxSource>(
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
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_owned_metrics_current_app_view(|owner, metrics_snapshot| {
            run_queue_apply_with_app_view_and_metrics_snapshot(
                owner,
                app,
                view,
                tx_source,
                metrics_snapshot,
                hold_preflight,
                flags,
                consequences,
                can_be_held_result,
            )
        })
    }

    pub fn apply_with_app_view_and_owned_metrics_and_log_messages<TxId, App, View, TxSource>(
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
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_owned_metrics_current_app_view(|owner, metrics_snapshot| {
            run_queue_apply_with_app_view_and_metrics_snapshot_and_log_messages(
                owner,
                app,
                view,
                tx_source,
                metrics_snapshot,
                hold_preflight,
                flags,
                consequences,
                can_be_held_result,
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_app_view_and_caller_preclaim<TxId, App, View, TxSource, RunPreclaimStage>(
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
        self.with_current_app_view_caller_preclaim(|owner| {
            run_queue_apply_with_app_view_and_caller_preclaim(
                owner,
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
        TxId,
        App,
        View,
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
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
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
        self.with_current_app_view_caller_preclaim(|owner| {
            run_queue_apply_with_app_view_and_caller_preclaim_and_log_messages(
                owner,
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
        TxId,
        App,
        View,
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
        self.with_owned_metrics_current_app_view_caller_preclaim(|owner, metrics_snapshot| {
            run_queue_apply_with_app_view_and_metrics_snapshot_and_caller_preclaim(
                owner,
                app,
                view,
                tx_source,
                metrics_snapshot,
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
        TxId,
        App,
        View,
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
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
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
        self.with_owned_metrics_current_app_view_caller_preclaim(|owner, metrics_snapshot| {
            run_queue_apply_with_app_view_and_metrics_snapshot_and_caller_preclaim_and_log_messages(
                owner,
                app,
                view,
                tx_source,
                metrics_snapshot,
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
        self.with_current_app_view_caller_preclaim_log_sinks(|owner| {
            run_queue_apply_with_app_view_and_caller_preclaim_and_log_sinks(
                owner,
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
    pub fn apply_with_app_view_and_owned_metrics_and_caller_preclaim_and_log_sinks<
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
        self.with_owned_metrics_current_app_view_caller_preclaim_log_sinks(
            |owner, metrics_snapshot| {
            run_queue_apply_with_app_view_and_metrics_snapshot_and_caller_preclaim_and_log_sinks(
                owner,
                app,
                view,
                tx_source,
                metrics_snapshot,
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

    pub fn apply_with_app_view_and_derived_hold_preflight<TxId, App, View, TxSource>(
        &mut self,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        flags: ApplyFlags,
        consequences: TxConsequences,
        can_be_held_result: Ter,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_current_app_view(|owner| {
            run_queue_apply_with_app_view_and_derived_hold_preflight(
                owner,
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
        TxId,
        App,
        View,
        TxSource,
    >(
        &mut self,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        flags: ApplyFlags,
        consequences: TxConsequences,
        can_be_held_result: Ter,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_current_app_view(|owner| {
            run_queue_apply_with_app_view_and_derived_hold_preflight_and_log_messages(
                owner,
                app,
                view,
                tx_source,
                flags,
                consequences,
                can_be_held_result,
            )
        })
    }

    pub fn apply_with_app_view_and_derived_hold_admission<TxId, App, View, TxSource>(
        &mut self,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        hold_preflight: QueueHoldPreflight,
        flags: ApplyFlags,
        consequences: TxConsequences,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_current_app_view(|owner| {
            run_queue_apply_with_app_view_and_derived_hold_admission(
                owner,
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
        TxId,
        App,
        View,
        TxSource,
    >(
        &mut self,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        hold_preflight: QueueHoldPreflight,
        flags: ApplyFlags,
        consequences: TxConsequences,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_current_app_view(|owner| {
            run_queue_apply_with_app_view_and_derived_hold_admission_and_log_messages(
                owner,
                app,
                view,
                tx_source,
                hold_preflight,
                flags,
                consequences,
            )
        })
    }

    pub fn apply_with_app_view_and_derived_preflight_facts<TxId, App, View, TxSource>(
        &mut self,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        can_be_held_result: Ter,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_current_app_view(|owner| {
            run_queue_apply_with_app_view_and_derived_preflight_facts(
                owner,
                app,
                view,
                tx_source,
                can_be_held_result,
            )
        })
    }

    pub fn apply_with_app_view_and_derived_preflight_facts_and_log_messages<
        TxId,
        App,
        View,
        TxSource,
    >(
        &mut self,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        can_be_held_result: Ter,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_current_app_view(|owner| {
            run_queue_apply_with_app_view_and_derived_preflight_facts_and_log_messages(
                owner,
                app,
                view,
                tx_source,
                can_be_held_result,
            )
        })
    }

    pub fn apply_with_app_view_and_derived_preflight_facts_and_hold_admission<
        TxId,
        App,
        View,
        TxSource,
    >(
        &mut self,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_current_app_view(|owner| {
            run_queue_apply_with_app_view_and_derived_preflight_facts_and_hold_admission(
                owner, app, view, tx_source,
            )
        })
    }

    pub fn apply_with_app_view_and_derived_preflight_facts_and_hold_admission_and_log_messages<
        TxId,
        App,
        View,
        TxSource,
    >(
        &mut self,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_current_app_view(|owner| {
            run_queue_apply_with_app_view_and_derived_preflight_facts_and_hold_admission_and_log_messages(
                owner, app, view, tx_source,
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_app_view_and_log_sinks<TxId, App, View, TxSource, TraceFn, DebugFn, InfoFn>(
        &mut self,
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
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
    {
        run_queue_apply_with_app_view_and_log_sinks(
            self,
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
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_app_view_and_owned_metrics_and_log_sinks<
        TxId,
        App,
        View,
        TxSource,
        TraceFn,
        DebugFn,
        InfoFn,
    >(
        &mut self,
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
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
    {
        self.with_owned_metrics_current_app_view_log_sinks(|owner, metrics_snapshot| {
            run_queue_apply_with_app_view_and_metrics_snapshot_and_log_sinks(
                owner,
                app,
                view,
                tx_source,
                metrics_snapshot,
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
        TxId,
        App,
        View,
        TxSource,
        TraceFn,
        DebugFn,
        InfoFn,
    >(
        &mut self,
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
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
    {
        self.with_owned_metrics_current_app_view_log_sinks(|owner, metrics_snapshot| {
            run_queue_apply_with_app_view_and_metrics_snapshot_and_log_sinks_and_derived_hold_admission(
                owner,
                app,
                view,
                tx_source,
                metrics_snapshot,
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
        TxId,
        App,
        View,
        TxSource,
    >(
        &mut self,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        hold_preflight: QueueHoldPreflight,
        flags: ApplyFlags,
        consequences: TxConsequences,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_owned_metrics_current_app_view(|owner, metrics_snapshot| {
            run_queue_apply_with_app_view_and_metrics_snapshot_and_derived_hold_admission_and_log_messages(
                owner,
                app,
                view,
                tx_source,
                metrics_snapshot,
                hold_preflight,
                flags,
                consequences,
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_app_view_and_log_sinks_and_derived_hold_admission<
        TxId,
        App,
        View,
        TxSource,
        TraceFn,
        DebugFn,
        InfoFn,
    >(
        &mut self,
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
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
    {
        run_queue_apply_with_app_view_and_log_sinks_and_derived_hold_admission(
            self,
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
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_app_view_and_owned_metrics_and_log_sinks_and_derived_hold_preflight<
        TxId,
        App,
        View,
        TxSource,
        TraceFn,
        DebugFn,
        InfoFn,
    >(
        &mut self,
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
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
    {
        self.with_owned_metrics_current_app_view_log_sinks(|owner, metrics_snapshot| {
            run_queue_apply_with_app_view_and_metrics_snapshot_and_log_sinks_and_derived_hold_preflight(
                owner,
                app,
                view,
                tx_source,
                metrics_snapshot,
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
        TxId,
        App,
        View,
        TxSource,
    >(
        &mut self,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        flags: ApplyFlags,
        consequences: TxConsequences,
        can_be_held_result: Ter,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_owned_metrics_current_app_view(|owner, metrics_snapshot| {
            run_queue_apply_with_app_view_and_metrics_snapshot_and_derived_hold_preflight_and_log_messages(
                owner,
                app,
                view,
                tx_source,
                metrics_snapshot,
                flags,
                consequences,
                can_be_held_result,
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_app_view_and_log_sinks_and_derived_hold_preflight<
        TxId,
        App,
        View,
        TxSource,
        TraceFn,
        DebugFn,
        InfoFn,
    >(
        &mut self,
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
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
    {
        run_queue_apply_with_app_view_and_log_sinks_and_derived_hold_preflight(
            self,
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
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_app_view_and_owned_metrics_and_log_sinks_and_derived_preflight_facts<
        TxId,
        App,
        View,
        TxSource,
        TraceFn,
        DebugFn,
        InfoFn,
    >(
        &mut self,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        trace: TraceFn,
        debug: DebugFn,
        info: InfoFn,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
    {
        self.with_owned_metrics_current_app_view_log_sinks(|owner, metrics_snapshot| {
            run_queue_apply_with_app_view_and_metrics_snapshot_and_log_sinks_and_derived_preflight_facts(
                owner,
                app,
                view,
                tx_source,
                metrics_snapshot,
                trace,
                debug,
                info,
            )
        })
    }

    pub fn apply_with_app_view_and_owned_metrics_and_derived_preflight_facts_and_log_messages<
        TxId,
        App,
        View,
        TxSource,
    >(
        &mut self,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_owned_metrics_current_app_view(|owner, metrics_snapshot| {
            run_queue_apply_with_app_view_and_metrics_snapshot_and_derived_preflight_facts_and_log_messages(
                owner,
                app,
                view,
                tx_source,
                metrics_snapshot,
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_app_view_and_log_sinks_and_derived_preflight_facts<
        TxId,
        App,
        View,
        TxSource,
        TraceFn,
        DebugFn,
        InfoFn,
    >(
        &mut self,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        can_be_held_result: Ter,
        trace: TraceFn,
        debug: DebugFn,
        info: InfoFn,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
    {
        run_queue_apply_with_app_view_and_log_sinks_and_derived_preflight_facts(
            self,
            app,
            view,
            tx_source,
            can_be_held_result,
            trace,
            debug,
            info,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_app_view_and_owned_metrics_and_log_sinks_and_derived_preflight_facts_and_hold_admission<
        TxId,
        App,
        View,
        TxSource,
        TraceFn,
        DebugFn,
        InfoFn,
    >(
        &mut self,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        trace: TraceFn,
        debug: DebugFn,
        info: InfoFn,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
    {
        self.with_owned_metrics_current_app_view_log_sinks(|owner, metrics_snapshot| {
            run_queue_apply_with_app_view_and_metrics_snapshot_and_log_sinks_and_derived_preflight_facts_and_hold_admission(
                owner,
                app,
                view,
                tx_source,
                metrics_snapshot,
                trace,
                debug,
                info,
            )
        })
    }

    pub fn apply_with_app_view_and_owned_metrics_and_derived_preflight_facts_and_hold_admission_and_log_messages<
        TxId,
        App,
        View,
        TxSource,
    >(
        &mut self,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_owned_metrics_current_app_view(|owner, metrics_snapshot| {
            run_queue_apply_with_app_view_and_metrics_snapshot_and_derived_preflight_facts_and_hold_admission_and_log_messages(
                owner,
                app,
                view,
                tx_source,
                metrics_snapshot,
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_app_view_and_log_sinks_and_derived_preflight_facts_and_hold_admission<
        TxId,
        App,
        View,
        TxSource,
        TraceFn,
        DebugFn,
        InfoFn,
    >(
        &mut self,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        trace: TraceFn,
        debug: DebugFn,
        info: InfoFn,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
    {
        run_queue_apply_with_app_view_and_log_sinks_and_derived_preflight_facts_and_hold_admission(
            self, app, view, tx_source, trace, debug, info,
        )
    }

    pub fn apply_after_preflight_with_app_view<TxId, App, View, TxSource>(
        &mut self,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
        can_be_held_result: Ter,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_current_app_view(|owner| {
            run_queue_apply_after_preflight_with_app_view(
                owner,
                app,
                view,
                tx_source,
                preflight_result,
                hold_preflight,
                can_be_held_result,
            )
        })
    }

    pub fn apply_after_preflight_with_app_view_and_owned_metrics<TxId, App, View, TxSource>(
        &mut self,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
        can_be_held_result: Ter,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_owned_metrics_current_app_view(|owner, metrics_snapshot| {
            run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot(
                owner,
                app,
                view,
                tx_source,
                metrics_snapshot,
                preflight_result,
                hold_preflight,
                can_be_held_result,
            )
        })
    }

    pub fn apply_after_preflight_with_app_view_and_owned_metrics_and_log_messages<
        TxId,
        App,
        View,
        TxSource,
    >(
        &mut self,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
        can_be_held_result: Ter,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_owned_metrics_current_app_view(|owner, metrics_snapshot| {
            run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot_and_log_messages(
                owner,
                app,
                view,
                tx_source,
                metrics_snapshot,
                preflight_result,
                hold_preflight,
                can_be_held_result,
            )
        })
    }

    pub fn apply_after_preflight_with_app_view_and_owned_metrics_and_derived_hold_preflight_and_log_messages<
        TxId,
        App,
        View,
        TxSource,
    >(
        &mut self,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        can_be_held_result: Ter,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_owned_metrics_current_app_view(|owner, metrics_snapshot| {
            run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot_and_derived_hold_preflight_and_log_messages(
                owner,
                app,
                view,
                tx_source,
                metrics_snapshot,
                preflight_result,
                can_be_held_result,
            )
        })
    }

    pub fn apply_after_preflight_with_app_view_and_owned_metrics_and_derived_hold_admission_and_log_messages<
        TxId,
        App,
        View,
        TxSource,
    >(
        &mut self,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_owned_metrics_current_app_view(|owner, metrics_snapshot| {
            run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot_and_derived_hold_admission_and_log_messages(
                owner,
                app,
                view,
                tx_source,
                metrics_snapshot,
                preflight_result,
                hold_preflight,
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_app_view_and_caller_preclaim<
        TxId,
        App,
        View,
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
        self.with_current_app_view_caller_preclaim(|owner| {
            run_queue_apply_after_preflight_with_app_view_and_caller_preclaim(
                owner,
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
        TxId,
        App,
        View,
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
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
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
        self.with_current_app_view_caller_preclaim(|owner| {
            run_queue_apply_after_preflight_with_app_view_and_caller_preclaim_and_log_messages(
                owner,
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
        TxId,
        App,
        View,
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
        self.with_owned_metrics_current_app_view_caller_preclaim(|owner, metrics_snapshot| {
            run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot_and_caller_preclaim(
                owner,
                app,
                view,
                tx_source,
                metrics_snapshot,
                preflight_result,
                hold_preflight,
                can_be_held_result,
                run_preclaim_stage,
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_app_view_and_owned_metrics_and_caller_preclaim_and_log_messages<
        TxId,
        App,
        View,
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
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
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
        self.with_owned_metrics_current_app_view_caller_preclaim(|owner, metrics_snapshot| {
            run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot_and_caller_preclaim_and_log_messages(
                owner,
                app,
                view,
                tx_source,
                metrics_snapshot,
                preflight_result,
                hold_preflight,
                can_be_held_result,
                run_preclaim_stage,
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_app_view_and_caller_preclaim_and_log_sinks<
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
        self.with_current_app_view_caller_preclaim_log_sinks(|owner| {
            run_queue_apply_after_preflight_with_app_view_and_caller_preclaim_and_log_sinks(
                owner,
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
    pub fn apply_after_preflight_with_app_view_and_owned_metrics_and_caller_preclaim_and_log_sinks<
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
        self.with_owned_metrics_current_app_view_caller_preclaim_log_sinks(
            |owner, metrics_snapshot| {
                run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot_and_caller_preclaim_and_log_sinks(
                owner,
                app,
                view,
                tx_source,
                metrics_snapshot,
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
        TxId,
        App,
        View,
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
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_current_app_view(|owner| {
            run_queue_apply_after_preflight_with_app_view_and_derived_hold_preflight(
                owner,
                app,
                view,
                tx_source,
                preflight_result,
                can_be_held_result,
            )
        })
    }

    pub fn apply_after_preflight_with_app_view_and_derived_hold_preflight_and_log_messages<
        TxId,
        App,
        View,
        TxSource,
    >(
        &mut self,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        can_be_held_result: Ter,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_current_app_view(|owner| {
            run_queue_apply_after_preflight_with_app_view_and_derived_hold_preflight_and_log_messages(
                owner,
                app,
                view,
                tx_source,
                preflight_result,
                can_be_held_result,
            )
        })
    }

    pub fn apply_after_preflight_with_app_view_and_derived_hold_admission<
        TxId,
        App,
        View,
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
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_current_app_view(|owner| {
            run_queue_apply_after_preflight_with_app_view_and_derived_hold_admission(
                owner,
                app,
                view,
                tx_source,
                preflight_result,
                hold_preflight,
            )
        })
    }

    pub fn apply_after_preflight_with_app_view_and_derived_hold_admission_and_log_messages<
        TxId,
        App,
        View,
        TxSource,
    >(
        &mut self,
        app: &mut App,
        view: &View,
        tx_source: &TxSource,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
    {
        self.with_current_app_view(|owner| {
            run_queue_apply_after_preflight_with_app_view_and_derived_hold_admission_and_log_messages(
                owner,
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
        TxId,
        App,
        View,
        TxSource,
        TraceFn,
        DebugFn,
        InfoFn,
    >(
        &mut self,
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
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
    {
        run_queue_apply_after_preflight_with_app_view_and_log_sinks(
            self,
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
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_app_view_and_owned_metrics_and_log_sinks<
        TxId,
        App,
        View,
        TxSource,
        TraceFn,
        DebugFn,
        InfoFn,
    >(
        &mut self,
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
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
    {
        self.with_owned_metrics_current_app_view_log_sinks(|owner, metrics_snapshot| {
            run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot_and_log_sinks(
                owner,
                app,
                view,
                tx_source,
                metrics_snapshot,
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
        TxId,
        App,
        View,
        TxSource,
        TraceFn,
        DebugFn,
        InfoFn,
    >(
        &mut self,
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
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
    {
        self.with_owned_metrics_current_app_view_log_sinks(|owner, metrics_snapshot| {
            run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot_and_log_sinks_and_derived_hold_admission(
                owner,
                app,
                view,
                tx_source,
                metrics_snapshot,
                preflight_result,
                hold_preflight,
                trace,
                debug,
                info,
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_app_view_and_log_sinks_and_derived_hold_admission<
        TxId,
        App,
        View,
        TxSource,
        TraceFn,
        DebugFn,
        InfoFn,
    >(
        &mut self,
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
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
    {
        run_queue_apply_after_preflight_with_app_view_and_log_sinks_and_derived_hold_admission(
            self,
            app,
            view,
            tx_source,
            preflight_result,
            hold_preflight,
            trace,
            debug,
            info,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_app_view_and_owned_metrics_and_log_sinks_and_derived_hold_preflight<
        TxId,
        App,
        View,
        TxSource,
        TraceFn,
        DebugFn,
        InfoFn,
    >(
        &mut self,
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
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
    {
        self.with_owned_metrics_current_app_view_log_sinks(|owner, metrics_snapshot| {
            run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot_and_log_sinks_and_derived_hold_preflight(
                owner,
                app,
                view,
                tx_source,
                metrics_snapshot,
                preflight_result,
                can_be_held_result,
                trace,
                debug,
                info,
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_app_view_and_log_sinks_and_derived_hold_preflight<
        TxId,
        App,
        View,
        TxSource,
        TraceFn,
        DebugFn,
        InfoFn,
    >(
        &mut self,
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
        TxId: Clone + Display,
        App: QueueApplyAppRuntime<Tx, Journal, ParentBatchId>,
        View: QueueApplyLedgerViewSource<Account>,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
    {
        run_queue_apply_after_preflight_with_app_view_and_log_sinks_and_derived_hold_preflight(
            self,
            app,
            view,
            tx_source,
            preflight_result,
            can_be_held_result,
            trace,
            debug,
            info,
        )
    }
}
