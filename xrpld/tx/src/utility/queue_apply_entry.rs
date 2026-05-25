//! Top branch order for `TxQ::apply(...)` after `preflight(...)`.
//!
//! This preserves the current canonical return order:
//! 1. return immediately if direct apply already produced a result,
//! 2. reject missing-account or missing-ticket prerequisites,
//! 3. otherwise continue into the landed queued-path carrier.

use std::fmt::Display;

use protocol::SeqProxy;

use crate::{
    ApplyResult, DirectApplyExecution, QueueApplyFeeContext, QueueApplyPrerequisite,
    QueueApplyQueueLogMessages, QueueApplyQueuedStage, QueueApplyQueuedStageWithLogMessagesResult,
    QueueViews, evaluate_queue_apply_prerequisite, run_direct_apply_with_trace_if_eligible,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueApplyEntryStage<Account, Tx, Journal, ParentBatchId, TxId> {
    DirectApplied(DirectApplyExecution<Account, TxId>),
    RejectPrerequisite(QueueApplyPrerequisite),
    Queued(QueueApplyQueuedStage<Account, Tx, Journal, ParentBatchId>),
}

impl<Account, Tx, Journal, ParentBatchId, TxId>
    QueueApplyEntryStage<Account, Tx, Journal, ParentBatchId, TxId>
{
    pub fn apply_result(&self) -> ApplyResult {
        match self {
            Self::DirectApplied(execution) => execution.attempt.apply_result.clone(),
            Self::RejectPrerequisite(prerequisite) => ApplyResult::new(
                prerequisite.ter().expect("prerequisite rejection"),
                false,
                false,
            ),
            Self::Queued(stage) => stage.apply_result(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueApplyEntryStageWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId> {
    pub stage: QueueApplyEntryStage<Account, Tx, Journal, ParentBatchId, TxId>,
    pub queue_log_messages: QueueApplyQueueLogMessages,
}

#[derive(Debug, Clone)]
pub struct QueueApplyEntryWithDirectApplyInputs<'a, Account, TxId> {
    pub transaction_id: TxId,
    pub account_exists: bool,
    pub account_seq_proxy: SeqProxy,
    pub tx_seq_proxy: SeqProxy,
    pub ticket_exists: bool,
    pub fee_context: QueueApplyFeeContext,
    pub applied_account: &'a Account,
}

impl<'a, Account, TxId> QueueApplyEntryWithDirectApplyInputs<'a, Account, TxId> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        transaction_id: TxId,
        account_exists: bool,
        account_seq_proxy: SeqProxy,
        tx_seq_proxy: SeqProxy,
        ticket_exists: bool,
        fee_context: QueueApplyFeeContext,
        applied_account: &'a Account,
    ) -> Self {
        Self {
            transaction_id,
            account_exists,
            account_seq_proxy,
            tx_seq_proxy,
            ticket_exists,
            fee_context,
            applied_account,
        }
    }
}

pub fn run_queue_apply_entry_stage<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    RunDirectApply,
    RunQueuedStage,
>(
    account_exists: bool,
    account_seq_proxy: SeqProxy,
    tx_seq_proxy: SeqProxy,
    ticket_exists: bool,
    run_direct_apply: RunDirectApply,
    run_queued_stage: RunQueuedStage,
) -> QueueApplyEntryStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    RunDirectApply: FnOnce() -> Option<DirectApplyExecution<Account, TxId>>,
    RunQueuedStage: FnOnce() -> QueueApplyQueuedStage<Account, Tx, Journal, ParentBatchId>,
{
    if let Some(execution) = run_direct_apply() {
        return QueueApplyEntryStage::DirectApplied(execution);
    }

    let prerequisite = evaluate_queue_apply_prerequisite(
        account_exists,
        account_seq_proxy,
        tx_seq_proxy,
        ticket_exists,
    );
    if prerequisite != QueueApplyPrerequisite::Ready {
        return QueueApplyEntryStage::RejectPrerequisite(prerequisite);
    }

    QueueApplyEntryStage::Queued(run_queued_stage())
}

pub fn run_queue_apply_entry_stage_with_log_messages<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    RunDirectApply,
    RunQueuedStage,
>(
    account_exists: bool,
    account_seq_proxy: SeqProxy,
    tx_seq_proxy: SeqProxy,
    ticket_exists: bool,
    run_direct_apply: RunDirectApply,
    run_queued_stage: RunQueuedStage,
) -> QueueApplyEntryStageWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
where
    RunDirectApply: FnOnce() -> Option<DirectApplyExecution<Account, TxId>>,
    RunQueuedStage:
        FnOnce() -> QueueApplyQueuedStageWithLogMessagesResult<Account, Tx, Journal, ParentBatchId>,
{
    if let Some(execution) = run_direct_apply() {
        return QueueApplyEntryStageWithLogMessagesResult {
            stage: QueueApplyEntryStage::DirectApplied(execution),
            queue_log_messages: QueueApplyQueueLogMessages::default(),
        };
    }

    let prerequisite = evaluate_queue_apply_prerequisite(
        account_exists,
        account_seq_proxy,
        tx_seq_proxy,
        ticket_exists,
    );
    if prerequisite != QueueApplyPrerequisite::Ready {
        return QueueApplyEntryStageWithLogMessagesResult {
            stage: QueueApplyEntryStage::RejectPrerequisite(prerequisite),
            queue_log_messages: QueueApplyQueueLogMessages::default(),
        };
    }

    let queued = run_queued_stage();

    QueueApplyEntryStageWithLogMessagesResult {
        stage: QueueApplyEntryStage::Queued(queued.stage),
        queue_log_messages: queued.queue_log_messages,
    }
}

pub fn run_queue_apply_entry_with_direct_apply<
    Account,
    T,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TraceFn,
    ApplyFn,
    RunQueuedStage,
>(
    views: &mut QueueViews<Account, T>,
    inputs: QueueApplyEntryWithDirectApplyInputs<'_, Account, TxId>,
    trace: TraceFn,
    apply: ApplyFn,
    run_queued_stage: RunQueuedStage,
) -> QueueApplyEntryStage<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    TxId: Clone + Display,
    TraceFn: FnMut(&str),
    ApplyFn: FnOnce() -> ApplyResult,
    RunQueuedStage: FnOnce(
        &mut QueueViews<Account, T>,
        QueueApplyFeeContext,
    ) -> QueueApplyQueuedStage<Account, Tx, Journal, ParentBatchId>,
{
    let direct_applied = run_direct_apply_with_trace_if_eligible(
        views,
        inputs.transaction_id,
        inputs.account_exists,
        inputs.account_seq_proxy,
        inputs.tx_seq_proxy,
        inputs.fee_context.fee_level_paid,
        inputs.fee_context.required_fee_level,
        inputs.applied_account,
        trace,
        apply,
    );

    run_queue_apply_entry_stage(
        inputs.account_exists,
        inputs.account_seq_proxy,
        inputs.tx_seq_proxy,
        inputs.ticket_exists,
        || direct_applied,
        || run_queued_stage(views, inputs.fee_context),
    )
}

pub fn run_queue_apply_entry_with_direct_apply_and_log_messages<
    Account,
    T,
    Tx,
    Journal,
    ParentBatchId,
    TxId,
    TraceFn,
    ApplyFn,
    RunQueuedStage,
>(
    views: &mut QueueViews<Account, T>,
    inputs: QueueApplyEntryWithDirectApplyInputs<'_, Account, TxId>,
    trace: TraceFn,
    apply: ApplyFn,
    run_queued_stage: RunQueuedStage,
) -> QueueApplyEntryStageWithLogMessagesResult<Account, Tx, Journal, ParentBatchId, TxId>
where
    Account: Clone + Display + Ord + PartialEq,
    TxId: Clone + Display,
    TraceFn: FnMut(&str),
    ApplyFn: FnOnce() -> ApplyResult,
    RunQueuedStage:
        FnOnce(
            &mut QueueViews<Account, T>,
            QueueApplyFeeContext,
        )
            -> QueueApplyQueuedStageWithLogMessagesResult<Account, Tx, Journal, ParentBatchId>,
{
    let direct_applied = run_direct_apply_with_trace_if_eligible(
        views,
        inputs.transaction_id,
        inputs.account_exists,
        inputs.account_seq_proxy,
        inputs.tx_seq_proxy,
        inputs.fee_context.fee_level_paid,
        inputs.fee_context.required_fee_level,
        inputs.applied_account,
        trace,
        apply,
    );

    run_queue_apply_entry_stage_with_log_messages(
        inputs.account_exists,
        inputs.account_seq_proxy,
        inputs.tx_seq_proxy,
        inputs.ticket_exists,
        || direct_applied,
        || run_queued_stage(views, inputs.fee_context),
    )
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, cell::RefCell, collections::BTreeMap};

    use basics::base_uint::Uint256;
    use protocol::{SeqProxy, Ter};

    use super::{
        QueueApplyEntryStage, QueueApplyEntryWithDirectApplyInputs, run_queue_apply_entry_stage,
        run_queue_apply_entry_with_direct_apply,
    };
    use crate::{
        ApplyResult, DirectApplyAttemptResult, DirectApplyExecution, QueueApplyFeeContext,
        QueueApplyPrerequisite, QueueApplyQueuedStage, QueueViews, TxConsequences, TxQAccount,
    };

    #[test]
    fn entry_stage_returns_direct_apply_before_prerequisite_checks() {
        let ran_queued = Cell::new(false);
        let direct = DirectApplyExecution {
            transaction_id: "tx",
            attempt: DirectApplyAttemptResult {
                apply_result: ApplyResult::new(Ter::TEF_NO_TICKET, false, false),
                removed_replacement: None::<crate::FeeQueueKey<&'static str>>,
            },
        };

        let stage = run_queue_apply_entry_stage(
            false,
            SeqProxy::sequence(10),
            SeqProxy::ticket(5),
            false,
            || Some(direct.clone()),
            || {
                ran_queued.set(true);
                QueueApplyQueuedStage::<
                    &'static str,
                    &'static str,
                    &'static str,
                    &'static str,
                >::MultiTxn(crate::QueueApplyMultiTxnStage::RejectPath(Ter::TER_PRE_SEQ))
            },
        );

        assert_eq!(stage, QueueApplyEntryStage::DirectApplied(direct));
        assert_eq!(
            stage.apply_result(),
            ApplyResult::new(Ter::TEF_NO_TICKET, false, false)
        );
        assert!(!ran_queued.get());
    }

    #[test]
    fn entry_stage_returns_missing_account_when_direct_apply_falls_through() {
        let ran_queued = Cell::new(false);

        let stage = run_queue_apply_entry_stage::<
            &'static str,
            &'static str,
            &'static str,
            &'static str,
            &'static str,
            _,
            _,
        >(
            false,
            SeqProxy::sequence(10),
            SeqProxy::sequence(10),
            false,
            || None,
            || {
                ran_queued.set(true);
                QueueApplyQueuedStage::<
                    &'static str,
                    &'static str,
                    &'static str,
                    &'static str,
                >::MultiTxn(crate::QueueApplyMultiTxnStage::RejectPath(Ter::TER_PRE_SEQ))
            },
        );

        assert_eq!(
            stage,
            QueueApplyEntryStage::RejectPrerequisite(QueueApplyPrerequisite::MissingAccount)
        );
        assert_eq!(
            stage.apply_result(),
            ApplyResult::new(Ter::TER_NO_ACCOUNT, false, false)
        );
        assert!(!ran_queued.get());
    }

    #[test]
    fn entry_stage_returns_missing_ticket_rejection_before_queue_stage() {
        let ran_queued = Cell::new(false);

        let stage = run_queue_apply_entry_stage::<
            &'static str,
            &'static str,
            &'static str,
            &'static str,
            &'static str,
            _,
            _,
        >(
            true,
            SeqProxy::sequence(8),
            SeqProxy::ticket(7),
            false,
            || None,
            || {
                ran_queued.set(true);
                QueueApplyQueuedStage::<
                    &'static str,
                    &'static str,
                    &'static str,
                    &'static str,
                >::MultiTxn(crate::QueueApplyMultiTxnStage::RejectPath(Ter::TER_PRE_SEQ))
            },
        );

        assert_eq!(
            stage,
            QueueApplyEntryStage::RejectPrerequisite(QueueApplyPrerequisite::MissingTicketPast)
        );
        assert_eq!(
            stage.apply_result(),
            ApplyResult::new(Ter::TEF_NO_TICKET, false, false)
        );
        assert!(!ran_queued.get());
    }

    #[test]
    fn entry_stage_delegates_to_queued_stage_after_direct_apply_and_prerequisites_clear() {
        let queued_result = QueueApplyQueuedStage::<
            &'static str,
            &'static str,
            &'static str,
            &'static str,
        >::MultiTxn(crate::QueueApplyMultiTxnStage::RejectPath(
            Ter::TER_PRE_SEQ,
        ));

        let stage = run_queue_apply_entry_stage(
            true,
            SeqProxy::sequence(8),
            SeqProxy::sequence(8),
            false,
            || None::<DirectApplyExecution<&'static str, &'static str>>,
            || queued_result.clone(),
        );

        assert_eq!(stage, QueueApplyEntryStage::Queued(queued_result));
        assert_eq!(
            stage.apply_result(),
            ApplyResult::new(Ter::TER_PRE_SEQ, false, false)
        );
    }

    #[test]
    fn entry_wrapper_with_direct_apply_returns_direct_apply_before_prerequisites_or_queue() {
        let mut traces = Vec::new();
        let mut queued_called = false;
        let mut views = QueueViews::<&'static str, ()>::new(BTreeMap::new(), vec![]);
        let fee_context = QueueApplyFeeContext {
            base_level: 256,
            fee_level_paid: 500,
            required_fee_level: 300,
        };

        let stage = run_queue_apply_entry_with_direct_apply(
            &mut views,
            QueueApplyEntryWithDirectApplyInputs::new(
                "ABC123",
                true,
                SeqProxy::sequence(8),
                SeqProxy::sequence(8),
                false,
                fee_context,
                &"acct",
            ),
            |line| traces.push(line.to_owned()),
            || ApplyResult::new(Ter::TES_SUCCESS, true, true),
            |_, _| {
                queued_called = true;
                QueueApplyQueuedStage::<&'static str, (), (), ()>::MultiTxn(
                    crate::QueueApplyMultiTxnStage::RejectPath(Ter::TER_PRE_SEQ),
                )
            },
        );

        assert!(matches!(stage, QueueApplyEntryStage::DirectApplied(_)));
        assert_eq!(
            stage.apply_result(),
            ApplyResult::new(Ter::TES_SUCCESS, true, true)
        );
        assert!(!queued_called);
        assert_eq!(
            traces,
            vec![
                "Applying transaction ABC123 to open ledger.".to_owned(),
                "New transaction ABC123 applied successfully with tesSUCCESS".to_owned(),
            ]
        );
    }

    #[test]
    fn entry_wrapper_with_direct_apply_falls_through_to_prerequisite_rejection() {
        let traces = RefCell::new(Vec::new());
        let mut views = QueueViews::<&'static str, ()>::new(BTreeMap::new(), vec![]);
        let fee_context = QueueApplyFeeContext {
            base_level: 256,
            fee_level_paid: 200,
            required_fee_level: 300,
        };

        let stage = run_queue_apply_entry_with_direct_apply::<
            &'static str,
            (),
            (),
            (),
            (),
            &'static str,
            _,
            _,
            _,
        >(
            &mut views,
            QueueApplyEntryWithDirectApplyInputs::new(
                "ABC123",
                true,
                SeqProxy::sequence(8),
                SeqProxy::ticket(7),
                false,
                fee_context,
                &"acct",
            ),
            |line| traces.borrow_mut().push(line.to_owned()),
            || unreachable!("ineligible direct apply should not invoke apply"),
            |_, _| unreachable!("missing ticket should return before queued stage"),
        );

        assert_eq!(
            stage,
            QueueApplyEntryStage::RejectPrerequisite(QueueApplyPrerequisite::MissingTicketPast)
        );
        assert!(traces.borrow().is_empty());
    }

    #[test]
    fn entry_wrapper_with_direct_apply_passes_fee_context_into_queued_fallthrough() {
        let account = "acct";
        let mut account_queue = TxQAccount::new(account);
        account_queue.add(
            SeqProxy::sequence(8),
            crate::MaybeTxCore::new(
                Uint256::from_u64(1),
                TxConsequences::new(1, SeqProxy::sequence(8)),
            ),
        );
        let mut views = QueueViews::new(BTreeMap::from([(account, account_queue)]), vec![]);
        let fee_context = QueueApplyFeeContext {
            base_level: 256,
            fee_level_paid: 200,
            required_fee_level: 300,
        };

        let stage = run_queue_apply_entry_with_direct_apply(
            &mut views,
            QueueApplyEntryWithDirectApplyInputs::new(
                "ABC123",
                true,
                SeqProxy::sequence(8),
                SeqProxy::sequence(9),
                true,
                fee_context,
                &account,
            ),
            |_| unreachable!("ineligible direct apply should not trace"),
            || unreachable!("ineligible direct apply should not invoke apply"),
            |_, context| {
                assert_eq!(context, fee_context);
                QueueApplyQueuedStage::<&'static str, (), (), ()>::MultiTxn(
                    crate::QueueApplyMultiTxnStage::RejectPath(Ter::TER_PRE_SEQ),
                )
            },
        );

        assert_eq!(
            stage,
            QueueApplyEntryStage::Queued(QueueApplyQueuedStage::MultiTxn(
                crate::QueueApplyMultiTxnStage::RejectPath(Ter::TER_PRE_SEQ)
            ))
        );
        assert_eq!(
            stage.apply_result(),
            ApplyResult::new(Ter::TER_PRE_SEQ, false, false)
        );
    }
}
