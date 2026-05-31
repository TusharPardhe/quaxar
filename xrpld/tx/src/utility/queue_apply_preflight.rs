//! Top `preflight(...)` gate inside `TxQ::apply(...)`.
//!
//! This preserves the current canonical return order:
//! 1. reject immediately when `preflight(...)` is not `tesSUCCESS`,
//! 2. otherwise continue into the landed direct-apply plus queue-entry carrier.

use std::fmt::Display;

use protocol::{Ter, any_apply_flags, is_tes_success, trans_token};

use crate::{
    ApplyFlags, ApplyResult, PreflightResult, QueueApplyEntryStage, QueueApplyFeeContextInputs,
    QueueApplyPrerequisite, QueueApplyQueueLogMessages, QueueApplyQueuedStage,
    QueueApplyQueuedStageWithLogMessagesResult, QueueViews, evaluate_queue_apply_fee_context,
    evaluate_queue_apply_prerequisite, prepare_direct_apply_if_eligible,
    run_prepared_direct_apply_with_trace, run_queue_apply_entry_stage,
    run_queue_apply_entry_stage_with_log_messages,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId> {
    RejectPreflight(ApplyResult),
    Entry(QueueApplyEntryStage<Account, Tx, Journal, ParentBatchId, TxId>),
}

impl<Account, Tx, Journal, ParentBatchId, TxId>
    QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
{
    pub fn apply_result(&self) -> ApplyResult {
        match self {
            Self::RejectPreflight(result) => result.clone(),
            Self::Entry(stage) => stage.apply_result(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueApplyPreflightStageWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
{
    pub stage: QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>,
    pub queue_log_messages: QueueApplyQueueLogMessages,
}

#[derive(Debug, Clone)]
pub struct QueueApplyPreflightWithDirectApplyInputs<'a, Account, TxId> {
    pub transaction_id: TxId,
    pub account_exists: bool,
    pub account_seq_proxy: protocol::SeqProxy,
    pub tx_seq_proxy: protocol::SeqProxy,
    pub ticket_exists: bool,
    pub fee_context_inputs: QueueApplyFeeContextInputs,
    pub applied_account: &'a Account,
}

impl<'a, Account, TxId> QueueApplyPreflightWithDirectApplyInputs<'a, Account, TxId> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        transaction_id: TxId,
        account_exists: bool,
        account_seq_proxy: protocol::SeqProxy,
        tx_seq_proxy: protocol::SeqProxy,
        ticket_exists: bool,
        fee_context_inputs: QueueApplyFeeContextInputs,
        applied_account: &'a Account,
    ) -> Self {
        Self {
            transaction_id,
            account_exists,
            account_seq_proxy,
            tx_seq_proxy,
            ticket_exists,
            fee_context_inputs,
            applied_account,
        }
    }
}

pub fn run_queue_apply_preflight_stage<
    Account,
    Tx,
    Consequences,
    Journal,
    ParentBatchId,
    TxId,
    RunDirectApply,
    RunQueuedStage,
>(
    preflight_result: &PreflightResult<Tx, Consequences, Journal, ParentBatchId>,
    account_exists: bool,
    account_seq_proxy: protocol::SeqProxy,
    tx_seq_proxy: protocol::SeqProxy,
    ticket_exists: bool,
    run_direct_apply: RunDirectApply,
    run_queued_stage: RunQueuedStage,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    RunDirectApply: FnOnce() -> Option<crate::DirectApplyExecution<Account, TxId>>,
    RunQueuedStage: FnOnce() -> crate::QueueApplyQueuedStage<Account, Tx, Journal, ParentBatchId>,
{
    tracing::trace!(target: "tx", tx_type = "apply", hash = %tx_seq_proxy, "Transaction preflight check");

    if !is_tes_success(preflight_result.ter) {
        let reason = trans_token(preflight_result.ter);
        tracing::warn!(target: "tx", tx_type = "apply", hash = %tx_seq_proxy, reason = %reason, "Transaction rejected from queue");
        return QueueApplyPreflightStage::RejectPreflight(ApplyResult::new(
            preflight_result.ter,
            false,
            false,
        ));
    }

    QueueApplyPreflightStage::Entry(run_queue_apply_entry_stage(
        account_exists,
        account_seq_proxy,
        tx_seq_proxy,
        ticket_exists,
        run_direct_apply,
        run_queued_stage,
    ))
}

pub fn run_queue_apply_preflight_stage_with_log_messages<
    Account,
    Tx,
    Consequences,
    Journal,
    ParentBatchId,
    TxId,
    RunDirectApply,
    RunQueuedStage,
>(
    preflight_result: &PreflightResult<Tx, Consequences, Journal, ParentBatchId>,
    account_exists: bool,
    account_seq_proxy: protocol::SeqProxy,
    tx_seq_proxy: protocol::SeqProxy,
    ticket_exists: bool,
    run_direct_apply: RunDirectApply,
    run_queued_stage: RunQueuedStage,
) -> QueueApplyPreflightStageWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
where
    RunDirectApply: FnOnce() -> Option<crate::DirectApplyExecution<Account, TxId>>,
    RunQueuedStage:
        FnOnce() -> QueueApplyQueuedStageWithLogMessagesResult<Account, Tx, Journal, ParentBatchId>,
{
    if !is_tes_success(preflight_result.ter) {
        return QueueApplyPreflightStageWithLogMessagesResult {
            stage: QueueApplyPreflightStage::RejectPreflight(ApplyResult::new(
                preflight_result.ter,
                false,
                false,
            )),
            queue_log_messages: QueueApplyQueueLogMessages::default(),
        };
    }

    let entry = run_queue_apply_entry_stage_with_log_messages(
        account_exists,
        account_seq_proxy,
        tx_seq_proxy,
        ticket_exists,
        run_direct_apply,
        run_queued_stage,
    );

    QueueApplyPreflightStageWithLogMessagesResult {
        stage: QueueApplyPreflightStage::Entry(entry.stage),
        queue_log_messages: entry.queue_log_messages,
    }
}

pub fn run_queue_apply_after_preflight_with_acquired_direct_apply<
    Account,
    Tx,
    Consequences,
    Journal,
    ParentBatchId,
    TxId,
    RunQueuedStage,
>(
    preflight_result: &PreflightResult<Tx, Consequences, Journal, ParentBatchId>,
    account_exists: bool,
    account_seq_proxy: protocol::SeqProxy,
    tx_seq_proxy: protocol::SeqProxy,
    ticket_exists: bool,
    direct_applied: Option<crate::DirectApplyExecution<Account, TxId>>,
    run_queued_stage: RunQueuedStage,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    RunQueuedStage: FnOnce() -> crate::QueueApplyQueuedStage<Account, Tx, Journal, ParentBatchId>,
{
    if !is_tes_success(preflight_result.ter) {
        return QueueApplyPreflightStage::RejectPreflight(ApplyResult::new(
            preflight_result.ter,
            false,
            false,
        ));
    }

    if direct_applied.is_none() && any_apply_flags(preflight_result.flags & ApplyFlags::DRY_RUN) {
        let prerequisite = evaluate_queue_apply_prerequisite(
            account_exists,
            account_seq_proxy,
            tx_seq_proxy,
            ticket_exists,
        );
        return if prerequisite == QueueApplyPrerequisite::Ready {
            QueueApplyPreflightStage::RejectPreflight(ApplyResult::new(
                Ter::TEL_CAN_NOT_QUEUE,
                false,
                false,
            ))
        } else {
            QueueApplyPreflightStage::Entry(QueueApplyEntryStage::RejectPrerequisite(prerequisite))
        };
    }

    QueueApplyPreflightStage::Entry(run_queue_apply_entry_stage(
        account_exists,
        account_seq_proxy,
        tx_seq_proxy,
        ticket_exists,
        || direct_applied,
        run_queued_stage,
    ))
}

pub fn run_queue_apply_after_preflight_with_acquired_direct_apply_and_log_messages<
    Account,
    Tx,
    Consequences,
    Journal,
    ParentBatchId,
    TxId,
    RunQueuedStage,
>(
    preflight_result: &PreflightResult<Tx, Consequences, Journal, ParentBatchId>,
    account_exists: bool,
    account_seq_proxy: protocol::SeqProxy,
    tx_seq_proxy: protocol::SeqProxy,
    ticket_exists: bool,
    direct_applied: Option<crate::DirectApplyExecution<Account, TxId>>,
    run_queued_stage: RunQueuedStage,
) -> QueueApplyPreflightStageWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
where
    RunQueuedStage:
        FnOnce() -> QueueApplyQueuedStageWithLogMessagesResult<Account, Tx, Journal, ParentBatchId>,
{
    if !is_tes_success(preflight_result.ter) {
        return QueueApplyPreflightStageWithLogMessagesResult {
            stage: QueueApplyPreflightStage::RejectPreflight(ApplyResult::new(
                preflight_result.ter,
                false,
                false,
            )),
            queue_log_messages: QueueApplyQueueLogMessages::default(),
        };
    }

    if direct_applied.is_none() && any_apply_flags(preflight_result.flags & ApplyFlags::DRY_RUN) {
        let prerequisite = evaluate_queue_apply_prerequisite(
            account_exists,
            account_seq_proxy,
            tx_seq_proxy,
            ticket_exists,
        );
        return QueueApplyPreflightStageWithLogMessagesResult {
            stage: if prerequisite == QueueApplyPrerequisite::Ready {
                QueueApplyPreflightStage::RejectPreflight(ApplyResult::new(
                    Ter::TEL_CAN_NOT_QUEUE,
                    false,
                    false,
                ))
            } else {
                QueueApplyPreflightStage::Entry(QueueApplyEntryStage::RejectPrerequisite(
                    prerequisite,
                ))
            },
            queue_log_messages: QueueApplyQueueLogMessages::default(),
        };
    }

    let entry = run_queue_apply_entry_stage_with_log_messages(
        account_exists,
        account_seq_proxy,
        tx_seq_proxy,
        ticket_exists,
        || direct_applied,
        run_queued_stage,
    );

    QueueApplyPreflightStageWithLogMessagesResult {
        stage: QueueApplyPreflightStage::Entry(entry.stage),
        queue_log_messages: entry.queue_log_messages,
    }
}

pub fn run_queue_apply_after_preflight_with_direct_apply<
    Account,
    T,
    Tx,
    Consequences,
    Journal,
    ParentBatchId,
    TxId,
    TraceFn,
    ApplyFn,
    RunQueuedStage,
>(
    preflight_result: &PreflightResult<Tx, Consequences, Journal, ParentBatchId>,
    views: &mut QueueViews<Account, T>,
    inputs: QueueApplyPreflightWithDirectApplyInputs<'_, Account, TxId>,
    trace: TraceFn,
    apply: ApplyFn,
    run_queued_stage: RunQueuedStage,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    TxId: Clone + Display,
    TraceFn: FnMut(&str),
    ApplyFn: FnOnce() -> ApplyResult,
    RunQueuedStage: FnOnce(
        &mut QueueViews<Account, T>,
        crate::QueueApplyFeeContext,
    ) -> QueueApplyQueuedStage<Account, Tx, Journal, ParentBatchId>,
{
    run_queue_apply_after_preflight_with_caller_direct_apply(
        preflight_result,
        views,
        inputs,
        |views, prepared| run_prepared_direct_apply_with_trace(views, prepared, trace, apply),
        run_queued_stage,
    )
}

pub fn run_queue_apply_after_preflight_with_direct_apply_and_log_messages<
    Account,
    T,
    Tx,
    Consequences,
    Journal,
    ParentBatchId,
    TxId,
    TraceFn,
    ApplyFn,
    RunQueuedStage,
>(
    preflight_result: &PreflightResult<Tx, Consequences, Journal, ParentBatchId>,
    views: &mut QueueViews<Account, T>,
    inputs: QueueApplyPreflightWithDirectApplyInputs<'_, Account, TxId>,
    trace: TraceFn,
    apply: ApplyFn,
    run_queued_stage: RunQueuedStage,
) -> QueueApplyPreflightStageWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    TxId: Clone + Display,
    TraceFn: FnMut(&str),
    ApplyFn: FnOnce() -> ApplyResult,
    RunQueuedStage:
        FnOnce(
            &mut QueueViews<Account, T>,
            crate::QueueApplyFeeContext,
        )
            -> QueueApplyQueuedStageWithLogMessagesResult<Account, Tx, Journal, ParentBatchId>,
{
    run_queue_apply_after_preflight_with_caller_direct_apply_and_log_messages(
        preflight_result,
        views,
        inputs,
        |views, prepared| run_prepared_direct_apply_with_trace(views, prepared, trace, apply),
        run_queued_stage,
    )
}

pub fn run_queue_apply_after_preflight_with_caller_direct_apply<
    Account,
    T,
    Tx,
    Consequences,
    Journal,
    ParentBatchId,
    TxId,
    RunDirectApply,
    RunQueuedStage,
>(
    preflight_result: &PreflightResult<Tx, Consequences, Journal, ParentBatchId>,
    views: &mut QueueViews<Account, T>,
    inputs: QueueApplyPreflightWithDirectApplyInputs<'_, Account, TxId>,
    run_direct_apply: RunDirectApply,
    run_queued_stage: RunQueuedStage,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    TxId: Clone + Display,
    RunDirectApply: FnOnce(
        &mut QueueViews<Account, T>,
        crate::PreparedDirectApply<'_, Account, TxId>,
    ) -> crate::DirectApplyExecution<Account, TxId>,
    RunQueuedStage: FnOnce(
        &mut QueueViews<Account, T>,
        crate::QueueApplyFeeContext,
    ) -> QueueApplyQueuedStage<Account, Tx, Journal, ParentBatchId>,
{
    if !is_tes_success(preflight_result.ter) {
        return QueueApplyPreflightStage::RejectPreflight(ApplyResult::new(
            preflight_result.ter,
            false,
            false,
        ));
    }

    let fee_context = evaluate_queue_apply_fee_context(inputs.fee_context_inputs);
    let direct_applied = prepare_direct_apply_if_eligible(
        inputs.transaction_id,
        inputs.account_exists,
        inputs.account_seq_proxy,
        inputs.tx_seq_proxy,
        fee_context.fee_level_paid,
        fee_context.required_fee_level,
        inputs.applied_account,
    )
    .map(|prepared| run_direct_apply(views, prepared));

    run_queue_apply_after_preflight_with_acquired_direct_apply(
        preflight_result,
        inputs.account_exists,
        inputs.account_seq_proxy,
        inputs.tx_seq_proxy,
        inputs.ticket_exists,
        direct_applied,
        || run_queued_stage(views, fee_context),
    )
}

pub fn run_queue_apply_after_preflight_with_caller_direct_apply_and_log_messages<
    Account,
    T,
    Tx,
    Consequences,
    Journal,
    ParentBatchId,
    TxId,
    RunDirectApply,
    RunQueuedStage,
>(
    preflight_result: &PreflightResult<Tx, Consequences, Journal, ParentBatchId>,
    views: &mut QueueViews<Account, T>,
    inputs: QueueApplyPreflightWithDirectApplyInputs<'_, Account, TxId>,
    run_direct_apply: RunDirectApply,
    run_queued_stage: RunQueuedStage,
) -> QueueApplyPreflightStageWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    TxId: Clone + Display,
    RunDirectApply: FnOnce(
        &mut QueueViews<Account, T>,
        crate::PreparedDirectApply<'_, Account, TxId>,
    ) -> crate::DirectApplyExecution<Account, TxId>,
    RunQueuedStage:
        FnOnce(
            &mut QueueViews<Account, T>,
            crate::QueueApplyFeeContext,
        )
            -> QueueApplyQueuedStageWithLogMessagesResult<Account, Tx, Journal, ParentBatchId>,
{
    if !is_tes_success(preflight_result.ter) {
        return QueueApplyPreflightStageWithLogMessagesResult {
            stage: QueueApplyPreflightStage::RejectPreflight(ApplyResult::new(
                preflight_result.ter,
                false,
                false,
            )),
            queue_log_messages: QueueApplyQueueLogMessages::default(),
        };
    }

    let fee_context = evaluate_queue_apply_fee_context(inputs.fee_context_inputs);
    let direct_applied = prepare_direct_apply_if_eligible(
        inputs.transaction_id,
        inputs.account_exists,
        inputs.account_seq_proxy,
        inputs.tx_seq_proxy,
        fee_context.fee_level_paid,
        fee_context.required_fee_level,
        inputs.applied_account,
    )
    .map(|prepared| run_direct_apply(views, prepared));

    run_queue_apply_after_preflight_with_acquired_direct_apply_and_log_messages(
        preflight_result,
        inputs.account_exists,
        inputs.account_seq_proxy,
        inputs.tx_seq_proxy,
        inputs.ticket_exists,
        direct_applied,
        || run_queued_stage(views, fee_context),
    )
}

pub fn run_queue_apply_preflight_with_direct_apply<
    Account,
    T,
    Tx,
    Consequences,
    Journal,
    ParentBatchId,
    TxId,
    TraceFn,
    ApplyFn,
    RunQueuedStage,
>(
    preflight_result: &PreflightResult<Tx, Consequences, Journal, ParentBatchId>,
    views: &mut QueueViews<Account, T>,
    inputs: QueueApplyPreflightWithDirectApplyInputs<'_, Account, TxId>,
    trace: TraceFn,
    apply: ApplyFn,
    run_queued_stage: RunQueuedStage,
) -> QueueApplyPreflightStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    TxId: Clone + Display,
    TraceFn: FnMut(&str),
    ApplyFn: FnOnce() -> ApplyResult,
    RunQueuedStage: FnOnce(
        &mut QueueViews<Account, T>,
        crate::QueueApplyFeeContext,
    ) -> QueueApplyQueuedStage<Account, Tx, Journal, ParentBatchId>,
{
    run_queue_apply_after_preflight_with_direct_apply(
        preflight_result,
        views,
        inputs,
        trace,
        apply,
        run_queued_stage,
    )
}

pub fn run_queue_apply_preflight_with_direct_apply_and_log_messages<
    Account,
    T,
    Tx,
    Consequences,
    Journal,
    ParentBatchId,
    TxId,
    TraceFn,
    ApplyFn,
    RunQueuedStage,
>(
    preflight_result: &PreflightResult<Tx, Consequences, Journal, ParentBatchId>,
    views: &mut QueueViews<Account, T>,
    inputs: QueueApplyPreflightWithDirectApplyInputs<'_, Account, TxId>,
    trace: TraceFn,
    apply: ApplyFn,
    run_queued_stage: RunQueuedStage,
) -> QueueApplyPreflightStageWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    TxId: Clone + Display,
    TraceFn: FnMut(&str),
    ApplyFn: FnOnce() -> ApplyResult,
    RunQueuedStage:
        FnOnce(
            &mut QueueViews<Account, T>,
            crate::QueueApplyFeeContext,
        )
            -> QueueApplyQueuedStageWithLogMessagesResult<Account, Tx, Journal, ParentBatchId>,
{
    run_queue_apply_after_preflight_with_direct_apply_and_log_messages(
        preflight_result,
        views,
        inputs,
        trace,
        apply,
        run_queued_stage,
    )
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, cell::RefCell, collections::BTreeMap};

    use protocol::{Rules, SeqProxy, Ter};

    use super::{
        QueueApplyPreflightStage, QueueApplyPreflightWithDirectApplyInputs,
        run_queue_apply_after_preflight_with_acquired_direct_apply,
        run_queue_apply_after_preflight_with_caller_direct_apply,
        run_queue_apply_after_preflight_with_direct_apply, run_queue_apply_preflight_stage,
        run_queue_apply_preflight_with_direct_apply,
    };
    use crate::{
        ApplyFlags, ApplyResult, DirectApplyAttemptResult, DirectApplyExecution, PreflightResult,
        PreparedDirectApply, QueueApplyEntryStage, QueueApplyFeeContextInputs,
        QueueApplyQueuedStage, QueueFeeMetricsSnapshot, QueueViews, TXQ_BASE_LEVEL,
    };

    fn preflight(
        ter: Ter,
    ) -> PreflightResult<&'static str, &'static str, &'static str, &'static str> {
        PreflightResult::new(
            "tx",
            None,
            Rules::new(std::iter::empty()),
            "normal",
            ApplyFlags::NONE,
            "journal",
            ter,
        )
    }

    #[test]
    fn preflight_stage_rejects_before_direct_apply_or_queue_entry() {
        let ran_direct_apply = Cell::new(false);
        let ran_queued = Cell::new(false);

        let stage = run_queue_apply_preflight_stage::<
            &'static str,
            &'static str,
            &'static str,
            &'static str,
            &'static str,
            &'static str,
            _,
            _,
        >(
            &preflight(Ter::TER_RETRY),
            true,
            SeqProxy::sequence(8),
            SeqProxy::sequence(8),
            false,
            || {
                ran_direct_apply.set(true);
                None
            },
            || {
                ran_queued.set(true);
                QueueApplyQueuedStage::MultiTxn(crate::QueueApplyMultiTxnStage::RejectPath(
                    Ter::TER_PRE_SEQ,
                ))
            },
        );

        assert_eq!(
            stage,
            QueueApplyPreflightStage::RejectPreflight(ApplyResult::new(
                Ter::TER_RETRY,
                false,
                false,
            ))
        );
        assert_eq!(
            stage.apply_result(),
            ApplyResult::new(Ter::TER_RETRY, false, false)
        );
        assert!(!ran_direct_apply.get());
        assert!(!ran_queued.get());
    }

    #[test]
    fn preflight_stage_delegates_to_entry_after_success() {
        let direct = DirectApplyExecution {
            transaction_id: "ABC123",
            attempt: DirectApplyAttemptResult {
                apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, true),
                removed_replacement: None::<crate::FeeQueueKey<&'static str>>,
            },
        };

        let stage = run_queue_apply_preflight_stage(
            &preflight(Ter::TES_SUCCESS),
            true,
            SeqProxy::sequence(8),
            SeqProxy::sequence(8),
            false,
            || Some(direct.clone()),
            || {
                QueueApplyQueuedStage::MultiTxn(crate::QueueApplyMultiTxnStage::RejectPath(
                    Ter::TER_PRE_SEQ,
                ))
            },
        );

        assert_eq!(
            stage,
            QueueApplyPreflightStage::Entry(QueueApplyEntryStage::DirectApplied(direct))
        );
        assert_eq!(
            stage.apply_result(),
            ApplyResult::new(Ter::TES_SUCCESS, true, true)
        );
    }

    #[test]
    fn acquired_direct_apply_stage_preserves_preflight_rejection_precedence() {
        let ran_queued = Cell::new(false);
        let direct = DirectApplyExecution {
            transaction_id: "ABC123",
            attempt: DirectApplyAttemptResult {
                apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, true),
                removed_replacement: None::<crate::FeeQueueKey<&'static str>>,
            },
        };

        let stage = run_queue_apply_after_preflight_with_acquired_direct_apply(
            &preflight(Ter::TER_RETRY),
            true,
            SeqProxy::sequence(8),
            SeqProxy::sequence(8),
            true,
            Some(direct),
            || {
                ran_queued.set(true);
                QueueApplyQueuedStage::MultiTxn(crate::QueueApplyMultiTxnStage::RejectPath(
                    Ter::TER_PRE_SEQ,
                ))
            },
        );

        assert_eq!(
            stage,
            QueueApplyPreflightStage::RejectPreflight(ApplyResult::new(
                Ter::TER_RETRY,
                false,
                false,
            ))
        );
        assert!(!ran_queued.get());
    }

    #[test]
    fn acquired_direct_apply_stage_returns_supplied_direct_apply_before_queue_path() {
        let ran_queued = Cell::new(false);
        let direct = DirectApplyExecution {
            transaction_id: "ABC123",
            attempt: DirectApplyAttemptResult {
                apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, true),
                removed_replacement: None::<crate::FeeQueueKey<&'static str>>,
            },
        };

        let stage = run_queue_apply_after_preflight_with_acquired_direct_apply(
            &preflight(Ter::TES_SUCCESS),
            true,
            SeqProxy::sequence(8),
            SeqProxy::sequence(8),
            true,
            Some(direct.clone()),
            || {
                ran_queued.set(true);
                QueueApplyQueuedStage::MultiTxn(crate::QueueApplyMultiTxnStage::RejectPath(
                    Ter::TER_PRE_SEQ,
                ))
            },
        );

        assert_eq!(
            stage,
            QueueApplyPreflightStage::Entry(QueueApplyEntryStage::DirectApplied(direct))
        );
        assert!(!ran_queued.get());
    }

    #[test]
    fn acquired_direct_apply_stage_falls_through_to_existing_queue_entry_path() {
        let ran_queued = Cell::new(false);

        let stage = run_queue_apply_after_preflight_with_acquired_direct_apply(
            &preflight(Ter::TES_SUCCESS),
            true,
            SeqProxy::sequence(8),
            SeqProxy::sequence(8),
            true,
            None::<DirectApplyExecution<&'static str, &'static str>>,
            || {
                ran_queued.set(true);
                QueueApplyQueuedStage::MultiTxn(crate::QueueApplyMultiTxnStage::RejectPath(
                    Ter::TER_PRE_SEQ,
                ))
            },
        );

        assert_eq!(
            stage,
            QueueApplyPreflightStage::Entry(QueueApplyEntryStage::Queued(
                QueueApplyQueuedStage::MultiTxn(crate::QueueApplyMultiTxnStage::RejectPath(
                    Ter::TER_PRE_SEQ,
                )),
            ))
        );
        assert!(ran_queued.get());
    }

    #[test]
    fn acquired_direct_apply_stage_rejects_dry_run_before_queue_path() {
        let ran_queued = Cell::new(false);
        let mut preflight = preflight(Ter::TES_SUCCESS);
        preflight.flags = ApplyFlags::DRY_RUN;

        let stage = run_queue_apply_after_preflight_with_acquired_direct_apply(
            &preflight,
            true,
            SeqProxy::sequence(8),
            SeqProxy::sequence(8),
            true,
            None::<DirectApplyExecution<&'static str, &'static str>>,
            || {
                ran_queued.set(true);
                QueueApplyQueuedStage::MultiTxn(crate::QueueApplyMultiTxnStage::RejectPath(
                    Ter::TER_PRE_SEQ,
                ))
            },
        );

        assert_eq!(
            stage.apply_result(),
            ApplyResult::new(Ter::TEL_CAN_NOT_QUEUE, false, false)
        );
        assert!(!ran_queued.get());
    }

    #[test]
    fn after_preflight_direct_apply_stage_returns_direct_apply_before_queue_path() {
        let traces = RefCell::new(Vec::new());
        let ran_queued = Cell::new(false);
        let mut views = QueueViews::<&'static str, ()>::new(BTreeMap::new(), vec![]);

        let stage = run_queue_apply_after_preflight_with_direct_apply(
            &preflight(Ter::TES_SUCCESS),
            &mut views,
            QueueApplyPreflightWithDirectApplyInputs::new(
                "ABC123",
                true,
                SeqProxy::sequence(8),
                SeqProxy::sequence(8),
                true,
                QueueApplyFeeContextInputs {
                    calculated_base_fee_drops: 10,
                    fee_paid_drops: 20,
                    default_base_fee_drops: 10,
                    metrics_snapshot: QueueFeeMetricsSnapshot {
                        txns_expected: 32,
                        escalation_multiplier: TXQ_BASE_LEVEL * 500,
                    },
                    open_ledger_tx_count: 0,
                    flags: ApplyFlags::NONE,
                },
                &"acct",
            ),
            |line| traces.borrow_mut().push(line.to_owned()),
            || ApplyResult::new(Ter::TES_SUCCESS, true, true),
            |_, _| {
                ran_queued.set(true);
                QueueApplyQueuedStage::<&'static str, &'static str, &'static str, &'static str>::MultiTxn(
                    crate::QueueApplyMultiTxnStage::RejectPath(Ter::TER_PRE_SEQ),
                )
            },
        );

        assert!(matches!(
            stage,
            QueueApplyPreflightStage::Entry(QueueApplyEntryStage::DirectApplied(_))
        ));
        assert!(!ran_queued.get());
        assert_eq!(
            traces.into_inner(),
            vec![
                "Applying transaction ABC123 to open ledger.".to_owned(),
                "New transaction ABC123 applied successfully with tesSUCCESS".to_owned(),
            ]
        );
    }

    #[test]
    fn after_preflight_caller_direct_apply_stage_exposes_prepared_execution_boundary() {
        let account = String::from("acct");
        let mut views = QueueViews::<String, ()>::new(BTreeMap::new(), vec![]);
        let applied = Cell::new(false);
        let ran_queued = Cell::new(false);

        let stage = run_queue_apply_after_preflight_with_caller_direct_apply(
            &preflight(Ter::TES_SUCCESS),
            &mut views,
            QueueApplyPreflightWithDirectApplyInputs::new(
                "ABC123",
                true,
                SeqProxy::sequence(8),
                SeqProxy::sequence(8),
                true,
                QueueApplyFeeContextInputs {
                    calculated_base_fee_drops: 10,
                    fee_paid_drops: 20,
                    default_base_fee_drops: 10,
                    metrics_snapshot: QueueFeeMetricsSnapshot {
                        txns_expected: 32,
                        escalation_multiplier: TXQ_BASE_LEVEL * 500,
                    },
                    open_ledger_tx_count: 0,
                    flags: ApplyFlags::NONE,
                },
                &account,
            ),
            |_views, prepared| {
                applied.set(true);
                assert_eq!(
                    prepared,
                    PreparedDirectApply {
                        transaction_id: "ABC123",
                        applied_account: &account,
                        applied_seq_proxy: SeqProxy::sequence(8),
                    }
                );
                DirectApplyExecution {
                    transaction_id: "ABC123",
                    attempt: DirectApplyAttemptResult {
                        apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, true),
                        removed_replacement: None,
                    },
                }
            },
            |_views, _fee_context| {
                ran_queued.set(true);
                QueueApplyQueuedStage::<String, &'static str, &'static str, &'static str>::MultiTxn(
                    crate::QueueApplyMultiTxnStage::RejectPath(Ter::TER_PRE_SEQ),
                )
            },
        );

        assert!(applied.get());
        assert!(!ran_queued.get());
        assert!(matches!(
            stage,
            QueueApplyPreflightStage::Entry(QueueApplyEntryStage::DirectApplied(_))
        ));
    }

    #[test]
    fn after_preflight_direct_apply_stage_falls_through_to_queue_path_when_ineligible() {
        let traces = RefCell::new(Vec::new());
        let ran_queued = Cell::new(false);
        let mut views = QueueViews::<&'static str, ()>::new(BTreeMap::new(), vec![]);

        let stage = run_queue_apply_after_preflight_with_direct_apply::<
            &'static str,
            (),
            &'static str,
            &'static str,
            &'static str,
            &'static str,
            &'static str,
            _,
            _,
            _,
        >(
            &preflight(Ter::TES_SUCCESS),
            &mut views,
            QueueApplyPreflightWithDirectApplyInputs::new(
                "ABC123",
                true,
                SeqProxy::sequence(8),
                SeqProxy::sequence(9),
                true,
                QueueApplyFeeContextInputs {
                    calculated_base_fee_drops: 10,
                    fee_paid_drops: 20,
                    default_base_fee_drops: 10,
                    metrics_snapshot: QueueFeeMetricsSnapshot {
                        txns_expected: 32,
                        escalation_multiplier: TXQ_BASE_LEVEL * 500,
                    },
                    open_ledger_tx_count: 0,
                    flags: ApplyFlags::NONE,
                },
                &"acct",
            ),
            |line| traces.borrow_mut().push(line.to_owned()),
            || unreachable!("ineligible direct apply should not call apply"),
            |_, _| {
                ran_queued.set(true);
                QueueApplyQueuedStage::MultiTxn(crate::QueueApplyMultiTxnStage::RejectPath(
                    Ter::TER_PRE_SEQ,
                ))
            },
        );

        assert_eq!(
            stage,
            QueueApplyPreflightStage::Entry(QueueApplyEntryStage::Queued(
                QueueApplyQueuedStage::MultiTxn(crate::QueueApplyMultiTxnStage::RejectPath(
                    Ter::TER_PRE_SEQ,
                )),
            ))
        );
        assert!(ran_queued.get());
        assert!(traces.borrow().is_empty());
    }

    #[test]
    fn preflight_wrapper_with_direct_apply_rejects_before_fee_or_entry_work() {
        let traces = RefCell::new(Vec::new());
        let mut views = QueueViews::<&'static str, ()>::new(BTreeMap::new(), vec![]);

        let stage = run_queue_apply_preflight_with_direct_apply::<
            &'static str,
            (),
            &'static str,
            &'static str,
            &'static str,
            &'static str,
            &'static str,
            _,
            _,
            _,
        >(
            &preflight(Ter::TER_RETRY),
            &mut views,
            QueueApplyPreflightWithDirectApplyInputs::new(
                "ABC123",
                true,
                SeqProxy::sequence(8),
                SeqProxy::sequence(8),
                true,
                QueueApplyFeeContextInputs {
                    calculated_base_fee_drops: -1,
                    fee_paid_drops: 0,
                    default_base_fee_drops: 0,
                    metrics_snapshot: QueueFeeMetricsSnapshot {
                        txns_expected: 32,
                        escalation_multiplier: TXQ_BASE_LEVEL * 500,
                    },
                    open_ledger_tx_count: 0,
                    flags: ApplyFlags::NONE,
                },
                &"acct",
            ),
            |line| traces.borrow_mut().push(line.to_owned()),
            || unreachable!("preflight rejection should return before direct apply"),
            |_, _| unreachable!("preflight rejection should return before queued stage"),
        );

        assert_eq!(
            stage,
            QueueApplyPreflightStage::RejectPreflight(ApplyResult::new(
                Ter::TER_RETRY,
                false,
                false,
            ))
        );
        assert!(traces.borrow().is_empty());
    }

    #[test]
    fn preflight_wrapper_with_direct_apply_delegates_success_into_entry_wrapper() {
        let traces = RefCell::new(Vec::new());
        let mut views = QueueViews::<&'static str, ()>::new(BTreeMap::new(), vec![]);

        let stage = run_queue_apply_preflight_with_direct_apply(
            &preflight(Ter::TES_SUCCESS),
            &mut views,
            QueueApplyPreflightWithDirectApplyInputs::new(
                "ABC123",
                true,
                SeqProxy::sequence(8),
                SeqProxy::sequence(8),
                true,
                QueueApplyFeeContextInputs {
                    calculated_base_fee_drops: 10,
                    fee_paid_drops: 20,
                    default_base_fee_drops: 10,
                    metrics_snapshot: QueueFeeMetricsSnapshot {
                        txns_expected: 32,
                        escalation_multiplier: TXQ_BASE_LEVEL * 500,
                    },
                    open_ledger_tx_count: 0,
                    flags: ApplyFlags::NONE,
                },
                &"acct",
            ),
            |line| traces.borrow_mut().push(line.to_owned()),
            || ApplyResult::new(Ter::TES_SUCCESS, true, true),
            |_, _| {
                QueueApplyQueuedStage::<&'static str, &'static str, &'static str, &'static str>::MultiTxn(
                    crate::QueueApplyMultiTxnStage::RejectPath(Ter::TER_PRE_SEQ),
                )
            },
        );

        assert!(matches!(
            stage,
            QueueApplyPreflightStage::Entry(QueueApplyEntryStage::DirectApplied(_))
        ));
        assert_eq!(
            traces.into_inner(),
            vec![
                "Applying transaction ABC123 to open ledger.".to_owned(),
                "New transaction ABC123 applied successfully with tesSUCCESS".to_owned(),
            ]
        );
    }
}
