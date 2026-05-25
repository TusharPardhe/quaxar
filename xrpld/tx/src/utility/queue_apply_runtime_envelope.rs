//! Higher execution runtime wrapper for the current `xrpld`
//! `TxQ::apply(...)` seam.
//!
//! This layer moves one step closer to the the reference implementation call shape by letting
//! one runtime object own:
//! 1. `preflight(...)`,
//! 2. trace emission for direct-apply attempts,
//! 3. direct-apply execution,
//! 4. multi-txn preparation,
//! 5. preclaim execution,
//! 6. try-clear execution,
//! 7. sandbox application.

use std::cell::RefCell;
use std::fmt::Display;

use protocol::Ter;

use crate::{
    ApplyFlags, PreclaimResult, PreflightResult, QueueApplyCallEnvelope,
    QueueApplyHoldPreflightTxSource, QueueApplyOwnerShell, QueueApplyPreclaimStage,
    QueueApplyPreclaimViewSource, QueueApplyPreflightStage, QueueApplyPreparedPreclaimInputs,
    QueueApplyTopWithLogMessagesResult, QueueApplyViewAdjustment, QueueHoldPreflight,
    TryClearAccountResult, TxConsequences, derive_queue_hold_preflight_from_tx_source,
};

pub trait QueueApplyExecutionRuntime<Tx, Journal, ParentBatchId> {
    fn run_preflight(&mut self) -> PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>;

    fn trace(&mut self, message: &str);

    fn direct_apply(&mut self) -> crate::ApplyResult;

    fn prepare_multitxn(&mut self, adjustment: QueueApplyViewAdjustment) -> bool;

    fn run_preclaim(
        &mut self,
        view_source: QueueApplyPreclaimViewSource,
    ) -> PreclaimResult<Tx, Journal, ParentBatchId>;

    fn run_try_clear(&mut self) -> crate::ApplyResult;

    fn apply_sandbox(&mut self);
}

pub trait QueueApplyCurrentPreclaimClearRuntime<Tx, Journal, ParentBatchId> {
    fn run_try_clear_with_current_preclaim(&mut self) -> TryClearAccountResult;
}

#[derive(Debug)]
pub struct QueueApplyRuntimeEnvelope<'a, Runtime> {
    runtime: &'a mut Runtime,
}

impl<'a, Runtime> QueueApplyRuntimeEnvelope<'a, Runtime> {
    pub fn new(runtime: &'a mut Runtime) -> Self {
        Self { runtime }
    }

    pub fn runtime(&mut self) -> &mut Runtime {
        self.runtime
    }
}

impl<'a, Runtime> QueueApplyRuntimeEnvelope<'a, Runtime> {
    fn derive_queue_apply_hold_preflight<Tx, Journal, ParentBatchId, TxSource, ViewSource>(
        call: &QueueApplyCallEnvelope<'_, TxSource, ViewSource>,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    ) -> QueueHoldPreflight
    where
        TxSource: crate::QueueApplyHoldPreflightTxSource,
    {
        derive_queue_hold_preflight_from_tx_source(call.tx_source(), preflight_result.flags)
    }

    fn run_preflight_and_derive_hold_preflight<Tx, Journal, ParentBatchId, TxSource, ViewSource>(
        &mut self,
        call: &QueueApplyCallEnvelope<'_, TxSource, ViewSource>,
    ) -> (
        PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        QueueHoldPreflight,
    )
    where
        Runtime: QueueApplyExecutionRuntime<Tx, Journal, ParentBatchId>,
        TxSource: crate::QueueApplyHoldPreflightTxSource,
    {
        let preflight_result = self.runtime.run_preflight();
        let hold_preflight = Self::derive_queue_apply_hold_preflight(call, &preflight_result);
        (preflight_result, hold_preflight)
    }

    fn with_runtime_refcell<R>(&mut self, run: impl FnOnce(&RefCell<&mut Runtime>) -> R) -> R {
        let runtime = RefCell::new(&mut *self.runtime);
        run(&runtime)
    }

    fn with_runtime_refcell_current_app_view<R>(
        &mut self,
        run: impl FnOnce(&RefCell<&mut Runtime>) -> R,
    ) -> R {
        self.with_runtime_refcell(run)
    }

    fn with_runtime_refcell_current_app_view_caller_preclaim<R>(
        &mut self,
        run: impl FnOnce(&RefCell<&mut Runtime>) -> R,
    ) -> R {
        self.with_runtime_refcell(run)
    }

    fn with_runtime_refcell_current_app_view_caller_preclaim_log_sinks<R>(
        &mut self,
        run: impl FnOnce(&RefCell<&mut Runtime>) -> R,
    ) -> R {
        self.with_runtime_refcell_current_app_view_caller_preclaim(run)
    }

    fn with_runtime_refcell_current_app_view_log_sinks<R>(
        &mut self,
        run: impl FnOnce(&RefCell<&mut Runtime>) -> R,
    ) -> R {
        self.with_runtime_refcell(run)
    }

    pub fn apply_with_derived_preflight_facts<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
        ViewSource,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        call: &QueueApplyCallEnvelope<'_, TxSource, ViewSource>,
        can_be_held_result: Ter,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
        TxId: Clone + Display,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: crate::QueueApplyObservedViewSource<Account>,
        Runtime: QueueApplyExecutionRuntime<Tx, Journal, ParentBatchId>,
    {
        let (preflight_result, hold_preflight) = self.run_preflight_and_derive_hold_preflight(call);
        self.with_runtime_refcell_current_app_view(|runtime| {
            call.apply_after_preflight(
                owner,
                &preflight_result,
                hold_preflight,
                can_be_held_result,
                |message| runtime.borrow_mut().trace(message),
                || runtime.borrow_mut().direct_apply(),
                |adjustment| runtime.borrow_mut().prepare_multitxn(adjustment),
                |view_source| runtime.borrow_mut().run_preclaim(view_source),
                || runtime.borrow_mut().run_try_clear(),
                || runtime.borrow_mut().apply_sandbox(),
            )
        })
    }

    pub fn apply_with_derived_preflight_facts_and_log_messages<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
        ViewSource,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        call: &QueueApplyCallEnvelope<'_, TxSource, ViewSource>,
        can_be_held_result: Ter,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
        TxId: Clone + Display,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: crate::QueueApplyObservedViewSource<Account>,
        Runtime: QueueApplyExecutionRuntime<Tx, Journal, ParentBatchId>,
    {
        self.with_runtime_refcell_current_app_view(|runtime| {
            call.apply_with_derived_preflight_facts_and_log_messages(
                owner,
                can_be_held_result,
                || runtime.borrow_mut().run_preflight(),
                |message| runtime.borrow_mut().trace(message),
                || runtime.borrow_mut().direct_apply(),
                |adjustment| runtime.borrow_mut().prepare_multitxn(adjustment),
                |view_source| runtime.borrow_mut().run_preclaim(view_source),
                || runtime.borrow_mut().run_try_clear(),
                || runtime.borrow_mut().apply_sandbox(),
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
        ViewSource,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        call: &QueueApplyCallEnvelope<'_, TxSource, ViewSource>,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
        TxId: Clone + Display,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: crate::QueueApplyObservedViewSource<Account>,
        Runtime: QueueApplyExecutionRuntime<Tx, Journal, ParentBatchId>,
    {
        let (preflight_result, hold_preflight) = self.run_preflight_and_derive_hold_preflight(call);
        let can_be_held_result = owner.owner().derive_can_be_held_result(
            call.tx_source(),
            call.view_source(),
            hold_preflight,
        );
        self.with_runtime_refcell_current_app_view(|runtime| {
            call.apply_after_preflight(
                owner,
                &preflight_result,
                hold_preflight,
                can_be_held_result,
                |message| runtime.borrow_mut().trace(message),
                || runtime.borrow_mut().direct_apply(),
                |adjustment| runtime.borrow_mut().prepare_multitxn(adjustment),
                |view_source| runtime.borrow_mut().run_preclaim(view_source),
                || runtime.borrow_mut().run_try_clear(),
                || runtime.borrow_mut().apply_sandbox(),
            )
        })
    }

    pub fn apply_with_derived_preflight_facts_and_hold_admission_and_log_messages<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
        ViewSource,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        call: &QueueApplyCallEnvelope<'_, TxSource, ViewSource>,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
        TxId: Clone + Display,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: crate::QueueApplyObservedViewSource<Account>,
        Runtime: QueueApplyExecutionRuntime<Tx, Journal, ParentBatchId>,
    {
        self.with_runtime_refcell_current_app_view(|runtime| {
            call.apply_with_derived_preflight_facts_and_hold_admission_and_log_messages(
                owner,
                || runtime.borrow_mut().run_preflight(),
                |message| runtime.borrow_mut().trace(message),
                || runtime.borrow_mut().direct_apply(),
                |adjustment| runtime.borrow_mut().prepare_multitxn(adjustment),
                |view_source| runtime.borrow_mut().run_preclaim(view_source),
                || runtime.borrow_mut().run_try_clear(),
                || runtime.borrow_mut().apply_sandbox(),
            )
        })
    }

    pub fn apply<Account, Tx, Journal, ParentBatchId, TxId, TxSource, ViewSource>(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        call: &QueueApplyCallEnvelope<'_, TxSource, ViewSource>,
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
        TxSource: crate::QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: crate::QueueApplyObservedViewSource<Account>,
        Runtime: QueueApplyExecutionRuntime<Tx, Journal, ParentBatchId>,
    {
        self.with_runtime_refcell_current_app_view(|runtime| {
            call.apply(
                owner,
                hold_preflight,
                flags,
                consequences,
                can_be_held_result,
                || runtime.borrow_mut().run_preflight(),
                |message| runtime.borrow_mut().trace(message),
                || runtime.borrow_mut().direct_apply(),
                |adjustment| runtime.borrow_mut().prepare_multitxn(adjustment),
                |view_source| runtime.borrow_mut().run_preclaim(view_source),
                || runtime.borrow_mut().run_try_clear(),
                || runtime.borrow_mut().apply_sandbox(),
            )
        })
    }

    pub fn apply_with_log_messages<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
        ViewSource,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        call: &QueueApplyCallEnvelope<'_, TxSource, ViewSource>,
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
        TxSource: crate::QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: crate::QueueApplyObservedViewSource<Account>,
        Runtime: QueueApplyExecutionRuntime<Tx, Journal, ParentBatchId>,
    {
        self.with_runtime_refcell_current_app_view(|runtime| {
            call.apply_with_log_messages(
                owner,
                hold_preflight,
                flags,
                consequences,
                can_be_held_result,
                || runtime.borrow_mut().run_preflight(),
                |message| runtime.borrow_mut().trace(message),
                || runtime.borrow_mut().direct_apply(),
                |adjustment| runtime.borrow_mut().prepare_multitxn(adjustment),
                |view_source| runtime.borrow_mut().run_preclaim(view_source),
                || runtime.borrow_mut().run_try_clear(),
                || runtime.borrow_mut().apply_sandbox(),
            )
        })
    }

    pub fn apply_with_derived_hold_preflight_and_log_messages<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
        ViewSource,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        call: &QueueApplyCallEnvelope<'_, TxSource, ViewSource>,
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
        TxSource: crate::QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: crate::QueueApplyObservedViewSource<Account>,
        Runtime: QueueApplyExecutionRuntime<Tx, Journal, ParentBatchId>,
    {
        self.with_runtime_refcell_current_app_view(|runtime| {
            call.apply_with_derived_hold_preflight_and_log_messages(
                owner,
                flags,
                consequences,
                can_be_held_result,
                || runtime.borrow_mut().run_preflight(),
                |message| runtime.borrow_mut().trace(message),
                || runtime.borrow_mut().direct_apply(),
                |adjustment| runtime.borrow_mut().prepare_multitxn(adjustment),
                |view_source| runtime.borrow_mut().run_preclaim(view_source),
                || runtime.borrow_mut().run_try_clear(),
                || runtime.borrow_mut().apply_sandbox(),
            )
        })
    }

    pub fn apply_with_derived_hold_admission_and_log_messages<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
        ViewSource,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        call: &QueueApplyCallEnvelope<'_, TxSource, ViewSource>,
        hold_preflight: QueueHoldPreflight,
        flags: ApplyFlags,
        consequences: TxConsequences,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
        TxId: Clone + Display,
        TxSource: crate::QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: crate::QueueApplyObservedViewSource<Account>,
        Runtime: QueueApplyExecutionRuntime<Tx, Journal, ParentBatchId>,
    {
        self.with_runtime_refcell_current_app_view(|runtime| {
            call.apply_with_derived_hold_admission_and_log_messages(
                owner,
                hold_preflight,
                flags,
                consequences,
                || runtime.borrow_mut().run_preflight(),
                |message| runtime.borrow_mut().trace(message),
                || runtime.borrow_mut().direct_apply(),
                |adjustment| runtime.borrow_mut().prepare_multitxn(adjustment),
                |view_source| runtime.borrow_mut().run_preclaim(view_source),
                || runtime.borrow_mut().run_try_clear(),
                || runtime.borrow_mut().apply_sandbox(),
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
        ViewSource,
        RunPreclaimStage,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        call: &QueueApplyCallEnvelope<'_, TxSource, ViewSource>,
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
        TxSource: crate::QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: crate::QueueApplyObservedViewSource<Account>,
        Runtime: QueueApplyCurrentPreclaimClearRuntime<Tx, Journal, ParentBatchId>
            + QueueApplyExecutionRuntime<Tx, Journal, ParentBatchId>,
        RunPreclaimStage: FnOnce(
            QueueApplyPreparedPreclaimInputs<Account>,
        ) -> Result<
            QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>,
            crate::ApplyResult,
        >,
    {
        self.with_runtime_refcell_current_app_view_caller_preclaim(|runtime| {
            owner.apply_with_caller_preclaim(
                call.tx_source(),
                call.view_source(),
                hold_preflight,
                flags,
                consequences,
                can_be_held_result,
                || runtime.borrow_mut().run_preflight(),
                |message| runtime.borrow_mut().trace(message),
                || runtime.borrow_mut().direct_apply(),
                |adjustment| runtime.borrow_mut().prepare_multitxn(adjustment),
                run_preclaim_stage,
                || runtime.borrow_mut().run_try_clear_with_current_preclaim(),
                || runtime.borrow_mut().apply_sandbox(),
            )
        })
    }

    pub fn apply_after_preflight<Account, Tx, Journal, ParentBatchId, TxId, TxSource, ViewSource>(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        call: &QueueApplyCallEnvelope<'_, TxSource, ViewSource>,
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
        TxSource: crate::QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: crate::QueueApplyObservedViewSource<Account>,
        Runtime: QueueApplyExecutionRuntime<Tx, Journal, ParentBatchId>,
    {
        self.with_runtime_refcell_current_app_view(|runtime| {
            call.apply_after_preflight(
                owner,
                preflight_result,
                hold_preflight,
                can_be_held_result,
                |message| runtime.borrow_mut().trace(message),
                || runtime.borrow_mut().direct_apply(),
                |adjustment| runtime.borrow_mut().prepare_multitxn(adjustment),
                |view_source| runtime.borrow_mut().run_preclaim(view_source),
                || runtime.borrow_mut().run_try_clear(),
                || runtime.borrow_mut().apply_sandbox(),
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
        ViewSource,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        call: &QueueApplyCallEnvelope<'_, TxSource, ViewSource>,
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
        TxSource: crate::QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: crate::QueueApplyObservedViewSource<Account>,
        Runtime: QueueApplyExecutionRuntime<Tx, Journal, ParentBatchId>,
    {
        self.with_runtime_refcell_current_app_view(|runtime| {
            call.apply_after_preflight_with_log_messages(
                owner,
                preflight_result,
                hold_preflight,
                can_be_held_result,
                |message| runtime.borrow_mut().trace(message),
                || runtime.borrow_mut().direct_apply(),
                |adjustment| runtime.borrow_mut().prepare_multitxn(adjustment),
                |view_source| runtime.borrow_mut().run_preclaim(view_source),
                || runtime.borrow_mut().run_try_clear(),
                || runtime.borrow_mut().apply_sandbox(),
            )
        })
    }

    pub fn apply_after_preflight_with_derived_hold_preflight_and_log_messages<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
        ViewSource,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        call: &QueueApplyCallEnvelope<'_, TxSource, ViewSource>,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        can_be_held_result: Ter,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
        TxId: Clone + Display,
        TxSource: crate::QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: crate::QueueApplyObservedViewSource<Account>,
        Runtime: QueueApplyExecutionRuntime<Tx, Journal, ParentBatchId>,
    {
        self.with_runtime_refcell_current_app_view(|runtime| {
            call.apply_after_preflight_with_derived_hold_preflight_and_log_messages(
                owner,
                preflight_result,
                can_be_held_result,
                |message| runtime.borrow_mut().trace(message),
                || runtime.borrow_mut().direct_apply(),
                |adjustment| runtime.borrow_mut().prepare_multitxn(adjustment),
                |view_source| runtime.borrow_mut().run_preclaim(view_source),
                || runtime.borrow_mut().run_try_clear(),
                || runtime.borrow_mut().apply_sandbox(),
            )
        })
    }

    pub fn apply_after_preflight_with_derived_hold_admission_and_log_messages<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
        ViewSource,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        call: &QueueApplyCallEnvelope<'_, TxSource, ViewSource>,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
        TxId: Clone + Display,
        TxSource: crate::QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: crate::QueueApplyObservedViewSource<Account>,
        Runtime: QueueApplyExecutionRuntime<Tx, Journal, ParentBatchId>,
    {
        self.with_runtime_refcell_current_app_view(|runtime| {
            call.apply_after_preflight_with_derived_hold_admission_and_log_messages(
                owner,
                preflight_result,
                hold_preflight,
                |message| runtime.borrow_mut().trace(message),
                || runtime.borrow_mut().direct_apply(),
                |adjustment| runtime.borrow_mut().prepare_multitxn(adjustment),
                |view_source| runtime.borrow_mut().run_preclaim(view_source),
                || runtime.borrow_mut().run_try_clear(),
                || runtime.borrow_mut().apply_sandbox(),
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_caller_preclaim_and_log_messages<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
        ViewSource,
        RunPreclaimStage,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        call: &QueueApplyCallEnvelope<'_, TxSource, ViewSource>,
        hold_preflight: QueueHoldPreflight,
        flags: ApplyFlags,
        consequences: TxConsequences,
        can_be_held_result: Ter,
        run_preclaim_stage: RunPreclaimStage,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
        TxId: Clone + Display,
        TxSource: crate::QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: crate::QueueApplyObservedViewSource<Account>,
        Runtime: QueueApplyCurrentPreclaimClearRuntime<Tx, Journal, ParentBatchId>
            + QueueApplyExecutionRuntime<Tx, Journal, ParentBatchId>,
        RunPreclaimStage: FnOnce(
            QueueApplyPreparedPreclaimInputs<Account>,
        ) -> Result<
            QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>,
            crate::ApplyResult,
        >,
    {
        self.with_runtime_refcell_current_app_view_caller_preclaim(|runtime| {
            call.apply_with_caller_preclaim_and_log_messages(
                owner,
                hold_preflight,
                flags,
                consequences,
                can_be_held_result,
                || runtime.borrow_mut().run_preflight(),
                |message| runtime.borrow_mut().trace(message),
                || runtime.borrow_mut().direct_apply(),
                |adjustment| runtime.borrow_mut().prepare_multitxn(adjustment),
                run_preclaim_stage,
                || runtime.borrow_mut().run_try_clear_with_current_preclaim(),
                || runtime.borrow_mut().apply_sandbox(),
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_caller_preclaim<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
        ViewSource,
        RunPreclaimStage,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        call: &QueueApplyCallEnvelope<'_, TxSource, ViewSource>,
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
        TxSource: crate::QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: crate::QueueApplyObservedViewSource<Account>,
        Runtime: QueueApplyCurrentPreclaimClearRuntime<Tx, Journal, ParentBatchId>
            + QueueApplyExecutionRuntime<Tx, Journal, ParentBatchId>,
        RunPreclaimStage: FnOnce(
            QueueApplyPreparedPreclaimInputs<Account>,
        ) -> Result<
            QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>,
            crate::ApplyResult,
        >,
    {
        self.with_runtime_refcell_current_app_view_caller_preclaim(|runtime| {
            owner.apply_after_preflight_with_caller_preclaim(
                call.tx_source(),
                call.view_source(),
                preflight_result,
                hold_preflight,
                can_be_held_result,
                |message| runtime.borrow_mut().trace(message),
                || runtime.borrow_mut().direct_apply(),
                |adjustment| runtime.borrow_mut().prepare_multitxn(adjustment),
                run_preclaim_stage,
                || runtime.borrow_mut().run_try_clear_with_current_preclaim(),
                || runtime.borrow_mut().apply_sandbox(),
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_caller_preclaim_and_log_messages<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
        ViewSource,
        RunPreclaimStage,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        call: &QueueApplyCallEnvelope<'_, TxSource, ViewSource>,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
        can_be_held_result: Ter,
        run_preclaim_stage: RunPreclaimStage,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
        TxId: Clone + Display,
        TxSource: crate::QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: crate::QueueApplyObservedViewSource<Account>,
        Runtime: QueueApplyCurrentPreclaimClearRuntime<Tx, Journal, ParentBatchId>
            + QueueApplyExecutionRuntime<Tx, Journal, ParentBatchId>,
        RunPreclaimStage: FnOnce(
            QueueApplyPreparedPreclaimInputs<Account>,
        ) -> Result<
            QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>,
            crate::ApplyResult,
        >,
    {
        self.with_runtime_refcell_current_app_view_caller_preclaim(|runtime| {
            call.apply_after_preflight_with_caller_preclaim_and_log_messages(
                owner,
                preflight_result,
                hold_preflight,
                can_be_held_result,
                |message| runtime.borrow_mut().trace(message),
                || runtime.borrow_mut().direct_apply(),
                |adjustment| runtime.borrow_mut().prepare_multitxn(adjustment),
                run_preclaim_stage,
                || runtime.borrow_mut().run_try_clear_with_current_preclaim(),
                || runtime.borrow_mut().apply_sandbox(),
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_caller_preclaim_and_log_sinks<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
        ViewSource,
        RunPreclaimStage,
        TraceFn,
        DebugFn,
        InfoFn,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        call: &QueueApplyCallEnvelope<'_, TxSource, ViewSource>,
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
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
        TxId: Clone + Display,
        TxSource: crate::QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: crate::QueueApplyObservedViewSource<Account>,
        Runtime: QueueApplyCurrentPreclaimClearRuntime<Tx, Journal, ParentBatchId>
            + QueueApplyExecutionRuntime<Tx, Journal, ParentBatchId>,
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
        self.with_runtime_refcell_current_app_view_caller_preclaim_log_sinks(|runtime| {
            call.apply_with_caller_preclaim_and_log_sinks(
                owner,
                hold_preflight,
                flags,
                consequences,
                can_be_held_result,
                || runtime.borrow_mut().run_preflight(),
                trace,
                debug,
                info,
                || runtime.borrow_mut().direct_apply(),
                |adjustment| runtime.borrow_mut().prepare_multitxn(adjustment),
                run_preclaim_stage,
                || runtime.borrow_mut().run_try_clear_with_current_preclaim(),
                || runtime.borrow_mut().apply_sandbox(),
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_log_sinks<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
        ViewSource,
        TraceFn,
        DebugFn,
        InfoFn,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        call: &QueueApplyCallEnvelope<'_, TxSource, ViewSource>,
        hold_preflight: QueueHoldPreflight,
        flags: ApplyFlags,
        consequences: TxConsequences,
        can_be_held_result: Ter,
        trace: TraceFn,
        debug: DebugFn,
        info: InfoFn,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
        TxId: Clone + Display,
        TxSource: crate::QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: crate::QueueApplyObservedViewSource<Account>,
        Runtime: QueueApplyExecutionRuntime<Tx, Journal, ParentBatchId>,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
    {
        self.with_runtime_refcell_current_app_view_log_sinks(|runtime| {
            call.apply_with_log_sinks(
                owner,
                hold_preflight,
                flags,
                consequences,
                can_be_held_result,
                || runtime.borrow_mut().run_preflight(),
                trace,
                debug,
                info,
                || runtime.borrow_mut().direct_apply(),
                |adjustment| runtime.borrow_mut().prepare_multitxn(adjustment),
                |view_source| runtime.borrow_mut().run_preclaim(view_source),
                || runtime.borrow_mut().run_try_clear(),
                || runtime.borrow_mut().apply_sandbox(),
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_log_sinks_and_derived_preflight_facts<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
        ViewSource,
        TraceFn,
        DebugFn,
        InfoFn,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        call: &QueueApplyCallEnvelope<'_, TxSource, ViewSource>,
        can_be_held_result: Ter,
        trace: TraceFn,
        debug: DebugFn,
        info: InfoFn,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
        TxId: Clone + Display,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: crate::QueueApplyObservedViewSource<Account>,
        Runtime: QueueApplyExecutionRuntime<Tx, Journal, ParentBatchId>,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
    {
        let (preflight_result, hold_preflight) = self.run_preflight_and_derive_hold_preflight(call);
        self.with_runtime_refcell_current_app_view_log_sinks(|runtime| {
            call.apply_after_preflight_with_log_sinks(
                owner,
                &preflight_result,
                hold_preflight,
                can_be_held_result,
                trace,
                debug,
                info,
                || runtime.borrow_mut().direct_apply(),
                |adjustment| runtime.borrow_mut().prepare_multitxn(adjustment),
                |view_source| runtime.borrow_mut().run_preclaim(view_source),
                || runtime.borrow_mut().run_try_clear(),
                || runtime.borrow_mut().apply_sandbox(),
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_log_sinks_and_derived_preflight_facts_and_hold_admission<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
        ViewSource,
        TraceFn,
        DebugFn,
        InfoFn,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        call: &QueueApplyCallEnvelope<'_, TxSource, ViewSource>,
        trace: TraceFn,
        debug: DebugFn,
        info: InfoFn,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
        TxId: Clone + Display,
        TxSource: QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: crate::QueueApplyObservedViewSource<Account>,
        Runtime: QueueApplyExecutionRuntime<Tx, Journal, ParentBatchId>,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
    {
        let (preflight_result, hold_preflight) = self.run_preflight_and_derive_hold_preflight(call);
        let can_be_held_result = owner.owner().derive_can_be_held_result(
            call.tx_source(),
            call.view_source(),
            hold_preflight,
        );
        self.with_runtime_refcell_current_app_view_log_sinks(|runtime| {
            call.apply_after_preflight_with_log_sinks(
                owner,
                &preflight_result,
                hold_preflight,
                can_be_held_result,
                trace,
                debug,
                info,
                || runtime.borrow_mut().direct_apply(),
                |adjustment| runtime.borrow_mut().prepare_multitxn(adjustment),
                |view_source| runtime.borrow_mut().run_preclaim(view_source),
                || runtime.borrow_mut().run_try_clear(),
                || runtime.borrow_mut().apply_sandbox(),
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_log_sinks<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
        ViewSource,
        TraceFn,
        DebugFn,
        InfoFn,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        call: &QueueApplyCallEnvelope<'_, TxSource, ViewSource>,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
        can_be_held_result: Ter,
        trace: TraceFn,
        debug: DebugFn,
        info: InfoFn,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
        TxId: Clone + Display,
        TxSource: crate::QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: crate::QueueApplyObservedViewSource<Account>,
        Runtime: QueueApplyExecutionRuntime<Tx, Journal, ParentBatchId>,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
    {
        self.with_runtime_refcell_current_app_view_log_sinks(|runtime| {
            call.apply_after_preflight_with_log_sinks(
                owner,
                preflight_result,
                hold_preflight,
                can_be_held_result,
                trace,
                debug,
                info,
                || runtime.borrow_mut().direct_apply(),
                |adjustment| runtime.borrow_mut().prepare_multitxn(adjustment),
                |view_source| runtime.borrow_mut().run_preclaim(view_source),
                || runtime.borrow_mut().run_try_clear(),
                || runtime.borrow_mut().apply_sandbox(),
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_caller_preclaim_and_log_sinks<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
        ViewSource,
        RunPreclaimStage,
        TraceFn,
        DebugFn,
        InfoFn,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        call: &QueueApplyCallEnvelope<'_, TxSource, ViewSource>,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
        can_be_held_result: Ter,
        run_preclaim_stage: RunPreclaimStage,
        trace: TraceFn,
        debug: DebugFn,
        info: InfoFn,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
        TxId: Clone + Display,
        TxSource: crate::QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: crate::QueueApplyObservedViewSource<Account>,
        Runtime: QueueApplyCurrentPreclaimClearRuntime<Tx, Journal, ParentBatchId>
            + QueueApplyExecutionRuntime<Tx, Journal, ParentBatchId>,
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
        self.with_runtime_refcell_current_app_view_caller_preclaim_log_sinks(|runtime| {
            call.apply_after_preflight_with_caller_preclaim_and_log_sinks(
                owner,
                preflight_result,
                hold_preflight,
                can_be_held_result,
                trace,
                debug,
                info,
                || runtime.borrow_mut().direct_apply(),
                |adjustment| runtime.borrow_mut().prepare_multitxn(adjustment),
                run_preclaim_stage,
                || runtime.borrow_mut().run_try_clear_with_current_preclaim(),
                || runtime.borrow_mut().apply_sandbox(),
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_owned_metrics<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
        ViewSource,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        call: &QueueApplyCallEnvelope<'_, TxSource, ViewSource>,
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
        TxSource: crate::QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: crate::QueueApplyObservedViewSource<Account>,
        Runtime: QueueApplyExecutionRuntime<Tx, Journal, ParentBatchId>,
    {
        self.with_runtime_refcell_current_app_view(|runtime| {
            call.apply_with_owned_metrics(
                owner,
                hold_preflight,
                flags,
                consequences,
                can_be_held_result,
                || runtime.borrow_mut().run_preflight(),
                |message| runtime.borrow_mut().trace(message),
                || runtime.borrow_mut().direct_apply(),
                |adjustment| runtime.borrow_mut().prepare_multitxn(adjustment),
                |view_source| runtime.borrow_mut().run_preclaim(view_source),
                || runtime.borrow_mut().run_try_clear(),
                || runtime.borrow_mut().apply_sandbox(),
            )
        })
    }

    pub fn apply_with_owned_metrics_and_log_messages<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
        ViewSource,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        call: &QueueApplyCallEnvelope<'_, TxSource, ViewSource>,
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
        TxSource: crate::QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: crate::QueueApplyObservedViewSource<Account>,
        Runtime: QueueApplyExecutionRuntime<Tx, Journal, ParentBatchId>,
    {
        self.with_runtime_refcell_current_app_view(|runtime| {
            call.apply_with_owned_metrics_and_log_messages(
                owner,
                hold_preflight,
                flags,
                consequences,
                can_be_held_result,
                || runtime.borrow_mut().run_preflight(),
                |message| runtime.borrow_mut().trace(message),
                || runtime.borrow_mut().direct_apply(),
                |adjustment| runtime.borrow_mut().prepare_multitxn(adjustment),
                |view_source| runtime.borrow_mut().run_preclaim(view_source),
                || runtime.borrow_mut().run_try_clear(),
                || runtime.borrow_mut().apply_sandbox(),
            )
        })
    }

    pub fn apply_with_owned_metrics_and_derived_preflight_facts_and_log_messages<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
        ViewSource,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        call: &QueueApplyCallEnvelope<'_, TxSource, ViewSource>,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
        TxId: Clone + Display,
        TxSource: crate::QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: crate::QueueApplyObservedViewSource<Account>,
        Runtime: QueueApplyExecutionRuntime<Tx, Journal, ParentBatchId>,
    {
        self.with_runtime_refcell_current_app_view(|runtime| {
            call.apply_with_owned_metrics_and_derived_preflight_facts_and_log_messages(
                owner,
                || runtime.borrow_mut().run_preflight(),
                |message| runtime.borrow_mut().trace(message),
                || runtime.borrow_mut().direct_apply(),
                |adjustment| runtime.borrow_mut().prepare_multitxn(adjustment),
                |view_source| runtime.borrow_mut().run_preclaim(view_source),
                || runtime.borrow_mut().run_try_clear(),
                || runtime.borrow_mut().apply_sandbox(),
            )
        })
    }

    pub fn apply_with_owned_metrics_and_derived_preflight_facts_and_hold_admission_and_log_messages<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
        ViewSource,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        call: &QueueApplyCallEnvelope<'_, TxSource, ViewSource>,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
        TxId: Clone + Display,
        TxSource: crate::QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: crate::QueueApplyObservedViewSource<Account>,
        Runtime: QueueApplyExecutionRuntime<Tx, Journal, ParentBatchId>,
    {
        self.with_runtime_refcell_current_app_view(|runtime| {
            call.apply_with_owned_metrics_and_derived_preflight_facts_and_hold_admission_and_log_messages(
                owner,
                || runtime.borrow_mut().run_preflight(),
                |message| runtime.borrow_mut().trace(message),
                || runtime.borrow_mut().direct_apply(),
                |adjustment| runtime.borrow_mut().prepare_multitxn(adjustment),
                |view_source| runtime.borrow_mut().run_preclaim(view_source),
                || runtime.borrow_mut().run_try_clear(),
                || runtime.borrow_mut().apply_sandbox(),
            )
        })
    }

    pub fn apply_with_owned_metrics_and_derived_hold_preflight_and_log_messages<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
        ViewSource,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        call: &QueueApplyCallEnvelope<'_, TxSource, ViewSource>,
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
        TxSource: crate::QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: crate::QueueApplyObservedViewSource<Account>,
        Runtime: QueueApplyExecutionRuntime<Tx, Journal, ParentBatchId>,
    {
        self.with_runtime_refcell_current_app_view(|runtime| {
            call.apply_with_owned_metrics_and_derived_hold_preflight_and_log_messages(
                owner,
                flags,
                consequences,
                can_be_held_result,
                || runtime.borrow_mut().run_preflight(),
                |message| runtime.borrow_mut().trace(message),
                || runtime.borrow_mut().direct_apply(),
                |adjustment| runtime.borrow_mut().prepare_multitxn(adjustment),
                |view_source| runtime.borrow_mut().run_preclaim(view_source),
                || runtime.borrow_mut().run_try_clear(),
                || runtime.borrow_mut().apply_sandbox(),
            )
        })
    }

    pub fn apply_with_owned_metrics_and_derived_hold_admission_and_log_messages<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
        ViewSource,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        call: &QueueApplyCallEnvelope<'_, TxSource, ViewSource>,
        hold_preflight: QueueHoldPreflight,
        flags: ApplyFlags,
        consequences: TxConsequences,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
        TxId: Clone + Display,
        TxSource: crate::QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: crate::QueueApplyObservedViewSource<Account>,
        Runtime: QueueApplyExecutionRuntime<Tx, Journal, ParentBatchId>,
    {
        self.with_runtime_refcell_current_app_view(|runtime| {
            call.apply_with_owned_metrics_and_derived_hold_admission_and_log_messages(
                owner,
                hold_preflight,
                flags,
                consequences,
                || runtime.borrow_mut().run_preflight(),
                |message| runtime.borrow_mut().trace(message),
                || runtime.borrow_mut().direct_apply(),
                |adjustment| runtime.borrow_mut().prepare_multitxn(adjustment),
                |view_source| runtime.borrow_mut().run_preclaim(view_source),
                || runtime.borrow_mut().run_try_clear(),
                || runtime.borrow_mut().apply_sandbox(),
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_owned_metrics<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
        ViewSource,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        call: &QueueApplyCallEnvelope<'_, TxSource, ViewSource>,
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
        TxSource: crate::QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: crate::QueueApplyObservedViewSource<Account>,
        Runtime: QueueApplyExecutionRuntime<Tx, Journal, ParentBatchId>,
    {
        self.with_runtime_refcell_current_app_view(|runtime| {
            call.apply_after_preflight_with_owned_metrics(
                owner,
                preflight_result,
                hold_preflight,
                can_be_held_result,
                |message| runtime.borrow_mut().trace(message),
                || runtime.borrow_mut().direct_apply(),
                |adjustment| runtime.borrow_mut().prepare_multitxn(adjustment),
                |view_source| runtime.borrow_mut().run_preclaim(view_source),
                || runtime.borrow_mut().run_try_clear(),
                || runtime.borrow_mut().apply_sandbox(),
            )
        })
    }

    pub fn apply_after_preflight_with_owned_metrics_and_log_messages<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
        ViewSource,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        call: &QueueApplyCallEnvelope<'_, TxSource, ViewSource>,
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
        TxSource: crate::QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: crate::QueueApplyObservedViewSource<Account>,
        Runtime: QueueApplyExecutionRuntime<Tx, Journal, ParentBatchId>,
    {
        self.with_runtime_refcell_current_app_view(|runtime| {
            call.apply_after_preflight_with_owned_metrics_and_log_messages(
                owner,
                preflight_result,
                hold_preflight,
                can_be_held_result,
                |message| runtime.borrow_mut().trace(message),
                || runtime.borrow_mut().direct_apply(),
                |adjustment| runtime.borrow_mut().prepare_multitxn(adjustment),
                |view_source| runtime.borrow_mut().run_preclaim(view_source),
                || runtime.borrow_mut().run_try_clear(),
                || runtime.borrow_mut().apply_sandbox(),
            )
        })
    }

    pub fn apply_after_preflight_with_owned_metrics_and_derived_hold_preflight_and_log_messages<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
        ViewSource,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        call: &QueueApplyCallEnvelope<'_, TxSource, ViewSource>,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        can_be_held_result: Ter,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
        TxId: Clone + Display,
        TxSource: crate::QueueApplyHoldPreflightTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: crate::QueueApplyObservedViewSource<Account>,
        Runtime: QueueApplyExecutionRuntime<Tx, Journal, ParentBatchId>,
    {
        self.with_runtime_refcell_current_app_view(|runtime| {
            call.apply_after_preflight_with_owned_metrics_and_derived_hold_preflight_and_log_messages(
                owner,
                preflight_result,
                can_be_held_result,
                |message| runtime.borrow_mut().trace(message),
                || runtime.borrow_mut().direct_apply(),
                |adjustment| runtime.borrow_mut().prepare_multitxn(adjustment),
                |view_source| runtime.borrow_mut().run_preclaim(view_source),
                || runtime.borrow_mut().run_try_clear(),
                || runtime.borrow_mut().apply_sandbox(),
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_owned_metrics_and_caller_preclaim<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
        ViewSource,
        RunPreclaimStage,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        call: &QueueApplyCallEnvelope<'_, TxSource, ViewSource>,
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
        TxSource: crate::QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: crate::QueueApplyObservedViewSource<Account>,
        Runtime: QueueApplyCurrentPreclaimClearRuntime<Tx, Journal, ParentBatchId>
            + QueueApplyExecutionRuntime<Tx, Journal, ParentBatchId>,
        RunPreclaimStage: FnOnce(
            QueueApplyPreparedPreclaimInputs<Account>,
        ) -> Result<
            QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>,
            crate::ApplyResult,
        >,
    {
        self.with_runtime_refcell_current_app_view_caller_preclaim(|runtime| {
            call.apply_with_owned_metrics_and_caller_preclaim(
                owner,
                hold_preflight,
                flags,
                consequences,
                can_be_held_result,
                || runtime.borrow_mut().run_preflight(),
                |message| runtime.borrow_mut().trace(message),
                || runtime.borrow_mut().direct_apply(),
                |adjustment| runtime.borrow_mut().prepare_multitxn(adjustment),
                run_preclaim_stage,
                || runtime.borrow_mut().run_try_clear_with_current_preclaim(),
                || runtime.borrow_mut().apply_sandbox(),
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_owned_metrics_and_caller_preclaim<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
        ViewSource,
        RunPreclaimStage,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        call: &QueueApplyCallEnvelope<'_, TxSource, ViewSource>,
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
        TxSource: crate::QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: crate::QueueApplyObservedViewSource<Account>,
        Runtime: QueueApplyCurrentPreclaimClearRuntime<Tx, Journal, ParentBatchId>
            + QueueApplyExecutionRuntime<Tx, Journal, ParentBatchId>,
        RunPreclaimStage: FnOnce(
            QueueApplyPreparedPreclaimInputs<Account>,
        ) -> Result<
            QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>,
            crate::ApplyResult,
        >,
    {
        self.with_runtime_refcell_current_app_view_caller_preclaim(|runtime| {
            call.apply_after_preflight_with_owned_metrics_and_caller_preclaim(
                owner,
                preflight_result,
                hold_preflight,
                can_be_held_result,
                |message| runtime.borrow_mut().trace(message),
                || runtime.borrow_mut().direct_apply(),
                |adjustment| runtime.borrow_mut().prepare_multitxn(adjustment),
                run_preclaim_stage,
                || runtime.borrow_mut().run_try_clear_with_current_preclaim(),
                || runtime.borrow_mut().apply_sandbox(),
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_owned_metrics_and_log_sinks<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
        ViewSource,
        TraceFn,
        DebugFn,
        InfoFn,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        call: &QueueApplyCallEnvelope<'_, TxSource, ViewSource>,
        hold_preflight: QueueHoldPreflight,
        flags: ApplyFlags,
        consequences: TxConsequences,
        can_be_held_result: Ter,
        trace: TraceFn,
        debug: DebugFn,
        info: InfoFn,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
        TxId: Clone + Display,
        TxSource: crate::QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: crate::QueueApplyObservedViewSource<Account>,
        Runtime: QueueApplyExecutionRuntime<Tx, Journal, ParentBatchId>,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
    {
        self.with_runtime_refcell_current_app_view_log_sinks(|runtime| {
            call.apply_with_owned_metrics_and_log_sinks(
                owner,
                hold_preflight,
                flags,
                consequences,
                can_be_held_result,
                || runtime.borrow_mut().run_preflight(),
                trace,
                debug,
                info,
                || runtime.borrow_mut().direct_apply(),
                |adjustment| runtime.borrow_mut().prepare_multitxn(adjustment),
                |view_source| runtime.borrow_mut().run_preclaim(view_source),
                || runtime.borrow_mut().run_try_clear(),
                || runtime.borrow_mut().apply_sandbox(),
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_owned_metrics_and_log_sinks<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
        ViewSource,
        TraceFn,
        DebugFn,
        InfoFn,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        call: &QueueApplyCallEnvelope<'_, TxSource, ViewSource>,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
        can_be_held_result: Ter,
        trace: TraceFn,
        debug: DebugFn,
        info: InfoFn,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
        TxId: Clone + Display,
        TxSource: crate::QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: crate::QueueApplyObservedViewSource<Account>,
        Runtime: QueueApplyExecutionRuntime<Tx, Journal, ParentBatchId>,
        TraceFn: FnMut(&str),
        DebugFn: FnMut(String),
        InfoFn: FnMut(String),
    {
        self.with_runtime_refcell_current_app_view_log_sinks(|runtime| {
            call.apply_after_preflight_with_owned_metrics_and_log_sinks(
                owner,
                preflight_result,
                hold_preflight,
                can_be_held_result,
                trace,
                debug,
                info,
                || runtime.borrow_mut().direct_apply(),
                |adjustment| runtime.borrow_mut().prepare_multitxn(adjustment),
                |view_source| runtime.borrow_mut().run_preclaim(view_source),
                || runtime.borrow_mut().run_try_clear(),
                || runtime.borrow_mut().apply_sandbox(),
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_owned_metrics_and_caller_preclaim_and_log_messages<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
        ViewSource,
        RunPreclaimStage,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        call: &QueueApplyCallEnvelope<'_, TxSource, ViewSource>,
        hold_preflight: QueueHoldPreflight,
        flags: ApplyFlags,
        consequences: TxConsequences,
        can_be_held_result: Ter,
        run_preclaim_stage: RunPreclaimStage,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
        TxId: Clone + Display,
        TxSource: crate::QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: crate::QueueApplyObservedViewSource<Account>,
        Runtime: QueueApplyCurrentPreclaimClearRuntime<Tx, Journal, ParentBatchId>
            + QueueApplyExecutionRuntime<Tx, Journal, ParentBatchId>,
        RunPreclaimStage: FnOnce(
            QueueApplyPreparedPreclaimInputs<Account>,
        ) -> Result<
            QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>,
            crate::ApplyResult,
        >,
    {
        self.with_runtime_refcell_current_app_view_caller_preclaim(|runtime| {
            call.apply_with_owned_metrics_and_caller_preclaim_and_log_messages(
                owner,
                hold_preflight,
                flags,
                consequences,
                can_be_held_result,
                || runtime.borrow_mut().run_preflight(),
                |message| runtime.borrow_mut().trace(message),
                || runtime.borrow_mut().direct_apply(),
                |adjustment| runtime.borrow_mut().prepare_multitxn(adjustment),
                run_preclaim_stage,
                || runtime.borrow_mut().run_try_clear_with_current_preclaim(),
                || runtime.borrow_mut().apply_sandbox(),
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_with_owned_metrics_and_caller_preclaim_and_log_sinks<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
        ViewSource,
        RunPreclaimStage,
        TraceFn,
        DebugFn,
        InfoFn,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        call: &QueueApplyCallEnvelope<'_, TxSource, ViewSource>,
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
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
        TxId: Clone + Display,
        TxSource: crate::QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: crate::QueueApplyObservedViewSource<Account>,
        Runtime: QueueApplyCurrentPreclaimClearRuntime<Tx, Journal, ParentBatchId>
            + QueueApplyExecutionRuntime<Tx, Journal, ParentBatchId>,
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
        self.with_runtime_refcell_current_app_view_caller_preclaim_log_sinks(|runtime| {
            call.apply_with_owned_metrics_and_caller_preclaim_and_log_sinks(
                owner,
                hold_preflight,
                flags,
                consequences,
                can_be_held_result,
                || runtime.borrow_mut().run_preflight(),
                trace,
                debug,
                info,
                || runtime.borrow_mut().direct_apply(),
                |adjustment| runtime.borrow_mut().prepare_multitxn(adjustment),
                run_preclaim_stage,
                || runtime.borrow_mut().run_try_clear_with_current_preclaim(),
                || runtime.borrow_mut().apply_sandbox(),
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_owned_metrics_and_derived_hold_admission_and_log_messages<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
        ViewSource,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        call: &QueueApplyCallEnvelope<'_, TxSource, ViewSource>,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
        TxId: Clone + Display,
        TxSource: crate::QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: crate::QueueApplyObservedViewSource<Account>,
        Runtime: QueueApplyExecutionRuntime<Tx, Journal, ParentBatchId>,
    {
        self.with_runtime_refcell_current_app_view_log_sinks(|runtime| {
            call.apply_after_preflight_with_owned_metrics_and_derived_hold_admission_and_log_messages(
                owner,
                preflight_result,
                hold_preflight,
                |message| runtime.borrow_mut().trace(message),
                || runtime.borrow_mut().direct_apply(),
                |adjustment| runtime.borrow_mut().prepare_multitxn(adjustment),
                |view_source| runtime.borrow_mut().run_preclaim(view_source),
                || runtime.borrow_mut().run_try_clear(),
                || runtime.borrow_mut().apply_sandbox(),
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_owned_metrics_and_caller_preclaim_and_log_messages<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
        ViewSource,
        RunPreclaimStage,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        call: &QueueApplyCallEnvelope<'_, TxSource, ViewSource>,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
        can_be_held_result: Ter,
        run_preclaim_stage: RunPreclaimStage,
    ) -> QueueApplyTopWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
        TxId: Clone + Display,
        TxSource: crate::QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: crate::QueueApplyObservedViewSource<Account>,
        Runtime: QueueApplyCurrentPreclaimClearRuntime<Tx, Journal, ParentBatchId>
            + QueueApplyExecutionRuntime<Tx, Journal, ParentBatchId>,
        RunPreclaimStage: FnOnce(
            QueueApplyPreparedPreclaimInputs<Account>,
        ) -> Result<
            QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>,
            crate::ApplyResult,
        >,
    {
        self.with_runtime_refcell_current_app_view_caller_preclaim(|runtime| {
            call.apply_after_preflight_with_owned_metrics_and_caller_preclaim_and_log_messages(
                owner,
                preflight_result,
                hold_preflight,
                can_be_held_result,
                |message| runtime.borrow_mut().trace(message),
                || runtime.borrow_mut().direct_apply(),
                |adjustment| runtime.borrow_mut().prepare_multitxn(adjustment),
                run_preclaim_stage,
                || runtime.borrow_mut().run_try_clear_with_current_preclaim(),
                || runtime.borrow_mut().apply_sandbox(),
            )
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_after_preflight_with_owned_metrics_and_caller_preclaim_and_log_sinks<
        Account,
        Tx,
        Journal,
        ParentBatchId,
        TxId,
        TxSource,
        ViewSource,
        RunPreclaimStage,
        TraceFn,
        DebugFn,
        InfoFn,
    >(
        &mut self,
        owner: &mut QueueApplyOwnerShell<Account, Tx, Journal, ParentBatchId>,
        call: &QueueApplyCallEnvelope<'_, TxSource, ViewSource>,
        preflight_result: &PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        hold_preflight: QueueHoldPreflight,
        can_be_held_result: Ter,
        run_preclaim_stage: RunPreclaimStage,
        trace: TraceFn,
        debug: DebugFn,
        info: InfoFn,
    ) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
    where
        Account: Clone + Display + Ord + PartialEq,
        Tx: Clone,
        Journal: Clone,
        ParentBatchId: Clone,
        TxId: Clone + Display,
        TxSource: crate::QueueApplyObservedTxSource<Account = Account, TransactionId = TxId>,
        ViewSource: crate::QueueApplyObservedViewSource<Account>,
        Runtime: QueueApplyCurrentPreclaimClearRuntime<Tx, Journal, ParentBatchId>
            + QueueApplyExecutionRuntime<Tx, Journal, ParentBatchId>,
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
        self.with_runtime_refcell_current_app_view_caller_preclaim_log_sinks(|runtime| {
            call.apply_after_preflight_with_owned_metrics_and_caller_preclaim_and_log_sinks(
                owner,
                preflight_result,
                hold_preflight,
                can_be_held_result,
                trace,
                debug,
                info,
                || runtime.borrow_mut().direct_apply(),
                |adjustment| runtime.borrow_mut().prepare_multitxn(adjustment),
                run_preclaim_stage,
                || runtime.borrow_mut().run_try_clear_with_current_preclaim(),
                || runtime.borrow_mut().apply_sandbox(),
            )
        })
    }
}
