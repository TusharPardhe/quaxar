//! Queued-path wrapper inside `TxQ::apply(...)` after direct apply is no
//! longer available.
//!
//! This composes the already-landed account-window inspection, queued-blocker
//! admission, replacement-fee rule, account-path split, pre-`multiTxn`
//! admission, queued-balance gate, staged view-adjustment, staged preclaim-view
//! preparation, and the later queue-flow carrier.

use basics::base_uint::Uint256;
use protocol::{SeqProxy, Ter};

use crate::{
    AccountQueueWindow, ApplyFlags, ApplyResult, FeeLevel64, MaybeTx, OrderCandidates,
    PreclaimResult, PreflightResult, QueueApplyAccountStage, QueueApplyFeeContext,
    QueueApplyFlowStage, QueueApplyFlowStageWithLogMessagesResult, QueueApplyFullQueueDecision,
    QueueApplyHoldFallback, QueueApplyMultiTxnInputs, QueueApplyMultiTxnStage, QueueApplyPath,
    QueueApplyPreclaimViewSource, QueueApplyPreclaimViewStage, QueueApplyQueueLogMessages,
    QueueApplyTryClearResult, QueueApplyViewAdjustment, QueueHoldPreflight, QueueViews,
    ReplacementFeeDecision, TxConsequences, evaluate_queue_apply_full_queue,
    run_queue_apply_account_stage, run_queue_apply_flow_stage,
    run_queue_apply_flow_stage_with_log_messages, run_queue_apply_flow_stage_with_log_sinks,
    run_queue_apply_multitxn_stage, run_queue_apply_preclaim_view_stage,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueApplyQueuedStage<Account, Tx, Journal, ParentBatchId> {
    Account(QueueApplyAccountStage<Account>),
    MultiTxn(QueueApplyMultiTxnStage),
    PreclaimView(QueueApplyPreclaimViewStage),
    Flow {
        window: AccountQueueWindow,
        replacement_decision: Option<ReplacementFeeDecision>,
        path: QueueApplyPath,
        view_adjustment: Option<QueueApplyViewAdjustment>,
        flow: QueueApplyFlowStage<Account, Tx, Journal, ParentBatchId>,
    },
}

impl<Account, Tx, Journal, ParentBatchId>
    QueueApplyQueuedStage<Account, Tx, Journal, ParentBatchId>
{
    pub fn apply_result(&self) -> ApplyResult {
        match self {
            Self::Account(stage) => stage.apply_result(),
            Self::MultiTxn(stage) => stage.apply_result(),
            Self::PreclaimView(stage) => stage.apply_result(),
            Self::Flow { flow, .. } => flow.apply_result(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueApplyQueuedStageWithLogMessagesResult<Account, Tx, Journal, ParentBatchId> {
    pub stage: QueueApplyQueuedStage<Account, Tx, Journal, ParentBatchId>,
    pub queue_log_messages: QueueApplyQueueLogMessages,
}

#[derive(Debug, Clone)]
pub struct QueueApplyQueuedWithFeeContextInputs<'a, Account> {
    pub account: Account,
    pub preflight: QueueHoldPreflight,
    pub is_blocker: bool,
    pub open_ledger_seq: u32,
    pub minimum_last_ledger_buffer: u32,
    pub maximum_txn_per_account: usize,
    pub retry_sequence_percent: u32,
    pub queue_is_full: bool,
    pub balance_drops: u64,
    pub reserve_drops: u64,
    pub base_fee_drops: u64,
    pub can_be_held_result: Ter,
    pub open_ledger_tx_count: usize,
    pub tx_id: Uint256,
    pub last_valid: Option<u32>,
    pub flags: ApplyFlags,
    pub order: &'a OrderCandidates,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueApplyPreparedFlowInputs<'a, Account, Tx, Journal, ParentBatchId> {
    pub preclaim_view_source: QueueApplyPreclaimViewSource,
    pub tx_seq_proxy: SeqProxy,
    pub first_relevant_retries_remaining: Option<i32>,
    pub fee_level_paid: FeeLevel64,
    pub base_level: FeeLevel64,
    pub required_fee_level: FeeLevel64,
    pub open_ledger_tx_count: usize,
    pub hold_fallback: QueueApplyHoldFallback,
    pub full_queue_decision: QueueApplyFullQueueDecision<Account>,
    pub replaced: Option<crate::FeeQueueKey<Account>>,
    pub account: Account,
    pub tx_id: Uint256,
    pub last_valid: Option<u32>,
    pub flags: ApplyFlags,
    pub pf_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    pub order: &'a OrderCandidates,
}

impl<'a, Account, Tx, Journal, ParentBatchId>
    QueueApplyPreparedFlowInputs<'a, Account, Tx, Journal, ParentBatchId>
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        preclaim_view_source: QueueApplyPreclaimViewSource,
        tx_seq_proxy: SeqProxy,
        first_relevant_retries_remaining: Option<i32>,
        fee_level_paid: FeeLevel64,
        base_level: FeeLevel64,
        required_fee_level: FeeLevel64,
        open_ledger_tx_count: usize,
        hold_fallback: QueueApplyHoldFallback,
        full_queue_decision: QueueApplyFullQueueDecision<Account>,
        replaced: Option<crate::FeeQueueKey<Account>>,
        account: Account,
        tx_id: Uint256,
        last_valid: Option<u32>,
        flags: ApplyFlags,
        pf_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        order: &'a OrderCandidates,
    ) -> Self {
        Self {
            preclaim_view_source,
            tx_seq_proxy,
            first_relevant_retries_remaining,
            fee_level_paid,
            base_level,
            required_fee_level,
            open_ledger_tx_count,
            hold_fallback,
            full_queue_decision,
            replaced,
            account,
            tx_id,
            last_valid,
            flags,
            pf_result,
            order,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueApplyPreparedQueuedFlowStage<'a, Account, Tx, Journal, ParentBatchId> {
    Account(QueueApplyAccountStage<Account>),
    MultiTxn(QueueApplyMultiTxnStage),
    PreclaimView(QueueApplyPreclaimViewStage),
    Flow {
        window: AccountQueueWindow,
        replacement_decision: Option<ReplacementFeeDecision>,
        path: QueueApplyPath,
        view_adjustment: Option<QueueApplyViewAdjustment>,
        prepared: QueueApplyPreparedFlowInputs<'a, Account, Tx, Journal, ParentBatchId>,
    },
}

impl<'a, Account, Tx, Journal, ParentBatchId>
    QueueApplyPreparedQueuedFlowStage<'a, Account, Tx, Journal, ParentBatchId>
{
    pub fn apply_result(&self) -> ApplyResult {
        match self {
            Self::Account(stage) => stage.apply_result(),
            Self::MultiTxn(stage) => stage.apply_result(),
            Self::PreclaimView(stage) => stage.apply_result(),
            Self::Flow { .. } => ApplyResult::new(Ter::TES_SUCCESS, false, false),
        }
    }
}

pub fn run_prepared_queue_apply_queued_flow_stage<
    'a,
    Account,
    Tx,
    Journal,
    ParentBatchId,
    RunFlow,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    stage: QueueApplyPreparedQueuedFlowStage<'a, Account, Tx, Journal, ParentBatchId>,
    run_flow: RunFlow,
) -> QueueApplyQueuedStage<Account, Tx, Journal, ParentBatchId>
where
    RunFlow: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedFlowInputs<'a, Account, Tx, Journal, ParentBatchId>,
    ) -> QueueApplyFlowStage<Account, Tx, Journal, ParentBatchId>,
{
    match stage {
        QueueApplyPreparedQueuedFlowStage::Account(stage) => QueueApplyQueuedStage::Account(stage),
        QueueApplyPreparedQueuedFlowStage::MultiTxn(stage) => {
            QueueApplyQueuedStage::MultiTxn(stage)
        }
        QueueApplyPreparedQueuedFlowStage::PreclaimView(stage) => {
            QueueApplyQueuedStage::PreclaimView(stage)
        }
        QueueApplyPreparedQueuedFlowStage::Flow {
            window,
            replacement_decision,
            path,
            view_adjustment,
            prepared,
        } => {
            let flow = run_flow(views, prepared);

            QueueApplyQueuedStage::Flow {
                window,
                replacement_decision,
                path,
                view_adjustment,
                flow,
            }
        }
    }
}

pub fn run_prepared_queue_apply_queued_flow_stage_with_log_messages<
    'a,
    Account,
    Tx,
    Journal,
    ParentBatchId,
    RunFlow,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    stage: QueueApplyPreparedQueuedFlowStage<'a, Account, Tx, Journal, ParentBatchId>,
    run_flow: RunFlow,
) -> QueueApplyQueuedStageWithLogMessagesResult<Account, Tx, Journal, ParentBatchId>
where
    RunFlow:
        FnOnce(
            &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
            QueueApplyPreparedFlowInputs<'a, Account, Tx, Journal, ParentBatchId>,
        )
            -> QueueApplyFlowStageWithLogMessagesResult<Account, Tx, Journal, ParentBatchId>,
{
    match stage {
        QueueApplyPreparedQueuedFlowStage::Account(stage) => {
            QueueApplyQueuedStageWithLogMessagesResult {
                stage: QueueApplyQueuedStage::Account(stage),
                queue_log_messages: QueueApplyQueueLogMessages::default(),
            }
        }
        QueueApplyPreparedQueuedFlowStage::MultiTxn(stage) => {
            QueueApplyQueuedStageWithLogMessagesResult {
                stage: QueueApplyQueuedStage::MultiTxn(stage),
                queue_log_messages: QueueApplyQueueLogMessages::default(),
            }
        }
        QueueApplyPreparedQueuedFlowStage::PreclaimView(stage) => {
            QueueApplyQueuedStageWithLogMessagesResult {
                stage: QueueApplyQueuedStage::PreclaimView(stage),
                queue_log_messages: QueueApplyQueueLogMessages::default(),
            }
        }
        QueueApplyPreparedQueuedFlowStage::Flow {
            window,
            replacement_decision,
            path,
            view_adjustment,
            prepared,
        } => {
            let flow = run_flow(views, prepared);

            QueueApplyQueuedStageWithLogMessagesResult {
                stage: QueueApplyQueuedStage::Flow {
                    window,
                    replacement_decision,
                    path,
                    view_adjustment,
                    flow: flow.stage,
                },
                queue_log_messages: flow.queue_log_messages,
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn derive_queue_apply_prepared_flow_stage<
    'a,
    Account,
    Tx,
    Journal,
    ParentBatchId,
    PrepareMultiTxn,
>(
    views: &QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    account_seq_proxy: SeqProxy,
    tx_seq_proxy: SeqProxy,
    inputs: QueueApplyQueuedWithFeeContextInputs<'a, Account>,
    fee_context: QueueApplyFeeContext,
    pf_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    prepare_multitxn: PrepareMultiTxn,
) -> QueueApplyPreparedQueuedFlowStage<'a, Account, Tx, Journal, ParentBatchId>
where
    Account: Clone + std::fmt::Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
{
    let QueueApplyQueuedWithFeeContextInputs {
        account,
        preflight,
        is_blocker,
        open_ledger_seq,
        minimum_last_ledger_buffer,
        maximum_txn_per_account,
        retry_sequence_percent,
        queue_is_full,
        balance_drops,
        reserve_drops,
        base_fee_drops,
        can_be_held_result,
        open_ledger_tx_count,
        tx_id,
        last_valid,
        flags,
        order,
    } = inputs;
    let QueueApplyFeeContext {
        fee_level_paid,
        required_fee_level,
        base_level,
    } = fee_context;

    let account_stage = run_queue_apply_account_stage(
        views,
        &account,
        account_seq_proxy,
        tx_seq_proxy,
        is_blocker,
        fee_level_paid,
        retry_sequence_percent,
    );
    let account_context = match &account_stage {
        QueueApplyAccountStage::Ready(context) => context,
        _ => return QueueApplyPreparedQueuedFlowStage::Account(account_stage),
    };
    let tx_q_account = views.accounts.get(&account);
    let window = account_context.window;
    let replacement_decision = account_context.replacement_decision;

    let multitxn_stage = run_queue_apply_multitxn_stage(
        tx_q_account,
        account_context,
        QueueApplyMultiTxnInputs {
            preflight,
            open_ledger_seq,
            minimum_last_ledger_buffer,
            maximum_txn_per_account,
            account_seq_proxy,
            tx_seq_proxy,
            balance_drops,
            reserve_drops,
            base_fee_drops,
            can_be_held_result,
            consequences: pf_result.consequences,
        },
    );
    let multitxn_context = match multitxn_stage {
        QueueApplyMultiTxnStage::Ready(context) => context,
        _ => return QueueApplyPreparedQueuedFlowStage::MultiTxn(multitxn_stage),
    };
    let path = multitxn_context.path;

    let first_relevant_retries_remaining = account_context.first_relevant_retries_remaining;
    let view_adjustment = multitxn_context.view_adjustment;
    let preclaim_view_stage =
        run_queue_apply_preclaim_view_stage(view_adjustment, prepare_multitxn);
    let preclaim_view_context = match preclaim_view_stage {
        QueueApplyPreclaimViewStage::Ready(context) => context,
        _ => return QueueApplyPreparedQueuedFlowStage::PreclaimView(preclaim_view_stage),
    };
    let hold_fallback = multitxn_context.hold_fallback;
    let full_queue_decision = evaluate_queue_apply_full_queue(
        window.replaces_existing,
        queue_is_full,
        &account,
        fee_level_paid,
        &views.fee_order,
        &views.accounts,
        |queued| queued.payload.fee_level,
    );
    let replaced = account_context.replaced.clone();

    QueueApplyPreparedQueuedFlowStage::Flow {
        window,
        replacement_decision,
        path,
        view_adjustment,
        prepared: QueueApplyPreparedFlowInputs::new(
            preclaim_view_context.view_source,
            tx_seq_proxy,
            first_relevant_retries_remaining,
            fee_level_paid,
            base_level,
            required_fee_level,
            open_ledger_tx_count,
            hold_fallback,
            full_queue_decision,
            replaced,
            account,
            tx_id,
            last_valid,
            flags,
            pf_result,
            order,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_queued_stage<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    RunPreclaim,
    RunTryClear,
    TryClearResult,
    ApplySandbox,
    PrepareMultiTxn,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    account: Account,
    account_seq_proxy: SeqProxy,
    tx_seq_proxy: SeqProxy,
    preflight: QueueHoldPreflight,
    is_blocker: bool,
    open_ledger_seq: u32,
    minimum_last_ledger_buffer: u32,
    maximum_txn_per_account: usize,
    retry_sequence_percent: u32,
    queue_is_full: bool,
    fee_level_paid: FeeLevel64,
    required_fee_level: FeeLevel64,
    base_level: FeeLevel64,
    balance_drops: u64,
    reserve_drops: u64,
    base_fee_drops: u64,
    can_be_held_result: Ter,
    open_ledger_tx_count: usize,
    tx_id: Uint256,
    last_valid: Option<u32>,
    flags: ApplyFlags,
    pf_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    order: &OrderCandidates,
    prepare_multitxn: PrepareMultiTxn,
    run_preclaim: RunPreclaim,
    run_try_clear: RunTryClear,
    apply_sandbox: ApplySandbox,
) -> QueueApplyQueuedStage<Account, Tx, Journal, ParentBatchId>
where
    Account: Clone + std::fmt::Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    RunPreclaim:
        FnOnce(crate::QueueApplyPreclaimViewSource) -> PreclaimResult<Tx, Journal, ParentBatchId>,
    RunTryClear: FnOnce() -> TryClearResult,
    TryClearResult: QueueApplyTryClearResult,
    ApplySandbox: FnOnce(),
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
{
    run_queue_apply_queued_stage_with_caller_flow(
        views,
        account,
        account_seq_proxy,
        tx_seq_proxy,
        preflight,
        is_blocker,
        open_ledger_seq,
        minimum_last_ledger_buffer,
        maximum_txn_per_account,
        retry_sequence_percent,
        queue_is_full,
        fee_level_paid,
        required_fee_level,
        base_level,
        balance_drops,
        reserve_drops,
        base_fee_drops,
        can_be_held_result,
        open_ledger_tx_count,
        tx_id,
        last_valid,
        flags,
        pf_result,
        order,
        prepare_multitxn,
        |views, prepared| {
            run_queue_apply_flow_stage(
                views,
                prepared.preclaim_view_source,
                prepared.tx_seq_proxy,
                prepared.first_relevant_retries_remaining,
                prepared.fee_level_paid,
                prepared.base_level,
                prepared.required_fee_level,
                prepared.open_ledger_tx_count,
                prepared.hold_fallback,
                prepared.full_queue_decision,
                prepared.replaced,
                prepared.account,
                prepared.tx_id,
                prepared.last_valid,
                prepared.flags,
                prepared.pf_result,
                prepared.order,
                run_preclaim,
                run_try_clear,
                apply_sandbox,
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_queued_stage_with_caller_flow<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    PrepareMultiTxn,
    RunFlow,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    account: Account,
    account_seq_proxy: SeqProxy,
    tx_seq_proxy: SeqProxy,
    preflight: QueueHoldPreflight,
    is_blocker: bool,
    open_ledger_seq: u32,
    minimum_last_ledger_buffer: u32,
    maximum_txn_per_account: usize,
    retry_sequence_percent: u32,
    queue_is_full: bool,
    fee_level_paid: FeeLevel64,
    required_fee_level: FeeLevel64,
    base_level: FeeLevel64,
    balance_drops: u64,
    reserve_drops: u64,
    base_fee_drops: u64,
    can_be_held_result: Ter,
    open_ledger_tx_count: usize,
    tx_id: Uint256,
    last_valid: Option<u32>,
    flags: ApplyFlags,
    pf_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    order: &OrderCandidates,
    prepare_multitxn: PrepareMultiTxn,
    run_flow: RunFlow,
) -> QueueApplyQueuedStage<Account, Tx, Journal, ParentBatchId>
where
    Account: Clone + std::fmt::Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
    RunFlow: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedFlowInputs<'_, Account, Tx, Journal, ParentBatchId>,
    ) -> QueueApplyFlowStage<Account, Tx, Journal, ParentBatchId>,
{
    run_prepared_queue_apply_queued_flow_stage(
        views,
        derive_queue_apply_prepared_flow_stage(
            &*views,
            account_seq_proxy,
            tx_seq_proxy,
            QueueApplyQueuedWithFeeContextInputs {
                account,
                preflight,
                is_blocker,
                open_ledger_seq,
                minimum_last_ledger_buffer,
                maximum_txn_per_account,
                retry_sequence_percent,
                queue_is_full,
                balance_drops,
                reserve_drops,
                base_fee_drops,
                can_be_held_result,
                open_ledger_tx_count,
                tx_id,
                last_valid,
                flags,
                order,
            },
            QueueApplyFeeContext {
                fee_level_paid,
                required_fee_level,
                base_level,
            },
            pf_result,
            prepare_multitxn,
        ),
        run_flow,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_queued_stage_with_log_messages<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    RunPreclaim,
    RunTryClear,
    TryClearResult,
    ApplySandbox,
    PrepareMultiTxn,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    account: Account,
    account_seq_proxy: SeqProxy,
    tx_seq_proxy: SeqProxy,
    preflight: QueueHoldPreflight,
    is_blocker: bool,
    open_ledger_seq: u32,
    minimum_last_ledger_buffer: u32,
    maximum_txn_per_account: usize,
    retry_sequence_percent: u32,
    queue_is_full: bool,
    fee_level_paid: FeeLevel64,
    required_fee_level: FeeLevel64,
    base_level: FeeLevel64,
    balance_drops: u64,
    reserve_drops: u64,
    base_fee_drops: u64,
    can_be_held_result: Ter,
    open_ledger_tx_count: usize,
    tx_id: Uint256,
    last_valid: Option<u32>,
    flags: ApplyFlags,
    pf_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    order: &OrderCandidates,
    prepare_multitxn: PrepareMultiTxn,
    run_preclaim: RunPreclaim,
    run_try_clear: RunTryClear,
    apply_sandbox: ApplySandbox,
) -> QueueApplyQueuedStageWithLogMessagesResult<Account, Tx, Journal, ParentBatchId>
where
    Account: Clone + std::fmt::Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    RunPreclaim:
        FnOnce(crate::QueueApplyPreclaimViewSource) -> PreclaimResult<Tx, Journal, ParentBatchId>,
    RunTryClear: FnOnce() -> TryClearResult,
    TryClearResult: QueueApplyTryClearResult,
    ApplySandbox: FnOnce(),
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
{
    run_queue_apply_queued_stage_with_log_messages_and_caller_flow(
        views,
        account,
        account_seq_proxy,
        tx_seq_proxy,
        preflight,
        is_blocker,
        open_ledger_seq,
        minimum_last_ledger_buffer,
        maximum_txn_per_account,
        retry_sequence_percent,
        queue_is_full,
        fee_level_paid,
        required_fee_level,
        base_level,
        balance_drops,
        reserve_drops,
        base_fee_drops,
        can_be_held_result,
        open_ledger_tx_count,
        tx_id,
        last_valid,
        flags,
        pf_result,
        order,
        prepare_multitxn,
        |views, prepared| {
            run_queue_apply_flow_stage_with_log_messages(
                views,
                prepared.preclaim_view_source,
                prepared.tx_seq_proxy,
                prepared.first_relevant_retries_remaining,
                prepared.fee_level_paid,
                prepared.base_level,
                prepared.required_fee_level,
                prepared.open_ledger_tx_count,
                prepared.hold_fallback,
                prepared.full_queue_decision,
                prepared.replaced,
                prepared.account,
                prepared.tx_id,
                prepared.last_valid,
                prepared.flags,
                prepared.pf_result,
                prepared.order,
                run_preclaim,
                run_try_clear,
                apply_sandbox,
            )
        },
    )
}

pub fn run_queue_apply_queued_stage_with_fee_context<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    RunPreclaim,
    RunTryClear,
    TryClearResult,
    ApplySandbox,
    PrepareMultiTxn,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    account_seq_proxy: SeqProxy,
    tx_seq_proxy: SeqProxy,
    inputs: QueueApplyQueuedWithFeeContextInputs<'_, Account>,
    fee_context: QueueApplyFeeContext,
    pf_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    prepare_multitxn: PrepareMultiTxn,
    run_preclaim: RunPreclaim,
    run_try_clear: RunTryClear,
    apply_sandbox: ApplySandbox,
) -> QueueApplyQueuedStage<Account, Tx, Journal, ParentBatchId>
where
    Account: Clone + std::fmt::Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    RunPreclaim:
        FnOnce(crate::QueueApplyPreclaimViewSource) -> PreclaimResult<Tx, Journal, ParentBatchId>,
    RunTryClear: FnOnce() -> TryClearResult,
    TryClearResult: QueueApplyTryClearResult,
    ApplySandbox: FnOnce(),
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
{
    run_queue_apply_queued_stage_with_fee_context_and_caller_flow(
        views,
        account_seq_proxy,
        tx_seq_proxy,
        inputs,
        fee_context,
        pf_result,
        prepare_multitxn,
        |views, prepared| {
            run_queue_apply_flow_stage(
                views,
                prepared.preclaim_view_source,
                prepared.tx_seq_proxy,
                prepared.first_relevant_retries_remaining,
                prepared.fee_level_paid,
                prepared.base_level,
                prepared.required_fee_level,
                prepared.open_ledger_tx_count,
                prepared.hold_fallback,
                prepared.full_queue_decision,
                prepared.replaced,
                prepared.account,
                prepared.tx_id,
                prepared.last_valid,
                prepared.flags,
                prepared.pf_result,
                prepared.order,
                run_preclaim,
                run_try_clear,
                apply_sandbox,
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_queued_stage_with_fee_context_and_log_sinks<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    RunPreclaim,
    RunTryClear,
    TryClearResult,
    ApplySandbox,
    PrepareMultiTxn,
    DebugSink,
    InfoSink,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    account_seq_proxy: SeqProxy,
    tx_seq_proxy: SeqProxy,
    inputs: QueueApplyQueuedWithFeeContextInputs<'_, Account>,
    fee_context: QueueApplyFeeContext,
    pf_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    prepare_multitxn: PrepareMultiTxn,
    run_preclaim: RunPreclaim,
    run_try_clear: RunTryClear,
    apply_sandbox: ApplySandbox,
    mut debug: DebugSink,
    mut info: InfoSink,
) -> QueueApplyQueuedStage<Account, Tx, Journal, ParentBatchId>
where
    Account: Clone + std::fmt::Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    RunPreclaim:
        FnOnce(crate::QueueApplyPreclaimViewSource) -> PreclaimResult<Tx, Journal, ParentBatchId>,
    RunTryClear: FnOnce() -> TryClearResult,
    TryClearResult: QueueApplyTryClearResult,
    ApplySandbox: FnOnce(),
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
    DebugSink: FnMut(String),
    InfoSink: FnMut(String),
{
    run_queue_apply_queued_stage_with_fee_context_and_caller_flow(
        views,
        account_seq_proxy,
        tx_seq_proxy,
        inputs,
        fee_context,
        pf_result,
        prepare_multitxn,
        |views, prepared| {
            run_queue_apply_flow_stage_with_log_sinks(
                views,
                prepared.preclaim_view_source,
                prepared.tx_seq_proxy,
                prepared.first_relevant_retries_remaining,
                prepared.fee_level_paid,
                prepared.base_level,
                prepared.required_fee_level,
                prepared.open_ledger_tx_count,
                prepared.hold_fallback,
                prepared.full_queue_decision,
                prepared.replaced,
                prepared.account,
                prepared.tx_id,
                prepared.last_valid,
                prepared.flags,
                prepared.pf_result,
                prepared.order,
                run_preclaim,
                run_try_clear,
                apply_sandbox,
                |message| debug(message),
                |message| info(message),
            )
        },
    )
}

pub fn run_queue_apply_queued_stage_with_fee_context_and_log_messages<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    RunPreclaim,
    RunTryClear,
    TryClearResult,
    ApplySandbox,
    PrepareMultiTxn,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    account_seq_proxy: SeqProxy,
    tx_seq_proxy: SeqProxy,
    inputs: QueueApplyQueuedWithFeeContextInputs<'_, Account>,
    fee_context: QueueApplyFeeContext,
    pf_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    prepare_multitxn: PrepareMultiTxn,
    run_preclaim: RunPreclaim,
    run_try_clear: RunTryClear,
    apply_sandbox: ApplySandbox,
) -> QueueApplyQueuedStageWithLogMessagesResult<Account, Tx, Journal, ParentBatchId>
where
    Account: Clone + std::fmt::Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    RunPreclaim:
        FnOnce(crate::QueueApplyPreclaimViewSource) -> PreclaimResult<Tx, Journal, ParentBatchId>,
    RunTryClear: FnOnce() -> TryClearResult,
    TryClearResult: QueueApplyTryClearResult,
    ApplySandbox: FnOnce(),
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
{
    run_queue_apply_queued_stage_with_fee_context_and_log_messages_and_caller_flow(
        views,
        account_seq_proxy,
        tx_seq_proxy,
        inputs,
        fee_context,
        pf_result,
        prepare_multitxn,
        |views, prepared| {
            run_queue_apply_flow_stage_with_log_messages(
                views,
                prepared.preclaim_view_source,
                prepared.tx_seq_proxy,
                prepared.first_relevant_retries_remaining,
                prepared.fee_level_paid,
                prepared.base_level,
                prepared.required_fee_level,
                prepared.open_ledger_tx_count,
                prepared.hold_fallback,
                prepared.full_queue_decision,
                prepared.replaced,
                prepared.account,
                prepared.tx_id,
                prepared.last_valid,
                prepared.flags,
                prepared.pf_result,
                prepared.order,
                run_preclaim,
                run_try_clear,
                apply_sandbox,
            )
        },
    )
}

pub fn run_queue_apply_queued_stage_with_fee_context_and_caller_flow<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    PrepareMultiTxn,
    RunFlow,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    account_seq_proxy: SeqProxy,
    tx_seq_proxy: SeqProxy,
    inputs: QueueApplyQueuedWithFeeContextInputs<'_, Account>,
    fee_context: QueueApplyFeeContext,
    pf_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    prepare_multitxn: PrepareMultiTxn,
    run_flow: RunFlow,
) -> QueueApplyQueuedStage<Account, Tx, Journal, ParentBatchId>
where
    Account: Clone + std::fmt::Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
    RunFlow: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedFlowInputs<'_, Account, Tx, Journal, ParentBatchId>,
    ) -> QueueApplyFlowStage<Account, Tx, Journal, ParentBatchId>,
{
    run_queue_apply_queued_stage_with_caller_flow(
        views,
        inputs.account,
        account_seq_proxy,
        tx_seq_proxy,
        inputs.preflight,
        inputs.is_blocker,
        inputs.open_ledger_seq,
        inputs.minimum_last_ledger_buffer,
        inputs.maximum_txn_per_account,
        inputs.retry_sequence_percent,
        inputs.queue_is_full,
        fee_context.fee_level_paid,
        fee_context.required_fee_level,
        fee_context.base_level,
        inputs.balance_drops,
        inputs.reserve_drops,
        inputs.base_fee_drops,
        inputs.can_be_held_result,
        inputs.open_ledger_tx_count,
        inputs.tx_id,
        inputs.last_valid,
        inputs.flags,
        pf_result,
        inputs.order,
        prepare_multitxn,
        run_flow,
    )
}

pub fn run_queue_apply_queued_stage_with_log_messages_and_caller_flow<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    PrepareMultiTxn,
    RunFlow,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    account: Account,
    account_seq_proxy: SeqProxy,
    tx_seq_proxy: SeqProxy,
    preflight: QueueHoldPreflight,
    is_blocker: bool,
    open_ledger_seq: u32,
    minimum_last_ledger_buffer: u32,
    maximum_txn_per_account: usize,
    retry_sequence_percent: u32,
    queue_is_full: bool,
    fee_level_paid: FeeLevel64,
    required_fee_level: FeeLevel64,
    base_level: FeeLevel64,
    balance_drops: u64,
    reserve_drops: u64,
    base_fee_drops: u64,
    can_be_held_result: Ter,
    open_ledger_tx_count: usize,
    tx_id: Uint256,
    last_valid: Option<u32>,
    flags: ApplyFlags,
    pf_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    order: &OrderCandidates,
    prepare_multitxn: PrepareMultiTxn,
    run_flow: RunFlow,
) -> QueueApplyQueuedStageWithLogMessagesResult<Account, Tx, Journal, ParentBatchId>
where
    Account: Clone + std::fmt::Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
    RunFlow:
        FnOnce(
            &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
            QueueApplyPreparedFlowInputs<'_, Account, Tx, Journal, ParentBatchId>,
        )
            -> QueueApplyFlowStageWithLogMessagesResult<Account, Tx, Journal, ParentBatchId>,
{
    run_prepared_queue_apply_queued_flow_stage_with_log_messages(
        views,
        derive_queue_apply_prepared_flow_stage(
            &*views,
            account_seq_proxy,
            tx_seq_proxy,
            QueueApplyQueuedWithFeeContextInputs {
                account,
                preflight,
                is_blocker,
                open_ledger_seq,
                minimum_last_ledger_buffer,
                maximum_txn_per_account,
                retry_sequence_percent,
                queue_is_full,
                balance_drops,
                reserve_drops,
                base_fee_drops,
                can_be_held_result,
                open_ledger_tx_count,
                tx_id,
                last_valid,
                flags,
                order,
            },
            QueueApplyFeeContext {
                fee_level_paid,
                required_fee_level,
                base_level,
            },
            pf_result,
            prepare_multitxn,
        ),
        run_flow,
    )
}

pub fn run_queue_apply_queued_stage_with_fee_context_and_log_messages_and_caller_flow<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    PrepareMultiTxn,
    RunFlow,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    account_seq_proxy: SeqProxy,
    tx_seq_proxy: SeqProxy,
    inputs: QueueApplyQueuedWithFeeContextInputs<'_, Account>,
    fee_context: QueueApplyFeeContext,
    pf_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    prepare_multitxn: PrepareMultiTxn,
    run_flow: RunFlow,
) -> QueueApplyQueuedStageWithLogMessagesResult<Account, Tx, Journal, ParentBatchId>
where
    Account: Clone + std::fmt::Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    PrepareMultiTxn: FnOnce(QueueApplyViewAdjustment) -> bool,
    RunFlow:
        FnOnce(
            &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
            QueueApplyPreparedFlowInputs<'_, Account, Tx, Journal, ParentBatchId>,
        )
            -> QueueApplyFlowStageWithLogMessagesResult<Account, Tx, Journal, ParentBatchId>,
{
    run_queue_apply_queued_stage_with_log_messages_and_caller_flow(
        views,
        inputs.account,
        account_seq_proxy,
        tx_seq_proxy,
        inputs.preflight,
        inputs.is_blocker,
        inputs.open_ledger_seq,
        inputs.minimum_last_ledger_buffer,
        inputs.maximum_txn_per_account,
        inputs.retry_sequence_percent,
        inputs.queue_is_full,
        fee_context.fee_level_paid,
        fee_context.required_fee_level,
        fee_context.base_level,
        inputs.balance_drops,
        inputs.reserve_drops,
        inputs.base_fee_drops,
        inputs.can_be_held_result,
        inputs.open_ledger_tx_count,
        inputs.tx_id,
        inputs.last_valid,
        inputs.flags,
        pf_result,
        inputs.order,
        prepare_multitxn,
        run_flow,
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use basics::base_uint::Uint256;
    use protocol::{Rules, SeqProxy, Ter};

    use super::{
        QueueApplyPreparedFlowInputs, QueueApplyQueuedStage, QueueApplyQueuedWithFeeContextInputs,
        run_queue_apply_queued_stage, run_queue_apply_queued_stage_with_caller_flow,
        run_queue_apply_queued_stage_with_fee_context,
    };
    use crate::{
        AccountQueueWindow, ApplyFlags, ApplyResult, BlockerQueueAdmission,
        MAYBE_TX_RETRIES_ALLOWED, MaybeTx, MaybeTxCore, OrderCandidates, PreflightResult,
        QueueApplyAccountStage, QueueApplyFeeContext, QueueApplyFlowStage, QueueApplyHoldFallback,
        QueueApplyMultiTxnStage, QueueApplyPath, QueueApplyPreclaimViewSource,
        QueueApplyQueueStage, QueueApplyTryClearStage, QueueApplyViewAdjustment,
        QueueHoldPreflight, QueueViews, QueuedBlockerAdmission, ReplacementFeeDecision,
        TxConsequences, TxConsequencesCategory, TxQAccount,
    };

    fn make_preflight(
        tx: &'static str,
        _seq_proxy: SeqProxy,
        flags: ApplyFlags,
        consequences: TxConsequences,
    ) -> PreflightResult<&'static str, TxConsequences, &'static str, &'static str> {
        PreflightResult::new(
            tx,
            None,
            Rules::new(std::iter::empty()),
            consequences,
            flags,
            "journal",
            Ter::TES_SUCCESS,
        )
    }

    fn queued(
        account: &'static str,
        seq_proxy: SeqProxy,
        tx_id: u64,
        fee_level: u64,
        flags: ApplyFlags,
        consequences: TxConsequences,
    ) -> MaybeTx<&'static str, &'static str, &'static str, &'static str> {
        MaybeTx::new(
            Uint256::from_u64(tx_id),
            fee_level,
            account,
            Some(200),
            seq_proxy,
            flags,
            make_preflight("tx", seq_proxy, flags, consequences),
        )
    }

    fn make_queued_inputs<'a>(
        account: &'static str,
        order: &'a OrderCandidates,
    ) -> QueueApplyQueuedWithFeeContextInputs<'a, &'static str> {
        QueueApplyQueuedWithFeeContextInputs {
            account,
            preflight: QueueHoldPreflight::new(false, false, ApplyFlags::NONE, None),
            is_blocker: false,
            open_ledger_seq: 100,
            minimum_last_ledger_buffer: 2,
            maximum_txn_per_account: 10,
            retry_sequence_percent: 25,
            queue_is_full: false,
            balance_drops: 1_000,
            reserve_drops: 200,
            base_fee_drops: 10,
            can_be_held_result: Ter::TES_SUCCESS,
            open_ledger_tx_count: 4,
            tx_id: Uint256::from_u64(9),
            last_valid: Some(250),
            flags: ApplyFlags::NONE,
            order,
        }
    }

    #[test]
    fn queued_stage_rejects_blocker_non_replacement_before_flow() {
        let mut queued_account = TxQAccount::new("acct");
        queued_account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                queued(
                    "acct",
                    SeqProxy::sequence(5),
                    5,
                    90,
                    ApplyFlags::NONE,
                    TxConsequences::with_category(
                        1,
                        SeqProxy::sequence(5),
                        TxConsequencesCategory::Blocker,
                    ),
                ),
                TxConsequences::with_category(
                    1,
                    SeqProxy::sequence(5),
                    TxConsequencesCategory::Blocker,
                ),
            ),
        );

        let mut views = QueueViews::new(BTreeMap::from([("acct", queued_account)]), vec![]);
        let stage = run_queue_apply_queued_stage(
            &mut views,
            "acct",
            SeqProxy::sequence(5),
            SeqProxy::sequence(6),
            QueueHoldPreflight::new(false, false, ApplyFlags::NONE, None),
            true,
            100,
            2,
            10,
            25,
            false,
            110,
            100,
            50,
            1_000,
            200,
            10,
            Ter::TES_SUCCESS,
            4,
            Uint256::from_u64(9),
            Some(250),
            ApplyFlags::NONE,
            make_preflight(
                "tx",
                SeqProxy::sequence(6),
                ApplyFlags::NONE,
                TxConsequences::new(1, SeqProxy::sequence(6)),
            ),
            &OrderCandidates::new(Uint256::from_u64(0)),
            |_| unreachable!("blocked queue path must not prepare multitxn"),
            |_| unreachable!("blocked queue path must not preclaim"),
            || -> crate::ApplyResult { unreachable!("blocked queue path must not try clear") },
            || unreachable!("blocked queue path must not apply sandbox"),
        );

        assert_eq!(
            stage,
            QueueApplyQueuedStage::Account(QueueApplyAccountStage::RejectBlockerAdmission(
                BlockerQueueAdmission::RejectsNonReplacementOfLoneEntry
            ))
        );
        assert_eq!(
            stage.apply_result(),
            ApplyResult::new(Ter::TEL_CAN_NOT_QUEUE_BLOCKS, false, false)
        );
    }

    #[test]
    fn queued_stage_with_fee_context_reuses_landed_stage_with_derived_fee_levels() {
        let mut queued_account = TxQAccount::new("acct");
        queued_account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                queued(
                    "acct",
                    SeqProxy::sequence(5),
                    5,
                    90,
                    ApplyFlags::NONE,
                    TxConsequences::with_category(
                        1,
                        SeqProxy::sequence(5),
                        TxConsequencesCategory::Blocker,
                    ),
                ),
                TxConsequences::with_category(
                    1,
                    SeqProxy::sequence(5),
                    TxConsequencesCategory::Blocker,
                ),
            ),
        );

        let mut views = QueueViews::new(BTreeMap::from([("acct", queued_account)]), vec![]);
        let order = OrderCandidates::new(Uint256::from_u64(0));
        let mut inputs = make_queued_inputs("acct", &order);
        inputs.is_blocker = true;

        let stage = run_queue_apply_queued_stage_with_fee_context(
            &mut views,
            SeqProxy::sequence(5),
            SeqProxy::sequence(6),
            inputs,
            QueueApplyFeeContext {
                base_level: 50,
                fee_level_paid: 110,
                required_fee_level: 100,
            },
            make_preflight(
                "tx",
                SeqProxy::sequence(6),
                ApplyFlags::NONE,
                TxConsequences::with_category(
                    1,
                    SeqProxy::sequence(6),
                    TxConsequencesCategory::Blocker,
                ),
            ),
            |_| true,
            |_| unreachable!("account-stage rejection should happen before preclaim"),
            || -> crate::ApplyResult {
                unreachable!("account-stage rejection should happen before try-clear")
            },
            || unreachable!("account-stage rejection should happen before sandbox apply"),
        );

        assert_eq!(
            stage,
            QueueApplyQueuedStage::Account(QueueApplyAccountStage::RejectBlockerAdmission(
                BlockerQueueAdmission::RejectsNonReplacementOfLoneEntry,
            ))
        );
    }

    #[test]
    fn queued_stage_rejects_existing_queued_blocker_before_flow() {
        let mut queued_account = TxQAccount::new("acct");
        queued_account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                queued(
                    "acct",
                    SeqProxy::sequence(5),
                    5,
                    90,
                    ApplyFlags::NONE,
                    TxConsequences::with_category(
                        1,
                        SeqProxy::sequence(5),
                        TxConsequencesCategory::Blocker,
                    ),
                ),
                TxConsequences::with_category(
                    1,
                    SeqProxy::sequence(5),
                    TxConsequencesCategory::Blocker,
                ),
            ),
        );

        let mut views = QueueViews::new(BTreeMap::from([("acct", queued_account)]), vec![]);
        let stage = run_queue_apply_queued_stage(
            &mut views,
            "acct",
            SeqProxy::sequence(5),
            SeqProxy::sequence(6),
            QueueHoldPreflight::new(false, false, ApplyFlags::NONE, None),
            false,
            100,
            2,
            10,
            25,
            false,
            110,
            100,
            50,
            1_000,
            200,
            10,
            Ter::TES_SUCCESS,
            4,
            Uint256::from_u64(9),
            Some(250),
            ApplyFlags::NONE,
            make_preflight(
                "tx",
                SeqProxy::sequence(6),
                ApplyFlags::NONE,
                TxConsequences::new(1, SeqProxy::sequence(6)),
            ),
            &OrderCandidates::new(Uint256::from_u64(0)),
            |_| unreachable!("queued blocker reject must not prepare multitxn"),
            |_| unreachable!("queued blocker reject must not preclaim"),
            || -> crate::ApplyResult { unreachable!("queued blocker reject must not try clear") },
            || unreachable!("queued blocker reject must not apply sandbox"),
        );

        assert_eq!(
            stage,
            QueueApplyQueuedStage::Account(QueueApplyAccountStage::RejectQueuedBlocker(
                QueuedBlockerAdmission::BlockedByQueuedBlocker
            ))
        );
        assert_eq!(
            stage.apply_result(),
            ApplyResult::new(Ter::TEL_CAN_NOT_QUEUE_BLOCKED, false, false)
        );
    }

    #[test]
    fn queued_stage_rejects_insufficient_replacement_fee_before_flow() {
        let mut queued_account = TxQAccount::new("acct");
        queued_account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                queued(
                    "acct",
                    SeqProxy::sequence(5),
                    5,
                    100,
                    ApplyFlags::NONE,
                    TxConsequences::new(1, SeqProxy::sequence(5)),
                ),
                TxConsequences::new(1, SeqProxy::sequence(5)),
            ),
        );

        let mut views = QueueViews::new(BTreeMap::from([("acct", queued_account)]), vec![]);
        let stage = run_queue_apply_queued_stage(
            &mut views,
            "acct",
            SeqProxy::sequence(5),
            SeqProxy::sequence(5),
            QueueHoldPreflight::new(false, false, ApplyFlags::NONE, None),
            false,
            100,
            2,
            10,
            25,
            false,
            125,
            100,
            50,
            1_000,
            200,
            10,
            Ter::TES_SUCCESS,
            4,
            Uint256::from_u64(9),
            Some(250),
            ApplyFlags::NONE,
            make_preflight(
                "tx",
                SeqProxy::sequence(5),
                ApplyFlags::NONE,
                TxConsequences::new(1, SeqProxy::sequence(5)),
            ),
            &OrderCandidates::new(Uint256::from_u64(0)),
            |_| unreachable!("replacement fee reject must not prepare multitxn"),
            |_| unreachable!("replacement fee reject must not preclaim"),
            || -> crate::ApplyResult { unreachable!("replacement fee reject must not try clear") },
            || unreachable!("replacement fee reject must not apply sandbox"),
        );

        assert_eq!(
            stage,
            QueueApplyQueuedStage::Account(QueueApplyAccountStage::RejectReplacementFee(
                ReplacementFeeDecision::InsufficientFee {
                    required_fee_level: 125
                }
            ))
        );
        assert_eq!(
            stage.apply_result(),
            ApplyResult::new(Ter::TEL_CAN_NOT_QUEUE_FEE, false, false)
        );
    }

    #[test]
    fn queued_stage_rejects_balance_before_queue_flow() {
        let mut queued_account = TxQAccount::new("acct");
        queued_account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                queued(
                    "acct",
                    SeqProxy::sequence(5),
                    5,
                    70,
                    ApplyFlags::NONE,
                    TxConsequences::with_potential_spend(70, SeqProxy::sequence(5), 100),
                ),
                TxConsequences::with_potential_spend(70, SeqProxy::sequence(5), 100),
            ),
        );
        queued_account.add(
            SeqProxy::sequence(7),
            MaybeTxCore::new(
                queued(
                    "acct",
                    SeqProxy::sequence(7),
                    7,
                    50,
                    ApplyFlags::NONE,
                    TxConsequences::with_potential_spend(50, SeqProxy::sequence(7), 100),
                ),
                TxConsequences::with_potential_spend(50, SeqProxy::sequence(7), 100),
            ),
        );

        let mut views = QueueViews::new(BTreeMap::from([("acct", queued_account)]), vec![]);
        let stage = run_queue_apply_queued_stage(
            &mut views,
            "acct",
            SeqProxy::sequence(5),
            SeqProxy::sequence(6),
            QueueHoldPreflight::new(false, false, ApplyFlags::NONE, None),
            false,
            100,
            2,
            10,
            25,
            false,
            110,
            100,
            50,
            120,
            1_000,
            10,
            Ter::TES_SUCCESS,
            4,
            Uint256::from_u64(9),
            Some(250),
            ApplyFlags::NONE,
            make_preflight(
                "tx",
                SeqProxy::sequence(6),
                ApplyFlags::NONE,
                TxConsequences::with_potential_spend(1, SeqProxy::sequence(6), 1),
            ),
            &OrderCandidates::new(Uint256::from_u64(0)),
            |_| true,
            |_| unreachable!("balance reject must not preclaim"),
            || -> crate::ApplyResult { unreachable!("balance reject must not try clear") },
            || unreachable!("balance reject must not apply sandbox"),
        );

        assert!(matches!(
            stage,
            QueueApplyQueuedStage::MultiTxn(QueueApplyMultiTxnStage::RejectBalance(_))
        ));
        assert_eq!(
            stage.apply_result(),
            ApplyResult::new(Ter::TEL_CAN_NOT_QUEUE_BALANCE, false, false)
        );
    }

    #[test]
    fn queued_stage_rejects_internal_when_multitxn_view_preparation_fails() {
        let mut queued_account = TxQAccount::new("acct");
        queued_account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                queued(
                    "acct",
                    SeqProxy::sequence(5),
                    5,
                    70,
                    ApplyFlags::NONE,
                    TxConsequences::with_potential_spend(70, SeqProxy::sequence(5), 100),
                ),
                TxConsequences::with_potential_spend(70, SeqProxy::sequence(5), 100),
            ),
        );
        queued_account.add(
            SeqProxy::sequence(7),
            MaybeTxCore::new(
                queued(
                    "acct",
                    SeqProxy::sequence(7),
                    7,
                    50,
                    ApplyFlags::NONE,
                    TxConsequences::with_potential_spend(50, SeqProxy::sequence(7), 100),
                ),
                TxConsequences::with_potential_spend(50, SeqProxy::sequence(7), 100),
            ),
        );

        let mut views = QueueViews::new(BTreeMap::from([("acct", queued_account)]), vec![]);
        let stage = run_queue_apply_queued_stage(
            &mut views,
            "acct",
            SeqProxy::sequence(5),
            SeqProxy::sequence(6),
            QueueHoldPreflight::new(false, false, ApplyFlags::NONE, None),
            false,
            100,
            2,
            10,
            25,
            false,
            110,
            100,
            50,
            1_000,
            200,
            10,
            Ter::TES_SUCCESS,
            4,
            Uint256::from_u64(9),
            Some(250),
            ApplyFlags::NONE,
            make_preflight(
                "tx",
                SeqProxy::sequence(6),
                ApplyFlags::NONE,
                TxConsequences::with_potential_spend(50, SeqProxy::sequence(6), 500),
            ),
            &OrderCandidates::new(Uint256::from_u64(0)),
            |_| false,
            |_| unreachable!("internal reject must not preclaim"),
            || -> crate::ApplyResult { unreachable!("internal reject must not try clear") },
            || unreachable!("internal reject must not apply sandbox"),
        );

        assert!(matches!(stage, QueueApplyQueuedStage::PreclaimView(_)));
        assert_eq!(
            stage.apply_result(),
            ApplyResult::new(Ter::TEF_INTERNAL, false, false)
        );
    }

    #[test]
    fn queued_stage_carries_view_adjustment_and_reaches_flow() {
        let mut queued_account = TxQAccount::new("acct");
        queued_account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                queued(
                    "acct",
                    SeqProxy::sequence(5),
                    5,
                    20,
                    ApplyFlags::NONE,
                    TxConsequences::with_potential_spend(20, SeqProxy::sequence(5), 100),
                ),
                TxConsequences::with_potential_spend(20, SeqProxy::sequence(5), 100),
            ),
        );
        queued_account.add(
            SeqProxy::sequence(7),
            MaybeTxCore::new(
                queued(
                    "acct",
                    SeqProxy::sequence(7),
                    7,
                    15,
                    ApplyFlags::NONE,
                    TxConsequences::with_potential_spend(15, SeqProxy::sequence(7), 50),
                ),
                TxConsequences::with_potential_spend(15, SeqProxy::sequence(7), 50),
            ),
        );

        let mut views = QueueViews::new(BTreeMap::from([("acct", queued_account)]), vec![]);
        let stage = run_queue_apply_queued_stage(
            &mut views,
            "acct",
            SeqProxy::sequence(5),
            SeqProxy::sequence(6),
            QueueHoldPreflight::new(false, false, ApplyFlags::NONE, None),
            false,
            100,
            2,
            10,
            25,
            false,
            110,
            100,
            50,
            1_000,
            200,
            10,
            Ter::TES_SUCCESS,
            4,
            Uint256::from_u64(9),
            Some(250),
            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
            make_preflight(
                "tx",
                SeqProxy::sequence(6),
                ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
                TxConsequences::with_potential_spend(50, SeqProxy::sequence(6), 500),
            ),
            &OrderCandidates::new(Uint256::from_u64(0)),
            |_| true,
            |source| {
                assert_eq!(source, QueueApplyPreclaimViewSource::MultiTxnOpenView);
                make_preflight(
                    "tx",
                    SeqProxy::sequence(6),
                    ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
                    TxConsequences::with_potential_spend(50, SeqProxy::sequence(6), 500),
                )
                .to_preclaim(9, Ter::TES_SUCCESS)
            },
            || ApplyResult::new(Ter::TER_RETRY, false, false),
            || unreachable!("failed try-clear must not apply sandbox"),
        );

        match stage {
            QueueApplyQueuedStage::Flow {
                window,
                replacement_decision,
                path,
                view_adjustment,
                flow,
            } => {
                assert_eq!(
                    window,
                    AccountQueueWindow {
                        account_is_in_queue: true,
                        first_relevant_seq_proxy: Some(SeqProxy::sequence(5)),
                        relevant_tx_count: 2,
                        replaces_existing: false,
                        front_is_blocker: false,
                    }
                );
                assert_eq!(replacement_decision, None);
                assert_eq!(
                    path,
                    QueueApplyPath::QueuedAccount {
                        requires_multi_txn: true
                    }
                );
                assert_eq!(
                    view_adjustment,
                    Some(QueueApplyViewAdjustment {
                        potential_total_spend_drops: 185,
                        adjusted_balance_drops: 815,
                        applied_sequence_value: 6,
                    })
                );
                assert!(matches!(
                    flow,
                    QueueApplyFlowStage::QueueOutcome {
                        try_clear: QueueApplyTryClearStage::ContinueAfterAttempt,
                        queue: QueueApplyQueueStage::Queued(_),
                        ..
                    }
                ));
            }
            other => panic!("expected flow stage, got {other:?}"),
        }
    }

    #[test]
    fn queued_stage_with_caller_flow_exposes_prepared_flow_boundary() {
        let mut queued_account = TxQAccount::new("acct");
        queued_account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                queued(
                    "acct",
                    SeqProxy::sequence(5),
                    5,
                    20,
                    ApplyFlags::NONE,
                    TxConsequences::with_potential_spend(20, SeqProxy::sequence(5), 100),
                ),
                TxConsequences::with_potential_spend(20, SeqProxy::sequence(5), 100),
            ),
        );
        queued_account.add(
            SeqProxy::sequence(7),
            MaybeTxCore::new(
                queued(
                    "acct",
                    SeqProxy::sequence(7),
                    7,
                    15,
                    ApplyFlags::NONE,
                    TxConsequences::with_potential_spend(15, SeqProxy::sequence(7), 50),
                ),
                TxConsequences::with_potential_spend(15, SeqProxy::sequence(7), 50),
            ),
        );

        let mut views = QueueViews::new(BTreeMap::from([("acct", queued_account)]), vec![]);
        let order = OrderCandidates::new(Uint256::from_u64(0));
        let preflight = make_preflight(
            "tx",
            SeqProxy::sequence(6),
            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
            TxConsequences::with_potential_spend(50, SeqProxy::sequence(6), 500),
        );

        let stage = run_queue_apply_queued_stage_with_caller_flow(
            &mut views,
            "acct",
            SeqProxy::sequence(5),
            SeqProxy::sequence(6),
            QueueHoldPreflight::new(false, false, ApplyFlags::NONE, None),
            false,
            100,
            2,
            10,
            25,
            false,
            110,
            100,
            50,
            1_000,
            200,
            10,
            Ter::TES_SUCCESS,
            4,
            Uint256::from_u64(9),
            Some(250),
            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
            preflight.clone(),
            &order,
            |_| true,
            |_views, prepared| {
                let QueueApplyPreparedFlowInputs {
                    preclaim_view_source,
                    tx_seq_proxy,
                    first_relevant_retries_remaining,
                    fee_level_paid,
                    base_level,
                    required_fee_level,
                    open_ledger_tx_count,
                    hold_fallback,
                    full_queue_decision,
                    replaced,
                    account,
                    tx_id,
                    last_valid,
                    flags,
                    pf_result,
                    order: prepared_order,
                } = prepared;

                assert_eq!(
                    preclaim_view_source,
                    QueueApplyPreclaimViewSource::MultiTxnOpenView
                );
                assert_eq!(tx_seq_proxy, SeqProxy::sequence(6));
                assert_eq!(
                    first_relevant_retries_remaining,
                    Some(MAYBE_TX_RETRIES_ALLOWED)
                );
                assert_eq!(fee_level_paid, 110);
                assert_eq!(base_level, 50);
                assert_eq!(required_fee_level, 100);
                assert_eq!(open_ledger_tx_count, 4);
                assert_eq!(
                    hold_fallback,
                    QueueApplyHoldFallback::AlreadyVerifiedByMultiTxn
                );
                assert_eq!(
                    full_queue_decision,
                    crate::QueueApplyFullQueueDecision::Bypass
                );
                assert_eq!(replaced, None);
                assert_eq!(account, "acct");
                assert_eq!(tx_id, Uint256::from_u64(9));
                assert_eq!(last_valid, Some(250));
                assert_eq!(flags, ApplyFlags::FAIL_HARD | ApplyFlags::RETRY);
                assert_eq!(pf_result, preflight);
                assert!(std::ptr::eq(prepared_order, &order));

                QueueApplyFlowStage::RejectPreclaim(ApplyResult::new(Ter::TER_RETRY, false, false))
            },
        );

        match stage {
            QueueApplyQueuedStage::Flow {
                window,
                replacement_decision,
                path,
                view_adjustment,
                flow,
            } => {
                assert_eq!(
                    window,
                    AccountQueueWindow {
                        account_is_in_queue: true,
                        first_relevant_seq_proxy: Some(SeqProxy::sequence(5)),
                        relevant_tx_count: 2,
                        replaces_existing: false,
                        front_is_blocker: false,
                    }
                );
                assert_eq!(replacement_decision, None);
                assert_eq!(
                    path,
                    QueueApplyPath::QueuedAccount {
                        requires_multi_txn: true
                    }
                );
                assert_eq!(
                    view_adjustment,
                    Some(QueueApplyViewAdjustment {
                        potential_total_spend_drops: 185,
                        adjusted_balance_drops: 815,
                        applied_sequence_value: 6,
                    })
                );
                assert_eq!(
                    flow,
                    QueueApplyFlowStage::RejectPreclaim(ApplyResult::new(
                        Ter::TER_RETRY,
                        false,
                        false,
                    ))
                );
            }
            other => panic!("expected flow stage, got {other:?}"),
        }
    }
}
