//! Explicit call envelope for the current `xrpld` `TxQ::apply(...)`
//! boundary.
//!
//! This layer bundles the two live observation inputs above the
//! landed apply owner shell:
//! - one transaction source,
//! - one ledger-view source.
//!
//! It makes the live tx/view call boundary explicit above the landed
//! `QueueApplyOwnerShell`.

use std::{cell::RefCell, fmt::Display};

use protocol::Ter;

use crate::{
    ApplyFlags, PreclaimResult, PreflightResult, QueueApplyHoldPreflightTxSource,
    QueueApplyObservedTxSource, QueueApplyObservedViewSource, QueueApplyOwnerShell,
    QueueApplyPreclaimStage, QueueApplyPreclaimViewSource, QueueApplyPreflightStage,
    QueueApplyPreparedPreclaimInputs, QueueApplyQueueLogMessages,
    QueueApplyTopWithLogMessagesResult, QueueApplyTryClearResult, QueueApplyViewAdjustment,
    QueueHoldPreflight, TxConsequences, derive_queue_hold_preflight_from_tx_source,
};

#[derive(Debug)]
pub struct QueueApplyCallEnvelope<'a, TxSource, ViewSource> {
    tx_source: &'a TxSource,
    view_source: &'a ViewSource,
}

impl<'a, TxSource, ViewSource> QueueApplyCallEnvelope<'a, TxSource, ViewSource> {
    pub fn new(tx_source: &'a TxSource, view_source: &'a ViewSource) -> Self {
        Self {
            tx_source,
            view_source,
        }
    }

    pub const fn tx_source(&self) -> &TxSource {
        self.tx_source
    }

    pub const fn view_source(&self) -> &ViewSource {
        self.view_source
    }
}

impl<'a, TxSource, ViewSource> QueueApplyCallEnvelope<'a, TxSource, ViewSource> {
    fn with_owner_sources<Account, Tx, Journal, ParentBatchId, TxId, R>(
        &self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        run: impl FnOnce(
            &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
            &'a TxSource,
            &'a ViewSource,
        ) -> R,
    ) -> R
    where
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
        TxId: Clone + Display,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: QueueApplyObservedViewSource<Account>,
    {
        run(owner, self.tx_source, self.view_source)
    }

    fn with_owner_sources_current_app_view<Account, Tx, Journal, ParentBatchId, TxId, R>(
        &self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        run: impl FnOnce(
            &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
            &TxSource,
            &ViewSource,
        ) -> R,
    ) -> R
    where
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
        TxId: Clone + Display,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: QueueApplyObservedViewSource<Account>,
    {
        self.with_owner_sources(owner, run)
    }

    fn with_owner_sources_current_app_view_caller_preclaim<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        R,
    >(
        &self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        run: impl FnOnce(
            &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
            &TxSource,
            &ViewSource,
        ) -> R,
    ) -> R
    where
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
        TxId: Clone + Display,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: QueueApplyObservedViewSource<Account>,
    {
        self.with_owner_sources(owner, run)
    }

    fn with_owner_sources_current_app_view_caller_preclaim_log_sinks<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        R,
    >(
        &self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        run: impl FnOnce(
            &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
            &TxSource,
            &ViewSource,
        ) -> R,
    ) -> R
    where
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
        TxId: Clone + Display,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: QueueApplyObservedViewSource<Account>,
    {
        self.with_owner_sources_current_app_view_caller_preclaim(owner, run)
    }

    fn with_owner_sources_current_app_view_log_sinks<Account, Tx, Journal, ParentBatchId, TxId, R>(
        &self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        run: impl FnOnce(
            &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
            &TxSource,
            &ViewSource,
        ) -> R,
    ) -> R
    where
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
        TxId: Clone + Display,
        TxSource: QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: QueueApplyObservedViewSource<Account>,
    {
        self.with_owner_sources(owner, run)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        RunPreflight,
        TraceFn,
        ApplyFn,
        PrepareMultiTxn,
        RunPreclaim,
        RunTryClear,
        TryClearResult,
        ApplySandbox,
    >(
        &self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
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
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
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
        self.with_owner_sources_current_app_view(owner, |owner, tx_source, view_source| {
            owner.apply(
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
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TraceFn,
        ApplyFn,
        PrepareMultiTxn,
        RunPreclaim,
        RunTryClear,
        TryClearResult,
        ApplySandbox,
    >(
        &self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
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
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
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
        self.with_owner_sources_current_app_view(owner, |owner, tx_source, view_source| {
            owner.apply_after_preflight(
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

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_log_messages<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        RunPreflight,
        TraceFn,
        ApplyFn,
        PrepareMultiTxn,
        RunPreclaim,
        RunTryClear,
        TryClearResult,
        ApplySandbox,
    >(
        &self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
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
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
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
        self.with_owner_sources_current_app_view(owner, |owner, tx_source, view_source| {
            owner.apply_with_log_messages(
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
    pub fn apply_after_preflight_with_log_messages<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TraceFn,
        ApplyFn,
        PrepareMultiTxn,
        RunPreclaim,
        RunTryClear,
        TryClearResult,
        ApplySandbox,
    >(
        &self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
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
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
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
        self.with_owner_sources_current_app_view(owner, |owner, tx_source, view_source| {
            owner.apply_after_preflight_with_log_messages(
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

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_derived_hold_preflight_and_log_messages<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TraceFn,
        ApplyFn,
        PrepareMultiTxn,
        RunPreclaim,
        RunTryClear,
        TryClearResult,
        ApplySandbox,
    >(
        &self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        can_be_held_result: Ter,
        trace: TraceFn,
        apply: ApplyFn,
        prepare_multitxn: PrepareMultiTxn,
        run_preclaim: RunPreclaim,
        run_try_clear: RunTryClear,
        apply_sandbox: ApplySandbox,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
        TxId: Clone + Display,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
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
        let hold_preflight =
            derive_queue_hold_preflight_from_tx_source(self.tx_source, preflight_result.flags);
        let queue_log_messages = RefCell::new(QueueApplyQueueLogMessages::default());
        let stage = self.apply_after_preflight_with_log_sinks(
            owner,
            preflight_result,
            hold_preflight,
            can_be_held_result,
            trace,
            |message| queue_log_messages.borrow_mut().debug.push(message),
            |message| queue_log_messages.borrow_mut().info.push(message),
            apply,
            prepare_multitxn,
            run_preclaim,
            run_try_clear,
            apply_sandbox,
        );
        QueueApplyTopWithLogMessagesResult {
            stage,
            queue_log_messages: queue_log_messages.into_inner(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_derived_hold_admission_and_log_messages<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TraceFn,
        ApplyFn,
        PrepareMultiTxn,
        RunPreclaim,
        RunTryClear,
        TryClearResult,
        ApplySandbox,
    >(
        &self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
        trace: TraceFn,
        apply: ApplyFn,
        prepare_multitxn: PrepareMultiTxn,
        run_preclaim: RunPreclaim,
        run_try_clear: RunTryClear,
        apply_sandbox: ApplySandbox,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
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
        let queue_log_messages = RefCell::new(QueueApplyQueueLogMessages::default());
        let stage = self.apply_after_preflight_with_log_sinks(
            owner,
            preflight_result,
            hold_preflight,
            owner.owner().derive_can_be_held_result(
                self.tx_source,
                self.view_source,
                hold_preflight,
            ),
            trace,
            |message| queue_log_messages.borrow_mut().debug.push(message),
            |message| queue_log_messages.borrow_mut().info.push(message),
            apply,
            prepare_multitxn,
            run_preclaim,
            run_try_clear,
            apply_sandbox,
        );
        QueueApplyTopWithLogMessagesResult {
            stage,
            queue_log_messages: queue_log_messages.into_inner(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_derived_preflight_facts<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        RunPreflight,
        TraceFn,
        ApplyFn,
        PrepareMultiTxn,
        RunPreclaim,
        RunTryClear,
        TryClearResult,
        ApplySandbox,
    >(
        &self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
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
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
        TxId: Clone + Display,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
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
        let preflight_result = run_preflight();
        let hold_preflight =
            derive_queue_hold_preflight_from_tx_source(self.tx_source, preflight_result.flags);
        self.apply_after_preflight(
            owner,
            &preflight_result,
            hold_preflight,
            can_be_held_result,
            trace,
            apply,
            prepare_multitxn,
            run_preclaim,
            run_try_clear,
            apply_sandbox,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_derived_preflight_facts_and_log_messages<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        RunPreflight,
        TraceFn,
        ApplyFn,
        PrepareMultiTxn,
        RunPreclaim,
        RunTryClear,
        TryClearResult,
        ApplySandbox,
    >(
        &self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
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
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
        TxId: Clone + Display,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
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
        let preflight_result = run_preflight();
        let hold_preflight =
            derive_queue_hold_preflight_from_tx_source(self.tx_source, preflight_result.flags);
        let queue_log_messages = RefCell::new(QueueApplyQueueLogMessages::default());
        let stage = self.apply_after_preflight_with_log_sinks(
            owner,
            &preflight_result,
            hold_preflight,
            can_be_held_result,
            trace,
            |message| queue_log_messages.borrow_mut().debug.push(message),
            |message| queue_log_messages.borrow_mut().info.push(message),
            apply,
            prepare_multitxn,
            run_preclaim,
            run_try_clear,
            apply_sandbox,
        );
        QueueApplyTopWithLogMessagesResult {
            stage,
            queue_log_messages: queue_log_messages.into_inner(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_derived_preflight_facts_and_hold_admission<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        RunPreflight,
        TraceFn,
        ApplyFn,
        PrepareMultiTxn,
        RunPreclaim,
        RunTryClear,
        TryClearResult,
        ApplySandbox,
    >(
        &self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        run_preflight: RunPreflight,
        trace: TraceFn,
        apply: ApplyFn,
        prepare_multitxn: PrepareMultiTxn,
        run_preclaim: RunPreclaim,
        run_try_clear: RunTryClear,
        apply_sandbox: ApplySandbox,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
        TxId: Clone + Display,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
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
        let preflight_result = run_preflight();
        let hold_preflight =
            derive_queue_hold_preflight_from_tx_source(self.tx_source, preflight_result.flags);
        let can_be_held_result = owner.owner().derive_can_be_held_result(
            self.tx_source,
            self.view_source,
            hold_preflight,
        );
        self.apply_after_preflight(
            owner,
            &preflight_result,
            hold_preflight,
            can_be_held_result,
            trace,
            apply,
            prepare_multitxn,
            run_preclaim,
            run_try_clear,
            apply_sandbox,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_derived_preflight_facts_and_hold_admission_and_log_messages<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        RunPreflight,
        TraceFn,
        ApplyFn,
        PrepareMultiTxn,
        RunPreclaim,
        RunTryClear,
        TryClearResult,
        ApplySandbox,
    >(
        &self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        run_preflight: RunPreflight,
        trace: TraceFn,
        apply: ApplyFn,
        prepare_multitxn: PrepareMultiTxn,
        run_preclaim: RunPreclaim,
        run_try_clear: RunTryClear,
        apply_sandbox: ApplySandbox,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
        TxId: Clone + Display,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
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
        let preflight_result = run_preflight();
        let hold_preflight =
            derive_queue_hold_preflight_from_tx_source(self.tx_source, preflight_result.flags);
        let can_be_held_result = owner.owner().derive_can_be_held_result(
            self.tx_source,
            self.view_source,
            hold_preflight,
        );
        let queue_log_messages = RefCell::new(QueueApplyQueueLogMessages::default());
        let stage = self.apply_after_preflight_with_log_sinks(
            owner,
            &preflight_result,
            hold_preflight,
            can_be_held_result,
            trace,
            |message| queue_log_messages.borrow_mut().debug.push(message),
            |message| queue_log_messages.borrow_mut().info.push(message),
            apply,
            prepare_multitxn,
            run_preclaim,
            run_try_clear,
            apply_sandbox,
        );
        QueueApplyTopWithLogMessagesResult {
            stage,
            queue_log_messages: queue_log_messages.into_inner(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_derived_hold_preflight_and_log_messages<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        RunPreflight,
        TraceFn,
        ApplyFn,
        PrepareMultiTxn,
        RunPreclaim,
        RunTryClear,
        TryClearResult,
        ApplySandbox,
    >(
        &self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
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
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
        TxId: Clone + Display,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
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
        let hold_preflight = derive_queue_hold_preflight_from_tx_source(self.tx_source, flags);
        let queue_log_messages = RefCell::new(QueueApplyQueueLogMessages::default());
        let stage = self.apply_with_log_sinks(
            owner,
            hold_preflight,
            flags,
            consequences,
            can_be_held_result,
            run_preflight,
            trace,
            |message| queue_log_messages.borrow_mut().debug.push(message),
            |message| queue_log_messages.borrow_mut().info.push(message),
            apply,
            prepare_multitxn,
            run_preclaim,
            run_try_clear,
            apply_sandbox,
        );
        QueueApplyTopWithLogMessagesResult {
            stage,
            queue_log_messages: queue_log_messages.into_inner(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_derived_hold_admission_and_log_messages<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        RunPreflight,
        TraceFn,
        ApplyFn,
        PrepareMultiTxn,
        RunPreclaim,
        RunTryClear,
        TryClearResult,
        ApplySandbox,
    >(
        &self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
        flags: ApplyFlags,
        consequences: TxConsequences,
        run_preflight: RunPreflight,
        trace: TraceFn,
        apply: ApplyFn,
        prepare_multitxn: PrepareMultiTxn,
        run_preclaim: RunPreclaim,
        run_try_clear: RunTryClear,
        apply_sandbox: ApplySandbox,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
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
        let queue_log_messages = RefCell::new(QueueApplyQueueLogMessages::default());
        let stage = self.apply_with_log_sinks(
            owner,
            hold_preflight,
            flags,
            consequences,
            owner.owner().derive_can_be_held_result(
                self.tx_source,
                self.view_source,
                hold_preflight,
            ),
            run_preflight,
            trace,
            |message| queue_log_messages.borrow_mut().debug.push(message),
            |message| queue_log_messages.borrow_mut().info.push(message),
            apply,
            prepare_multitxn,
            run_preclaim,
            run_try_clear,
            apply_sandbox,
        );
        QueueApplyTopWithLogMessagesResult {
            stage,
            queue_log_messages: queue_log_messages.into_inner(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_caller_preclaim<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        RunPreflight,
        TraceFn,
        ApplyFn,
        PrepareMultiTxn,
        RunPreclaimStage,
        RunTryClear,
        TryClearResult,
        ApplySandbox,
    >(
        &self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
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
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
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
        self.with_owner_sources_current_app_view_caller_preclaim(
            owner,
            |owner, tx_source, view_source| {
                owner.apply_with_caller_preclaim(
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
    pub fn apply_with_caller_preclaim_and_log_messages<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        RunPreflight,
        TraceFn,
        ApplyFn,
        PrepareMultiTxn,
        RunPreclaimStage,
        RunTryClear,
        TryClearResult,
        ApplySandbox,
    >(
        &self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
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
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
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
        self.with_owner_sources_current_app_view_caller_preclaim(
            owner,
            |owner, tx_source, view_source| {
                let log_messages = RefCell::new(QueueApplyQueueLogMessages::default());
                let stage = owner.apply_with_caller_preclaim_and_log_sinks(
                    tx_source,
                    view_source,
                    hold_preflight,
                    flags,
                    consequences,
                    can_be_held_result,
                    run_preflight,
                    trace,
                    |message| {
                        log_messages.borrow_mut().debug.push(message);
                    },
                    |message| {
                        log_messages.borrow_mut().info.push(message);
                    },
                    apply,
                    prepare_multitxn,
                    run_preclaim_stage,
                    run_try_clear,
                    apply_sandbox,
                );
                QueueApplyTopWithLogMessagesResult {
                    stage,
                    queue_log_messages: log_messages.into_inner(),
                }
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_caller_preclaim_and_log_sinks<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
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
        &self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
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
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
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
        self.with_owner_sources_current_app_view_caller_preclaim_log_sinks(
            owner,
            |owner, tx_source, view_source| {
                owner.apply_with_caller_preclaim_and_log_sinks(
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
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
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
        &self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
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
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
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
        self.with_owner_sources_current_app_view_log_sinks(
            owner,
            |owner, tx_source, view_source| {
                owner.apply_with_log_sinks(
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
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
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
        &self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
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
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
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
        self.with_owner_sources_current_app_view_log_sinks(
            owner,
            |owner, tx_source, view_source| {
                owner.apply_after_preflight_with_log_sinks(
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
    pub fn apply_after_preflight_with_caller_preclaim<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TraceFn,
        ApplyFn,
        PrepareMultiTxn,
        RunPreclaimStage,
        RunTryClear,
        TryClearResult,
        ApplySandbox,
    >(
        &self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
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
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
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
        self.with_owner_sources_current_app_view_caller_preclaim(
            owner,
            |owner, tx_source, view_source| {
                owner.apply_after_preflight_with_caller_preclaim(
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
    pub fn apply_after_preflight_with_caller_preclaim_and_log_messages<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TraceFn,
        ApplyFn,
        PrepareMultiTxn,
        RunPreclaimStage,
        RunTryClear,
        TryClearResult,
        ApplySandbox,
    >(
        &self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
        can_be_held_result: Ter,
        trace: TraceFn,
        apply: ApplyFn,
        prepare_multitxn: PrepareMultiTxn,
        run_preclaim_stage: RunPreclaimStage,
        run_try_clear: RunTryClear,
        apply_sandbox: ApplySandbox,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
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
        self.with_owner_sources_current_app_view_caller_preclaim(
            owner,
            |owner, tx_source, view_source| {
                let log_messages = RefCell::new(QueueApplyQueueLogMessages::default());
                let stage = owner.apply_after_preflight_with_caller_preclaim_and_log_sinks(
                    tx_source,
                    view_source,
                    preflight_result,
                    hold_preflight,
                    can_be_held_result,
                    trace,
                    |message| {
                        log_messages.borrow_mut().debug.push(message);
                    },
                    |message| {
                        log_messages.borrow_mut().info.push(message);
                    },
                    apply,
                    prepare_multitxn,
                    run_preclaim_stage,
                    run_try_clear,
                    apply_sandbox,
                );
                QueueApplyTopWithLogMessagesResult {
                    stage,
                    queue_log_messages: log_messages.into_inner(),
                }
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_caller_preclaim_and_log_sinks<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
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
        &self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
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
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
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
        self.with_owner_sources_current_app_view_caller_preclaim_log_sinks(
            owner,
            |owner, tx_source, view_source| {
                owner.apply_after_preflight_with_caller_preclaim_and_log_sinks(
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
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        RunPreflight,
        TraceFn,
        ApplyFn,
        PrepareMultiTxn,
        RunPreclaim,
        RunTryClear,
        TryClearResult,
        ApplySandbox,
    >(
        &self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
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
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
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
        self.with_owner_sources(owner, |owner, tx_source, view_source| {
            owner.apply_with_owned_metrics(
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
    pub fn apply_with_owned_metrics_and_log_messages<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        RunPreflight,
        TraceFn,
        ApplyFn,
        PrepareMultiTxn,
        RunPreclaim,
        RunTryClear,
        TryClearResult,
        ApplySandbox,
    >(
        &self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
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
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
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
        let queue_log_messages = RefCell::new(QueueApplyQueueLogMessages::default());
        let stage = self.apply_with_owned_metrics_and_log_sinks(
            owner,
            hold_preflight,
            flags,
            consequences,
            can_be_held_result,
            run_preflight,
            trace,
            |message| queue_log_messages.borrow_mut().debug.push(message),
            |message| queue_log_messages.borrow_mut().info.push(message),
            apply,
            prepare_multitxn,
            run_preclaim,
            run_try_clear,
            apply_sandbox,
        );
        QueueApplyTopWithLogMessagesResult {
            stage,
            queue_log_messages: queue_log_messages.into_inner(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_owned_metrics_and_derived_preflight_facts_and_log_messages<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        RunPreflight,
        TraceFn,
        ApplyFn,
        PrepareMultiTxn,
        RunPreclaim,
        RunTryClear,
        TryClearResult,
        ApplySandbox,
    >(
        &self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        run_preflight: RunPreflight,
        trace: TraceFn,
        apply: ApplyFn,
        prepare_multitxn: PrepareMultiTxn,
        run_preclaim: RunPreclaim,
        run_try_clear: RunTryClear,
        apply_sandbox: ApplySandbox,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
        TxId: Clone + Display,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
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
        let preflight_result = run_preflight();
        let hold_preflight =
            derive_queue_hold_preflight_from_tx_source(self.tx_source, preflight_result.flags);
        let queue_log_messages = RefCell::new(QueueApplyQueueLogMessages::default());
        let stage = self.apply_after_preflight_with_owned_metrics_and_log_sinks(
            owner,
            &preflight_result,
            hold_preflight,
            owner.owner().derive_can_be_held_result(
                self.tx_source,
                self.view_source,
                hold_preflight,
            ),
            trace,
            |message| queue_log_messages.borrow_mut().debug.push(message),
            |message| queue_log_messages.borrow_mut().info.push(message),
            apply,
            prepare_multitxn,
            run_preclaim,
            run_try_clear,
            apply_sandbox,
        );
        QueueApplyTopWithLogMessagesResult {
            stage,
            queue_log_messages: queue_log_messages.into_inner(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_owned_metrics_and_derived_preflight_facts_and_hold_admission_and_log_messages<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        RunPreflight,
        TraceFn,
        ApplyFn,
        PrepareMultiTxn,
        RunPreclaim,
        RunTryClear,
        TryClearResult,
        ApplySandbox,
    >(
        &self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        run_preflight: RunPreflight,
        trace: TraceFn,
        apply: ApplyFn,
        prepare_multitxn: PrepareMultiTxn,
        run_preclaim: RunPreclaim,
        run_try_clear: RunTryClear,
        apply_sandbox: ApplySandbox,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
        TxId: Clone + Display,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
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
        let preflight_result = run_preflight();
        let hold_preflight =
            derive_queue_hold_preflight_from_tx_source(self.tx_source, preflight_result.flags);
        let queue_log_messages = RefCell::new(QueueApplyQueueLogMessages::default());
        let stage = self.apply_after_preflight_with_owned_metrics_and_log_sinks(
            owner,
            &preflight_result,
            hold_preflight,
            owner.owner().derive_can_be_held_result(
                self.tx_source,
                self.view_source,
                hold_preflight,
            ),
            trace,
            |message| queue_log_messages.borrow_mut().debug.push(message),
            |message| queue_log_messages.borrow_mut().info.push(message),
            apply,
            prepare_multitxn,
            run_preclaim,
            run_try_clear,
            apply_sandbox,
        );
        QueueApplyTopWithLogMessagesResult {
            stage,
            queue_log_messages: queue_log_messages.into_inner(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_owned_metrics_and_derived_hold_preflight_and_log_messages<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        RunPreflight,
        TraceFn,
        ApplyFn,
        PrepareMultiTxn,
        RunPreclaim,
        RunTryClear,
        TryClearResult,
        ApplySandbox,
    >(
        &self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
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
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
        TxId: Clone + Display,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
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
        let hold_preflight = derive_queue_hold_preflight_from_tx_source(self.tx_source, flags);
        let queue_log_messages = RefCell::new(QueueApplyQueueLogMessages::default());
        let stage = self.apply_with_owned_metrics_and_log_sinks(
            owner,
            hold_preflight,
            flags,
            consequences,
            can_be_held_result,
            run_preflight,
            trace,
            |message| queue_log_messages.borrow_mut().debug.push(message),
            |message| queue_log_messages.borrow_mut().info.push(message),
            apply,
            prepare_multitxn,
            run_preclaim,
            run_try_clear,
            apply_sandbox,
        );
        QueueApplyTopWithLogMessagesResult {
            stage,
            queue_log_messages: queue_log_messages.into_inner(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_owned_metrics_and_derived_hold_admission_and_log_messages<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        RunPreflight,
        TraceFn,
        ApplyFn,
        PrepareMultiTxn,
        RunPreclaim,
        RunTryClear,
        TryClearResult,
        ApplySandbox,
    >(
        &self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
        flags: ApplyFlags,
        consequences: TxConsequences,
        run_preflight: RunPreflight,
        trace: TraceFn,
        apply: ApplyFn,
        prepare_multitxn: PrepareMultiTxn,
        run_preclaim: RunPreclaim,
        run_try_clear: RunTryClear,
        apply_sandbox: ApplySandbox,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
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
        let queue_log_messages = RefCell::new(QueueApplyQueueLogMessages::default());
        let stage = self.apply_with_owned_metrics_and_log_sinks(
            owner,
            hold_preflight,
            flags,
            consequences,
            owner.owner().derive_can_be_held_result(
                self.tx_source,
                self.view_source,
                hold_preflight,
            ),
            run_preflight,
            trace,
            |message| queue_log_messages.borrow_mut().debug.push(message),
            |message| queue_log_messages.borrow_mut().info.push(message),
            apply,
            prepare_multitxn,
            run_preclaim,
            run_try_clear,
            apply_sandbox,
        );
        QueueApplyTopWithLogMessagesResult {
            stage,
            queue_log_messages: queue_log_messages.into_inner(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_owned_metrics<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TraceFn,
        ApplyFn,
        PrepareMultiTxn,
        RunPreclaim,
        RunTryClear,
        TryClearResult,
        ApplySandbox,
    >(
        &self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
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
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
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
        self.with_owner_sources(owner, |owner, tx_source, view_source| {
            owner.apply_after_preflight_with_owned_metrics(
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

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_owned_metrics_and_log_messages<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TraceFn,
        ApplyFn,
        PrepareMultiTxn,
        RunPreclaim,
        RunTryClear,
        TryClearResult,
        ApplySandbox,
    >(
        &self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
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
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
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
        let queue_log_messages = RefCell::new(QueueApplyQueueLogMessages::default());
        let stage = self.apply_after_preflight_with_owned_metrics_and_log_sinks(
            owner,
            preflight_result,
            hold_preflight,
            can_be_held_result,
            trace,
            |message| queue_log_messages.borrow_mut().debug.push(message),
            |message| queue_log_messages.borrow_mut().info.push(message),
            apply,
            prepare_multitxn,
            run_preclaim,
            run_try_clear,
            apply_sandbox,
        );
        QueueApplyTopWithLogMessagesResult {
            stage,
            queue_log_messages: queue_log_messages.into_inner(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_owned_metrics_and_derived_hold_preflight_and_log_messages<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TraceFn,
        ApplyFn,
        PrepareMultiTxn,
        RunPreclaim,
        RunTryClear,
        TryClearResult,
        ApplySandbox,
    >(
        &self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        can_be_held_result: Ter,
        trace: TraceFn,
        apply: ApplyFn,
        prepare_multitxn: PrepareMultiTxn,
        run_preclaim: RunPreclaim,
        run_try_clear: RunTryClear,
        apply_sandbox: ApplySandbox,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
        TxId: Clone + Display,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
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
        let hold_preflight =
            derive_queue_hold_preflight_from_tx_source(self.tx_source, preflight_result.flags);
        let queue_log_messages = RefCell::new(QueueApplyQueueLogMessages::default());
        let stage = self.apply_after_preflight_with_owned_metrics_and_log_sinks(
            owner,
            preflight_result,
            hold_preflight,
            can_be_held_result,
            trace,
            |message| queue_log_messages.borrow_mut().debug.push(message),
            |message| queue_log_messages.borrow_mut().info.push(message),
            apply,
            prepare_multitxn,
            run_preclaim,
            run_try_clear,
            apply_sandbox,
        );
        QueueApplyTopWithLogMessagesResult {
            stage,
            queue_log_messages: queue_log_messages.into_inner(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_owned_metrics_and_caller_preclaim<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        RunPreflight,
        TraceFn,
        ApplyFn,
        PrepareMultiTxn,
        RunPreclaimStage,
        RunTryClear,
        TryClearResult,
        ApplySandbox,
    >(
        &self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
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
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
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
        self.with_owner_sources_current_app_view_caller_preclaim(
            owner,
            |owner, tx_source, view_source| {
                owner.apply_with_owned_metrics_and_caller_preclaim(
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
    pub fn apply_after_preflight_with_owned_metrics_and_caller_preclaim<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TraceFn,
        ApplyFn,
        PrepareMultiTxn,
        RunPreclaimStage,
        RunTryClear,
        TryClearResult,
        ApplySandbox,
    >(
        &self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
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
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
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
        self.with_owner_sources_current_app_view_caller_preclaim(
            owner,
            |owner, tx_source, view_source| {
                owner.apply_after_preflight_with_owned_metrics_and_caller_preclaim(
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
    pub fn apply_with_owned_metrics_and_log_sinks<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
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
        &self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
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
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
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
        self.with_owner_sources_current_app_view_log_sinks(
            owner,
            |owner, tx_source, view_source| {
                owner.apply_with_owned_metrics_and_log_sinks(
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
    pub fn apply_after_preflight_with_owned_metrics_and_log_sinks<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
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
        &self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
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
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
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
        self.with_owner_sources_current_app_view_log_sinks(
            owner,
            |owner, tx_source, view_source| {
                owner.apply_after_preflight_with_owned_metrics_and_log_sinks(
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
    pub fn apply_with_owned_metrics_and_caller_preclaim_and_log_messages<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        RunPreflight,
        TraceFn,
        ApplyFn,
        PrepareMultiTxn,
        RunPreclaimStage,
        RunTryClear,
        TryClearResult,
        ApplySandbox,
    >(
        &self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
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
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
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
        self.with_owner_sources_current_app_view_caller_preclaim_log_sinks(
            owner,
            |owner, tx_source, view_source| {
                let log_messages = RefCell::new(QueueApplyQueueLogMessages::default());
                let stage = owner.apply_with_owned_metrics_and_caller_preclaim_and_log_sinks(
                    tx_source,
                    view_source,
                    hold_preflight,
                    flags,
                    consequences,
                    can_be_held_result,
                    run_preflight,
                    trace,
                    |message| {
                        log_messages.borrow_mut().debug.push(message);
                    },
                    |message| {
                        log_messages.borrow_mut().info.push(message);
                    },
                    apply,
                    prepare_multitxn,
                    run_preclaim_stage,
                    run_try_clear,
                    apply_sandbox,
                );
                QueueApplyTopWithLogMessagesResult {
                    stage,
                    queue_log_messages: log_messages.into_inner(),
                }
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_owned_metrics_and_caller_preclaim_and_log_sinks<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
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
        &self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
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
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
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
        self.with_owner_sources_current_app_view_caller_preclaim_log_sinks(
            owner,
            |owner, tx_source, view_source| {
                owner.apply_with_owned_metrics_and_caller_preclaim_and_log_sinks(
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
    pub fn apply_after_preflight_with_owned_metrics_and_derived_hold_admission_and_log_messages<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TraceFn,
        ApplyFn,
        PrepareMultiTxn,
        RunPreclaim,
        RunTryClear,
        TryClearResult,
        ApplySandbox,
    >(
        &self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
        trace: TraceFn,
        apply: ApplyFn,
        prepare_multitxn: PrepareMultiTxn,
        run_preclaim: RunPreclaim,
        run_try_clear: RunTryClear,
        apply_sandbox: ApplySandbox,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
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
        let can_be_held_result = owner.owner().derive_can_be_held_result(
            self.tx_source,
            self.view_source,
            hold_preflight,
        );
        let queue_log_messages = RefCell::new(QueueApplyQueueLogMessages::default());
        let stage = self.apply_after_preflight_with_owned_metrics_and_log_sinks(
            owner,
            preflight_result,
            hold_preflight,
            can_be_held_result,
            trace,
            |message| queue_log_messages.borrow_mut().debug.push(message),
            |message| queue_log_messages.borrow_mut().info.push(message),
            apply,
            prepare_multitxn,
            run_preclaim,
            run_try_clear,
            apply_sandbox,
        );
        QueueApplyTopWithLogMessagesResult {
            stage,
            queue_log_messages: queue_log_messages.into_inner(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_owned_metrics_and_caller_preclaim_and_log_messages<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TraceFn,
        ApplyFn,
        PrepareMultiTxn,
        RunPreclaimStage,
        RunTryClear,
        TryClearResult,
        ApplySandbox,
    >(
        &self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
        can_be_held_result: Ter,
        trace: TraceFn,
        apply: ApplyFn,
        prepare_multitxn: PrepareMultiTxn,
        run_preclaim_stage: RunPreclaimStage,
        run_try_clear: RunTryClear,
        apply_sandbox: ApplySandbox,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
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
        self.with_owner_sources_current_app_view_caller_preclaim_log_sinks(
            owner,
            |owner, tx_source, view_source| {
                let log_messages = RefCell::new(QueueApplyQueueLogMessages::default());
                let stage = owner
                    .apply_after_preflight_with_owned_metrics_and_caller_preclaim_and_log_sinks(
                        tx_source,
                        view_source,
                        preflight_result,
                        hold_preflight,
                        can_be_held_result,
                        trace,
                        |message| {
                            log_messages.borrow_mut().debug.push(message);
                        },
                        |message| {
                            log_messages.borrow_mut().info.push(message);
                        },
                        apply,
                        prepare_multitxn,
                        run_preclaim_stage,
                        run_try_clear,
                        apply_sandbox,
                    );
                QueueApplyTopWithLogMessagesResult {
                    stage,
                    queue_log_messages: log_messages.into_inner(),
                }
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_owned_metrics_and_caller_preclaim_and_log_sinks<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
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
        &self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
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
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
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
        self.with_owner_sources_current_app_view_caller_preclaim_log_sinks(
            owner,
            |owner, tx_source, view_source| {
                owner.apply_after_preflight_with_owned_metrics_and_caller_preclaim_and_log_sinks(
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
}
