//! Higher Rust migration seam for the owner loop inside `TxQ::accept(...)`.
//!
//! This layer now joins the already-landed:
//! 1. accept gate (`evaluate_accept_candidate(...)`),
//! 2. failed-candidate policy (`decide_failed_candidate(...)`),
//! 3. synchronized queue/account view mutations (`QueueViews`),
//! 4. required-fee calculation over a fixed fee-metrics snapshot,
//! 5. the post-loop parent-hash warning/rebuild step for fee-order tie breaks,
//! 6. a higher accept-call shell that can internalize the current open-ledger
//!    fee progression and near-full queue check from explicit call-state inputs.

use std::cell::Cell;
use std::fmt::Display;

use basics::base_uint::Uint256;
use protocol::{SeqProxy, Ter, trans_token};

use crate::{
    AcceptCandidateGate, ApplyFlags, ApplyResult, FailedCandidateAction, FeeLevel64, FeeQueueEntry,
    FeeQueueKey, MaybeTx, OrderCandidates, PenaltyUpdate, QueueAdvanceCandidate,
    QueueFeeMetricsSnapshot, QueueFeeMetricsState, QueueViewNext, QueueViews,
    decide_failed_candidate, evaluate_accept_candidate, evaluate_required_fee_level,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueAcceptCandidate<Account> {
    pub key: FeeQueueKey<Account>,
    pub tx_id: Uint256,
    pub fee_level: FeeLevel64,
    pub retries_remaining: i32,
    pub flags: ApplyFlags,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueAcceptIteration<Account> {
    SkipNotFirst {
        candidate: QueueAcceptCandidate<Account>,
        next_candidate: Option<FeeQueueKey<Account>>,
    },
    StopInsufficientFee {
        candidate: QueueAcceptCandidate<Account>,
        required_fee_level: FeeLevel64,
    },
    Applied {
        candidate: QueueAcceptCandidate<Account>,
        txn_result: Ter,
        next_candidate: Option<FeeQueueKey<Account>>,
    },
    RemovedFailed {
        candidate: QueueAcceptCandidate<Account>,
        txn_result: Ter,
        penalty_update: PenaltyUpdate,
        next_candidate: Option<FeeQueueKey<Account>>,
    },
    DroppedCurrentTicket {
        candidate: QueueAcceptCandidate<Account>,
        txn_result: Ter,
        next_candidate: Option<FeeQueueKey<Account>>,
    },
    RetainedFailed {
        candidate: QueueAcceptCandidate<Account>,
        txn_result: Ter,
        next_retries_remaining: i32,
        dropped_last_from_account: bool,
        removed_tail: Option<FeeQueueKey<Account>>,
        next_candidate: Option<FeeQueueKey<Account>>,
    },
}

impl<Account> QueueAcceptIteration<Account> {
    pub fn ledger_changed(&self) -> bool {
        matches!(self, Self::Applied { .. })
    }

    pub fn next_candidate(&self) -> Option<&FeeQueueKey<Account>> {
        match self {
            Self::SkipNotFirst { next_candidate, .. }
            | Self::Applied { next_candidate, .. }
            | Self::RemovedFailed { next_candidate, .. }
            | Self::DroppedCurrentTicket { next_candidate, .. }
            | Self::RetainedFailed { next_candidate, .. } => next_candidate.as_ref(),
            Self::StopInsufficientFee { .. } => None,
        }
    }

    pub fn stop_candidate(&self) -> Option<&QueueAcceptCandidate<Account>> {
        match self {
            Self::StopInsufficientFee { candidate, .. } => Some(candidate),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedQueueAcceptApply<Account> {
    pub candidate: QueueAcceptCandidate<Account>,
    pub required_fee_level: FeeLevel64,
    pub queue_nearly_full: bool,
    pub candidate_index: usize,
    pub account_retry_penalty: bool,
    pub account_drop_penalty: bool,
    pub account_txn_count: usize,
    pub order: OrderCandidates,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueAcceptPreparedIteration<Account> {
    Complete(QueueAcceptIteration<Account>),
    Ready(PreparedQueueAcceptApply<Account>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedQueueAcceptCall<Account> {
    pub prepared_apply: PreparedQueueAcceptApply<Account>,
    pub metrics_snapshot: QueueFeeMetricsSnapshot,
    pub call_state: QueueAcceptCallState,
    pub previous_parent_hash_comp: Uint256,
    pub loop_messages: QueueAcceptLoopLogMessages,
    pub ledger_changed: bool,
    pub processed_candidates: usize,
    pub applied_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueAcceptPreparedCallStep<Account> {
    Complete(QueueAcceptWithMetricsResult<Account>),
    Ready(PreparedQueueAcceptCall<Account>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueAcceptStageResult<Account> {
    pub ledger_changed: bool,
    pub processed_candidates: usize,
    pub stop_candidate: Option<FeeQueueKey<Account>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueAcceptRebuildResult {
    pub parent_hash_unchanged: bool,
    pub next_order: OrderCandidates,
    pub starting_size: usize,
    pub rebuilt_size: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueAcceptStageWithRebuildResult<Account> {
    pub stage: QueueAcceptStageResult<Account>,
    pub rebuild: QueueAcceptRebuildResult,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct QueueAcceptOwnerState {
    pub parent_hash_comp: Uint256,
}

impl QueueAcceptOwnerState {
    pub const fn new(parent_hash_comp: Uint256) -> Self {
        Self { parent_hash_comp }
    }

    pub const fn current_order(self) -> OrderCandidates {
        OrderCandidates::new(self.parent_hash_comp)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueAcceptOwnerResult<Account> {
    pub accept: QueueAcceptStageWithRebuildResult<Account>,
    pub owner_state: QueueAcceptOwnerState,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QueueAcceptLoopLogMessages {
    pub trace: Vec<String>,
    pub debug: Vec<String>,
    pub info: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QueueAcceptLogMessages {
    pub loop_messages: QueueAcceptLoopLogMessages,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueAcceptStageWithLogMessagesResult<Account> {
    pub stage: QueueAcceptStageResult<Account>,
    pub loop_messages: QueueAcceptLoopLogMessages,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueAcceptWithMetricsResult<Account> {
    pub owner: QueueAcceptOwnerResult<Account>,
    pub metrics_snapshot: QueueFeeMetricsSnapshot,
    pub log_messages: QueueAcceptLogMessages,
}

impl<Account> QueueAcceptWithMetricsResult<Account> {
    pub const fn ledger_changed(&self) -> bool {
        self.owner.accept.stage.ledger_changed
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueAcceptCallState {
    pub open_ledger_tx_count: usize,
    pub current_max_size: Option<usize>,
    pub next_parent_hash_comp: Uint256,
}

impl QueueAcceptCallState {
    pub const fn new(
        open_ledger_tx_count: usize,
        current_max_size: Option<usize>,
        next_parent_hash_comp: Uint256,
    ) -> Self {
        Self {
            open_ledger_tx_count,
            current_max_size,
            next_parent_hash_comp,
        }
    }
}

pub fn queue_accept_is_nearly_full(
    current_queue_size: usize,
    current_max_size: Option<usize>,
) -> bool {
    current_max_size.is_some_and(|max_size| {
        current_queue_size
            >= max_size
                .saturating_mul(95)
                .checked_div(100)
                .unwrap_or(usize::MAX)
    })
}

fn queue_view_next_to_option<Account>(
    next: QueueViewNext<Account>,
) -> Option<FeeQueueKey<Account>> {
    match next {
        QueueViewNext::End => None,
        QueueViewNext::FeeNext(key) | QueueViewNext::AccountNext(key) => Some(key),
    }
}

fn queue_accept_account_state<Account, Tx, Journal, ParentBatchId>(
    views: &QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    current: &FeeQueueKey<Account>,
) -> (SeqProxy, bool, bool, usize)
where
    Account: Clone + Ord + PartialEq,
{
    let queued_account = views
        .accounts
        .get(&current.account)
        .expect("xrpl::TxQ::accept : account found");
    let front_seq_proxy = queued_account
        .transactions
        .first_key_value()
        .map(|(seq_proxy, _)| *seq_proxy)
        .expect("xrpl::TxQ::accept : account has queued transactions");

    (
        front_seq_proxy,
        queued_account.retry_penalty,
        queued_account.drop_penalty,
        queued_account.transactions.len(),
    )
}

fn snapshot_candidate<Account, Tx, Journal, ParentBatchId>(
    views: &QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    current: &FeeQueueKey<Account>,
) -> QueueAcceptCandidate<Account>
where
    Account: Clone + Ord + PartialEq,
{
    let queued_account = views
        .accounts
        .get(&current.account)
        .expect("xrpl::TxQ::accept : account found");
    let queued = queued_account
        .transactions
        .get(&current.seq_proxy)
        .expect("xrpl::TxQ::accept : candidate found in account");

    QueueAcceptCandidate {
        key: current.clone(),
        tx_id: queued.payload.tx_id,
        fee_level: queued.payload.fee_level,
        retries_remaining: queued.payload.retries_remaining,
        flags: queued.payload.flags,
    }
}

pub fn format_queue_accept_skip_not_first_trace_message<Account, TxId>(
    transaction_id: TxId,
    account: Account,
) -> String
where
    Account: Display,
    TxId: Display,
{
    format!(
        "Skipping queued transaction {transaction_id} from account {account} as it is not the first."
    )
}

pub fn format_queue_accept_fee_trace_message<Account, TxId>(
    transaction_id: TxId,
    account: Account,
    fee_level_paid: FeeLevel64,
    required_fee_level: FeeLevel64,
) -> String
where
    Account: Display,
    TxId: Display,
{
    format!(
        "Queued transaction {transaction_id} from account {account} has fee level of {fee_level_paid} needs at least {required_fee_level}"
    )
}

pub fn format_queue_accept_apply_trace_message<TxId>(transaction_id: TxId) -> String
where
    TxId: Display,
{
    format!("Applying queued transaction {transaction_id} to open ledger.")
}

pub fn format_queue_accept_applied_debug_message<TxId>(
    transaction_id: TxId,
    txn_result: Ter,
) -> String
where
    TxId: Display,
{
    format!(
        "Queued transaction {transaction_id} applied successfully with {}. Remove from queue.",
        trans_token(txn_result)
    )
}

pub fn format_queue_accept_remove_failed_debug_message<TxId>(
    transaction_id: TxId,
    txn_result: Ter,
) -> String
where
    TxId: Display,
{
    format!(
        "Queued transaction {transaction_id} failed with {}. Remove from queue.",
        trans_token(txn_result)
    )
}

pub fn format_queue_accept_leave_in_queue_debug_message<TxId>(
    transaction_id: TxId,
    txn_result: Ter,
    did_apply: bool,
    flags: ApplyFlags,
) -> String
where
    TxId: Display,
{
    format!(
        "Queued transaction {transaction_id} failed with {}. Leave in queue. Applied: {did_apply}. Flags: {flags}",
        trans_token(txn_result)
    )
}

pub fn format_queue_accept_drop_ticket_info_message<Account, TxId>(
    transaction_id: TxId,
    txn_result: Ter,
    account: Account,
) -> String
where
    Account: Display,
    TxId: Display,
{
    format!(
        "Queue is nearly full, and transaction {transaction_id} failed with {}. Removing ticketed tx from account {account}",
        trans_token(txn_result)
    )
}

pub fn format_queue_accept_drop_last_info_message<Account, TxId>(
    transaction_id: TxId,
    txn_result: Ter,
    account: Account,
) -> String
where
    Account: Display,
    TxId: Display,
{
    format!(
        "Queue is nearly full, and transaction {transaction_id} failed with {}. Removing last item from account {account}",
        trans_token(txn_result)
    )
}

fn run_queue_accept_iteration_impl<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    ApplyFn,
    TraceFn,
    DebugFn,
    InfoFn,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    current: &FeeQueueKey<Account>,
    required_fee_level: FeeLevel64,
    queue_nearly_full: bool,
    order: &OrderCandidates,
    apply: ApplyFn,
    mut trace: TraceFn,
    mut debug: DebugFn,
    mut info: InfoFn,
) -> QueueAcceptIteration<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    ApplyFn: FnOnce(&mut MaybeTx<Tx, Account, Journal, ParentBatchId>) -> ApplyResult,
    TraceFn: FnMut(String),
    DebugFn: FnMut(String),
    InfoFn: FnMut(String),
{
    let prepared = prepare_queue_accept_iteration(
        views,
        current,
        required_fee_level,
        queue_nearly_full,
        order,
    );

    match prepared {
        QueueAcceptPreparedIteration::Complete(iteration) => {
            match &iteration {
                QueueAcceptIteration::SkipNotFirst { candidate, .. } => {
                    trace(format_queue_accept_skip_not_first_trace_message(
                        candidate.tx_id,
                        &candidate.key.account,
                    ));
                }
                QueueAcceptIteration::StopInsufficientFee {
                    candidate,
                    required_fee_level,
                } => {
                    trace(format_queue_accept_fee_trace_message(
                        candidate.tx_id,
                        &candidate.key.account,
                        candidate.fee_level,
                        *required_fee_level,
                    ));
                }
                QueueAcceptIteration::Applied { .. }
                | QueueAcceptIteration::RemovedFailed { .. }
                | QueueAcceptIteration::DroppedCurrentTicket { .. }
                | QueueAcceptIteration::RetainedFailed { .. } => {}
            }
            iteration
        }
        QueueAcceptPreparedIteration::Ready(prepared) => {
            trace(format_queue_accept_fee_trace_message(
                prepared.candidate.tx_id,
                &prepared.candidate.key.account,
                prepared.candidate.fee_level,
                prepared.required_fee_level,
            ));
            trace(format_queue_accept_apply_trace_message(
                prepared.candidate.tx_id,
            ));
            run_prepared_queue_accept_apply_impl(
                views,
                prepared,
                apply,
                |message| debug(message),
                |message| info(message),
            )
        }
    }
}

pub fn prepare_queue_accept_iteration<Account, Tx, Journal, ParentBatchId>(
    views: &QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    current: &FeeQueueKey<Account>,
    required_fee_level: FeeLevel64,
    queue_nearly_full: bool,
    order: &OrderCandidates,
) -> QueueAcceptPreparedIteration<Account>
where
    Account: Clone + Display + Ord + PartialEq,
{
    let candidate_index = views
        .find_fee_candidate_index(current)
        .expect("xrpl::TxQ::accept : candidate found in byFee");
    let candidate = snapshot_candidate(views, current);
    let (front_seq_proxy, account_retry_penalty, account_drop_penalty, account_txn_count) =
        queue_accept_account_state(views, current);

    match evaluate_accept_candidate(
        current.seq_proxy,
        front_seq_proxy,
        candidate.fee_level,
        required_fee_level,
    ) {
        AcceptCandidateGate::SkipNotFirst => {
            QueueAcceptPreparedIteration::Complete(QueueAcceptIteration::SkipNotFirst {
                candidate,
                next_candidate: views.next_fee_candidate_key(current),
            })
        }
        AcceptCandidateGate::StopInsufficientFee => {
            QueueAcceptPreparedIteration::Complete(QueueAcceptIteration::StopInsufficientFee {
                candidate,
                required_fee_level,
            })
        }
        AcceptCandidateGate::TryApply => {
            QueueAcceptPreparedIteration::Ready(PreparedQueueAcceptApply {
                candidate,
                required_fee_level,
                queue_nearly_full,
                candidate_index,
                account_retry_penalty,
                account_drop_penalty,
                account_txn_count,
                order: *order,
            })
        }
    }
}

fn run_prepared_queue_accept_apply_impl<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    ApplyFn,
    DebugFn,
    InfoFn,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    prepared: PreparedQueueAcceptApply<Account>,
    apply: ApplyFn,
    mut debug: DebugFn,
    mut info: InfoFn,
) -> QueueAcceptIteration<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    ApplyFn: FnOnce(&mut MaybeTx<Tx, Account, Journal, ParentBatchId>) -> ApplyResult,
    DebugFn: FnMut(String),
    InfoFn: FnMut(String),
{
    let apply_result = {
        let queued = &mut views
            .accounts
            .get_mut(&prepared.candidate.key.account)
            .expect("xrpl::TxQ::accept : account found")
            .transactions
            .get_mut(&prepared.candidate.key.seq_proxy)
            .expect("xrpl::TxQ::accept : candidate found in account")
            .payload;
        apply(queued)
    };

    finalize_prepared_queue_accept_apply_impl(
        views,
        prepared,
        apply_result,
        |m| debug(m),
        |m| info(m),
    )
}

fn finalize_prepared_queue_accept_apply_impl<Account, Tx, Journal, ParentBatchId, DebugFn, InfoFn>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    prepared: PreparedQueueAcceptApply<Account>,
    apply_result: ApplyResult,
    mut debug: DebugFn,
    mut info: InfoFn,
) -> QueueAcceptIteration<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    DebugFn: FnMut(String),
    InfoFn: FnMut(String),
{
    let PreparedQueueAcceptApply {
        candidate,
        required_fee_level: _required_fee_level,
        queue_nearly_full,
        candidate_index,
        account_retry_penalty,
        account_drop_penalty,
        account_txn_count,
        order,
    } = prepared;
    let current = candidate.key.clone();

    if apply_result.applied {
        debug(format_queue_accept_applied_debug_message(
            candidate.tx_id,
            apply_result.ter,
        ));
        let advance = views.erase_and_advance(candidate_index, &order);
        return QueueAcceptIteration::Applied {
            candidate,
            txn_result: apply_result.ter,
            next_candidate: queue_view_next_to_option(advance.next),
        };
    }

    let decision = decide_failed_candidate(
        apply_result.ter,
        candidate.retries_remaining,
        account_retry_penalty,
        account_drop_penalty,
        account_txn_count,
        queue_nearly_full,
        current.seq_proxy,
    );

    match decision.penalty_update {
        PenaltyUpdate::None => {}
        PenaltyUpdate::MarkDropPenalty => {
            views
                .accounts
                .get_mut(&current.account)
                .expect("xrpl::TxQ::accept : account found")
                .drop_penalty = true;
        }
        PenaltyUpdate::MarkRetryPenalty => {
            views
                .accounts
                .get_mut(&current.account)
                .expect("xrpl::TxQ::accept : account found")
                .retry_penalty = true;
        }
    }

    match decision.action {
        FailedCandidateAction::RemoveCurrent => {
            debug(format_queue_accept_remove_failed_debug_message(
                candidate.tx_id,
                apply_result.ter,
            ));
            let advance = views.erase_and_advance(candidate_index, &order);
            QueueAcceptIteration::RemovedFailed {
                candidate,
                txn_result: apply_result.ter,
                penalty_update: decision.penalty_update,
                next_candidate: queue_view_next_to_option(advance.next),
            }
        }
        FailedCandidateAction::DropCurrentTicket => {
            debug(format_queue_accept_leave_in_queue_debug_message(
                candidate.tx_id,
                apply_result.ter,
                false,
                candidate.flags,
            ));
            info(format_queue_accept_drop_ticket_info_message(
                candidate.tx_id,
                apply_result.ter,
                &candidate.key.account,
            ));
            let advance = views.erase_and_advance(candidate_index, &order);
            QueueAcceptIteration::DroppedCurrentTicket {
                candidate,
                txn_result: apply_result.ter,
                next_candidate: queue_view_next_to_option(advance.next),
            }
        }
        FailedCandidateAction::KeepQueued | FailedCandidateAction::DropLastFromAccount => {
            debug(format_queue_accept_leave_in_queue_debug_message(
                candidate.tx_id,
                apply_result.ter,
                false,
                candidate.flags,
            ));
            if decision.action == FailedCandidateAction::DropLastFromAccount {
                info(format_queue_accept_drop_last_info_message(
                    candidate.tx_id,
                    apply_result.ter,
                    &candidate.key.account,
                ));
            }

            let next_retries_remaining = decision.next_retries_remaining;
            {
                let queued = views
                    .accounts
                    .get_mut(&current.account)
                    .expect("xrpl::TxQ::accept : account found")
                    .transactions
                    .get_mut(&current.seq_proxy)
                    .expect("xrpl::TxQ::accept : candidate found in account");
                queued.payload.retries_remaining = next_retries_remaining;
                queued.payload.last_result = decision.last_result;
            }

            let removed_tail = if decision.action == FailedCandidateAction::DropLastFromAccount {
                let tail_key = views
                    .accounts
                    .get(&current.account)
                    .and_then(|queued_account| {
                        queued_account
                            .transactions
                            .last_key_value()
                            .map(|(seq_proxy, _)| {
                                FeeQueueKey::new(current.account.clone(), *seq_proxy)
                            })
                    })
                    .expect("xrpl::TxQ::accept : last account candidate exists");

                if tail_key == current {
                    None
                } else {
                    Some(views.remove_fee_candidate_by_key(&tail_key))
                }
            } else {
                None
            };

            QueueAcceptIteration::RetainedFailed {
                candidate,
                txn_result: apply_result.ter,
                next_retries_remaining,
                dropped_last_from_account: decision.action
                    == FailedCandidateAction::DropLastFromAccount,
                removed_tail,
                next_candidate: views.next_fee_candidate_key(&current),
            }
        }
    }
}

pub fn finalize_prepared_queue_accept_apply<Account, Tx, Journal, ParentBatchId>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    prepared: PreparedQueueAcceptApply<Account>,
    apply_result: ApplyResult,
) -> QueueAcceptIteration<Account>
where
    Account: Clone + Display + Ord + PartialEq,
{
    finalize_prepared_queue_accept_apply_impl(views, prepared, apply_result, |_| {}, |_| {})
}

pub fn run_prepared_queue_accept_apply<Account, Tx, Journal, ParentBatchId, ApplyFn>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    prepared: PreparedQueueAcceptApply<Account>,
    apply: ApplyFn,
) -> QueueAcceptIteration<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    ApplyFn: FnOnce(&mut MaybeTx<Tx, Account, Journal, ParentBatchId>) -> ApplyResult,
{
    run_prepared_queue_accept_apply_impl(views, prepared, apply, |_| {}, |_| {})
}

pub fn run_queue_accept_iteration<Account, Tx, Journal, ParentBatchId, ApplyFn>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    current: &FeeQueueKey<Account>,
    required_fee_level: FeeLevel64,
    queue_nearly_full: bool,
    order: &OrderCandidates,
    apply: ApplyFn,
) -> QueueAcceptIteration<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    ApplyFn: FnOnce(&mut MaybeTx<Tx, Account, Journal, ParentBatchId>) -> ApplyResult,
{
    run_queue_accept_iteration_impl(
        views,
        current,
        required_fee_level,
        queue_nearly_full,
        order,
        apply,
        |_| {},
        |_| {},
        |_| {},
    )
}

fn run_queue_accept_stage_impl<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    RequiredFeeFn,
    QueueFullFn,
    ApplyFn,
    TraceFn,
    DebugFn,
    InfoFn,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    metrics_snapshot: QueueFeeMetricsSnapshot,
    order: &OrderCandidates,
    mut required_fee_level: RequiredFeeFn,
    mut queue_nearly_full: QueueFullFn,
    mut apply: ApplyFn,
    mut trace: TraceFn,
    mut debug: DebugFn,
    mut info: InfoFn,
) -> QueueAcceptStageResult<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    RequiredFeeFn: FnMut(
        QueueFeeMetricsSnapshot,
        &QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    ) -> FeeLevel64,
    QueueFullFn: FnMut(&QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>) -> bool,
    ApplyFn: FnMut(&mut MaybeTx<Tx, Account, Journal, ParentBatchId>) -> ApplyResult,
    TraceFn: FnMut(String),
    DebugFn: FnMut(String),
    InfoFn: FnMut(String),
{
    let mut ledger_changed = false;
    let mut processed_candidates = 0;
    let mut current = views.fee_order.first().map(|entry| entry.key.clone());

    while let Some(candidate_key) = current {
        let required_fee_level = required_fee_level(metrics_snapshot, views);
        let queue_nearly_full = queue_nearly_full(views);
        let iteration = run_queue_accept_iteration_impl(
            views,
            &candidate_key,
            required_fee_level,
            queue_nearly_full,
            order,
            |queued| apply(queued),
            |message| trace(message),
            |message| debug(message),
            |message| info(message),
        );
        processed_candidates += 1;
        ledger_changed |= iteration.ledger_changed();

        if let Some(stop_candidate) = iteration.stop_candidate() {
            return QueueAcceptStageResult {
                ledger_changed,
                processed_candidates,
                stop_candidate: Some(stop_candidate.key.clone()),
            };
        }

        current = iteration.next_candidate().cloned();
    }

    QueueAcceptStageResult {
        ledger_changed,
        processed_candidates,
        stop_candidate: None,
    }
}

pub fn run_queue_accept_stage<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    RequiredFeeFn,
    QueueFullFn,
    ApplyFn,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    metrics_snapshot: QueueFeeMetricsSnapshot,
    order: &OrderCandidates,
    required_fee_level: RequiredFeeFn,
    queue_nearly_full: QueueFullFn,
    apply: ApplyFn,
) -> QueueAcceptStageResult<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    RequiredFeeFn: FnMut(
        QueueFeeMetricsSnapshot,
        &QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    ) -> FeeLevel64,
    QueueFullFn: FnMut(&QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>) -> bool,
    ApplyFn: FnMut(&mut MaybeTx<Tx, Account, Journal, ParentBatchId>) -> ApplyResult,
{
    run_queue_accept_stage_impl(
        views,
        metrics_snapshot,
        order,
        required_fee_level,
        queue_nearly_full,
        apply,
        |_| {},
        |_| {},
        |_| {},
    )
}

pub fn run_queue_accept_stage_with_log_messages<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    RequiredFeeFn,
    QueueFullFn,
    ApplyFn,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    metrics_snapshot: QueueFeeMetricsSnapshot,
    order: &OrderCandidates,
    required_fee_level: RequiredFeeFn,
    queue_nearly_full: QueueFullFn,
    apply: ApplyFn,
) -> QueueAcceptStageWithLogMessagesResult<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    RequiredFeeFn: FnMut(
        QueueFeeMetricsSnapshot,
        &QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    ) -> FeeLevel64,
    QueueFullFn: FnMut(&QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>) -> bool,
    ApplyFn: FnMut(&mut MaybeTx<Tx, Account, Journal, ParentBatchId>) -> ApplyResult,
{
    let mut trace_messages = Vec::new();
    let mut debug_messages = Vec::new();
    let mut info_messages = Vec::new();
    let stage = run_queue_accept_stage_impl(
        views,
        metrics_snapshot,
        order,
        required_fee_level,
        queue_nearly_full,
        apply,
        |message| trace_messages.push(message),
        |message| debug_messages.push(message),
        |message| info_messages.push(message),
    );

    QueueAcceptStageWithLogMessagesResult {
        stage,
        loop_messages: QueueAcceptLoopLogMessages {
            trace: trace_messages,
            debug: debug_messages,
            info: info_messages,
        },
    }
}

pub fn rebuild_queue_accept_fee_order<Account, Tx, Journal, ParentBatchId>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    previous_parent_hash_comp: Uint256,
    next_parent_hash_comp: Uint256,
) -> QueueAcceptRebuildResult
where
    Account: Clone + Ord + PartialEq,
{
    let starting_size = views.fee_order.len();
    let next_order = OrderCandidates::new(next_parent_hash_comp);
    let parent_hash_unchanged = previous_parent_hash_comp == next_parent_hash_comp;
    let rebuilt_entries = views
        .accounts
        .iter()
        .flat_map(|(account_key, account_queue)| {
            assert!(
                account_queue.account == *account_key,
                "xrpl::TxQ::accept : account matches key"
            );

            account_queue
                .transactions
                .iter()
                .map(|(seq_proxy, queued)| {
                    FeeQueueEntry::new(
                        FeeQueueKey::new(account_key.clone(), *seq_proxy),
                        QueueAdvanceCandidate {
                            fee_level: queued.payload.fee_level,
                            tx_id: queued.payload.tx_id,
                            seq_proxy: *seq_proxy,
                        },
                    )
                })
        })
        .collect::<Vec<_>>();

    views.fee_order.clear();

    for entry in rebuilt_entries {
        views.insert_fee_entry(entry, &next_order);
    }

    let rebuilt_size = views.fee_order.len();
    assert_eq!(
        rebuilt_size, starting_size,
        "xrpl::TxQ::accept : byFee size match"
    );

    QueueAcceptRebuildResult {
        parent_hash_unchanged,
        next_order,
        starting_size,
        rebuilt_size,
    }
}

pub fn run_queue_accept_stage_with_rebuild<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    RequiredFeeFn,
    QueueFullFn,
    ApplyFn,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    metrics_snapshot: QueueFeeMetricsSnapshot,
    current_order: &OrderCandidates,
    previous_parent_hash_comp: Uint256,
    next_parent_hash_comp: Uint256,
    required_fee_level: RequiredFeeFn,
    queue_nearly_full: QueueFullFn,
    apply: ApplyFn,
) -> QueueAcceptStageWithRebuildResult<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    RequiredFeeFn: FnMut(
        QueueFeeMetricsSnapshot,
        &QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    ) -> FeeLevel64,
    QueueFullFn: FnMut(&QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>) -> bool,
    ApplyFn: FnMut(&mut MaybeTx<Tx, Account, Journal, ParentBatchId>) -> ApplyResult,
{
    let stage = run_queue_accept_stage(
        views,
        metrics_snapshot,
        current_order,
        required_fee_level,
        queue_nearly_full,
        apply,
    );
    let rebuild =
        rebuild_queue_accept_fee_order(views, previous_parent_hash_comp, next_parent_hash_comp);

    QueueAcceptStageWithRebuildResult { stage, rebuild }
}

pub fn run_queue_accept_with_owner_state<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    RequiredFeeFn,
    QueueFullFn,
    ApplyFn,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    metrics_snapshot: QueueFeeMetricsSnapshot,
    owner_state: &mut QueueAcceptOwnerState,
    next_parent_hash_comp: Uint256,
    required_fee_level: RequiredFeeFn,
    queue_nearly_full: QueueFullFn,
    apply: ApplyFn,
) -> QueueAcceptOwnerResult<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    RequiredFeeFn: FnMut(
        QueueFeeMetricsSnapshot,
        &QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    ) -> FeeLevel64,
    QueueFullFn: FnMut(&QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>) -> bool,
    ApplyFn: FnMut(&mut MaybeTx<Tx, Account, Journal, ParentBatchId>) -> ApplyResult,
{
    let current_order = owner_state.current_order();
    let accept = run_queue_accept_stage_with_rebuild(
        views,
        metrics_snapshot,
        &current_order,
        owner_state.parent_hash_comp,
        next_parent_hash_comp,
        required_fee_level,
        queue_nearly_full,
        apply,
    );
    owner_state.parent_hash_comp = next_parent_hash_comp;

    QueueAcceptOwnerResult {
        accept,
        owner_state: *owner_state,
    }
}

pub fn format_queue_accept_parent_hash_unchanged_warning<Hash>(parent_hash: Hash) -> String
where
    Hash: Display,
{
    format!("Parent ledger hash unchanged from {parent_hash}")
}

fn finalize_queue_accept_owner_result<Account, Tx, Journal, ParentBatchId, WarnFn>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    metrics_snapshot: QueueFeeMetricsSnapshot,
    owner_state: &mut QueueAcceptOwnerState,
    previous_parent_hash_comp: Uint256,
    next_parent_hash_comp: Uint256,
    stage: QueueAcceptStageResult<Account>,
    loop_messages: QueueAcceptLoopLogMessages,
    mut warn: WarnFn,
) -> QueueAcceptWithMetricsResult<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    WarnFn: FnMut(&str),
{
    let rebuild =
        rebuild_queue_accept_fee_order(views, previous_parent_hash_comp, next_parent_hash_comp);
    owner_state.parent_hash_comp = next_parent_hash_comp;
    let warning = rebuild
        .parent_hash_unchanged
        .then(|| format_queue_accept_parent_hash_unchanged_warning(next_parent_hash_comp));
    if let Some(message) = warning.as_deref() {
        warn(message);
    }
    let owner = QueueAcceptOwnerResult {
        accept: QueueAcceptStageWithRebuildResult {
            stage,
            rebuild: rebuild.clone(),
        },
        owner_state: *owner_state,
    };

    QueueAcceptWithMetricsResult {
        owner,
        metrics_snapshot,
        log_messages: QueueAcceptLogMessages {
            loop_messages,
            warning,
        },
    }
}

fn continue_queue_accept_with_call_state_impl<Account, Tx, Journal, ParentBatchId>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    owner_state: &mut QueueAcceptOwnerState,
    metrics_snapshot: QueueFeeMetricsSnapshot,
    call_state: QueueAcceptCallState,
    previous_parent_hash_comp: Uint256,
    mut loop_messages: QueueAcceptLoopLogMessages,
    mut ledger_changed: bool,
    mut processed_candidates: usize,
    applied_count: usize,
    mut current: Option<FeeQueueKey<Account>>,
) -> QueueAcceptPreparedCallStep<Account>
where
    Account: Clone + Display + Ord + PartialEq,
{
    let order = OrderCandidates::new(previous_parent_hash_comp);

    while let Some(candidate_key) = current {
        let required_fee_level = evaluate_required_fee_level(
            metrics_snapshot,
            call_state
                .open_ledger_tx_count
                .saturating_add(applied_count),
            ApplyFlags::NONE,
        );
        let queue_nearly_full =
            queue_accept_is_nearly_full(views.fee_order.len(), call_state.current_max_size);
        let prepared = prepare_queue_accept_iteration(
            views,
            &candidate_key,
            required_fee_level,
            queue_nearly_full,
            &order,
        );
        processed_candidates += 1;

        match prepared {
            QueueAcceptPreparedIteration::Complete(iteration) => {
                append_queue_accept_iteration_messages(&iteration, &mut loop_messages);
                ledger_changed |= iteration.ledger_changed();

                if let Some(stop_candidate) = iteration.stop_candidate() {
                    return QueueAcceptPreparedCallStep::Complete(
                        finalize_queue_accept_owner_result(
                            views,
                            metrics_snapshot,
                            owner_state,
                            previous_parent_hash_comp,
                            call_state.next_parent_hash_comp,
                            QueueAcceptStageResult {
                                ledger_changed,
                                processed_candidates,
                                stop_candidate: Some(stop_candidate.key.clone()),
                            },
                            loop_messages,
                            |_| {},
                        ),
                    );
                }

                current = iteration.next_candidate().cloned();
            }
            QueueAcceptPreparedIteration::Ready(prepared_apply) => {
                loop_messages
                    .trace
                    .push(format_queue_accept_fee_trace_message(
                        prepared_apply.candidate.tx_id,
                        &prepared_apply.candidate.key.account,
                        prepared_apply.candidate.fee_level,
                        prepared_apply.required_fee_level,
                    ));
                loop_messages
                    .trace
                    .push(format_queue_accept_apply_trace_message(
                        prepared_apply.candidate.tx_id,
                    ));
                return QueueAcceptPreparedCallStep::Ready(PreparedQueueAcceptCall {
                    prepared_apply,
                    metrics_snapshot,
                    call_state,
                    previous_parent_hash_comp,
                    loop_messages,
                    ledger_changed,
                    processed_candidates,
                    applied_count,
                });
            }
        }
    }

    QueueAcceptPreparedCallStep::Complete(finalize_queue_accept_owner_result(
        views,
        metrics_snapshot,
        owner_state,
        previous_parent_hash_comp,
        call_state.next_parent_hash_comp,
        QueueAcceptStageResult {
            ledger_changed,
            processed_candidates,
            stop_candidate: None,
        },
        loop_messages,
        |_| {},
    ))
}

fn append_queue_accept_iteration_messages<Account>(
    iteration: &QueueAcceptIteration<Account>,
    loop_messages: &mut QueueAcceptLoopLogMessages,
) where
    Account: Display,
{
    match iteration {
        QueueAcceptIteration::SkipNotFirst { candidate, .. } => {
            loop_messages
                .trace
                .push(format_queue_accept_skip_not_first_trace_message(
                    candidate.tx_id,
                    &candidate.key.account,
                ));
        }
        QueueAcceptIteration::StopInsufficientFee {
            candidate,
            required_fee_level,
        } => {
            loop_messages
                .trace
                .push(format_queue_accept_fee_trace_message(
                    candidate.tx_id,
                    &candidate.key.account,
                    candidate.fee_level,
                    *required_fee_level,
                ));
        }
        QueueAcceptIteration::Applied {
            candidate,
            txn_result,
            ..
        } => {
            loop_messages
                .debug
                .push(format_queue_accept_applied_debug_message(
                    candidate.tx_id,
                    *txn_result,
                ));
        }
        QueueAcceptIteration::RemovedFailed {
            candidate,
            txn_result,
            ..
        } => {
            loop_messages
                .debug
                .push(format_queue_accept_remove_failed_debug_message(
                    candidate.tx_id,
                    *txn_result,
                ));
        }
        QueueAcceptIteration::DroppedCurrentTicket {
            candidate,
            txn_result,
            ..
        } => {
            loop_messages
                .info
                .push(format_queue_accept_drop_ticket_info_message(
                    candidate.tx_id,
                    *txn_result,
                    &candidate.key.account,
                ));
        }
        QueueAcceptIteration::RetainedFailed {
            candidate,
            txn_result,
            dropped_last_from_account,
            ..
        } => {
            loop_messages
                .debug
                .push(format_queue_accept_leave_in_queue_debug_message(
                    candidate.tx_id,
                    *txn_result,
                    false,
                    candidate.flags,
                ));
            if *dropped_last_from_account {
                loop_messages
                    .info
                    .push(format_queue_accept_drop_last_info_message(
                        candidate.tx_id,
                        *txn_result,
                        &candidate.key.account,
                    ));
            }
        }
    }
}

pub fn run_queue_accept_with_metrics_and_owner_state<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    RequiredFeeFn,
    QueueFullFn,
    ApplyFn,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    metrics: &QueueFeeMetricsState,
    owner_state: &mut QueueAcceptOwnerState,
    next_parent_hash_comp: Uint256,
    required_fee_level: RequiredFeeFn,
    queue_nearly_full: QueueFullFn,
    apply: ApplyFn,
) -> QueueAcceptWithMetricsResult<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    RequiredFeeFn: FnMut(
        QueueFeeMetricsSnapshot,
        &QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    ) -> FeeLevel64,
    QueueFullFn: FnMut(&QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>) -> bool,
    ApplyFn: FnMut(&mut MaybeTx<Tx, Account, Journal, ParentBatchId>) -> ApplyResult,
{
    run_queue_accept_with_log_sinks_and_owner_state(
        views,
        metrics,
        owner_state,
        next_parent_hash_comp,
        required_fee_level,
        queue_nearly_full,
        |_| {},
        |_| {},
        |_| {},
        |_| {},
        apply,
    )
}

pub fn prepare_queue_accept_with_call_state<Account, Tx, Journal, ParentBatchId>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    metrics: &QueueFeeMetricsState,
    owner_state: &mut QueueAcceptOwnerState,
    call_state: QueueAcceptCallState,
) -> QueueAcceptPreparedCallStep<Account>
where
    Account: Clone + Display + Ord + PartialEq,
{
    continue_queue_accept_with_call_state_impl(
        views,
        owner_state,
        metrics.snapshot(),
        call_state,
        owner_state.parent_hash_comp,
        QueueAcceptLoopLogMessages::default(),
        false,
        0,
        0,
        views.fee_order.first().map(|entry| entry.key.clone()),
    )
}

pub fn resume_prepared_queue_accept_with_call_state<Account, Tx, Journal, ParentBatchId>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    owner_state: &mut QueueAcceptOwnerState,
    prepared_call: PreparedQueueAcceptCall<Account>,
    apply_result: ApplyResult,
) -> QueueAcceptPreparedCallStep<Account>
where
    Account: Clone + Display + Ord + PartialEq,
{
    let iteration =
        finalize_prepared_queue_accept_apply(views, prepared_call.prepared_apply, apply_result);
    let mut loop_messages = prepared_call.loop_messages;
    append_queue_accept_iteration_messages(&iteration, &mut loop_messages);
    let ledger_changed = prepared_call.ledger_changed || iteration.ledger_changed();
    let applied_count = prepared_call
        .applied_count
        .saturating_add(usize::from(iteration.ledger_changed()));

    continue_queue_accept_with_call_state_impl(
        views,
        owner_state,
        prepared_call.metrics_snapshot,
        prepared_call.call_state,
        prepared_call.previous_parent_hash_comp,
        loop_messages,
        ledger_changed,
        prepared_call.processed_candidates,
        applied_count,
        iteration.next_candidate().cloned(),
    )
}

pub fn run_prepared_queue_accept_call<Account, Tx, Journal, ParentBatchId, ApplyFn>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    owner_state: &mut QueueAcceptOwnerState,
    prepared_call: PreparedQueueAcceptCall<Account>,
    apply: ApplyFn,
) -> QueueAcceptPreparedCallStep<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    ApplyFn: FnOnce(&mut MaybeTx<Tx, Account, Journal, ParentBatchId>) -> ApplyResult,
{
    let apply_result = {
        let key = &prepared_call.prepared_apply.candidate.key;
        let queued = &mut views
            .accounts
            .get_mut(&key.account)
            .expect("xrpld::TxQ::accept : account found")
            .transactions
            .get_mut(&key.seq_proxy)
            .expect("xrpld::TxQ::accept : candidate found in account")
            .payload;
        apply(queued)
    };

    resume_prepared_queue_accept_with_call_state(views, owner_state, prepared_call, apply_result)
}

pub fn run_queue_accept_with_caller_prepared_apply<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    RunPreparedApply,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    metrics: &QueueFeeMetricsState,
    owner_state: &mut QueueAcceptOwnerState,
    call_state: QueueAcceptCallState,
    mut run_prepared_apply: RunPreparedApply,
) -> QueueAcceptWithMetricsResult<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    RunPreparedApply: FnMut(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        &mut QueueAcceptOwnerState,
        PreparedQueueAcceptCall<Account>,
    ) -> QueueAcceptPreparedCallStep<Account>,
{
    let mut step = prepare_queue_accept_with_call_state(views, metrics, owner_state, call_state);

    loop {
        match step {
            QueueAcceptPreparedCallStep::Complete(result) => return result,
            QueueAcceptPreparedCallStep::Ready(prepared) => {
                step = run_prepared_apply(views, owner_state, prepared);
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct QueueAcceptEmittedLoopMessageCounts {
    trace: usize,
    debug: usize,
    info: usize,
}

fn emit_new_queue_accept_loop_messages<TraceFn, DebugFn, InfoFn>(
    loop_messages: &QueueAcceptLoopLogMessages,
    emitted_counts: &mut QueueAcceptEmittedLoopMessageCounts,
    trace: &mut TraceFn,
    debug: &mut DebugFn,
    info: &mut InfoFn,
) where
    TraceFn: FnMut(&str),
    DebugFn: FnMut(&str),
    InfoFn: FnMut(&str),
{
    for message in &loop_messages.trace[emitted_counts.trace..] {
        trace(message);
    }
    emitted_counts.trace = loop_messages.trace.len();

    for message in &loop_messages.debug[emitted_counts.debug..] {
        debug(message);
    }
    emitted_counts.debug = loop_messages.debug.len();

    for message in &loop_messages.info[emitted_counts.info..] {
        info(message);
    }
    emitted_counts.info = loop_messages.info.len();
}

pub fn run_queue_accept_with_caller_prepared_apply_and_log_sinks<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    RunPreparedApply,
    TraceFn,
    DebugFn,
    InfoFn,
    WarnFn,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    metrics: &QueueFeeMetricsState,
    owner_state: &mut QueueAcceptOwnerState,
    call_state: QueueAcceptCallState,
    mut trace: TraceFn,
    mut debug: DebugFn,
    mut info: InfoFn,
    mut warn: WarnFn,
    mut run_prepared_apply: RunPreparedApply,
) -> QueueAcceptWithMetricsResult<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    RunPreparedApply: FnMut(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        &mut QueueAcceptOwnerState,
        PreparedQueueAcceptCall<Account>,
    ) -> QueueAcceptPreparedCallStep<Account>,
    TraceFn: FnMut(&str),
    DebugFn: FnMut(&str),
    InfoFn: FnMut(&str),
    WarnFn: FnMut(&str),
{
    let mut emitted_counts = QueueAcceptEmittedLoopMessageCounts::default();
    let mut step = prepare_queue_accept_with_call_state(views, metrics, owner_state, call_state);

    loop {
        match step {
            QueueAcceptPreparedCallStep::Complete(result) => {
                emit_new_queue_accept_loop_messages(
                    &result.log_messages.loop_messages,
                    &mut emitted_counts,
                    &mut trace,
                    &mut debug,
                    &mut info,
                );
                if let Some(message) = &result.log_messages.warning {
                    warn(message);
                }
                return result;
            }
            QueueAcceptPreparedCallStep::Ready(prepared) => {
                emit_new_queue_accept_loop_messages(
                    &prepared.loop_messages,
                    &mut emitted_counts,
                    &mut trace,
                    &mut debug,
                    &mut info,
                );
                step = run_prepared_apply(views, owner_state, prepared);
            }
        }
    }
}

pub fn run_queue_accept_with_log_sinks_and_owner_state<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    RequiredFeeFn,
    QueueFullFn,
    ApplyFn,
    TraceFn,
    DebugFn,
    InfoFn,
    WarnFn,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    metrics: &QueueFeeMetricsState,
    owner_state: &mut QueueAcceptOwnerState,
    next_parent_hash_comp: Uint256,
    required_fee_level: RequiredFeeFn,
    queue_nearly_full: QueueFullFn,
    mut trace: TraceFn,
    mut debug: DebugFn,
    mut info: InfoFn,
    mut warn: WarnFn,
    apply: ApplyFn,
) -> QueueAcceptWithMetricsResult<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    RequiredFeeFn: FnMut(
        QueueFeeMetricsSnapshot,
        &QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    ) -> FeeLevel64,
    QueueFullFn: FnMut(&QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>) -> bool,
    ApplyFn: FnMut(&mut MaybeTx<Tx, Account, Journal, ParentBatchId>) -> ApplyResult,
    TraceFn: FnMut(&str),
    DebugFn: FnMut(&str),
    InfoFn: FnMut(&str),
    WarnFn: FnMut(&str),
{
    let metrics_snapshot = metrics.snapshot();
    let current_order = owner_state.current_order();
    let mut trace_messages = Vec::new();
    let mut debug_messages = Vec::new();
    let mut info_messages = Vec::new();
    let stage = run_queue_accept_stage_impl(
        views,
        metrics_snapshot,
        &current_order,
        required_fee_level,
        queue_nearly_full,
        apply,
        |message| {
            trace(&message);
            trace_messages.push(message);
        },
        |message| {
            debug(&message);
            debug_messages.push(message);
        },
        |message| {
            info(&message);
            info_messages.push(message);
        },
    );
    finalize_queue_accept_owner_result(
        views,
        metrics_snapshot,
        owner_state,
        current_order.parent_hash_comp,
        next_parent_hash_comp,
        stage,
        QueueAcceptLoopLogMessages {
            trace: trace_messages,
            debug: debug_messages,
            info: info_messages,
        },
        |message| warn(message),
    )
}

pub fn run_queue_accept_with_call_state<Account, Tx, Journal, ParentBatchId, ApplyFn>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    metrics: &QueueFeeMetricsState,
    owner_state: &mut QueueAcceptOwnerState,
    call_state: QueueAcceptCallState,
    apply: ApplyFn,
) -> QueueAcceptWithMetricsResult<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    ApplyFn: FnMut(&mut MaybeTx<Tx, Account, Journal, ParentBatchId>) -> ApplyResult,
{
    run_queue_accept_with_call_state_and_log_sinks(
        views,
        metrics,
        owner_state,
        call_state,
        |_| {},
        |_| {},
        |_| {},
        |_| {},
        apply,
    )
}

pub fn run_queue_accept_with_call_state_and_log_sinks<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    ApplyFn,
    TraceFn,
    DebugFn,
    InfoFn,
    WarnFn,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    metrics: &QueueFeeMetricsState,
    owner_state: &mut QueueAcceptOwnerState,
    call_state: QueueAcceptCallState,
    trace: TraceFn,
    debug: DebugFn,
    info: InfoFn,
    warn: WarnFn,
    mut apply: ApplyFn,
) -> QueueAcceptWithMetricsResult<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    ApplyFn: FnMut(&mut MaybeTx<Tx, Account, Journal, ParentBatchId>) -> ApplyResult,
    TraceFn: FnMut(&str),
    DebugFn: FnMut(&str),
    InfoFn: FnMut(&str),
    WarnFn: FnMut(&str),
{
    let applied_count = Cell::new(0_usize);

    run_queue_accept_with_log_sinks_and_owner_state(
        views,
        metrics,
        owner_state,
        call_state.next_parent_hash_comp,
        |metrics_snapshot, _views| {
            evaluate_required_fee_level(
                metrics_snapshot,
                call_state
                    .open_ledger_tx_count
                    .saturating_add(applied_count.get()),
                ApplyFlags::NONE,
            )
        },
        |views| queue_accept_is_nearly_full(views.fee_order.len(), call_state.current_max_size),
        trace,
        debug,
        info,
        warn,
        |queued| {
            let result = apply(queued);
            if result.applied {
                applied_count.set(applied_count.get().saturating_add(1));
            }
            result
        },
    )
}

#[cfg(test)]
mod tests;
