//! Final normal queue-hold path inside `TxQ::apply(...)`.
//!
//! This composes the already-landed no-`multiTxn` hold fallback, full-queue
//! decision/mutation, and final enqueue mutation in the exact the reference implementation
//! order.

use std::cell::RefCell;
use std::fmt::Display;

use basics::base_uint::Uint256;
use protocol::{SeqProxy, Ter, trans_token};

use crate::{
    ApplyFlags, ApplyResult, FeeLevel64, FeeQueueKey, MaybeTx, OrderCandidates, PreflightResult,
    QueueApplyEnqueueResult, QueueApplyFullQueueDecision, QueueApplyHoldFallback, QueueViews,
    TxConsequences, apply_queue_apply_full_queue_decision, enqueue_queue_apply_candidate,
};

#[derive(Debug, Clone)]
pub struct QueueApplyPreparedHoldInputs<'a, Account, Tx, Journal, ParentBatchId> {
    pub hold_fallback: QueueApplyHoldFallback,
    pub full_queue_decision: QueueApplyFullQueueDecision<Account>,
    pub replaced: Option<FeeQueueKey<Account>>,
    pub account: Account,
    pub tx_id: Uint256,
    pub last_valid: Option<u32>,
    pub seq_proxy: SeqProxy,
    pub fee_level: FeeLevel64,
    pub flags: ApplyFlags,
    pub pf_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    pub order: &'a OrderCandidates,
}

impl<'a, Account, Tx, Journal, ParentBatchId>
    QueueApplyPreparedHoldInputs<'a, Account, Tx, Journal, ParentBatchId>
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        hold_fallback: QueueApplyHoldFallback,
        full_queue_decision: QueueApplyFullQueueDecision<Account>,
        replaced: Option<FeeQueueKey<Account>>,
        account: Account,
        tx_id: Uint256,
        last_valid: Option<u32>,
        seq_proxy: SeqProxy,
        fee_level: FeeLevel64,
        flags: ApplyFlags,
        pf_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        order: &'a OrderCandidates,
    ) -> Self {
        Self {
            hold_fallback,
            full_queue_decision,
            replaced,
            account,
            tx_id,
            last_valid,
            seq_proxy,
            fee_level,
            flags,
            pf_result,
            order,
        }
    }
}

#[derive(Debug, Clone)]
pub struct QueueApplyPreparedFullQueueInputs<'a, Account, Tx, Journal, ParentBatchId> {
    pub full_queue_decision: QueueApplyFullQueueDecision<Account>,
    pub replaced: Option<FeeQueueKey<Account>>,
    pub account: Account,
    pub tx_id: Uint256,
    pub last_valid: Option<u32>,
    pub seq_proxy: SeqProxy,
    pub fee_level: FeeLevel64,
    pub flags: ApplyFlags,
    pub pf_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    pub order: &'a OrderCandidates,
}

impl<'a, Account, Tx, Journal, ParentBatchId>
    QueueApplyPreparedFullQueueInputs<'a, Account, Tx, Journal, ParentBatchId>
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        full_queue_decision: QueueApplyFullQueueDecision<Account>,
        replaced: Option<FeeQueueKey<Account>>,
        account: Account,
        tx_id: Uint256,
        last_valid: Option<u32>,
        seq_proxy: SeqProxy,
        fee_level: FeeLevel64,
        flags: ApplyFlags,
        pf_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        order: &'a OrderCandidates,
    ) -> Self {
        Self {
            full_queue_decision,
            replaced,
            account,
            tx_id,
            last_valid,
            seq_proxy,
            fee_level,
            flags,
            pf_result,
            order,
        }
    }
}

#[derive(Debug, Clone)]
pub struct QueueApplyPreparedEnqueueInputs<'a, Account, Tx, Journal, ParentBatchId> {
    pub full_queue_decision: QueueApplyFullQueueDecision<Account>,
    pub replaced: Option<FeeQueueKey<Account>>,
    pub account: Account,
    pub tx_id: Uint256,
    pub last_valid: Option<u32>,
    pub seq_proxy: SeqProxy,
    pub fee_level: FeeLevel64,
    pub flags: ApplyFlags,
    pub pf_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    pub order: &'a OrderCandidates,
}

impl<'a, Account, Tx, Journal, ParentBatchId>
    QueueApplyPreparedEnqueueInputs<'a, Account, Tx, Journal, ParentBatchId>
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        full_queue_decision: QueueApplyFullQueueDecision<Account>,
        replaced: Option<FeeQueueKey<Account>>,
        account: Account,
        tx_id: Uint256,
        last_valid: Option<u32>,
        seq_proxy: SeqProxy,
        fee_level: FeeLevel64,
        flags: ApplyFlags,
        pf_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
        order: &'a OrderCandidates,
    ) -> Self {
        Self {
            full_queue_decision,
            replaced,
            account,
            tx_id,
            last_valid,
            seq_proxy,
            fee_level,
            flags,
            pf_result,
            order,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QueueApplyQueueLogMessages {
    pub trace: Vec<String>,
    pub debug: Vec<String>,
    pub info: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueApplyQueueStageWithLogMessagesResult<Account> {
    pub stage: QueueApplyQueueStage<Account>,
    pub log_messages: QueueApplyQueueLogMessages,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueApplyQueueStage<Account> {
    RejectCannotHold(Ter),
    RejectFull,
    Queued(QueueApplyEnqueueResult<Account>),
}

impl<Account> QueueApplyQueueStage<Account> {
    pub const fn ter(&self) -> Ter {
        match self {
            Self::RejectCannotHold(ter) => *ter,
            Self::RejectFull => Ter::TEL_CAN_NOT_QUEUE_FULL,
            Self::Queued(result) => result.ter(),
        }
    }

    pub const fn apply_result(&self) -> ApplyResult {
        ApplyResult::new(self.ter(), false, false)
    }
}

pub fn format_queue_apply_full_queue_same_account_info_message<Account, TxId>(
    transaction_id: TxId,
    account: Account,
) -> String
where
    Account: Display,
    TxId: Display,
{
    format!(
        "Queue is full, and transaction {} would kick a transaction from the same account ({}) out of the queue.",
        transaction_id, account
    )
}

pub fn format_queue_apply_full_queue_evict_info_message<Account, TxId>(
    evicted_account: Account,
    end_effective_fee_level: FeeLevel64,
    transaction_id: TxId,
    fee_level_paid: FeeLevel64,
) -> String
where
    Account: Display,
    TxId: Display,
{
    format!(
        "Removing last item of account {} from queue with average fee of {} in favor of {} with fee of {}",
        evicted_account, end_effective_fee_level, transaction_id, fee_level_paid
    )
}

pub fn format_queue_apply_full_queue_lower_fee_info_message<TxId>(transaction_id: TxId) -> String
where
    TxId: Display,
{
    format!(
        "Queue is full, and transaction {} fee is lower than end item's account average fee",
        transaction_id
    )
}

pub fn format_queue_apply_enqueue_debug_message<Account, TxId>(
    transaction_id: TxId,
    txn_result: Ter,
    existing_account: bool,
    account: Account,
    flags: ApplyFlags,
) -> String
where
    Account: Display,
    TxId: Display,
{
    let account_state = if existing_account { "existing" } else { "new" };
    format!(
        "Added transaction {} with result {} from {} account {} to queue. Flags: {}",
        transaction_id,
        trans_token(txn_result),
        account_state,
        account,
        flags
    )
}

pub fn run_queue_apply_queue_stage<Account, Tx, Journal, ParentBatchId>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    hold_fallback: QueueApplyHoldFallback,
    full_queue_decision: QueueApplyFullQueueDecision<Account>,
    replaced: Option<FeeQueueKey<Account>>,
    account: Account,
    tx_id: Uint256,
    last_valid: Option<u32>,
    seq_proxy: SeqProxy,
    fee_level: FeeLevel64,
    flags: ApplyFlags,
    pf_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    order: &OrderCandidates,
) -> QueueApplyQueueStage<Account>
where
    Account: Clone + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
{
    run_queue_apply_queue_stage_with_caller_hold(
        views,
        hold_fallback,
        full_queue_decision,
        replaced,
        account,
        tx_id,
        last_valid,
        seq_proxy,
        fee_level,
        flags,
        pf_result,
        order,
        |_views, prepared| prepared.hold_fallback,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_queue_stage_with_log_sinks<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    DebugSink,
    InfoSink,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    hold_fallback: QueueApplyHoldFallback,
    full_queue_decision: QueueApplyFullQueueDecision<Account>,
    replaced: Option<FeeQueueKey<Account>>,
    account: Account,
    tx_id: Uint256,
    last_valid: Option<u32>,
    seq_proxy: SeqProxy,
    fee_level: FeeLevel64,
    flags: ApplyFlags,
    pf_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    order: &OrderCandidates,
    debug: DebugSink,
    info: InfoSink,
) -> QueueApplyQueueStage<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    DebugSink: FnMut(String),
    InfoSink: FnMut(String),
{
    run_queue_apply_queue_stage_with_log_sinks_and_caller_hold(
        views,
        hold_fallback,
        full_queue_decision,
        replaced,
        account,
        tx_id,
        last_valid,
        seq_proxy,
        fee_level,
        flags,
        pf_result,
        order,
        |_views, prepared| prepared.hold_fallback,
        debug,
        info,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_queue_stage_with_log_sinks_and_caller_hold<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    RunHoldStage,
    DebugSink,
    InfoSink,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    hold_fallback: QueueApplyHoldFallback,
    full_queue_decision: QueueApplyFullQueueDecision<Account>,
    replaced: Option<FeeQueueKey<Account>>,
    account: Account,
    tx_id: Uint256,
    last_valid: Option<u32>,
    seq_proxy: SeqProxy,
    fee_level: FeeLevel64,
    flags: ApplyFlags,
    pf_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    order: &OrderCandidates,
    run_hold_stage: RunHoldStage,
    mut debug: DebugSink,
    mut info: InfoSink,
) -> QueueApplyQueueStage<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    RunHoldStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedHoldInputs<'_, Account, Tx, Journal, ParentBatchId>,
    ) -> QueueApplyHoldFallback,
    DebugSink: FnMut(String),
    InfoSink: FnMut(String),
{
    run_queue_apply_queue_stage_with_callers(
        views,
        hold_fallback,
        full_queue_decision,
        replaced,
        account,
        tx_id,
        last_valid,
        seq_proxy,
        fee_level,
        flags,
        pf_result,
        order,
        run_hold_stage,
        |views, prepared| {
            let decision =
                apply_queue_apply_full_queue_decision(views, prepared.full_queue_decision);

            match &decision {
                QueueApplyFullQueueDecision::Bypass => {}
                QueueApplyFullQueueDecision::RejectFullSameAccount => {
                    info(format_queue_apply_full_queue_same_account_info_message(
                        prepared.tx_id,
                        prepared.account.clone(),
                    ));
                }
                QueueApplyFullQueueDecision::RejectFullLowerFee { .. } => {
                    info(format_queue_apply_full_queue_lower_fee_info_message(
                        prepared.tx_id,
                    ));
                }
                QueueApplyFullQueueDecision::EvictCheapest {
                    dropped,
                    end_effective_fee_level,
                } => {
                    info(format_queue_apply_full_queue_evict_info_message(
                        dropped.account.clone(),
                        *end_effective_fee_level,
                        prepared.tx_id,
                        prepared.fee_level,
                    ));
                }
            }

            decision
        },
        |views, prepared| {
            let account = prepared.account.clone();
            let tx_id = prepared.tx_id;
            let txn_result = prepared.pf_result.ter;
            let enqueue = enqueue_queue_apply_candidate(
                views,
                prepared.replaced,
                prepared.account,
                prepared.tx_id,
                prepared.last_valid,
                prepared.seq_proxy,
                prepared.fee_level,
                prepared.flags,
                prepared.pf_result,
                prepared.order,
            );
            debug(format_queue_apply_enqueue_debug_message(
                tx_id,
                txn_result,
                !enqueue.account_created,
                account,
                enqueue.stored_flags,
            ));
            enqueue
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_queue_stage_with_log_messages<Account, Tx, Journal, ParentBatchId>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    hold_fallback: QueueApplyHoldFallback,
    full_queue_decision: QueueApplyFullQueueDecision<Account>,
    replaced: Option<FeeQueueKey<Account>>,
    account: Account,
    tx_id: Uint256,
    last_valid: Option<u32>,
    seq_proxy: SeqProxy,
    fee_level: FeeLevel64,
    flags: ApplyFlags,
    pf_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    order: &OrderCandidates,
) -> QueueApplyQueueStageWithLogMessagesResult<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
{
    let log_messages = RefCell::new(QueueApplyQueueLogMessages::default());
    let stage = run_queue_apply_queue_stage_with_log_sinks(
        views,
        hold_fallback,
        full_queue_decision,
        replaced,
        account,
        tx_id,
        last_valid,
        seq_proxy,
        fee_level,
        flags,
        pf_result,
        order,
        |message| {
            log_messages.borrow_mut().debug.push(message);
        },
        |message| log_messages.borrow_mut().info.push(message),
    );

    QueueApplyQueueStageWithLogMessagesResult {
        stage,
        log_messages: log_messages.into_inner(),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_queue_stage_with_log_messages_and_caller_enqueue<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    RunFullQueueStage,
    RunEnqueueStage,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    hold_fallback: QueueApplyHoldFallback,
    full_queue_decision: QueueApplyFullQueueDecision<Account>,
    replaced: Option<FeeQueueKey<Account>>,
    account: Account,
    tx_id: Uint256,
    last_valid: Option<u32>,
    seq_proxy: SeqProxy,
    fee_level: FeeLevel64,
    flags: ApplyFlags,
    pf_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    order: &OrderCandidates,
    run_full_queue_stage: RunFullQueueStage,
    run_enqueue_stage: RunEnqueueStage,
) -> QueueApplyQueueStageWithLogMessagesResult<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    RunFullQueueStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedFullQueueInputs<'_, Account, Tx, Journal, ParentBatchId>,
    ) -> QueueApplyFullQueueDecision<Account>,
    RunEnqueueStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedEnqueueInputs<'_, Account, Tx, Journal, ParentBatchId>,
    ) -> QueueApplyEnqueueResult<Account>,
{
    run_queue_apply_queue_stage_with_log_messages_and_caller_enqueue_and_caller_full_queue_and_caller_hold(
        views,
        hold_fallback,
        full_queue_decision,
        replaced,
        account,
        tx_id,
        last_valid,
        seq_proxy,
        fee_level,
        flags,
        pf_result,
        order,
        |_views, prepared| prepared.hold_fallback,
        run_full_queue_stage,
        run_enqueue_stage,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_queue_stage_with_log_messages_and_caller_enqueue_and_caller_full_queue_and_caller_hold<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    RunHoldStage,
    RunFullQueueStage,
    RunEnqueueStage,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    hold_fallback: QueueApplyHoldFallback,
    full_queue_decision: QueueApplyFullQueueDecision<Account>,
    replaced: Option<FeeQueueKey<Account>>,
    account: Account,
    tx_id: Uint256,
    last_valid: Option<u32>,
    seq_proxy: SeqProxy,
    fee_level: FeeLevel64,
    flags: ApplyFlags,
    pf_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    order: &OrderCandidates,
    run_hold_stage: RunHoldStage,
    run_full_queue_stage: RunFullQueueStage,
    run_enqueue_stage: RunEnqueueStage,
) -> QueueApplyQueueStageWithLogMessagesResult<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    RunHoldStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedHoldInputs<'_, Account, Tx, Journal, ParentBatchId>,
    ) -> QueueApplyHoldFallback,
    RunFullQueueStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedFullQueueInputs<'_, Account, Tx, Journal, ParentBatchId>,
    ) -> QueueApplyFullQueueDecision<Account>,
    RunEnqueueStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedEnqueueInputs<'_, Account, Tx, Journal, ParentBatchId>,
    ) -> QueueApplyEnqueueResult<Account>,
{
    let log_messages = RefCell::new(QueueApplyQueueLogMessages::default());
    let stage =
        run_queue_apply_queue_stage_with_log_sinks_and_caller_enqueue_and_caller_full_queue_and_caller_hold(
            views,
            hold_fallback,
            full_queue_decision,
            replaced,
            account,
            tx_id,
            last_valid,
            seq_proxy,
            fee_level,
            flags,
            pf_result,
            order,
            run_hold_stage,
            run_full_queue_stage,
            run_enqueue_stage,
            |message| {
                log_messages.borrow_mut().debug.push(message);
            },
            |message| log_messages.borrow_mut().info.push(message),
        );

    QueueApplyQueueStageWithLogMessagesResult {
        stage,
        log_messages: log_messages.into_inner(),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_queue_stage_with_caller_hold<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    RunHoldStage,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    hold_fallback: QueueApplyHoldFallback,
    full_queue_decision: QueueApplyFullQueueDecision<Account>,
    replaced: Option<FeeQueueKey<Account>>,
    account: Account,
    tx_id: Uint256,
    last_valid: Option<u32>,
    seq_proxy: SeqProxy,
    fee_level: FeeLevel64,
    flags: ApplyFlags,
    pf_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    order: &OrderCandidates,
    run_hold_stage: RunHoldStage,
) -> QueueApplyQueueStage<Account>
where
    Account: Clone + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    RunHoldStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedHoldInputs<'_, Account, Tx, Journal, ParentBatchId>,
    ) -> QueueApplyHoldFallback,
{
    run_queue_apply_queue_stage_with_callers(
        views,
        hold_fallback,
        full_queue_decision,
        replaced,
        account,
        tx_id,
        last_valid,
        seq_proxy,
        fee_level,
        flags,
        pf_result,
        order,
        run_hold_stage,
        |views, prepared| {
            apply_queue_apply_full_queue_decision(views, prepared.full_queue_decision)
        },
        |views, prepared| {
            enqueue_queue_apply_candidate(
                views,
                prepared.replaced,
                prepared.account,
                prepared.tx_id,
                prepared.last_valid,
                prepared.seq_proxy,
                prepared.fee_level,
                prepared.flags,
                prepared.pf_result,
                prepared.order,
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_queue_stage_with_caller_full_queue<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    RunFullQueueStage,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    hold_fallback: QueueApplyHoldFallback,
    full_queue_decision: QueueApplyFullQueueDecision<Account>,
    replaced: Option<FeeQueueKey<Account>>,
    account: Account,
    tx_id: Uint256,
    last_valid: Option<u32>,
    seq_proxy: SeqProxy,
    fee_level: FeeLevel64,
    flags: ApplyFlags,
    pf_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    order: &OrderCandidates,
    run_full_queue_stage: RunFullQueueStage,
) -> QueueApplyQueueStage<Account>
where
    Account: Clone + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    RunFullQueueStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedFullQueueInputs<'_, Account, Tx, Journal, ParentBatchId>,
    ) -> QueueApplyFullQueueDecision<Account>,
{
    run_queue_apply_queue_stage_with_caller_full_queue_and_caller_hold(
        views,
        hold_fallback,
        full_queue_decision,
        replaced,
        account,
        tx_id,
        last_valid,
        seq_proxy,
        fee_level,
        flags,
        pf_result,
        order,
        |_views, prepared| prepared.hold_fallback,
        run_full_queue_stage,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_queue_stage_with_log_sinks_and_caller_full_queue<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    RunFullQueueStage,
    DebugSink,
    InfoSink,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    hold_fallback: QueueApplyHoldFallback,
    full_queue_decision: QueueApplyFullQueueDecision<Account>,
    replaced: Option<FeeQueueKey<Account>>,
    account: Account,
    tx_id: Uint256,
    last_valid: Option<u32>,
    seq_proxy: SeqProxy,
    fee_level: FeeLevel64,
    flags: ApplyFlags,
    pf_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    order: &OrderCandidates,
    run_full_queue_stage: RunFullQueueStage,
    debug: DebugSink,
    info: InfoSink,
) -> QueueApplyQueueStage<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    RunFullQueueStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedFullQueueInputs<'_, Account, Tx, Journal, ParentBatchId>,
    ) -> QueueApplyFullQueueDecision<Account>,
    DebugSink: FnMut(String),
    InfoSink: FnMut(String),
{
    run_queue_apply_queue_stage_with_log_sinks_and_caller_full_queue_and_caller_hold(
        views,
        hold_fallback,
        full_queue_decision,
        replaced,
        account,
        tx_id,
        last_valid,
        seq_proxy,
        fee_level,
        flags,
        pf_result,
        order,
        |_views, prepared| prepared.hold_fallback,
        run_full_queue_stage,
        debug,
        info,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_queue_stage_with_caller_full_queue_and_caller_hold<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    RunHoldStage,
    RunFullQueueStage,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    hold_fallback: QueueApplyHoldFallback,
    full_queue_decision: QueueApplyFullQueueDecision<Account>,
    replaced: Option<FeeQueueKey<Account>>,
    account: Account,
    tx_id: Uint256,
    last_valid: Option<u32>,
    seq_proxy: SeqProxy,
    fee_level: FeeLevel64,
    flags: ApplyFlags,
    pf_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    order: &OrderCandidates,
    run_hold_stage: RunHoldStage,
    run_full_queue_stage: RunFullQueueStage,
) -> QueueApplyQueueStage<Account>
where
    Account: Clone + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    RunHoldStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedHoldInputs<'_, Account, Tx, Journal, ParentBatchId>,
    ) -> QueueApplyHoldFallback,
    RunFullQueueStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedFullQueueInputs<'_, Account, Tx, Journal, ParentBatchId>,
    ) -> QueueApplyFullQueueDecision<Account>,
{
    run_queue_apply_queue_stage_with_callers(
        views,
        hold_fallback,
        full_queue_decision,
        replaced,
        account,
        tx_id,
        last_valid,
        seq_proxy,
        fee_level,
        flags,
        pf_result,
        order,
        run_hold_stage,
        run_full_queue_stage,
        |views, prepared| {
            enqueue_queue_apply_candidate(
                views,
                prepared.replaced,
                prepared.account,
                prepared.tx_id,
                prepared.last_valid,
                prepared.seq_proxy,
                prepared.fee_level,
                prepared.flags,
                prepared.pf_result,
                prepared.order,
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_queue_stage_with_log_sinks_and_caller_full_queue_and_caller_hold<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    RunHoldStage,
    RunFullQueueStage,
    DebugSink,
    InfoSink,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    hold_fallback: QueueApplyHoldFallback,
    full_queue_decision: QueueApplyFullQueueDecision<Account>,
    replaced: Option<FeeQueueKey<Account>>,
    account: Account,
    tx_id: Uint256,
    last_valid: Option<u32>,
    seq_proxy: SeqProxy,
    fee_level: FeeLevel64,
    flags: ApplyFlags,
    pf_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    order: &OrderCandidates,
    run_hold_stage: RunHoldStage,
    run_full_queue_stage: RunFullQueueStage,
    mut debug: DebugSink,
    mut info: InfoSink,
) -> QueueApplyQueueStage<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    RunHoldStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedHoldInputs<'_, Account, Tx, Journal, ParentBatchId>,
    ) -> QueueApplyHoldFallback,
    RunFullQueueStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedFullQueueInputs<'_, Account, Tx, Journal, ParentBatchId>,
    ) -> QueueApplyFullQueueDecision<Account>,
    DebugSink: FnMut(String),
    InfoSink: FnMut(String),
{
    run_queue_apply_queue_stage_with_callers(
        views,
        hold_fallback,
        full_queue_decision,
        replaced,
        account,
        tx_id,
        last_valid,
        seq_proxy,
        fee_level,
        flags,
        pf_result,
        order,
        run_hold_stage,
        |views, prepared| {
            let info_tx_id = prepared.tx_id;
            let info_account = prepared.account.clone();
            let info_fee_level = prepared.fee_level;
            let decision = run_full_queue_stage(views, prepared);

            match &decision {
                QueueApplyFullQueueDecision::Bypass => {}
                QueueApplyFullQueueDecision::RejectFullSameAccount => {
                    info(format_queue_apply_full_queue_same_account_info_message(
                        info_tx_id,
                        info_account.clone(),
                    ));
                }
                QueueApplyFullQueueDecision::RejectFullLowerFee { .. } => {
                    info(format_queue_apply_full_queue_lower_fee_info_message(
                        info_tx_id,
                    ));
                }
                QueueApplyFullQueueDecision::EvictCheapest {
                    dropped,
                    end_effective_fee_level,
                } => {
                    info(format_queue_apply_full_queue_evict_info_message(
                        dropped.account.clone(),
                        *end_effective_fee_level,
                        info_tx_id,
                        info_fee_level,
                    ));
                }
            }

            decision
        },
        |views, prepared| {
            let result = enqueue_queue_apply_candidate(
                views,
                prepared.replaced,
                prepared.account.clone(),
                prepared.tx_id,
                prepared.last_valid,
                prepared.seq_proxy,
                prepared.fee_level,
                prepared.flags,
                prepared.pf_result,
                prepared.order,
            );

            debug(format_queue_apply_enqueue_debug_message(
                prepared.tx_id,
                result.ter(),
                !result.account_created,
                prepared.account,
                result.stored_flags,
            ));

            result
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_queue_stage_with_caller_enqueue<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    RunFullQueueStage,
    RunEnqueueStage,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    hold_fallback: QueueApplyHoldFallback,
    full_queue_decision: QueueApplyFullQueueDecision<Account>,
    replaced: Option<FeeQueueKey<Account>>,
    account: Account,
    tx_id: Uint256,
    last_valid: Option<u32>,
    seq_proxy: SeqProxy,
    fee_level: FeeLevel64,
    flags: ApplyFlags,
    pf_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    order: &OrderCandidates,
    run_full_queue_stage: RunFullQueueStage,
    run_enqueue_stage: RunEnqueueStage,
) -> QueueApplyQueueStage<Account>
where
    Account: Clone + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    RunFullQueueStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedFullQueueInputs<'_, Account, Tx, Journal, ParentBatchId>,
    ) -> QueueApplyFullQueueDecision<Account>,
    RunEnqueueStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedEnqueueInputs<'_, Account, Tx, Journal, ParentBatchId>,
    ) -> QueueApplyEnqueueResult<Account>,
{
    run_queue_apply_queue_stage_with_caller_enqueue_and_caller_full_queue_and_caller_hold(
        views,
        hold_fallback,
        full_queue_decision,
        replaced,
        account,
        tx_id,
        last_valid,
        seq_proxy,
        fee_level,
        flags,
        pf_result,
        order,
        |_views, prepared| prepared.hold_fallback,
        run_full_queue_stage,
        run_enqueue_stage,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_queue_stage_with_log_sinks_and_caller_enqueue<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    RunFullQueueStage,
    RunEnqueueStage,
    DebugSink,
    InfoSink,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    hold_fallback: QueueApplyHoldFallback,
    full_queue_decision: QueueApplyFullQueueDecision<Account>,
    replaced: Option<FeeQueueKey<Account>>,
    account: Account,
    tx_id: Uint256,
    last_valid: Option<u32>,
    seq_proxy: SeqProxy,
    fee_level: FeeLevel64,
    flags: ApplyFlags,
    pf_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    order: &OrderCandidates,
    run_full_queue_stage: RunFullQueueStage,
    run_enqueue_stage: RunEnqueueStage,
    debug: DebugSink,
    info: InfoSink,
) -> QueueApplyQueueStage<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    RunFullQueueStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedFullQueueInputs<'_, Account, Tx, Journal, ParentBatchId>,
    ) -> QueueApplyFullQueueDecision<Account>,
    RunEnqueueStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedEnqueueInputs<'_, Account, Tx, Journal, ParentBatchId>,
    ) -> QueueApplyEnqueueResult<Account>,
    DebugSink: FnMut(String),
    InfoSink: FnMut(String),
{
    run_queue_apply_queue_stage_with_log_sinks_and_caller_enqueue_and_caller_full_queue_and_caller_hold(
        views,
        hold_fallback,
        full_queue_decision,
        replaced,
        account,
        tx_id,
        last_valid,
        seq_proxy,
        fee_level,
        flags,
        pf_result,
        order,
        |_views, prepared| prepared.hold_fallback,
        run_full_queue_stage,
        run_enqueue_stage,
        debug,
        info,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_queue_stage_with_caller_enqueue_and_caller_full_queue_and_caller_hold<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    RunHoldStage,
    RunFullQueueStage,
    RunEnqueueStage,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    hold_fallback: QueueApplyHoldFallback,
    full_queue_decision: QueueApplyFullQueueDecision<Account>,
    replaced: Option<FeeQueueKey<Account>>,
    account: Account,
    tx_id: Uint256,
    last_valid: Option<u32>,
    seq_proxy: SeqProxy,
    fee_level: FeeLevel64,
    flags: ApplyFlags,
    pf_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    order: &OrderCandidates,
    run_hold_stage: RunHoldStage,
    run_full_queue_stage: RunFullQueueStage,
    run_enqueue_stage: RunEnqueueStage,
) -> QueueApplyQueueStage<Account>
where
    Account: Clone + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    RunHoldStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedHoldInputs<'_, Account, Tx, Journal, ParentBatchId>,
    ) -> QueueApplyHoldFallback,
    RunFullQueueStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedFullQueueInputs<'_, Account, Tx, Journal, ParentBatchId>,
    ) -> QueueApplyFullQueueDecision<Account>,
    RunEnqueueStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedEnqueueInputs<'_, Account, Tx, Journal, ParentBatchId>,
    ) -> QueueApplyEnqueueResult<Account>,
{
    run_queue_apply_queue_stage_with_callers(
        views,
        hold_fallback,
        full_queue_decision,
        replaced,
        account,
        tx_id,
        last_valid,
        seq_proxy,
        fee_level,
        flags,
        pf_result,
        order,
        run_hold_stage,
        run_full_queue_stage,
        run_enqueue_stage,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_queue_stage_with_log_sinks_and_caller_enqueue_and_caller_full_queue_and_caller_hold<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    RunHoldStage,
    RunFullQueueStage,
    RunEnqueueStage,
    DebugSink,
    InfoSink,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    hold_fallback: QueueApplyHoldFallback,
    full_queue_decision: QueueApplyFullQueueDecision<Account>,
    replaced: Option<FeeQueueKey<Account>>,
    account: Account,
    tx_id: Uint256,
    last_valid: Option<u32>,
    seq_proxy: SeqProxy,
    fee_level: FeeLevel64,
    flags: ApplyFlags,
    pf_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    order: &OrderCandidates,
    run_hold_stage: RunHoldStage,
    run_full_queue_stage: RunFullQueueStage,
    run_enqueue_stage: RunEnqueueStage,
    mut debug: DebugSink,
    mut info: InfoSink,
) -> QueueApplyQueueStage<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    RunHoldStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedHoldInputs<'_, Account, Tx, Journal, ParentBatchId>,
    ) -> QueueApplyHoldFallback,
    RunFullQueueStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedFullQueueInputs<'_, Account, Tx, Journal, ParentBatchId>,
    ) -> QueueApplyFullQueueDecision<Account>,
    RunEnqueueStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedEnqueueInputs<'_, Account, Tx, Journal, ParentBatchId>,
    ) -> QueueApplyEnqueueResult<Account>,
    DebugSink: FnMut(String),
    InfoSink: FnMut(String),
{
    run_queue_apply_queue_stage_with_callers(
        views,
        hold_fallback,
        full_queue_decision,
        replaced,
        account,
        tx_id,
        last_valid,
        seq_proxy,
        fee_level,
        flags,
        pf_result,
        order,
        run_hold_stage,
        |views, prepared| {
            let info_tx_id = prepared.tx_id;
            let info_account = prepared.account.clone();
            let info_fee_level = prepared.fee_level;
            let decision = run_full_queue_stage(views, prepared);

            match &decision {
                QueueApplyFullQueueDecision::Bypass => {}
                QueueApplyFullQueueDecision::RejectFullSameAccount => {
                    info(format_queue_apply_full_queue_same_account_info_message(
                        info_tx_id,
                        info_account.clone(),
                    ));
                }
                QueueApplyFullQueueDecision::RejectFullLowerFee { .. } => {
                    info(format_queue_apply_full_queue_lower_fee_info_message(
                        info_tx_id,
                    ));
                }
                QueueApplyFullQueueDecision::EvictCheapest {
                    dropped,
                    end_effective_fee_level,
                } => {
                    info(format_queue_apply_full_queue_evict_info_message(
                        dropped.account.clone(),
                        *end_effective_fee_level,
                        info_tx_id,
                        info_fee_level,
                    ));
                }
            }

            decision
        },
        |views, prepared| {
            let result = run_enqueue_stage(views, prepared.clone());

            debug(format_queue_apply_enqueue_debug_message(
                prepared.tx_id,
                result.ter(),
                !result.account_created,
                prepared.account,
                result.stored_flags,
            ));

            result
        },
    )
}

#[allow(clippy::too_many_arguments)]
fn run_queue_apply_queue_stage_with_callers<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    RunHoldStage,
    RunFullQueueStage,
    RunEnqueueStage,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    hold_fallback: QueueApplyHoldFallback,
    full_queue_decision: QueueApplyFullQueueDecision<Account>,
    replaced: Option<FeeQueueKey<Account>>,
    account: Account,
    tx_id: Uint256,
    last_valid: Option<u32>,
    seq_proxy: SeqProxy,
    fee_level: FeeLevel64,
    flags: ApplyFlags,
    pf_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    order: &OrderCandidates,
    run_hold_stage: RunHoldStage,
    run_full_queue_stage: RunFullQueueStage,
    run_enqueue_stage: RunEnqueueStage,
) -> QueueApplyQueueStage<Account>
where
    Account: Clone + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    RunHoldStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedHoldInputs<'_, Account, Tx, Journal, ParentBatchId>,
    ) -> QueueApplyHoldFallback,
    RunFullQueueStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedFullQueueInputs<'_, Account, Tx, Journal, ParentBatchId>,
    ) -> QueueApplyFullQueueDecision<Account>,
    RunEnqueueStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedEnqueueInputs<'_, Account, Tx, Journal, ParentBatchId>,
    ) -> QueueApplyEnqueueResult<Account>,
{
    let hold_fallback = run_hold_stage(
        views,
        QueueApplyPreparedHoldInputs::new(
            hold_fallback,
            full_queue_decision.clone(),
            replaced.clone(),
            account.clone(),
            tx_id,
            last_valid,
            seq_proxy,
            fee_level,
            flags,
            pf_result.clone(),
            order,
        ),
    );
    if let Some(ter) = hold_fallback.ter() {
        return QueueApplyQueueStage::RejectCannotHold(ter);
    }

    let full_queue_decision = run_full_queue_stage(
        views,
        QueueApplyPreparedFullQueueInputs::new(
            full_queue_decision,
            replaced.clone(),
            account.clone(),
            tx_id,
            last_valid,
            seq_proxy,
            fee_level,
            flags,
            pf_result.clone(),
            order,
        ),
    );
    if full_queue_decision.rejects_full() {
        return QueueApplyQueueStage::RejectFull;
    }

    QueueApplyQueueStage::Queued(run_enqueue_stage(
        views,
        QueueApplyPreparedEnqueueInputs::new(
            full_queue_decision,
            replaced,
            account,
            tx_id,
            last_valid,
            seq_proxy,
            fee_level,
            flags,
            pf_result,
            order,
        ),
    ))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use basics::base_uint::Uint256;
    use protocol::{Rules, SeqProxy, Ter};

    use super::{
        QueueApplyQueueLogMessages, QueueApplyQueueStage,
        QueueApplyQueueStageWithLogMessagesResult, format_queue_apply_enqueue_debug_message,
        format_queue_apply_full_queue_evict_info_message, run_queue_apply_queue_stage,
        run_queue_apply_queue_stage_with_caller_enqueue,
        run_queue_apply_queue_stage_with_caller_enqueue_and_caller_full_queue_and_caller_hold,
        run_queue_apply_queue_stage_with_caller_full_queue,
        run_queue_apply_queue_stage_with_caller_hold,
        run_queue_apply_queue_stage_with_log_messages,
        run_queue_apply_queue_stage_with_log_messages_and_caller_enqueue,
        run_queue_apply_queue_stage_with_log_sinks_and_caller_enqueue,
        run_queue_apply_queue_stage_with_log_sinks_and_caller_full_queue,
    };
    use crate::{
        ApplyFlags, ApplyResult, FeeQueueEntry, FeeQueueKey, MaybeTx, MaybeTxCore, OrderCandidates,
        PreflightResult, QueueAdvanceCandidate, QueueApplyEnqueueResult,
        QueueApplyFullQueueDecision, QueueApplyHoldFallback, QueueViews, TxConsequences,
        TxQAccount, apply_queue_apply_full_queue_decision,
    };

    fn make_preflight(
        tx: &'static str,
        seq_proxy: SeqProxy,
        flags: ApplyFlags,
    ) -> PreflightResult<&'static str, TxConsequences, &'static str, &'static str> {
        PreflightResult::new(
            tx,
            None,
            Rules::new(std::iter::empty()),
            TxConsequences::new(1, seq_proxy),
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
    ) -> MaybeTx<&'static str, &'static str, &'static str, &'static str> {
        MaybeTx::new(
            Uint256::from_u64(tx_id),
            fee_level,
            account,
            Some(200),
            seq_proxy,
            flags,
            make_preflight("tx", seq_proxy, flags),
        )
    }

    fn fee_entry(
        account: &'static str,
        seq_proxy: SeqProxy,
        tx_id: u64,
        fee_level: u64,
    ) -> FeeQueueEntry<&'static str> {
        FeeQueueEntry::new(
            FeeQueueKey::new(account, seq_proxy),
            QueueAdvanceCandidate {
                fee_level,
                tx_id: Uint256::from_u64(tx_id),
                seq_proxy,
            },
        )
    }

    #[test]
    fn queue_stage_rejects_hold_failures_before_queue_mutation() {
        let existing = queued("a", SeqProxy::sequence(5), 5, 90, ApplyFlags::NONE);
        let mut account_a = TxQAccount::new("a");
        account_a.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(existing, TxConsequences::new(1, SeqProxy::sequence(5))),
        );
        let mut views = QueueViews::new(
            BTreeMap::from([("a", account_a)]),
            vec![fee_entry("a", SeqProxy::sequence(5), 5, 90)],
        );

        let stage = run_queue_apply_queue_stage(
            &mut views,
            QueueApplyHoldFallback::RejectCannotHold(Ter::TEL_CAN_NOT_QUEUE),
            QueueApplyFullQueueDecision::Bypass,
            None,
            "a",
            Uint256::from_u64(9),
            Some(250),
            SeqProxy::sequence(6),
            110,
            ApplyFlags::NONE,
            make_preflight("tx", SeqProxy::sequence(6), ApplyFlags::NONE),
            &OrderCandidates::new(Uint256::from_u64(0)),
        );

        assert_eq!(
            stage,
            QueueApplyQueueStage::RejectCannotHold(Ter::TEL_CAN_NOT_QUEUE)
        );
        assert_eq!(
            stage.apply_result(),
            ApplyResult::new(Ter::TEL_CAN_NOT_QUEUE, false, false)
        );
        assert_eq!(views.fee_order.len(), 1);
        assert!(
            views.accounts["a"]
                .transactions
                .contains_key(&SeqProxy::sequence(5))
        );
    }

    #[test]
    fn queue_stage_rejects_full_queue_before_enqueue() {
        let existing = queued("a", SeqProxy::sequence(5), 5, 90, ApplyFlags::NONE);
        let mut account_a = TxQAccount::new("a");
        account_a.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(existing, TxConsequences::new(1, SeqProxy::sequence(5))),
        );
        let mut views = QueueViews::new(
            BTreeMap::from([("a", account_a)]),
            vec![fee_entry("a", SeqProxy::sequence(5), 5, 90)],
        );

        let stage = run_queue_apply_queue_stage(
            &mut views,
            QueueApplyHoldFallback::HoldAllowed,
            QueueApplyFullQueueDecision::RejectFullSameAccount,
            None,
            "a",
            Uint256::from_u64(9),
            Some(250),
            SeqProxy::sequence(6),
            110,
            ApplyFlags::NONE,
            make_preflight("tx", SeqProxy::sequence(6), ApplyFlags::NONE),
            &OrderCandidates::new(Uint256::from_u64(0)),
        );

        assert_eq!(stage, QueueApplyQueueStage::RejectFull);
        assert_eq!(
            stage.apply_result(),
            ApplyResult::new(Ter::TEL_CAN_NOT_QUEUE_FULL, false, false)
        );
        assert_eq!(views.fee_order.len(), 1);
        assert!(
            !views.accounts["a"]
                .transactions
                .contains_key(&SeqProxy::sequence(6))
        );
    }

    #[test]
    fn queue_stage_can_evict_and_enqueue_in() {
        let mut account_a = TxQAccount::new("a");
        account_a.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                queued("a", SeqProxy::sequence(5), 5, 90, ApplyFlags::NONE),
                TxConsequences::new(1, SeqProxy::sequence(5)),
            ),
        );

        let mut account_b = TxQAccount::new("b");
        account_b.add(
            SeqProxy::sequence(8),
            MaybeTxCore::new(
                queued("b", SeqProxy::sequence(8), 8, 50, ApplyFlags::NONE),
                TxConsequences::new(1, SeqProxy::sequence(8)),
            ),
        );

        let mut views = QueueViews::new(
            BTreeMap::from([("a", account_a), ("b", account_b)]),
            vec![
                fee_entry("a", SeqProxy::sequence(5), 5, 90),
                fee_entry("b", SeqProxy::sequence(8), 8, 50),
            ],
        );

        let stage = run_queue_apply_queue_stage(
            &mut views,
            QueueApplyHoldFallback::AlreadyVerifiedByMultiTxn,
            QueueApplyFullQueueDecision::EvictCheapest {
                dropped: FeeQueueKey::new("b", SeqProxy::sequence(8)),
                end_effective_fee_level: 50,
            },
            None,
            "c",
            Uint256::from_u64(9),
            Some(250),
            SeqProxy::sequence(6),
            110,
            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
            make_preflight(
                "tx",
                SeqProxy::sequence(6),
                ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
            ),
            &OrderCandidates::new(Uint256::from_u64(0)),
        );

        assert_eq!(
            stage,
            QueueApplyQueueStage::Queued(QueueApplyEnqueueResult {
                queued: FeeQueueKey::new("c", SeqProxy::sequence(6)),
                removed_replacement: None,
                account_created: true,
                stored_flags: ApplyFlags::FAIL_HARD,
            })
        );
        assert!(views.accounts.contains_key("b"));
        assert!(views.accounts["b"].transactions.is_empty());
        assert!(views.accounts.contains_key("c"));
        assert_eq!(
            views
                .fee_order
                .iter()
                .map(|entry| entry.key.clone())
                .collect::<Vec<_>>(),
            vec![
                FeeQueueKey::new("c", SeqProxy::sequence(6)),
                FeeQueueKey::new("a", SeqProxy::sequence(5)),
            ]
        );
    }

    #[test]
    fn queue_stage_replaces_existing_entry_before_enqueue() {
        let mut account_a = TxQAccount::new("a");
        account_a.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                queued(
                    "a",
                    SeqProxy::sequence(5),
                    5,
                    90,
                    ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
                ),
                TxConsequences::new(1, SeqProxy::sequence(5)),
            ),
        );
        let mut views = QueueViews::new(
            BTreeMap::from([("a", account_a)]),
            vec![fee_entry("a", SeqProxy::sequence(5), 5, 90)],
        );

        let stage = run_queue_apply_queue_stage(
            &mut views,
            QueueApplyHoldFallback::HoldAllowed,
            QueueApplyFullQueueDecision::Bypass,
            Some(FeeQueueKey::new("a", SeqProxy::sequence(5))),
            "a",
            Uint256::from_u64(9),
            Some(250),
            SeqProxy::sequence(5),
            110,
            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
            make_preflight(
                "replacement",
                SeqProxy::sequence(5),
                ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
            ),
            &OrderCandidates::new(Uint256::from_u64(0)),
        );

        assert_eq!(
            stage,
            QueueApplyQueueStage::Queued(QueueApplyEnqueueResult {
                queued: FeeQueueKey::new("a", SeqProxy::sequence(5)),
                removed_replacement: Some(FeeQueueKey::new("a", SeqProxy::sequence(5))),
                account_created: false,
                stored_flags: ApplyFlags::FAIL_HARD,
            })
        );
        assert_eq!(views.fee_order.len(), 1);
        let stored = &views.accounts["a"].transactions[&SeqProxy::sequence(5)].payload;
        assert_eq!(stored.tx_id, Uint256::from_u64(9));
        assert_eq!(stored.flags, ApplyFlags::FAIL_HARD);
    }

    #[test]
    fn queue_stage_with_caller_enqueue_exposes_prepared_enqueue_boundary() {
        let mut account_a = TxQAccount::new("a");
        account_a.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                queued("a", SeqProxy::sequence(5), 5, 90, ApplyFlags::NONE),
                TxConsequences::new(1, SeqProxy::sequence(5)),
            ),
        );
        let mut views = QueueViews::new(
            BTreeMap::from([("a", account_a)]),
            vec![fee_entry("a", SeqProxy::sequence(5), 5, 90)],
        );
        let preflight = make_preflight("tx", SeqProxy::sequence(6), ApplyFlags::RETRY);
        let order = OrderCandidates::new(Uint256::from_u64(0));

        let stage = run_queue_apply_queue_stage_with_caller_enqueue(
            &mut views,
            QueueApplyHoldFallback::AlreadyVerifiedByMultiTxn,
            QueueApplyFullQueueDecision::Bypass,
            Some(FeeQueueKey::new("a", SeqProxy::sequence(5))),
            "a",
            Uint256::from_u64(9),
            Some(250),
            SeqProxy::sequence(6),
            110,
            ApplyFlags::RETRY,
            preflight.clone(),
            &order,
            |views, prepared| {
                apply_queue_apply_full_queue_decision(views, prepared.full_queue_decision)
            },
            |_views, prepared| {
                assert_eq!(
                    prepared.full_queue_decision,
                    QueueApplyFullQueueDecision::Bypass
                );
                assert_eq!(
                    prepared.replaced,
                    Some(FeeQueueKey::new("a", SeqProxy::sequence(5)))
                );
                assert_eq!(prepared.account, "a");
                assert_eq!(prepared.tx_id, Uint256::from_u64(9));
                assert_eq!(prepared.last_valid, Some(250));
                assert_eq!(prepared.seq_proxy, SeqProxy::sequence(6));
                assert_eq!(prepared.fee_level, 110);
                assert_eq!(prepared.flags, ApplyFlags::RETRY);
                assert_eq!(prepared.pf_result, preflight);
                assert!(std::ptr::eq(prepared.order, &order));

                QueueApplyEnqueueResult {
                    queued: FeeQueueKey::new("a", SeqProxy::sequence(6)),
                    removed_replacement: Some(FeeQueueKey::new("a", SeqProxy::sequence(5))),
                    account_created: false,
                    stored_flags: ApplyFlags::NONE,
                }
            },
        );

        assert_eq!(
            stage,
            QueueApplyQueueStage::Queued(QueueApplyEnqueueResult {
                queued: FeeQueueKey::new("a", SeqProxy::sequence(6)),
                removed_replacement: Some(FeeQueueKey::new("a", SeqProxy::sequence(5))),
                account_created: false,
                stored_flags: ApplyFlags::NONE,
            })
        );
        assert_eq!(
            stage.apply_result(),
            ApplyResult::new(Ter::TER_QUEUED, false, false)
        );
        assert_eq!(views.fee_order.len(), 1);
        assert!(
            views.accounts["a"]
                .transactions
                .contains_key(&SeqProxy::sequence(5))
        );
    }

    #[test]
    fn queue_stage_with_caller_enqueue_and_caller_full_queue_and_caller_hold_exposes_all_boundaries()
     {
        let mut account_a = TxQAccount::new("a");
        account_a.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                queued("a", SeqProxy::sequence(5), 5, 90, ApplyFlags::NONE),
                TxConsequences::new(1, SeqProxy::sequence(5)),
            ),
        );
        let mut views = QueueViews::new(
            BTreeMap::from([("a", account_a)]),
            vec![fee_entry("a", SeqProxy::sequence(5), 5, 90)],
        );
        let preflight = make_preflight("tx", SeqProxy::sequence(6), ApplyFlags::RETRY);
        let order = OrderCandidates::new(Uint256::from_u64(0));

        let stage =
            run_queue_apply_queue_stage_with_caller_enqueue_and_caller_full_queue_and_caller_hold(
                &mut views,
                QueueApplyHoldFallback::AlreadyVerifiedByMultiTxn,
                QueueApplyFullQueueDecision::Bypass,
                Some(FeeQueueKey::new("a", SeqProxy::sequence(5))),
                "a",
                Uint256::from_u64(9),
                Some(250),
                SeqProxy::sequence(6),
                110,
                ApplyFlags::RETRY,
                preflight.clone(),
                &order,
                |_views, prepared| {
                    assert_eq!(
                        prepared.hold_fallback,
                        QueueApplyHoldFallback::AlreadyVerifiedByMultiTxn
                    );
                    assert_eq!(
                        prepared.full_queue_decision,
                        QueueApplyFullQueueDecision::Bypass
                    );
                    assert_eq!(
                        prepared.replaced,
                        Some(FeeQueueKey::new("a", SeqProxy::sequence(5)))
                    );
                    assert_eq!(prepared.account, "a");
                    assert_eq!(prepared.tx_id, Uint256::from_u64(9));
                    assert_eq!(prepared.last_valid, Some(250));
                    assert_eq!(prepared.seq_proxy, SeqProxy::sequence(6));
                    assert_eq!(prepared.fee_level, 110);
                    assert_eq!(prepared.flags, ApplyFlags::RETRY);
                    assert_eq!(prepared.pf_result, preflight.clone());
                    assert!(std::ptr::eq(prepared.order, &order));

                    QueueApplyHoldFallback::AlreadyVerifiedByMultiTxn
                },
                |_views, prepared| {
                    assert_eq!(
                        prepared.full_queue_decision,
                        QueueApplyFullQueueDecision::Bypass
                    );
                    assert_eq!(
                        prepared.replaced,
                        Some(FeeQueueKey::new("a", SeqProxy::sequence(5)))
                    );
                    assert_eq!(prepared.account, "a");
                    assert_eq!(prepared.tx_id, Uint256::from_u64(9));
                    assert_eq!(prepared.last_valid, Some(250));
                    assert_eq!(prepared.seq_proxy, SeqProxy::sequence(6));
                    assert_eq!(prepared.fee_level, 110);
                    assert_eq!(prepared.flags, ApplyFlags::RETRY);
                    assert_eq!(prepared.pf_result, preflight.clone());
                    assert!(std::ptr::eq(prepared.order, &order));

                    QueueApplyFullQueueDecision::Bypass
                },
                |_views, prepared| {
                    assert_eq!(
                        prepared.full_queue_decision,
                        QueueApplyFullQueueDecision::Bypass
                    );
                    assert_eq!(
                        prepared.replaced,
                        Some(FeeQueueKey::new("a", SeqProxy::sequence(5)))
                    );
                    assert_eq!(prepared.account, "a");
                    assert_eq!(prepared.tx_id, Uint256::from_u64(9));
                    assert_eq!(prepared.last_valid, Some(250));
                    assert_eq!(prepared.seq_proxy, SeqProxy::sequence(6));
                    assert_eq!(prepared.fee_level, 110);
                    assert_eq!(prepared.flags, ApplyFlags::RETRY);
                    assert_eq!(prepared.pf_result, preflight);
                    assert!(std::ptr::eq(prepared.order, &order));

                    QueueApplyEnqueueResult {
                        queued: FeeQueueKey::new("a", SeqProxy::sequence(6)),
                        removed_replacement: Some(FeeQueueKey::new("a", SeqProxy::sequence(5))),
                        account_created: false,
                        stored_flags: ApplyFlags::NONE,
                    }
                },
            );

        assert_eq!(
            stage,
            QueueApplyQueueStage::Queued(QueueApplyEnqueueResult {
                queued: FeeQueueKey::new("a", SeqProxy::sequence(6)),
                removed_replacement: Some(FeeQueueKey::new("a", SeqProxy::sequence(5))),
                account_created: false,
                stored_flags: ApplyFlags::NONE,
            })
        );
        assert_eq!(
            stage.apply_result(),
            ApplyResult::new(Ter::TER_QUEUED, false, false)
        );
        assert_eq!(views.fee_order.len(), 1);
        assert!(
            views.accounts["a"]
                .transactions
                .contains_key(&SeqProxy::sequence(5))
        );
    }

    #[test]
    fn queue_stage_with_caller_full_queue_exposes_prepared_full_queue_boundary() {
        let mut account_a = TxQAccount::new("a");
        account_a.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                queued("a", SeqProxy::sequence(5), 5, 90, ApplyFlags::NONE),
                TxConsequences::new(1, SeqProxy::sequence(5)),
            ),
        );
        let mut views = QueueViews::new(
            BTreeMap::from([("a", account_a)]),
            vec![fee_entry("a", SeqProxy::sequence(5), 5, 90)],
        );
        let preflight = make_preflight("tx", SeqProxy::sequence(6), ApplyFlags::RETRY);
        let order = OrderCandidates::new(Uint256::from_u64(0));

        let stage = run_queue_apply_queue_stage_with_caller_full_queue(
            &mut views,
            QueueApplyHoldFallback::AlreadyVerifiedByMultiTxn,
            QueueApplyFullQueueDecision::Bypass,
            None,
            "a",
            Uint256::from_u64(9),
            Some(250),
            SeqProxy::sequence(6),
            110,
            ApplyFlags::RETRY,
            preflight.clone(),
            &order,
            |_views, prepared| {
                assert_eq!(
                    prepared.full_queue_decision,
                    QueueApplyFullQueueDecision::Bypass
                );
                assert_eq!(prepared.replaced, None);
                assert_eq!(prepared.account, "a");
                assert_eq!(prepared.tx_id, Uint256::from_u64(9));
                assert_eq!(prepared.last_valid, Some(250));
                assert_eq!(prepared.seq_proxy, SeqProxy::sequence(6));
                assert_eq!(prepared.fee_level, 110);
                assert_eq!(prepared.flags, ApplyFlags::RETRY);
                assert_eq!(prepared.pf_result, preflight);
                assert!(std::ptr::eq(prepared.order, &order));

                QueueApplyFullQueueDecision::Bypass
            },
        );

        assert_eq!(
            stage,
            QueueApplyQueueStage::Queued(QueueApplyEnqueueResult {
                queued: FeeQueueKey::new("a", SeqProxy::sequence(6)),
                removed_replacement: None,
                account_created: false,
                stored_flags: ApplyFlags::NONE,
            })
        );
        assert_eq!(
            stage.apply_result(),
            ApplyResult::new(Ter::TER_QUEUED, false, false)
        );
        let stored = &views.accounts["a"].transactions[&SeqProxy::sequence(6)].payload;
        assert_eq!(stored.tx_id, Uint256::from_u64(9));
        assert_eq!(stored.flags, ApplyFlags::NONE);
    }

    #[test]
    fn queue_stage_with_log_sinks_and_caller_full_queue_preserves_logs_and_boundary() {
        let mut account_a = TxQAccount::new("a");
        account_a.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                queued("a", SeqProxy::sequence(5), 5, 90, ApplyFlags::NONE),
                TxConsequences::new(1, SeqProxy::sequence(5)),
            ),
        );
        let mut views = QueueViews::new(
            BTreeMap::from([("a", account_a)]),
            vec![fee_entry("a", SeqProxy::sequence(5), 5, 90)],
        );
        let preflight = make_preflight("tx", SeqProxy::sequence(6), ApplyFlags::RETRY);
        let order = OrderCandidates::new(Uint256::from_u64(0));
        let mut debug_messages = Vec::new();
        let mut info_messages = Vec::new();

        let stage = run_queue_apply_queue_stage_with_log_sinks_and_caller_full_queue(
            &mut views,
            QueueApplyHoldFallback::AlreadyVerifiedByMultiTxn,
            QueueApplyFullQueueDecision::Bypass,
            None,
            "a",
            Uint256::from_u64(9),
            Some(250),
            SeqProxy::sequence(6),
            110,
            ApplyFlags::RETRY,
            preflight.clone(),
            &order,
            |_views, prepared| {
                assert_eq!(
                    prepared.full_queue_decision,
                    QueueApplyFullQueueDecision::Bypass
                );
                assert_eq!(prepared.replaced, None);
                assert_eq!(prepared.account, "a");
                assert_eq!(prepared.tx_id, Uint256::from_u64(9));
                assert_eq!(prepared.last_valid, Some(250));
                assert_eq!(prepared.seq_proxy, SeqProxy::sequence(6));
                assert_eq!(prepared.fee_level, 110);
                assert_eq!(prepared.flags, ApplyFlags::RETRY);
                assert_eq!(prepared.pf_result, preflight);
                assert!(std::ptr::eq(prepared.order, &order));

                QueueApplyFullQueueDecision::Bypass
            },
            |message| debug_messages.push(message),
            |message| info_messages.push(message),
        );

        assert_eq!(
            stage,
            QueueApplyQueueStage::Queued(QueueApplyEnqueueResult {
                queued: FeeQueueKey::new("a", SeqProxy::sequence(6)),
                removed_replacement: None,
                account_created: false,
                stored_flags: ApplyFlags::NONE,
            })
        );
        assert!(info_messages.is_empty());
        assert_eq!(
            debug_messages,
            vec![format_queue_apply_enqueue_debug_message(
                Uint256::from_u64(9),
                Ter::TER_QUEUED,
                true,
                "a",
                ApplyFlags::NONE,
            )]
        );
    }

    #[test]
    fn queue_stage_with_log_sinks_and_caller_enqueue_preserves_logs_and_boundary() {
        let mut account_a = TxQAccount::new("a");
        account_a.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                queued("a", SeqProxy::sequence(5), 5, 90, ApplyFlags::NONE),
                TxConsequences::new(1, SeqProxy::sequence(5)),
            ),
        );
        let mut views = QueueViews::new(
            BTreeMap::from([("a", account_a)]),
            vec![fee_entry("a", SeqProxy::sequence(5), 5, 90)],
        );
        let preflight = make_preflight("tx", SeqProxy::sequence(6), ApplyFlags::RETRY);
        let order = OrderCandidates::new(Uint256::from_u64(0));
        let mut debug_messages = Vec::new();
        let mut info_messages = Vec::new();

        let stage = run_queue_apply_queue_stage_with_log_sinks_and_caller_enqueue(
            &mut views,
            QueueApplyHoldFallback::AlreadyVerifiedByMultiTxn,
            QueueApplyFullQueueDecision::Bypass,
            Some(FeeQueueKey::new("a", SeqProxy::sequence(5))),
            "a",
            Uint256::from_u64(9),
            Some(250),
            SeqProxy::sequence(6),
            110,
            ApplyFlags::RETRY,
            preflight.clone(),
            &order,
            |_views, prepared| {
                assert_eq!(
                    prepared.full_queue_decision,
                    QueueApplyFullQueueDecision::Bypass
                );
                assert_eq!(
                    prepared.replaced,
                    Some(FeeQueueKey::new("a", SeqProxy::sequence(5)))
                );
                assert_eq!(prepared.account, "a");
                assert_eq!(prepared.tx_id, Uint256::from_u64(9));
                assert_eq!(prepared.last_valid, Some(250));
                assert_eq!(prepared.seq_proxy, SeqProxy::sequence(6));
                assert_eq!(prepared.fee_level, 110);
                assert_eq!(prepared.flags, ApplyFlags::RETRY);
                assert_eq!(prepared.pf_result, preflight.clone());
                assert!(std::ptr::eq(prepared.order, &order));

                QueueApplyFullQueueDecision::Bypass
            },
            |_views, prepared| {
                assert_eq!(
                    prepared.full_queue_decision,
                    QueueApplyFullQueueDecision::Bypass
                );
                assert_eq!(
                    prepared.replaced,
                    Some(FeeQueueKey::new("a", SeqProxy::sequence(5)))
                );
                assert_eq!(prepared.account, "a");
                assert_eq!(prepared.tx_id, Uint256::from_u64(9));
                assert_eq!(prepared.last_valid, Some(250));
                assert_eq!(prepared.seq_proxy, SeqProxy::sequence(6));
                assert_eq!(prepared.fee_level, 110);
                assert_eq!(prepared.flags, ApplyFlags::RETRY);
                assert_eq!(prepared.pf_result, preflight);
                assert!(std::ptr::eq(prepared.order, &order));

                QueueApplyEnqueueResult {
                    queued: FeeQueueKey::new("a", SeqProxy::sequence(6)),
                    removed_replacement: Some(FeeQueueKey::new("a", SeqProxy::sequence(5))),
                    account_created: false,
                    stored_flags: ApplyFlags::NONE,
                }
            },
            |message| debug_messages.push(message),
            |message| info_messages.push(message),
        );

        assert_eq!(
            stage,
            QueueApplyQueueStage::Queued(QueueApplyEnqueueResult {
                queued: FeeQueueKey::new("a", SeqProxy::sequence(6)),
                removed_replacement: Some(FeeQueueKey::new("a", SeqProxy::sequence(5))),
                account_created: false,
                stored_flags: ApplyFlags::NONE,
            })
        );
        assert!(info_messages.is_empty());
        assert_eq!(
            debug_messages,
            vec![format_queue_apply_enqueue_debug_message(
                Uint256::from_u64(9),
                Ter::TER_QUEUED,
                true,
                "a",
                ApplyFlags::NONE,
            )]
        );
    }

    #[test]
    fn queue_stage_with_log_messages_and_caller_enqueue_preserves_logs_and_boundary() {
        let mut account_a = TxQAccount::new("a");
        account_a.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                queued("a", SeqProxy::sequence(5), 5, 90, ApplyFlags::NONE),
                TxConsequences::new(1, SeqProxy::sequence(5)),
            ),
        );

        let mut account_b = TxQAccount::new("b");
        account_b.add(
            SeqProxy::sequence(8),
            MaybeTxCore::new(
                queued("b", SeqProxy::sequence(8), 8, 50, ApplyFlags::NONE),
                TxConsequences::new(1, SeqProxy::sequence(8)),
            ),
        );

        let mut views = QueueViews::new(
            BTreeMap::from([("a", account_a), ("b", account_b)]),
            vec![
                fee_entry("a", SeqProxy::sequence(5), 5, 90),
                fee_entry("b", SeqProxy::sequence(8), 8, 50),
            ],
        );
        let preflight = make_preflight(
            "tx",
            SeqProxy::sequence(6),
            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        );
        let order = OrderCandidates::new(Uint256::from_u64(0));

        let result = run_queue_apply_queue_stage_with_log_messages_and_caller_enqueue(
            &mut views,
            QueueApplyHoldFallback::AlreadyVerifiedByMultiTxn,
            QueueApplyFullQueueDecision::EvictCheapest {
                dropped: FeeQueueKey::new("b", SeqProxy::sequence(8)),
                end_effective_fee_level: 50,
            },
            None,
            "c",
            Uint256::from_u64(9),
            Some(250),
            SeqProxy::sequence(6),
            110,
            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
            preflight.clone(),
            &order,
            |views, prepared| {
                assert_eq!(
                    prepared.full_queue_decision,
                    QueueApplyFullQueueDecision::EvictCheapest {
                        dropped: FeeQueueKey::new("b", SeqProxy::sequence(8)),
                        end_effective_fee_level: 50,
                    }
                );
                assert_eq!(prepared.replaced, None);
                assert_eq!(prepared.account, "c");
                assert_eq!(prepared.tx_id, Uint256::from_u64(9));
                assert_eq!(prepared.last_valid, Some(250));
                assert_eq!(prepared.seq_proxy, SeqProxy::sequence(6));
                assert_eq!(prepared.fee_level, 110);
                assert_eq!(prepared.flags, ApplyFlags::FAIL_HARD | ApplyFlags::RETRY);
                assert_eq!(prepared.pf_result, preflight.clone());
                assert!(std::ptr::eq(prepared.order, &order));

                apply_queue_apply_full_queue_decision(views, prepared.full_queue_decision)
            },
            |_views, prepared| {
                assert_eq!(prepared.replaced, None);
                assert_eq!(prepared.account, "c");
                assert_eq!(prepared.tx_id, Uint256::from_u64(9));
                assert_eq!(prepared.last_valid, Some(250));
                assert_eq!(prepared.seq_proxy, SeqProxy::sequence(6));
                assert_eq!(prepared.fee_level, 110);
                assert_eq!(prepared.flags, ApplyFlags::FAIL_HARD | ApplyFlags::RETRY);
                assert_eq!(
                    prepared.full_queue_decision,
                    QueueApplyFullQueueDecision::EvictCheapest {
                        dropped: FeeQueueKey::new("b", SeqProxy::sequence(8)),
                        end_effective_fee_level: 50,
                    }
                );
                assert_eq!(prepared.pf_result, preflight);
                assert!(std::ptr::eq(prepared.order, &order));

                QueueApplyEnqueueResult {
                    queued: FeeQueueKey::new("c", SeqProxy::sequence(6)),
                    removed_replacement: None,
                    account_created: true,
                    stored_flags: ApplyFlags::FAIL_HARD,
                }
            },
        );

        assert_eq!(
            result,
            QueueApplyQueueStageWithLogMessagesResult {
                stage: QueueApplyQueueStage::Queued(QueueApplyEnqueueResult {
                    queued: FeeQueueKey::new("c", SeqProxy::sequence(6)),
                    removed_replacement: None,
                    account_created: true,
                    stored_flags: ApplyFlags::FAIL_HARD,
                }),
                log_messages: QueueApplyQueueLogMessages {
                    trace: vec![],
                    debug: vec![format_queue_apply_enqueue_debug_message(
                        Uint256::from_u64(9),
                        Ter::TER_QUEUED,
                        false,
                        "c",
                        ApplyFlags::FAIL_HARD,
                    )],
                    info: vec![format_queue_apply_full_queue_evict_info_message(
                        "b",
                        50,
                        Uint256::from_u64(9),
                        110,
                    )],
                },
            }
        );
    }

    #[test]
    fn queue_stage_with_caller_hold_exposes_prepared_hold_boundary() {
        let mut account_a = TxQAccount::new("a");
        account_a.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                queued("a", SeqProxy::sequence(5), 5, 90, ApplyFlags::NONE),
                TxConsequences::new(1, SeqProxy::sequence(5)),
            ),
        );
        let mut views = QueueViews::new(
            BTreeMap::from([("a", account_a)]),
            vec![fee_entry("a", SeqProxy::sequence(5), 5, 90)],
        );
        let preflight = make_preflight("tx", SeqProxy::sequence(6), ApplyFlags::RETRY);
        let order = OrderCandidates::new(Uint256::from_u64(0));

        let stage = run_queue_apply_queue_stage_with_caller_hold(
            &mut views,
            QueueApplyHoldFallback::AlreadyVerifiedByMultiTxn,
            QueueApplyFullQueueDecision::Bypass,
            None,
            "a",
            Uint256::from_u64(9),
            Some(250),
            SeqProxy::sequence(6),
            110,
            ApplyFlags::RETRY,
            preflight.clone(),
            &order,
            |_views, prepared| {
                assert_eq!(
                    prepared.hold_fallback,
                    QueueApplyHoldFallback::AlreadyVerifiedByMultiTxn
                );
                assert_eq!(
                    prepared.full_queue_decision,
                    QueueApplyFullQueueDecision::Bypass
                );
                assert_eq!(prepared.replaced, None);
                assert_eq!(prepared.account, "a");
                assert_eq!(prepared.tx_id, Uint256::from_u64(9));
                assert_eq!(prepared.last_valid, Some(250));
                assert_eq!(prepared.seq_proxy, SeqProxy::sequence(6));
                assert_eq!(prepared.fee_level, 110);
                assert_eq!(prepared.flags, ApplyFlags::RETRY);
                assert_eq!(prepared.pf_result, preflight);
                assert!(std::ptr::eq(prepared.order, &order));

                QueueApplyHoldFallback::AlreadyVerifiedByMultiTxn
            },
        );

        assert_eq!(
            stage,
            QueueApplyQueueStage::Queued(QueueApplyEnqueueResult {
                queued: FeeQueueKey::new("a", SeqProxy::sequence(6)),
                removed_replacement: None,
                account_created: false,
                stored_flags: ApplyFlags::NONE,
            })
        );
        assert_eq!(
            stage.apply_result(),
            ApplyResult::new(Ter::TER_QUEUED, false, false)
        );
        let stored = &views.accounts["a"].transactions[&SeqProxy::sequence(6)].payload;
        assert_eq!(stored.tx_id, Uint256::from_u64(9));
        assert_eq!(stored.flags, ApplyFlags::NONE);
    }

    #[test]
    fn queue_stage_with_log_messages_collects_full_queue_and_enqueue_logs() {
        let mut account_a = TxQAccount::new("a");
        account_a.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                queued("a", SeqProxy::sequence(5), 5, 90, ApplyFlags::NONE),
                TxConsequences::new(1, SeqProxy::sequence(5)),
            ),
        );

        let mut account_b = TxQAccount::new("b");
        account_b.add(
            SeqProxy::sequence(8),
            MaybeTxCore::new(
                queued("b", SeqProxy::sequence(8), 8, 50, ApplyFlags::NONE),
                TxConsequences::new(1, SeqProxy::sequence(8)),
            ),
        );

        let mut views = QueueViews::new(
            BTreeMap::from([("a", account_a), ("b", account_b)]),
            vec![
                fee_entry("a", SeqProxy::sequence(5), 5, 90),
                fee_entry("b", SeqProxy::sequence(8), 8, 50),
            ],
        );

        let result = run_queue_apply_queue_stage_with_log_messages(
            &mut views,
            QueueApplyHoldFallback::AlreadyVerifiedByMultiTxn,
            QueueApplyFullQueueDecision::EvictCheapest {
                dropped: FeeQueueKey::new("b", SeqProxy::sequence(8)),
                end_effective_fee_level: 50,
            },
            None,
            "c",
            Uint256::from_u64(9),
            Some(250),
            SeqProxy::sequence(6),
            110,
            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
            make_preflight(
                "tx",
                SeqProxy::sequence(6),
                ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
            ),
            &OrderCandidates::new(Uint256::from_u64(0)),
        );

        assert_eq!(
            result.stage,
            QueueApplyQueueStage::Queued(QueueApplyEnqueueResult {
                queued: FeeQueueKey::new("c", SeqProxy::sequence(6)),
                removed_replacement: None,
                account_created: true,
                stored_flags: ApplyFlags::FAIL_HARD,
            })
        );
        assert_eq!(
            result.log_messages.info,
            vec![format_queue_apply_full_queue_evict_info_message(
                "b",
                50,
                Uint256::from_u64(9),
                110,
            )]
        );
        assert_eq!(
            result.log_messages.debug,
            vec![format_queue_apply_enqueue_debug_message(
                Uint256::from_u64(9),
                Ter::TES_SUCCESS,
                false,
                "c",
                ApplyFlags::FAIL_HARD,
            )]
        );
        assert!(result.log_messages.trace.is_empty());
    }
}
