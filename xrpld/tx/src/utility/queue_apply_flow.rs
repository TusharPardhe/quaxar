//! Next `TxQ::apply(...)` composition layer.
//!
//! This stitches the landed `preclaim(...)`, clear-ahead, and normal queue
//! path seams into one auditable result carrier.

use std::fmt::Display;

use basics::base_uint::Uint256;
use protocol::SeqProxy;

use crate::{
    ApplyFlags, ApplyResult, FeeLevel64, FeeQueueKey, MaybeTx, OrderCandidates, PreclaimResult,
    PreflightResult, QueueApplyFullQueueDecision, QueueApplyHoldFallback, QueueApplyPreclaimStage,
    QueueApplyPreclaimViewSource, QueueApplyPreparedHoldInputs, QueueApplyQueueLogMessages,
    QueueApplyQueueStage, QueueApplyQueueStageWithLogMessagesResult, QueueApplyTryClearGate,
    QueueApplyTryClearResult, QueueApplyTryClearStage, QueueViews, TxConsequences,
    evaluate_queue_apply_try_clear_gate, run_queue_apply_preclaim_stage,
    run_queue_apply_queue_stage_with_caller_hold, run_queue_apply_queue_stage_with_log_messages,
    run_queue_apply_queue_stage_with_log_sinks, run_queue_apply_try_clear_stage,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueApplyPreparedPreclaimInputs<Account> {
    pub view_source: QueueApplyPreclaimViewSource,
    pub fee_level_paid: FeeLevel64,
    pub base_level: FeeLevel64,
    pub required_fee_level: FeeLevel64,
    pub open_ledger_tx_count: usize,
    pub tx_id: Uint256,
    pub account: Account,
}

impl<Account> QueueApplyPreparedPreclaimInputs<Account> {
    pub fn new(
        view_source: QueueApplyPreclaimViewSource,
        fee_level_paid: FeeLevel64,
        base_level: FeeLevel64,
        required_fee_level: FeeLevel64,
        open_ledger_tx_count: usize,
        tx_id: Uint256,
        account: Account,
    ) -> Self {
        Self {
            view_source,
            fee_level_paid,
            base_level,
            required_fee_level,
            open_ledger_tx_count,
            tx_id,
            account,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueApplyPreparedTryClearInputs<Tx, Journal, ParentBatchId> {
    pub preclaim: QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>,
    pub gate: QueueApplyTryClearGate,
}

impl<Tx, Journal, ParentBatchId> QueueApplyPreparedTryClearInputs<Tx, Journal, ParentBatchId> {
    pub fn new(
        preclaim: QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>,
        gate: QueueApplyTryClearGate,
    ) -> Self {
        Self { preclaim, gate }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueApplyPreparedPostPreclaimInputs<'a, Account, Tx, Journal, ParentBatchId> {
    pub prepared_try_clear: QueueApplyPreparedTryClearInputs<Tx, Journal, ParentBatchId>,
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
    QueueApplyPreparedPostPreclaimInputs<'a, Account, Tx, Journal, ParentBatchId>
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        prepared_try_clear: QueueApplyPreparedTryClearInputs<Tx, Journal, ParentBatchId>,
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
            prepared_try_clear,
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
pub struct QueueApplyPreparedQueueInputs<'a, Account, Tx, Journal, ParentBatchId> {
    pub preclaim: QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>,
    pub try_clear: QueueApplyTryClearStage,
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
    QueueApplyPreparedQueueInputs<'a, Account, Tx, Journal, ParentBatchId>
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        preclaim: QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>,
        try_clear: QueueApplyTryClearStage,
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
            preclaim,
            try_clear,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueApplyFlowStageWithLogMessagesResult<Account, Tx, Journal, ParentBatchId> {
    pub stage: QueueApplyFlowStage<Account, Tx, Journal, ParentBatchId>,
    pub queue_log_messages: QueueApplyQueueLogMessages,
}

pub fn run_prepared_queue_apply_queue_stage<Account, Tx, Journal, ParentBatchId>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    prepared_queue: QueueApplyPreparedQueueInputs<'_, Account, Tx, Journal, ParentBatchId>,
) -> QueueApplyQueueStage<Account>
where
    Account: Clone + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
{
    run_prepared_queue_apply_queue_stage_with_caller_hold(
        views,
        prepared_queue,
        |_views, prepared| prepared.hold_fallback,
    )
}

pub fn run_prepared_queue_apply_queue_stage_with_caller_hold<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    RunHoldStage,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    prepared_queue: QueueApplyPreparedQueueInputs<'_, Account, Tx, Journal, ParentBatchId>,
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
    // The resolved preclaim/try-clear carriers stay owned by the higher flow
    // result; the normal queue stage only needs the remaining queue facts.
    let QueueApplyPreparedQueueInputs {
        preclaim: _,
        try_clear: _,
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
    } = prepared_queue;

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
        run_hold_stage,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn derive_queue_apply_prepared_post_preclaim_inputs<'a, Account, Tx, Journal, ParentBatchId>(
    tx_seq_proxy: SeqProxy,
    first_relevant_retries_remaining: Option<i32>,
    fee_level_paid: FeeLevel64,
    base_level: FeeLevel64,
    required_fee_level: FeeLevel64,
    hold_fallback: QueueApplyHoldFallback,
    full_queue_decision: QueueApplyFullQueueDecision<Account>,
    replaced: Option<FeeQueueKey<Account>>,
    account: Account,
    tx_id: Uint256,
    last_valid: Option<u32>,
    flags: ApplyFlags,
    pf_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    order: &'a OrderCandidates,
    preclaim: QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>,
) -> QueueApplyPreparedPostPreclaimInputs<'a, Account, Tx, Journal, ParentBatchId> {
    let gate = evaluate_queue_apply_try_clear_gate(
        tx_seq_proxy,
        first_relevant_retries_remaining,
        preclaim.view_source.has_multi_txn(),
        fee_level_paid,
        required_fee_level,
        base_level,
    );

    QueueApplyPreparedPostPreclaimInputs::new(
        QueueApplyPreparedTryClearInputs::new(preclaim, gate),
        hold_fallback,
        full_queue_decision,
        replaced,
        account,
        tx_id,
        last_valid,
        tx_seq_proxy,
        fee_level_paid,
        flags,
        pf_result,
        order,
    )
}

pub fn run_prepared_queue_apply_post_preclaim_stage<Account, Tx, Journal, ParentBatchId>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    prepared_post_preclaim: QueueApplyPreparedPostPreclaimInputs<
        '_,
        Account,
        Tx,
        Journal,
        ParentBatchId,
    >,
    run_try_clear_stage: impl FnOnce(
        QueueApplyPreparedTryClearInputs<Tx, Journal, ParentBatchId>,
    ) -> QueueApplyTryClearStage,
) -> QueueApplyFlowStage<Account, Tx, Journal, ParentBatchId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
{
    run_prepared_queue_apply_post_preclaim_stage_with_caller_queue(
        views,
        prepared_post_preclaim,
        run_try_clear_stage,
        |views, prepared| run_prepared_queue_apply_queue_stage(views, prepared),
    )
}

pub fn run_prepared_queue_apply_post_preclaim_stage_with_log_sinks<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    RunTryClearStage,
    DebugSink,
    InfoSink,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    prepared_post_preclaim: QueueApplyPreparedPostPreclaimInputs<
        '_,
        Account,
        Tx,
        Journal,
        ParentBatchId,
    >,
    run_try_clear_stage: RunTryClearStage,
    mut debug: DebugSink,
    mut info: InfoSink,
) -> QueueApplyFlowStage<Account, Tx, Journal, ParentBatchId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    RunTryClearStage: FnOnce(
        QueueApplyPreparedTryClearInputs<Tx, Journal, ParentBatchId>,
    ) -> QueueApplyTryClearStage,
    DebugSink: FnMut(String),
    InfoSink: FnMut(String),
{
    run_prepared_queue_apply_post_preclaim_stage_with_log_sinks_and_caller_queue(
        views,
        prepared_post_preclaim,
        run_try_clear_stage,
        |views, prepared| {
            run_queue_apply_queue_stage_with_log_sinks(
                views,
                prepared.hold_fallback,
                prepared.full_queue_decision,
                prepared.replaced,
                prepared.account,
                prepared.tx_id,
                prepared.last_valid,
                prepared.seq_proxy,
                prepared.fee_level,
                prepared.flags,
                prepared.pf_result,
                prepared.order,
                |message| debug(message),
                |message| info(message),
            )
        },
    )
}

pub fn run_prepared_queue_apply_post_preclaim_stage_with_log_messages<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    RunTryClearStage,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    prepared_post_preclaim: QueueApplyPreparedPostPreclaimInputs<
        '_,
        Account,
        Tx,
        Journal,
        ParentBatchId,
    >,
    run_try_clear_stage: RunTryClearStage,
) -> QueueApplyFlowStageWithLogMessagesResult<Account, Tx, Journal, ParentBatchId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    RunTryClearStage: FnOnce(
        QueueApplyPreparedTryClearInputs<Tx, Journal, ParentBatchId>,
    ) -> QueueApplyTryClearStage,
{
    run_prepared_queue_apply_post_preclaim_stage_with_log_messages_and_caller_queue(
        views,
        prepared_post_preclaim,
        run_try_clear_stage,
        |views, prepared| {
            run_queue_apply_queue_stage_with_log_messages(
                views,
                prepared.hold_fallback,
                prepared.full_queue_decision,
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

pub fn run_prepared_queue_apply_post_preclaim_stage_with_log_messages_and_caller_queue<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    RunTryClearStage,
    RunQueueStage,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    prepared_post_preclaim: QueueApplyPreparedPostPreclaimInputs<
        '_,
        Account,
        Tx,
        Journal,
        ParentBatchId,
    >,
    run_try_clear_stage: RunTryClearStage,
    run_queue_stage: RunQueueStage,
) -> QueueApplyFlowStageWithLogMessagesResult<Account, Tx, Journal, ParentBatchId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    RunTryClearStage: FnOnce(
        QueueApplyPreparedTryClearInputs<Tx, Journal, ParentBatchId>,
    ) -> QueueApplyTryClearStage,
    RunQueueStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedQueueInputs<'_, Account, Tx, Journal, ParentBatchId>,
    ) -> QueueApplyQueueStageWithLogMessagesResult<Account>,
{
    let QueueApplyPreparedPostPreclaimInputs {
        prepared_try_clear,
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
    } = prepared_post_preclaim;

    let preclaim = prepared_try_clear.preclaim.clone();
    let try_clear = run_try_clear_stage(prepared_try_clear);

    if matches!(try_clear, QueueApplyTryClearStage::ApplySandboxAndReturn(_)) {
        return QueueApplyFlowStageWithLogMessagesResult {
            stage: QueueApplyFlowStage::ReturnTryClear {
                preclaim,
                try_clear,
            },
            queue_log_messages: QueueApplyQueueLogMessages::default(),
        };
    }

    let queue = run_queue_stage(
        views,
        QueueApplyPreparedQueueInputs::new(
            preclaim.clone(),
            try_clear.clone(),
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
        ),
    );

    QueueApplyFlowStageWithLogMessagesResult {
        stage: QueueApplyFlowStage::QueueOutcome {
            preclaim,
            try_clear,
            queue: queue.stage,
        },
        queue_log_messages: queue.log_messages,
    }
}

pub fn run_prepared_queue_apply_post_preclaim_stage_with_log_sinks_and_caller_queue<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    RunTryClearStage,
    RunQueueStage,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    prepared_post_preclaim: QueueApplyPreparedPostPreclaimInputs<
        '_,
        Account,
        Tx,
        Journal,
        ParentBatchId,
    >,
    run_try_clear_stage: RunTryClearStage,
    run_queue_stage: RunQueueStage,
) -> QueueApplyFlowStage<Account, Tx, Journal, ParentBatchId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    RunTryClearStage: FnOnce(
        QueueApplyPreparedTryClearInputs<Tx, Journal, ParentBatchId>,
    ) -> QueueApplyTryClearStage,
    RunQueueStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedQueueInputs<'_, Account, Tx, Journal, ParentBatchId>,
    ) -> QueueApplyQueueStage<Account>,
{
    run_prepared_queue_apply_post_preclaim_stage_with_caller_queue(
        views,
        prepared_post_preclaim,
        run_try_clear_stage,
        run_queue_stage,
    )
}

pub fn run_prepared_queue_apply_post_preclaim_stage_with_caller_queue<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    RunTryClearStage,
    RunQueueStage,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    prepared_post_preclaim: QueueApplyPreparedPostPreclaimInputs<
        '_,
        Account,
        Tx,
        Journal,
        ParentBatchId,
    >,
    run_try_clear_stage: RunTryClearStage,
    run_queue_stage: RunQueueStage,
) -> QueueApplyFlowStage<Account, Tx, Journal, ParentBatchId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    RunTryClearStage: FnOnce(
        QueueApplyPreparedTryClearInputs<Tx, Journal, ParentBatchId>,
    ) -> QueueApplyTryClearStage,
    RunQueueStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedQueueInputs<'_, Account, Tx, Journal, ParentBatchId>,
    ) -> QueueApplyQueueStage<Account>,
{
    let QueueApplyPreparedPostPreclaimInputs {
        prepared_try_clear,
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
    } = prepared_post_preclaim;

    let preclaim = prepared_try_clear.preclaim.clone();
    let try_clear = run_try_clear_stage(prepared_try_clear);

    if matches!(try_clear, QueueApplyTryClearStage::ApplySandboxAndReturn(_)) {
        return QueueApplyFlowStage::ReturnTryClear {
            preclaim,
            try_clear,
        };
    }

    let queue = run_queue_stage(
        views,
        QueueApplyPreparedQueueInputs::new(
            preclaim.clone(),
            try_clear.clone(),
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
        ),
    );

    QueueApplyFlowStage::QueueOutcome {
        preclaim,
        try_clear,
        queue,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueApplyFlowStage<Account, Tx, Journal, ParentBatchId> {
    RejectPreclaim(ApplyResult),
    ReturnTryClear {
        preclaim: QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>,
        try_clear: QueueApplyTryClearStage,
    },
    QueueOutcome {
        preclaim: QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>,
        try_clear: QueueApplyTryClearStage,
        queue: QueueApplyQueueStage<Account>,
    },
}

impl<Account, Tx, Journal, ParentBatchId> QueueApplyFlowStage<Account, Tx, Journal, ParentBatchId> {
    pub fn apply_result(&self) -> ApplyResult {
        match self {
            Self::RejectPreclaim(result) => result.clone(),
            Self::ReturnTryClear { try_clear, .. } => match try_clear {
                QueueApplyTryClearStage::ApplySandboxAndReturn(result) => result.clone(),
                QueueApplyTryClearStage::Bypass | QueueApplyTryClearStage::ContinueAfterAttempt => {
                    unreachable!("xrpl::TxQ::apply : applied try-clear result")
                }
            },
            Self::QueueOutcome { queue, .. } => queue.apply_result(),
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_flow_stage<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    RunPreclaim,
    RunTryClear,
    TryClearResult,
    ApplySandbox,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    preclaim_view_source: QueueApplyPreclaimViewSource,
    tx_seq_proxy: SeqProxy,
    first_relevant_retries_remaining: Option<i32>,
    fee_level_paid: FeeLevel64,
    base_level: FeeLevel64,
    required_fee_level: FeeLevel64,
    open_ledger_tx_count: usize,
    hold_fallback: QueueApplyHoldFallback,
    full_queue_decision: QueueApplyFullQueueDecision<Account>,
    replaced: Option<FeeQueueKey<Account>>,
    account: Account,
    tx_id: Uint256,
    last_valid: Option<u32>,
    flags: ApplyFlags,
    pf_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    order: &OrderCandidates,
    run_preclaim: RunPreclaim,
    run_try_clear: RunTryClear,
    apply_sandbox: ApplySandbox,
) -> QueueApplyFlowStage<Account, Tx, Journal, ParentBatchId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    RunPreclaim:
        FnOnce(crate::QueueApplyPreclaimViewSource) -> PreclaimResult<Tx, Journal, ParentBatchId>,
    RunTryClear: FnOnce() -> TryClearResult,
    TryClearResult: QueueApplyTryClearResult,
    ApplySandbox: FnOnce(),
{
    run_queue_apply_flow_stage_with_caller_preclaim(
        views,
        tx_seq_proxy,
        first_relevant_retries_remaining,
        hold_fallback,
        full_queue_decision,
        replaced,
        last_valid,
        flags,
        pf_result,
        order,
        QueueApplyPreparedPreclaimInputs::new(
            preclaim_view_source,
            fee_level_paid,
            base_level,
            required_fee_level,
            open_ledger_tx_count,
            tx_id,
            account,
        ),
        |prepared| {
            run_queue_apply_preclaim_stage(
                prepared.view_source,
                prepared.fee_level_paid,
                prepared.base_level,
                prepared.required_fee_level,
                prepared.open_ledger_tx_count,
                prepared.tx_id,
                prepared.account,
                run_preclaim,
            )
        },
        run_try_clear,
        apply_sandbox,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_flow_stage_with_log_sinks<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    RunPreclaim,
    RunTryClear,
    TryClearResult,
    ApplySandbox,
    DebugSink,
    InfoSink,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    preclaim_view_source: QueueApplyPreclaimViewSource,
    tx_seq_proxy: SeqProxy,
    first_relevant_retries_remaining: Option<i32>,
    fee_level_paid: FeeLevel64,
    base_level: FeeLevel64,
    required_fee_level: FeeLevel64,
    open_ledger_tx_count: usize,
    hold_fallback: QueueApplyHoldFallback,
    full_queue_decision: QueueApplyFullQueueDecision<Account>,
    replaced: Option<FeeQueueKey<Account>>,
    account: Account,
    tx_id: Uint256,
    last_valid: Option<u32>,
    flags: ApplyFlags,
    pf_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    order: &OrderCandidates,
    run_preclaim: RunPreclaim,
    run_try_clear: RunTryClear,
    apply_sandbox: ApplySandbox,
    debug: DebugSink,
    info: InfoSink,
) -> QueueApplyFlowStage<Account, Tx, Journal, ParentBatchId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    RunPreclaim:
        FnOnce(crate::QueueApplyPreclaimViewSource) -> PreclaimResult<Tx, Journal, ParentBatchId>,
    RunTryClear: FnOnce() -> TryClearResult,
    TryClearResult: QueueApplyTryClearResult,
    ApplySandbox: FnOnce(),
    DebugSink: FnMut(String),
    InfoSink: FnMut(String),
{
    let preclaim = match run_queue_apply_preclaim_stage(
        preclaim_view_source,
        fee_level_paid,
        base_level,
        required_fee_level,
        open_ledger_tx_count,
        tx_id,
        account.clone(),
        run_preclaim,
    ) {
        Ok(stage) => stage,
        Err(result) => return QueueApplyFlowStage::RejectPreclaim(result),
    };

    run_prepared_queue_apply_post_preclaim_stage_with_log_sinks(
        views,
        derive_queue_apply_prepared_post_preclaim_inputs(
            tx_seq_proxy,
            first_relevant_retries_remaining,
            fee_level_paid,
            base_level,
            required_fee_level,
            hold_fallback,
            full_queue_decision,
            replaced,
            account,
            tx_id,
            last_valid,
            flags,
            pf_result,
            order,
            preclaim,
        ),
        |prepared| run_queue_apply_try_clear_stage(prepared.gate, run_try_clear, apply_sandbox),
        debug,
        info,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_flow_stage_with_log_messages<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    RunPreclaim,
    RunTryClear,
    TryClearResult,
    ApplySandbox,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    preclaim_view_source: QueueApplyPreclaimViewSource,
    tx_seq_proxy: SeqProxy,
    first_relevant_retries_remaining: Option<i32>,
    fee_level_paid: FeeLevel64,
    base_level: FeeLevel64,
    required_fee_level: FeeLevel64,
    open_ledger_tx_count: usize,
    hold_fallback: QueueApplyHoldFallback,
    full_queue_decision: QueueApplyFullQueueDecision<Account>,
    replaced: Option<FeeQueueKey<Account>>,
    account: Account,
    tx_id: Uint256,
    last_valid: Option<u32>,
    flags: ApplyFlags,
    pf_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    order: &OrderCandidates,
    run_preclaim: RunPreclaim,
    run_try_clear: RunTryClear,
    apply_sandbox: ApplySandbox,
) -> QueueApplyFlowStageWithLogMessagesResult<Account, Tx, Journal, ParentBatchId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    RunPreclaim:
        FnOnce(crate::QueueApplyPreclaimViewSource) -> PreclaimResult<Tx, Journal, ParentBatchId>,
    RunTryClear: FnOnce() -> TryClearResult,
    TryClearResult: QueueApplyTryClearResult,
    ApplySandbox: FnOnce(),
{
    let preclaim = match run_queue_apply_preclaim_stage(
        preclaim_view_source,
        fee_level_paid,
        base_level,
        required_fee_level,
        open_ledger_tx_count,
        tx_id,
        account.clone(),
        run_preclaim,
    ) {
        Ok(stage) => stage,
        Err(result) => {
            return QueueApplyFlowStageWithLogMessagesResult {
                stage: QueueApplyFlowStage::RejectPreclaim(result),
                queue_log_messages: QueueApplyQueueLogMessages::default(),
            };
        }
    };

    run_prepared_queue_apply_post_preclaim_stage_with_log_messages(
        views,
        derive_queue_apply_prepared_post_preclaim_inputs(
            tx_seq_proxy,
            first_relevant_retries_remaining,
            fee_level_paid,
            base_level,
            required_fee_level,
            hold_fallback,
            full_queue_decision,
            replaced,
            account,
            tx_id,
            last_valid,
            flags,
            pf_result,
            order,
            preclaim,
        ),
        |prepared| run_queue_apply_try_clear_stage(prepared.gate, run_try_clear, apply_sandbox),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_flow_stage_with_caller_preclaim<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    RunPreclaimStage,
    RunTryClear,
    TryClearResult,
    ApplySandbox,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    tx_seq_proxy: SeqProxy,
    first_relevant_retries_remaining: Option<i32>,
    hold_fallback: QueueApplyHoldFallback,
    full_queue_decision: QueueApplyFullQueueDecision<Account>,
    replaced: Option<FeeQueueKey<Account>>,
    last_valid: Option<u32>,
    flags: ApplyFlags,
    pf_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    order: &OrderCandidates,
    prepared_preclaim: QueueApplyPreparedPreclaimInputs<Account>,
    run_preclaim_stage: RunPreclaimStage,
    run_try_clear: RunTryClear,
    apply_sandbox: ApplySandbox,
) -> QueueApplyFlowStage<Account, Tx, Journal, ParentBatchId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    RunPreclaimStage:
        FnOnce(
            QueueApplyPreparedPreclaimInputs<Account>,
        ) -> Result<QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>, ApplyResult>,
    RunTryClear: FnOnce() -> TryClearResult,
    TryClearResult: QueueApplyTryClearResult,
    ApplySandbox: FnOnce(),
{
    let QueueApplyPreparedPreclaimInputs {
        view_source: preclaim_view_source,
        fee_level_paid,
        base_level,
        required_fee_level,
        open_ledger_tx_count,
        tx_id,
        account,
    } = prepared_preclaim;

    let preclaim = match run_preclaim_stage(QueueApplyPreparedPreclaimInputs::new(
        preclaim_view_source,
        fee_level_paid,
        base_level,
        required_fee_level,
        open_ledger_tx_count,
        tx_id.clone(),
        account.clone(),
    )) {
        Ok(stage) => stage,
        Err(result) => return QueueApplyFlowStage::RejectPreclaim(result),
    };

    run_prepared_queue_apply_post_preclaim_stage(
        views,
        derive_queue_apply_prepared_post_preclaim_inputs(
            tx_seq_proxy,
            first_relevant_retries_remaining,
            fee_level_paid,
            base_level,
            required_fee_level,
            hold_fallback,
            full_queue_decision,
            replaced,
            account,
            tx_id,
            last_valid,
            flags,
            pf_result,
            order,
            preclaim,
        ),
        |prepared| run_queue_apply_try_clear_stage(prepared.gate, run_try_clear, apply_sandbox),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_flow_stage_with_caller_preclaim_and_caller_try_clear<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    RunTryClearStage,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    tx_seq_proxy: SeqProxy,
    first_relevant_retries_remaining: Option<i32>,
    fee_level_paid: FeeLevel64,
    base_level: FeeLevel64,
    required_fee_level: FeeLevel64,
    hold_fallback: QueueApplyHoldFallback,
    full_queue_decision: QueueApplyFullQueueDecision<Account>,
    replaced: Option<FeeQueueKey<Account>>,
    account: Account,
    tx_id: Uint256,
    last_valid: Option<u32>,
    flags: ApplyFlags,
    pf_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    order: &OrderCandidates,
    preclaim: QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>,
    run_try_clear_stage: RunTryClearStage,
) -> QueueApplyFlowStage<Account, Tx, Journal, ParentBatchId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    RunTryClearStage: FnOnce(
        QueueApplyPreparedTryClearInputs<Tx, Journal, ParentBatchId>,
    ) -> QueueApplyTryClearStage,
{
    run_prepared_queue_apply_post_preclaim_stage(
        views,
        derive_queue_apply_prepared_post_preclaim_inputs(
            tx_seq_proxy,
            first_relevant_retries_remaining,
            fee_level_paid,
            base_level,
            required_fee_level,
            hold_fallback,
            full_queue_decision,
            replaced,
            account,
            tx_id,
            last_valid,
            flags,
            pf_result,
            order,
            preclaim,
        ),
        run_try_clear_stage,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_flow_stage_with_caller_preclaim_and_caller_try_clear_and_caller_queue<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    RunQueueStage,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    tx_seq_proxy: SeqProxy,
    fee_level_paid: FeeLevel64,
    hold_fallback: QueueApplyHoldFallback,
    full_queue_decision: QueueApplyFullQueueDecision<Account>,
    replaced: Option<FeeQueueKey<Account>>,
    account: Account,
    tx_id: Uint256,
    last_valid: Option<u32>,
    flags: ApplyFlags,
    pf_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    order: &OrderCandidates,
    preclaim: QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>,
    try_clear: QueueApplyTryClearStage,
    run_queue_stage: RunQueueStage,
) -> QueueApplyFlowStage<Account, Tx, Journal, ParentBatchId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    RunQueueStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedQueueInputs<'_, Account, Tx, Journal, ParentBatchId>,
    ) -> QueueApplyQueueStage<Account>,
{
    let queue = run_queue_stage(
        views,
        QueueApplyPreparedQueueInputs::new(
            preclaim.clone(),
            try_clear.clone(),
            hold_fallback,
            full_queue_decision,
            replaced,
            account,
            tx_id,
            last_valid,
            tx_seq_proxy,
            fee_level_paid,
            flags,
            pf_result,
            order,
        ),
    );

    QueueApplyFlowStage::QueueOutcome {
        preclaim,
        try_clear,
        queue,
    }
}

#[allow(clippy::too_many_arguments)]
pub fn run_queue_apply_flow_stage_with_log_messages_and_caller_preclaim_and_caller_try_clear_and_caller_queue<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    RunQueueStage,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    tx_seq_proxy: SeqProxy,
    fee_level_paid: FeeLevel64,
    hold_fallback: QueueApplyHoldFallback,
    full_queue_decision: QueueApplyFullQueueDecision<Account>,
    replaced: Option<FeeQueueKey<Account>>,
    account: Account,
    tx_id: Uint256,
    last_valid: Option<u32>,
    flags: ApplyFlags,
    pf_result: PreflightResult<Tx, TxConsequences, Journal, ParentBatchId>,
    order: &OrderCandidates,
    preclaim: QueueApplyPreclaimStage<Tx, Journal, ParentBatchId>,
    try_clear: QueueApplyTryClearStage,
    run_queue_stage: RunQueueStage,
) -> QueueApplyFlowStageWithLogMessagesResult<Account, Tx, Journal, ParentBatchId>
where
    Account: Clone + Display + Ord + PartialEq,
    Tx: Clone,
    Journal: Clone,
    ParentBatchId: Clone,
    RunQueueStage: FnOnce(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        QueueApplyPreparedQueueInputs<'_, Account, Tx, Journal, ParentBatchId>,
    ) -> QueueApplyQueueStageWithLogMessagesResult<Account>,
{
    let queue = run_queue_stage(
        views,
        QueueApplyPreparedQueueInputs::new(
            preclaim.clone(),
            try_clear.clone(),
            hold_fallback,
            full_queue_decision,
            replaced,
            account,
            tx_id,
            last_valid,
            tx_seq_proxy,
            fee_level_paid,
            flags,
            pf_result,
            order,
        ),
    );

    QueueApplyFlowStageWithLogMessagesResult {
        stage: QueueApplyFlowStage::QueueOutcome {
            preclaim,
            try_clear,
            queue: queue.stage,
        },
        queue_log_messages: queue.log_messages,
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, collections::BTreeMap};

    use basics::base_uint::Uint256;
    use protocol::{Rules, SeqProxy, Ter};

    use super::{
        QueueApplyFlowStage, QueueApplyPreparedPreclaimInputs, QueueApplyPreparedTryClearInputs,
        run_prepared_queue_apply_queue_stage_with_caller_hold, run_queue_apply_flow_stage,
        run_queue_apply_flow_stage_with_caller_preclaim,
        run_queue_apply_flow_stage_with_caller_preclaim_and_caller_try_clear,
        run_queue_apply_flow_stage_with_caller_preclaim_and_caller_try_clear_and_caller_queue,
    };
    use crate::{
        ApplyFlags, ApplyResult, FeeQueueEntry, FeeQueueKey, MaybeTx, MaybeTxCore, OrderCandidates,
        PreclaimResult, PreflightResult, QueueAdvanceCandidate, QueueApplyEnqueueResult,
        QueueApplyFullQueueDecision, QueueApplyHoldFallback, QueueApplyPreclaimStage,
        QueueApplyPreclaimViewSource, QueueApplyQueueStage, QueueApplyTryClearGate,
        QueueApplyTryClearStage, QueueViews, TxConsequences, TxQAccount,
    };

    fn make_preclaim(
        likely_to_claim_fee: bool,
        ter: Ter,
    ) -> PreclaimResult<&'static str, &'static str, &'static str> {
        let mut result =
            PreclaimResult::new(7, "tx", None::<&str>, ApplyFlags::NONE, "journal", ter);
        result.likely_to_claim_fee = likely_to_claim_fee;
        result
    }

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
    fn flow_stage_returns_preclaim_rejection_before_try_clear_or_queue() {
        let ran_try_clear = Cell::new(false);
        let applied_sandbox = Cell::new(false);
        let mut views = QueueViews::new(BTreeMap::new(), vec![]);

        let stage = run_queue_apply_flow_stage(
            &mut views,
            crate::QueueApplyPreclaimViewSource::CurrentView,
            SeqProxy::sequence(6),
            None,
            110,
            100,
            105,
            4,
            QueueApplyHoldFallback::HoldAllowed,
            QueueApplyFullQueueDecision::Bypass,
            None,
            "acct",
            Uint256::from_u64(9),
            Some(250),
            ApplyFlags::NONE,
            make_preflight("tx", SeqProxy::sequence(6), ApplyFlags::NONE),
            &OrderCandidates::new(Uint256::from_u64(0)),
            |_| make_preclaim(false, Ter::TER_RETRY),
            || {
                ran_try_clear.set(true);
                ApplyResult::new(Ter::TES_SUCCESS, true, false)
            },
            || {
                applied_sandbox.set(true);
            },
        );

        assert_eq!(
            stage,
            QueueApplyFlowStage::RejectPreclaim(ApplyResult::new(Ter::TER_RETRY, false, false,))
        );
        assert_eq!(
            stage.apply_result(),
            ApplyResult::new(Ter::TER_RETRY, false, false)
        );
        assert!(!ran_try_clear.get());
        assert!(!applied_sandbox.get());
        assert!(views.accounts.is_empty());
    }

    #[test]
    fn flow_stage_returns_applied_try_clear_before_queue_stage() {
        let ran_try_clear = Cell::new(false);
        let applied_sandbox = Cell::new(false);
        let mut views = QueueViews::new(BTreeMap::new(), vec![]);

        let stage = run_queue_apply_flow_stage(
            &mut views,
            crate::QueueApplyPreclaimViewSource::MultiTxnOpenView,
            SeqProxy::sequence(6),
            Some(crate::MAYBE_TX_RETRIES_ALLOWED),
            110,
            100,
            105,
            4,
            QueueApplyHoldFallback::AlreadyVerifiedByMultiTxn,
            QueueApplyFullQueueDecision::Bypass,
            None,
            "acct",
            Uint256::from_u64(9),
            Some(250),
            ApplyFlags::NONE,
            make_preflight("tx", SeqProxy::sequence(6), ApplyFlags::NONE),
            &OrderCandidates::new(Uint256::from_u64(0)),
            |_| make_preclaim(true, Ter::TES_SUCCESS),
            || {
                ran_try_clear.set(true);
                ApplyResult::new(Ter::TES_SUCCESS, true, false)
            },
            || {
                applied_sandbox.set(true);
            },
        );

        assert!(matches!(
            stage,
            QueueApplyFlowStage::ReturnTryClear {
                try_clear: QueueApplyTryClearStage::ApplySandboxAndReturn(_),
                ..
            }
        ));
        assert_eq!(
            stage.apply_result(),
            ApplyResult::new(Ter::TES_SUCCESS, true, false)
        );
        assert!(ran_try_clear.get());
        assert!(applied_sandbox.get());
        assert!(views.accounts.is_empty());
    }

    #[test]
    fn flow_stage_continues_to_queue_after_unsuccessful_try_clear_attempt() {
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

        let stage = run_queue_apply_flow_stage(
            &mut views,
            crate::QueueApplyPreclaimViewSource::MultiTxnOpenView,
            SeqProxy::sequence(6),
            Some(crate::MAYBE_TX_RETRIES_ALLOWED),
            110,
            100,
            105,
            4,
            QueueApplyHoldFallback::AlreadyVerifiedByMultiTxn,
            QueueApplyFullQueueDecision::Bypass,
            None,
            "c",
            Uint256::from_u64(9),
            Some(250),
            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
            make_preflight(
                "tx",
                SeqProxy::sequence(6),
                ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
            ),
            &OrderCandidates::new(Uint256::from_u64(0)),
            |_| make_preclaim(true, Ter::TES_SUCCESS),
            || ApplyResult::new(Ter::TER_RETRY, false, false),
            || unreachable!("sandbox should not apply"),
        );

        assert!(matches!(
            stage,
            QueueApplyFlowStage::QueueOutcome {
                try_clear: QueueApplyTryClearStage::ContinueAfterAttempt,
                queue: QueueApplyQueueStage::Queued(QueueApplyEnqueueResult {
                    queued: _,
                    removed_replacement: None,
                    account_created: true,
                    stored_flags: ApplyFlags::FAIL_HARD,
                }),
                ..
            }
        ));
        assert_eq!(
            stage.apply_result(),
            ApplyResult::new(Ter::TER_QUEUED, false, false)
        );
        assert!(views.accounts.contains_key("c"));
    }

    #[test]
    fn flow_stage_with_caller_preclaim_exposes_prepared_preclaim_boundary() {
        let ran_try_clear = Cell::new(false);
        let applied_sandbox = Cell::new(false);
        let mut views = QueueViews::new(BTreeMap::new(), vec![]);

        let stage = run_queue_apply_flow_stage_with_caller_preclaim(
            &mut views,
            SeqProxy::sequence(6),
            Some(crate::MAYBE_TX_RETRIES_ALLOWED),
            QueueApplyHoldFallback::AlreadyVerifiedByMultiTxn,
            QueueApplyFullQueueDecision::Bypass,
            None,
            Some(250),
            ApplyFlags::RETRY,
            make_preflight("tx", SeqProxy::sequence(6), ApplyFlags::RETRY),
            &OrderCandidates::new(Uint256::from_u64(0)),
            QueueApplyPreparedPreclaimInputs::new(
                QueueApplyPreclaimViewSource::MultiTxnOpenView,
                110,
                50,
                100,
                4,
                Uint256::from_u64(9),
                "acct",
            ),
            |prepared| {
                assert_eq!(
                    prepared,
                    QueueApplyPreparedPreclaimInputs::new(
                        QueueApplyPreclaimViewSource::MultiTxnOpenView,
                        110,
                        50,
                        100,
                        4,
                        Uint256::from_u64(9),
                        "acct",
                    )
                );

                Ok(QueueApplyPreclaimStage {
                    view_source: prepared.view_source,
                    trace_message: "trace".to_string(),
                    preclaim_result: make_preclaim(true, Ter::TES_SUCCESS),
                })
            },
            || {
                ran_try_clear.set(true);
                ApplyResult::new(Ter::TES_SUCCESS, true, false)
            },
            || {
                applied_sandbox.set(true);
            },
        );

        assert!(matches!(
            stage,
            QueueApplyFlowStage::ReturnTryClear {
                preclaim: QueueApplyPreclaimStage {
                    view_source: QueueApplyPreclaimViewSource::MultiTxnOpenView,
                    ..
                },
                try_clear: QueueApplyTryClearStage::ApplySandboxAndReturn(_),
            }
        ));
        assert_eq!(
            stage.apply_result(),
            ApplyResult::new(Ter::TES_SUCCESS, true, false)
        );
        assert!(ran_try_clear.get());
        assert!(applied_sandbox.get());
        assert!(views.accounts.is_empty());
    }

    #[test]
    fn flow_stage_with_caller_try_clear_exposes_prepared_try_clear_boundary() {
        let mut views = QueueViews::new(BTreeMap::new(), vec![]);
        let preclaim = QueueApplyPreclaimStage {
            view_source: QueueApplyPreclaimViewSource::MultiTxnOpenView,
            trace_message: "trace".to_string(),
            preclaim_result: make_preclaim(true, Ter::TES_SUCCESS),
        };

        let stage = run_queue_apply_flow_stage_with_caller_preclaim_and_caller_try_clear(
            &mut views,
            SeqProxy::sequence(6),
            Some(crate::MAYBE_TX_RETRIES_ALLOWED),
            110,
            50,
            100,
            QueueApplyHoldFallback::AlreadyVerifiedByMultiTxn,
            QueueApplyFullQueueDecision::Bypass,
            None,
            "acct",
            Uint256::from_u64(9),
            Some(250),
            ApplyFlags::RETRY,
            make_preflight("tx", SeqProxy::sequence(6), ApplyFlags::RETRY),
            &OrderCandidates::new(Uint256::from_u64(0)),
            preclaim.clone(),
            |prepared| {
                assert_eq!(
                    prepared,
                    QueueApplyPreparedTryClearInputs::new(
                        preclaim,
                        QueueApplyTryClearGate::AttemptClearAhead,
                    )
                );

                QueueApplyTryClearStage::ApplySandboxAndReturn(ApplyResult::new(
                    Ter::TES_SUCCESS,
                    true,
                    false,
                ))
            },
        );

        assert!(matches!(
            stage,
            QueueApplyFlowStage::ReturnTryClear {
                preclaim: QueueApplyPreclaimStage {
                    view_source: QueueApplyPreclaimViewSource::MultiTxnOpenView,
                    ..
                },
                try_clear: QueueApplyTryClearStage::ApplySandboxAndReturn(_),
            }
        ));
        assert_eq!(
            stage.apply_result(),
            ApplyResult::new(Ter::TES_SUCCESS, true, false)
        );
        assert!(views.accounts.is_empty());
    }

    #[test]
    fn flow_stage_with_caller_queue_exposes_prepared_queue_boundary() {
        let mut views = QueueViews::new(BTreeMap::new(), vec![]);
        let preclaim = QueueApplyPreclaimStage {
            view_source: QueueApplyPreclaimViewSource::MultiTxnOpenView,
            trace_message: "trace".to_string(),
            preclaim_result: make_preclaim(true, Ter::TES_SUCCESS),
        };
        let try_clear = QueueApplyTryClearStage::ContinueAfterAttempt;
        let preflight = make_preflight("tx", SeqProxy::sequence(6), ApplyFlags::RETRY);
        let order = OrderCandidates::new(Uint256::from_u64(0));

        let stage =
            run_queue_apply_flow_stage_with_caller_preclaim_and_caller_try_clear_and_caller_queue(
                &mut views,
                SeqProxy::sequence(6),
                110,
                QueueApplyHoldFallback::AlreadyVerifiedByMultiTxn,
                QueueApplyFullQueueDecision::Bypass,
                None,
                "acct",
                Uint256::from_u64(9),
                Some(250),
                ApplyFlags::RETRY,
                preflight.clone(),
                &order,
                preclaim.clone(),
                try_clear.clone(),
                |_views, prepared| {
                    assert_eq!(prepared.preclaim, preclaim);
                    assert_eq!(prepared.try_clear, try_clear);
                    assert_eq!(
                        prepared.hold_fallback,
                        QueueApplyHoldFallback::AlreadyVerifiedByMultiTxn
                    );
                    assert_eq!(
                        prepared.full_queue_decision,
                        QueueApplyFullQueueDecision::Bypass
                    );
                    assert_eq!(prepared.replaced, None);
                    assert_eq!(prepared.account, "acct");
                    assert_eq!(prepared.tx_id, Uint256::from_u64(9));
                    assert_eq!(prepared.last_valid, Some(250));
                    assert_eq!(prepared.seq_proxy, SeqProxy::sequence(6));
                    assert_eq!(prepared.fee_level, 110);
                    assert_eq!(prepared.flags, ApplyFlags::RETRY);
                    assert_eq!(prepared.pf_result, preflight);
                    assert!(std::ptr::eq(prepared.order, &order));

                    QueueApplyQueueStage::RejectFull
                },
            );

        assert!(matches!(
            stage,
            QueueApplyFlowStage::QueueOutcome {
                preclaim: QueueApplyPreclaimStage {
                    view_source: QueueApplyPreclaimViewSource::MultiTxnOpenView,
                    ..
                },
                try_clear: QueueApplyTryClearStage::ContinueAfterAttempt,
                queue: QueueApplyQueueStage::RejectFull,
            }
        ));
        assert_eq!(
            stage.apply_result(),
            ApplyResult::new(Ter::TEL_CAN_NOT_QUEUE_FULL, false, false)
        );
        assert!(views.accounts.is_empty());
    }

    #[test]
    fn prepared_queue_runner_with_caller_hold_exposes_remaining_normal_queue_handoff() {
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
        let preclaim = QueueApplyPreclaimStage {
            view_source: QueueApplyPreclaimViewSource::MultiTxnOpenView,
            trace_message: "trace".to_string(),
            preclaim_result: make_preclaim(true, Ter::TES_SUCCESS),
        };
        let try_clear = QueueApplyTryClearStage::ContinueAfterAttempt;
        let preflight = make_preflight("tx", SeqProxy::sequence(6), ApplyFlags::RETRY);
        let order = OrderCandidates::new(Uint256::from_u64(0));

        let stage = run_prepared_queue_apply_queue_stage_with_caller_hold(
            &mut views,
            super::QueueApplyPreparedQueueInputs::new(
                preclaim,
                try_clear,
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
            ),
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
}
