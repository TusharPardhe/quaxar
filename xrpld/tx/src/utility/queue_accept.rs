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
mod tests {
    use std::cell::{Cell, RefCell};
    use std::collections::BTreeMap;

    use basics::base_uint::Uint256;
    use protocol::{Rules, SeqProxy, Ter};

    use super::{
        PreparedQueueAcceptApply, PreparedQueueAcceptCall, QueueAcceptCallState,
        QueueAcceptCandidate, QueueAcceptIteration, QueueAcceptLogMessages,
        QueueAcceptLoopLogMessages, QueueAcceptOwnerResult, QueueAcceptOwnerState,
        QueueAcceptPreparedCallStep, QueueAcceptPreparedIteration, QueueAcceptRebuildResult,
        QueueAcceptStageResult, QueueAcceptStageWithLogMessagesResult,
        QueueAcceptStageWithRebuildResult, QueueAcceptWithMetricsResult,
        format_queue_accept_apply_trace_message, format_queue_accept_drop_last_info_message,
        format_queue_accept_fee_trace_message, format_queue_accept_leave_in_queue_debug_message,
        format_queue_accept_parent_hash_unchanged_warning,
        format_queue_accept_skip_not_first_trace_message, prepare_queue_accept_iteration,
        prepare_queue_accept_with_call_state, queue_accept_is_nearly_full,
        rebuild_queue_accept_fee_order, resume_prepared_queue_accept_with_call_state,
        run_prepared_queue_accept_apply, run_prepared_queue_accept_call,
        run_queue_accept_iteration, run_queue_accept_stage,
        run_queue_accept_stage_with_log_messages, run_queue_accept_stage_with_rebuild,
        run_queue_accept_with_call_state, run_queue_accept_with_call_state_and_log_sinks,
        run_queue_accept_with_caller_prepared_apply,
        run_queue_accept_with_caller_prepared_apply_and_log_sinks,
        run_queue_accept_with_log_sinks_and_owner_state,
        run_queue_accept_with_metrics_and_owner_state, run_queue_accept_with_owner_state,
    };
    use crate::{
        ApplyFlags, ApplyResult, FeeLevel64, FeeQueueEntry, FeeQueueKey, MaybeTx, MaybeTxCore,
        OrderCandidates, PreflightResult, QueueFeeMetricsConfig, QueueFeeMetricsSnapshot,
        QueueFeeMetricsState, QueueViews, TXQ_BASE_LEVEL, TxConsequences, TxQAccount,
    };

    fn queued(
        account: &'static str,
        seq_proxy: SeqProxy,
        tx_id: u64,
        fee_level: u64,
    ) -> MaybeTx<&'static str, &'static str, &'static str, &'static str> {
        MaybeTx::new(
            Uint256::from_u64(tx_id),
            fee_level,
            account,
            Some(200),
            seq_proxy,
            ApplyFlags::NONE,
            PreflightResult::new(
                "tx",
                None::<&str>,
                Rules::new(std::iter::empty()),
                TxConsequences::new(1, seq_proxy),
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            ),
        )
    }

    fn queue_candidate(
        seq_proxy: SeqProxy,
        tx_id: u64,
        fee_level: u64,
    ) -> crate::QueueAdvanceCandidate {
        crate::QueueAdvanceCandidate {
            fee_level,
            tx_id: Uint256::from_u64(tx_id),
            seq_proxy,
        }
    }

    fn metrics_state() -> QueueFeeMetricsState {
        QueueFeeMetricsState::new(QueueFeeMetricsConfig {
            ledgers_in_queue: 3,
            queue_size_min: 20,
            minimum_escalation_multiplier: TXQ_BASE_LEVEL * 500,
            minimum_txn_in_ledger: 32,
            target_txn_in_ledger: 256,
            maximum_txn_in_ledger: Some(400),
            normal_consensus_increase_percent: 20,
            slow_consensus_decrease_percent: 50,
        })
    }

    fn metrics_state_with(
        minimum_txn_in_ledger: usize,
        minimum_escalation_multiplier: FeeLevel64,
    ) -> QueueFeeMetricsState {
        QueueFeeMetricsState::new(QueueFeeMetricsConfig {
            ledgers_in_queue: 3,
            queue_size_min: 20,
            minimum_escalation_multiplier,
            minimum_txn_in_ledger,
            target_txn_in_ledger: minimum_txn_in_ledger,
            maximum_txn_in_ledger: Some(minimum_txn_in_ledger),
            normal_consensus_increase_percent: 20,
            slow_consensus_decrease_percent: 50,
        })
    }

    #[test]
    fn accept_nearly_full_helper_matches_current_cpp_threshold_shape() {
        assert!(!queue_accept_is_nearly_full(1, None));
        assert!(!queue_accept_is_nearly_full(8, Some(10)));
        assert!(queue_accept_is_nearly_full(9, Some(10)));
        assert!(queue_accept_is_nearly_full(2, Some(2)));
    }

    #[test]
    fn accept_iteration_prepare_returns_execution_token_only_for_apply_path() {
        let mut account = TxQAccount::new("acct");
        account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                queued("acct", SeqProxy::sequence(5), 5, 300),
                TxConsequences::new(1, SeqProxy::sequence(5)),
            ),
        );
        let views = QueueViews::new(
            BTreeMap::from([("acct", account)]),
            vec![FeeQueueEntry::new(
                FeeQueueKey::new("acct", SeqProxy::sequence(5)),
                queue_candidate(SeqProxy::sequence(5), 5, 300),
            )],
        );

        assert_eq!(
            prepare_queue_accept_iteration(
                &views,
                &FeeQueueKey::new("acct", SeqProxy::sequence(5)),
                256,
                true,
                &OrderCandidates::new(Uint256::from_u64(9)),
            ),
            QueueAcceptPreparedIteration::Ready(PreparedQueueAcceptApply {
                candidate: QueueAcceptCandidate {
                    key: FeeQueueKey::new("acct", SeqProxy::sequence(5)),
                    tx_id: Uint256::from_u64(5),
                    fee_level: 300,
                    retries_remaining: 10,
                    flags: ApplyFlags::NONE,
                },
                required_fee_level: 256,
                queue_nearly_full: true,
                candidate_index: 0,
                account_retry_penalty: false,
                account_drop_penalty: false,
                account_txn_count: 1,
                order: OrderCandidates::new(Uint256::from_u64(9)),
            })
        );
    }

    #[test]
    fn prepared_accept_apply_runner_matches_direct_iteration_behavior() {
        let build_views = || {
            let mut account = TxQAccount::new("acct");
            account.drop_penalty = true;
            account.add(
                SeqProxy::sequence(5),
                MaybeTxCore::new(
                    queued("acct", SeqProxy::sequence(5), 5, 300),
                    TxConsequences::new(1, SeqProxy::sequence(5)),
                ),
            );
            account.add(
                SeqProxy::sequence(9),
                MaybeTxCore::new(
                    queued("acct", SeqProxy::sequence(9), 9, 60),
                    TxConsequences::new(1, SeqProxy::sequence(9)),
                ),
            );
            QueueViews::new(
                BTreeMap::from([("acct", account)]),
                vec![
                    FeeQueueEntry::new(
                        FeeQueueKey::new("acct", SeqProxy::sequence(5)),
                        queue_candidate(SeqProxy::sequence(5), 5, 300),
                    ),
                    FeeQueueEntry::new(
                        FeeQueueKey::new("acct", SeqProxy::sequence(9)),
                        queue_candidate(SeqProxy::sequence(9), 9, 60),
                    ),
                ],
            )
        };

        let mut prepared_views = build_views();
        let prepared = match prepare_queue_accept_iteration(
            &prepared_views,
            &FeeQueueKey::new("acct", SeqProxy::sequence(5)),
            256,
            true,
            &OrderCandidates::new(Uint256::from_u64(9)),
        ) {
            QueueAcceptPreparedIteration::Ready(prepared) => prepared,
            QueueAcceptPreparedIteration::Complete(iteration) => {
                panic!("expected ready iteration, got {iteration:?}")
            }
        };
        let prepared_result =
            run_prepared_queue_accept_apply(&mut prepared_views, prepared, |_queued| {
                ApplyResult::new(Ter::TER_RETRY, false, false)
            });

        let mut direct_views = build_views();
        let direct_result = run_queue_accept_iteration(
            &mut direct_views,
            &FeeQueueKey::new("acct", SeqProxy::sequence(5)),
            256,
            true,
            &OrderCandidates::new(Uint256::from_u64(9)),
            |_queued| ApplyResult::new(Ter::TER_RETRY, false, false),
        );

        assert_eq!(prepared_result, direct_result);
        assert_eq!(prepared_views, direct_views);
    }

    #[test]
    fn accept_iteration_skips_later_sequence_candidates_until_the_front_moves() {
        let mut account = TxQAccount::new("acct");
        account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                queued("acct", SeqProxy::sequence(5), 5, 100),
                TxConsequences::new(1, SeqProxy::sequence(5)),
            ),
        );
        account.add(
            SeqProxy::sequence(7),
            MaybeTxCore::new(
                queued("acct", SeqProxy::sequence(7), 7, 120),
                TxConsequences::new(1, SeqProxy::sequence(7)),
            ),
        );

        let mut views = QueueViews::new(
            BTreeMap::from([("acct", account)]),
            vec![
                FeeQueueEntry::new(
                    FeeQueueKey::new("acct", SeqProxy::sequence(7)),
                    queue_candidate(SeqProxy::sequence(7), 7, 120),
                ),
                FeeQueueEntry::new(
                    FeeQueueKey::new("acct", SeqProxy::sequence(5)),
                    queue_candidate(SeqProxy::sequence(5), 5, 100),
                ),
            ],
        );

        let iteration = run_queue_accept_iteration(
            &mut views,
            &FeeQueueKey::new("acct", SeqProxy::sequence(7)),
            90,
            false,
            &OrderCandidates::new(Uint256::from_u64(0)),
            |_| panic!("skip path must not try apply"),
        );

        assert_eq!(
            iteration,
            QueueAcceptIteration::SkipNotFirst {
                candidate: QueueAcceptCandidate {
                    key: FeeQueueKey::new("acct", SeqProxy::sequence(7)),
                    tx_id: Uint256::from_u64(7),
                    fee_level: 120,
                    retries_remaining: 10,
                    flags: ApplyFlags::NONE,
                },
                next_candidate: Some(FeeQueueKey::new("acct", SeqProxy::sequence(5))),
            }
        );
    }

    #[test]
    fn accept_iteration_drops_last_account_item_on_near_full_soft_failure() {
        let mut account_a = TxQAccount::new("a");
        account_a.drop_penalty = true;
        account_a.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                queued("a", SeqProxy::sequence(5), 5, 100),
                TxConsequences::new(1, SeqProxy::sequence(5)),
            ),
        );
        account_a.add(
            SeqProxy::sequence(9),
            MaybeTxCore::new(
                queued("a", SeqProxy::sequence(9), 9, 60),
                TxConsequences::new(1, SeqProxy::sequence(9)),
            ),
        );

        let mut account_b = TxQAccount::new("b");
        account_b.add(
            SeqProxy::sequence(7),
            MaybeTxCore::new(
                queued("b", SeqProxy::sequence(7), 7, 95),
                TxConsequences::new(1, SeqProxy::sequence(7)),
            ),
        );

        let mut views = QueueViews::new(
            BTreeMap::from([("a", account_a), ("b", account_b)]),
            vec![
                FeeQueueEntry::new(
                    FeeQueueKey::new("a", SeqProxy::sequence(5)),
                    queue_candidate(SeqProxy::sequence(5), 5, 100),
                ),
                FeeQueueEntry::new(
                    FeeQueueKey::new("b", SeqProxy::sequence(7)),
                    queue_candidate(SeqProxy::sequence(7), 7, 95),
                ),
                FeeQueueEntry::new(
                    FeeQueueKey::new("a", SeqProxy::sequence(9)),
                    queue_candidate(SeqProxy::sequence(9), 9, 60),
                ),
            ],
        );

        let iteration = run_queue_accept_iteration(
            &mut views,
            &FeeQueueKey::new("a", SeqProxy::sequence(5)),
            80,
            true,
            &OrderCandidates::new(Uint256::from_u64(0)),
            |_| ApplyResult::new(Ter::TER_RETRY, false, false),
        );

        assert_eq!(
            iteration,
            QueueAcceptIteration::RetainedFailed {
                candidate: QueueAcceptCandidate {
                    key: FeeQueueKey::new("a", SeqProxy::sequence(5)),
                    tx_id: Uint256::from_u64(5),
                    fee_level: 100,
                    retries_remaining: 10,
                    flags: ApplyFlags::NONE,
                },
                txn_result: Ter::TER_RETRY,
                next_retries_remaining: 9,
                dropped_last_from_account: true,
                removed_tail: Some(FeeQueueKey::new("a", SeqProxy::sequence(9))),
                next_candidate: Some(FeeQueueKey::new("b", SeqProxy::sequence(7))),
            }
        );
        assert_eq!(
            views.accounts["a"].transactions[&SeqProxy::sequence(5)]
                .payload
                .retries_remaining,
            9
        );
        assert_eq!(
            views.accounts["a"].transactions[&SeqProxy::sequence(5)]
                .payload
                .last_result,
            Some(Ter::TER_RETRY)
        );
        assert!(
            !views.accounts["a"]
                .transactions
                .contains_key(&SeqProxy::sequence(9))
        );
    }

    #[test]
    fn accept_stage_with_log_messages_preserves_cpp_trace_order_for_skip_then_stop() {
        let mut account = TxQAccount::new("acct");
        account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                queued("acct", SeqProxy::sequence(5), 5, 80),
                TxConsequences::new(1, SeqProxy::sequence(5)),
            ),
        );
        account.add(
            SeqProxy::sequence(7),
            MaybeTxCore::new(
                queued("acct", SeqProxy::sequence(7), 7, 120),
                TxConsequences::new(1, SeqProxy::sequence(7)),
            ),
        );

        let mut views = QueueViews::new(
            BTreeMap::from([("acct", account)]),
            vec![
                FeeQueueEntry::new(
                    FeeQueueKey::new("acct", SeqProxy::sequence(7)),
                    queue_candidate(SeqProxy::sequence(7), 7, 120),
                ),
                FeeQueueEntry::new(
                    FeeQueueKey::new("acct", SeqProxy::sequence(5)),
                    queue_candidate(SeqProxy::sequence(5), 5, 80),
                ),
            ],
        );

        let result = run_queue_accept_stage_with_log_messages(
            &mut views,
            QueueFeeMetricsSnapshot {
                txns_expected: 32,
                escalation_multiplier: 400,
            },
            &OrderCandidates::new(Uint256::from_u64(0)),
            |_snapshot, _views| 90,
            |_views| false,
            |_queued| panic!("skip/stop path must not try apply"),
        );

        assert_eq!(
            result,
            QueueAcceptStageWithLogMessagesResult {
                stage: QueueAcceptStageResult {
                    ledger_changed: false,
                    processed_candidates: 2,
                    stop_candidate: Some(FeeQueueKey::new("acct", SeqProxy::sequence(5))),
                },
                loop_messages: QueueAcceptLoopLogMessages {
                    trace: vec![
                        format_queue_accept_skip_not_first_trace_message(
                            Uint256::from_u64(7),
                            "acct",
                        ),
                        format_queue_accept_fee_trace_message(Uint256::from_u64(5), "acct", 80, 90,),
                    ],
                    debug: Vec::new(),
                    info: Vec::new(),
                },
            }
        );
    }

    #[test]
    fn accept_stage_applies_until_a_candidate_falls_below_the_required_fee() {
        let mut account_a = TxQAccount::new("a");
        account_a.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                queued("a", SeqProxy::sequence(5), 5, 120),
                TxConsequences::new(1, SeqProxy::sequence(5)),
            ),
        );

        let mut account_b = TxQAccount::new("b");
        account_b.add(
            SeqProxy::sequence(7),
            MaybeTxCore::new(
                queued("b", SeqProxy::sequence(7), 7, 80),
                TxConsequences::new(1, SeqProxy::sequence(7)),
            ),
        );

        let mut views = QueueViews::new(
            BTreeMap::from([("a", account_a), ("b", account_b)]),
            vec![
                FeeQueueEntry::new(
                    FeeQueueKey::new("a", SeqProxy::sequence(5)),
                    queue_candidate(SeqProxy::sequence(5), 5, 120),
                ),
                FeeQueueEntry::new(
                    FeeQueueKey::new("b", SeqProxy::sequence(7)),
                    queue_candidate(SeqProxy::sequence(7), 7, 80),
                ),
            ],
        );

        let result = run_queue_accept_stage(
            &mut views,
            QueueFeeMetricsSnapshot {
                txns_expected: 32,
                escalation_multiplier: 500,
            },
            &OrderCandidates::new(Uint256::from_u64(0)),
            |snapshot, _views| snapshot.escalation_multiplier / 5,
            |_views| false,
            |_queued| ApplyResult::new(Ter::TES_SUCCESS, true, false),
        );

        assert_eq!(
            result,
            QueueAcceptStageResult {
                ledger_changed: true,
                processed_candidates: 2,
                stop_candidate: Some(FeeQueueKey::new("b", SeqProxy::sequence(7))),
            }
        );
        assert!(views.accounts["a"].empty());
        assert!(
            views.accounts["b"]
                .transactions
                .contains_key(&SeqProxy::sequence(7))
        );
    }

    #[test]
    fn accept_rebuild_reorders_equal_fee_candidates_using_the_new_parent_hash() {
        let mut account_a = TxQAccount::new("a");
        account_a.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                queued("a", SeqProxy::sequence(5), 1, 100),
                TxConsequences::new(1, SeqProxy::sequence(5)),
            ),
        );

        let mut account_b = TxQAccount::new("b");
        account_b.add(
            SeqProxy::sequence(7),
            MaybeTxCore::new(
                queued("b", SeqProxy::sequence(7), 2, 100),
                TxConsequences::new(1, SeqProxy::sequence(7)),
            ),
        );

        let mut views = QueueViews::new(
            BTreeMap::from([("a", account_a), ("b", account_b)]),
            vec![
                FeeQueueEntry::new(
                    FeeQueueKey::new("a", SeqProxy::sequence(5)),
                    queue_candidate(SeqProxy::sequence(5), 1, 100),
                ),
                FeeQueueEntry::new(
                    FeeQueueKey::new("b", SeqProxy::sequence(7)),
                    queue_candidate(SeqProxy::sequence(7), 2, 100),
                ),
            ],
        );

        let result =
            rebuild_queue_accept_fee_order(&mut views, Uint256::from_u64(0), Uint256::from_u64(3));

        assert_eq!(
            result,
            QueueAcceptRebuildResult {
                parent_hash_unchanged: false,
                next_order: OrderCandidates::new(Uint256::from_u64(3)),
                starting_size: 2,
                rebuilt_size: 2,
            }
        );
        assert_eq!(
            views
                .fee_order
                .iter()
                .map(|entry| entry.key.clone())
                .collect::<Vec<_>>(),
            vec![
                FeeQueueKey::new("b", SeqProxy::sequence(7)),
                FeeQueueKey::new("a", SeqProxy::sequence(5)),
            ]
        );
    }

    #[test]
    fn accept_stage_with_rebuild_reports_unchanged_parent_hash_and_preserves_size() {
        let mut account_a = TxQAccount::new("a");
        account_a.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                queued("a", SeqProxy::sequence(5), 5, 120),
                TxConsequences::new(1, SeqProxy::sequence(5)),
            ),
        );

        let mut account_b = TxQAccount::new("b");
        account_b.add(
            SeqProxy::sequence(7),
            MaybeTxCore::new(
                queued("b", SeqProxy::sequence(7), 7, 100),
                TxConsequences::new(1, SeqProxy::sequence(7)),
            ),
        );

        let mut views = QueueViews::new(
            BTreeMap::from([("a", account_a), ("b", account_b)]),
            vec![
                FeeQueueEntry::new(
                    FeeQueueKey::new("a", SeqProxy::sequence(5)),
                    queue_candidate(SeqProxy::sequence(5), 5, 120),
                ),
                FeeQueueEntry::new(
                    FeeQueueKey::new("b", SeqProxy::sequence(7)),
                    queue_candidate(SeqProxy::sequence(7), 7, 100),
                ),
            ],
        );

        let result = run_queue_accept_stage_with_rebuild(
            &mut views,
            QueueFeeMetricsSnapshot {
                txns_expected: 32,
                escalation_multiplier: 400,
            },
            &OrderCandidates::new(Uint256::from_u64(0)),
            Uint256::from_u64(9),
            Uint256::from_u64(9),
            |snapshot, _views| snapshot.escalation_multiplier / 4,
            |_views| false,
            |_queued| ApplyResult::new(Ter::TER_RETRY, false, false),
        );

        assert_eq!(
            result,
            QueueAcceptStageWithRebuildResult {
                stage: QueueAcceptStageResult {
                    ledger_changed: false,
                    processed_candidates: 2,
                    stop_candidate: None,
                },
                rebuild: QueueAcceptRebuildResult {
                    parent_hash_unchanged: true,
                    next_order: OrderCandidates::new(Uint256::from_u64(9)),
                    starting_size: 2,
                    rebuilt_size: 2,
                },
            }
        );
    }

    #[test]
    fn accept_owner_wrapper_updates_parent_hash_state_after_rebuild() {
        let mut account_a = TxQAccount::new("a");
        account_a.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                queued("a", SeqProxy::sequence(5), 1, 100),
                TxConsequences::new(1, SeqProxy::sequence(5)),
            ),
        );

        let mut account_b = TxQAccount::new("b");
        account_b.add(
            SeqProxy::sequence(7),
            MaybeTxCore::new(
                queued("b", SeqProxy::sequence(7), 2, 100),
                TxConsequences::new(1, SeqProxy::sequence(7)),
            ),
        );

        let mut views = QueueViews::new(
            BTreeMap::from([("a", account_a), ("b", account_b)]),
            vec![
                FeeQueueEntry::new(
                    FeeQueueKey::new("a", SeqProxy::sequence(5)),
                    queue_candidate(SeqProxy::sequence(5), 1, 100),
                ),
                FeeQueueEntry::new(
                    FeeQueueKey::new("b", SeqProxy::sequence(7)),
                    queue_candidate(SeqProxy::sequence(7), 2, 100),
                ),
            ],
        );
        let mut owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(0));

        let result = run_queue_accept_with_owner_state(
            &mut views,
            QueueFeeMetricsSnapshot {
                txns_expected: 32,
                escalation_multiplier: 400,
            },
            &mut owner_state,
            Uint256::from_u64(3),
            |snapshot, _views| snapshot.escalation_multiplier / 4,
            |_views| false,
            |_queued| ApplyResult::new(Ter::TER_RETRY, false, false),
        );

        assert_eq!(
            result,
            QueueAcceptOwnerResult {
                accept: QueueAcceptStageWithRebuildResult {
                    stage: QueueAcceptStageResult {
                        ledger_changed: false,
                        processed_candidates: 2,
                        stop_candidate: None,
                    },
                    rebuild: QueueAcceptRebuildResult {
                        parent_hash_unchanged: false,
                        next_order: OrderCandidates::new(Uint256::from_u64(3)),
                        starting_size: 2,
                        rebuilt_size: 2,
                    },
                },
                owner_state: QueueAcceptOwnerState::new(Uint256::from_u64(3)),
            }
        );
        assert_eq!(
            owner_state,
            QueueAcceptOwnerState::new(Uint256::from_u64(3))
        );
        assert_eq!(
            views
                .fee_order
                .iter()
                .map(|entry| entry.key.clone())
                .collect::<Vec<_>>(),
            vec![
                FeeQueueKey::new("b", SeqProxy::sequence(7)),
                FeeQueueKey::new("a", SeqProxy::sequence(5)),
            ]
        );
    }

    #[test]
    fn accept_owner_wrapper_reports_unchanged_parent_hash_without_changing_state() {
        let mut account = TxQAccount::new("acct");
        account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                queued("acct", SeqProxy::sequence(5), 5, 120),
                TxConsequences::new(1, SeqProxy::sequence(5)),
            ),
        );

        let mut views = QueueViews::new(
            BTreeMap::from([("acct", account)]),
            vec![FeeQueueEntry::new(
                FeeQueueKey::new("acct", SeqProxy::sequence(5)),
                queue_candidate(SeqProxy::sequence(5), 5, 120),
            )],
        );
        let mut owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(9));

        let result = run_queue_accept_with_owner_state(
            &mut views,
            QueueFeeMetricsSnapshot {
                txns_expected: 32,
                escalation_multiplier: 400,
            },
            &mut owner_state,
            Uint256::from_u64(9),
            |snapshot, _views| snapshot.escalation_multiplier / 4,
            |_views| false,
            |_queued| ApplyResult::new(Ter::TER_RETRY, false, false),
        );

        assert!(result.accept.rebuild.parent_hash_unchanged);
        assert_eq!(
            result.owner_state,
            QueueAcceptOwnerState::new(Uint256::from_u64(9))
        );
        assert_eq!(
            owner_state,
            QueueAcceptOwnerState::new(Uint256::from_u64(9))
        );
    }

    #[test]
    fn accept_parent_hash_unchanged_warning_matches_current_cpp_wording_shape() {
        assert_eq!(
            format_queue_accept_parent_hash_unchanged_warning(Uint256::from_u64(9)),
            format!("Parent ledger hash unchanged from {}", Uint256::from_u64(9))
        );
    }

    #[test]
    fn accept_metrics_owner_wrapper_uses_snapshot_and_emits_unchanged_parent_warning() {
        let mut account = TxQAccount::new("acct");
        account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                queued("acct", SeqProxy::sequence(5), 5, 120),
                TxConsequences::new(1, SeqProxy::sequence(5)),
            ),
        );

        let mut views = QueueViews::new(
            BTreeMap::from([("acct", account)]),
            vec![FeeQueueEntry::new(
                FeeQueueKey::new("acct", SeqProxy::sequence(5)),
                queue_candidate(SeqProxy::sequence(5), 5, 120),
            )],
        );
        let metrics = metrics_state();
        let mut owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(9));

        let result = run_queue_accept_with_metrics_and_owner_state(
            &mut views,
            &metrics,
            &mut owner_state,
            Uint256::from_u64(9),
            |snapshot, _views| snapshot.escalation_multiplier / 2_000,
            |_views| false,
            |_queued| ApplyResult::new(Ter::TER_RETRY, false, false),
        );

        assert_eq!(
            result,
            QueueAcceptWithMetricsResult {
                owner: QueueAcceptOwnerResult {
                    accept: QueueAcceptStageWithRebuildResult {
                        stage: QueueAcceptStageResult {
                            ledger_changed: false,
                            processed_candidates: 1,
                            stop_candidate: None,
                        },
                        rebuild: QueueAcceptRebuildResult {
                            parent_hash_unchanged: true,
                            next_order: OrderCandidates::new(Uint256::from_u64(9)),
                            starting_size: 1,
                            rebuilt_size: 1,
                        },
                    },
                    owner_state: QueueAcceptOwnerState::new(Uint256::from_u64(9)),
                },
                metrics_snapshot: metrics.snapshot(),
                log_messages: QueueAcceptLogMessages {
                    loop_messages: QueueAcceptLoopLogMessages {
                        trace: vec![
                            format_queue_accept_fee_trace_message(
                                Uint256::from_u64(5),
                                "acct",
                                120,
                                metrics.snapshot().escalation_multiplier / 2_000,
                            ),
                            format_queue_accept_apply_trace_message(Uint256::from_u64(5)),
                        ],
                        debug: vec![format_queue_accept_leave_in_queue_debug_message(
                            Uint256::from_u64(5),
                            Ter::TER_RETRY,
                            false,
                            ApplyFlags::NONE,
                        )],
                        info: Vec::new(),
                    },
                    warning: Some(format_queue_accept_parent_hash_unchanged_warning(
                        Uint256::from_u64(9),
                    )),
                },
            }
        );
    }

    #[test]
    fn accept_log_sink_owner_wrapper_emitsed_messages_and_returns_same_payload() {
        let mut account = TxQAccount::new("acct");
        account.drop_penalty = true;
        account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                queued("acct", SeqProxy::sequence(5), 5, 120),
                TxConsequences::new(1, SeqProxy::sequence(5)),
            ),
        );
        account.add(
            SeqProxy::sequence(9),
            MaybeTxCore::new(
                queued("acct", SeqProxy::sequence(9), 9, 60),
                TxConsequences::new(1, SeqProxy::sequence(9)),
            ),
        );

        let mut views = QueueViews::new(
            BTreeMap::from([("acct", account)]),
            vec![
                FeeQueueEntry::new(
                    FeeQueueKey::new("acct", SeqProxy::sequence(5)),
                    queue_candidate(SeqProxy::sequence(5), 5, 120),
                ),
                FeeQueueEntry::new(
                    FeeQueueKey::new("acct", SeqProxy::sequence(9)),
                    queue_candidate(SeqProxy::sequence(9), 9, 60),
                ),
            ],
        );
        let metrics = metrics_state();
        let mut owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(9));
        let emitted = RefCell::new(Vec::new());

        let result = run_queue_accept_with_log_sinks_and_owner_state(
            &mut views,
            &metrics,
            &mut owner_state,
            Uint256::from_u64(9),
            |snapshot, _views| snapshot.escalation_multiplier / 2_000,
            |_views| true,
            |message| emitted.borrow_mut().push(format!("trace:{message}")),
            |message| emitted.borrow_mut().push(format!("debug:{message}")),
            |message| emitted.borrow_mut().push(format!("info:{message}")),
            |message| emitted.borrow_mut().push(format!("warn:{message}")),
            |_queued| ApplyResult::new(Ter::TER_RETRY, false, false),
        );

        let expected_trace = vec![
            format_queue_accept_fee_trace_message(
                Uint256::from_u64(5),
                "acct",
                120,
                metrics.snapshot().escalation_multiplier / 2_000,
            ),
            format_queue_accept_apply_trace_message(Uint256::from_u64(5)),
        ];
        let expected_debug = vec![format_queue_accept_leave_in_queue_debug_message(
            Uint256::from_u64(5),
            Ter::TER_RETRY,
            false,
            ApplyFlags::NONE,
        )];
        let expected_info = vec![format_queue_accept_drop_last_info_message(
            Uint256::from_u64(5),
            Ter::TER_RETRY,
            "acct",
        )];
        let expected_warning =
            format_queue_accept_parent_hash_unchanged_warning(Uint256::from_u64(9));

        assert_eq!(
            emitted.into_inner(),
            vec![
                format!("trace:{}", expected_trace[0]),
                format!("trace:{}", expected_trace[1]),
                format!("debug:{}", expected_debug[0]),
                format!("info:{}", expected_info[0]),
                format!("warn:{expected_warning}"),
            ]
        );
        assert_eq!(result.log_messages.loop_messages.trace, expected_trace);
        assert_eq!(result.log_messages.loop_messages.debug, expected_debug);
        assert_eq!(result.log_messages.loop_messages.info, expected_info);
        assert_eq!(result.log_messages.warning, Some(expected_warning));
    }

    #[test]
    fn accept_log_sink_owner_wrapper_skips_warning_sink_when_parent_hash_changes() {
        let mut account = TxQAccount::new("acct");
        account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                queued("acct", SeqProxy::sequence(5), 5, 120),
                TxConsequences::new(1, SeqProxy::sequence(5)),
            ),
        );

        let mut views = QueueViews::new(
            BTreeMap::from([("acct", account)]),
            vec![FeeQueueEntry::new(
                FeeQueueKey::new("acct", SeqProxy::sequence(5)),
                queue_candidate(SeqProxy::sequence(5), 5, 120),
            )],
        );
        let metrics = metrics_state();
        let mut owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(9));
        let mut warnings = Vec::new();

        let result = run_queue_accept_with_log_sinks_and_owner_state(
            &mut views,
            &metrics,
            &mut owner_state,
            Uint256::from_u64(10),
            |snapshot, _views| snapshot.escalation_multiplier / 2_000,
            |_views| false,
            |_message| {},
            |_message| {},
            |_message| {},
            |message| warnings.push(message.to_owned()),
            |_queued| ApplyResult::new(Ter::TER_RETRY, false, false),
        );

        assert!(warnings.is_empty());
        assert_eq!(result.log_messages.warning, None);
        assert_eq!(
            result.owner.owner_state,
            QueueAcceptOwnerState::new(Uint256::from_u64(10))
        );
    }

    #[test]
    fn accept_call_state_wrapper_recomputes_required_fee_after_successful_apply() {
        let mut account_a = TxQAccount::new("a");
        account_a.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                queued("a", SeqProxy::sequence(5), 5, 5_000),
                TxConsequences::new(1, SeqProxy::sequence(5)),
            ),
        );

        let mut account_b = TxQAccount::new("b");
        account_b.add(
            SeqProxy::sequence(7),
            MaybeTxCore::new(
                queued("b", SeqProxy::sequence(7), 7, 1_000),
                TxConsequences::new(1, SeqProxy::sequence(7)),
            ),
        );

        let mut views = QueueViews::new(
            BTreeMap::from([("a", account_a), ("b", account_b)]),
            vec![
                FeeQueueEntry::new(
                    FeeQueueKey::new("a", SeqProxy::sequence(5)),
                    queue_candidate(SeqProxy::sequence(5), 5, 5_000),
                ),
                FeeQueueEntry::new(
                    FeeQueueKey::new("b", SeqProxy::sequence(7)),
                    queue_candidate(SeqProxy::sequence(7), 7, 1_000),
                ),
            ],
        );
        let metrics = metrics_state_with(1, 1_000);
        let mut owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(0));
        let apply_calls = Cell::new(0_usize);

        let result = run_queue_accept_with_call_state(
            &mut views,
            &metrics,
            &mut owner_state,
            QueueAcceptCallState::new(1, Some(10), Uint256::from_u64(3)),
            |_queued| {
                apply_calls.set(apply_calls.get() + 1);
                ApplyResult::new(Ter::TES_SUCCESS, true, false)
            },
        );

        assert_eq!(apply_calls.get(), 1);
        assert_eq!(
            result.owner.accept.stage,
            QueueAcceptStageResult {
                ledger_changed: true,
                processed_candidates: 2,
                stop_candidate: Some(FeeQueueKey::new("b", SeqProxy::sequence(7))),
            }
        );
        assert!(views.accounts["a"].empty());
        assert!(
            views.accounts["b"]
                .transactions
                .contains_key(&SeqProxy::sequence(7))
        );
    }

    #[test]
    fn accept_call_state_wrapper_uses_internal_near_full_check() {
        let mut account = TxQAccount::new("acct");
        account.drop_penalty = true;
        account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                queued("acct", SeqProxy::sequence(5), 5, 300),
                TxConsequences::new(1, SeqProxy::sequence(5)),
            ),
        );
        account.add(
            SeqProxy::sequence(9),
            MaybeTxCore::new(
                queued("acct", SeqProxy::sequence(9), 9, 60),
                TxConsequences::new(1, SeqProxy::sequence(9)),
            ),
        );

        let mut views = QueueViews::new(
            BTreeMap::from([("acct", account)]),
            vec![
                FeeQueueEntry::new(
                    FeeQueueKey::new("acct", SeqProxy::sequence(5)),
                    queue_candidate(SeqProxy::sequence(5), 5, 300),
                ),
                FeeQueueEntry::new(
                    FeeQueueKey::new("acct", SeqProxy::sequence(9)),
                    queue_candidate(SeqProxy::sequence(9), 9, 60),
                ),
            ],
        );
        let metrics = metrics_state();
        let mut owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(9));

        let result = run_queue_accept_with_call_state(
            &mut views,
            &metrics,
            &mut owner_state,
            QueueAcceptCallState::new(32, Some(2), Uint256::from_u64(9)),
            |_queued| ApplyResult::new(Ter::TER_RETRY, false, false),
        );

        assert_eq!(
            result.log_messages.loop_messages.info,
            vec![format_queue_accept_drop_last_info_message(
                Uint256::from_u64(5),
                Ter::TER_RETRY,
                "acct",
            )]
        );
        assert!(
            !views.accounts["acct"]
                .transactions
                .contains_key(&SeqProxy::sequence(9))
        );
    }

    #[test]
    fn prepare_accept_call_state_skips_later_sequence_before_returning_ready_apply() {
        let mut account = TxQAccount::new("acct");
        account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                queued("acct", SeqProxy::sequence(5), 5, 300),
                TxConsequences::new(1, SeqProxy::sequence(5)),
            ),
        );
        account.add(
            SeqProxy::sequence(9),
            MaybeTxCore::new(
                queued("acct", SeqProxy::sequence(9), 9, 500),
                TxConsequences::new(1, SeqProxy::sequence(9)),
            ),
        );

        let mut views = QueueViews::new(
            BTreeMap::from([("acct", account)]),
            vec![
                FeeQueueEntry::new(
                    FeeQueueKey::new("acct", SeqProxy::sequence(9)),
                    queue_candidate(SeqProxy::sequence(9), 9, 500),
                ),
                FeeQueueEntry::new(
                    FeeQueueKey::new("acct", SeqProxy::sequence(5)),
                    queue_candidate(SeqProxy::sequence(5), 5, 300),
                ),
            ],
        );
        let metrics = metrics_state();
        let mut owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(11));

        assert_eq!(
            prepare_queue_accept_with_call_state(
                &mut views,
                &metrics,
                &mut owner_state,
                QueueAcceptCallState::new(0, Some(10), Uint256::from_u64(13)),
            ),
            QueueAcceptPreparedCallStep::Ready(PreparedQueueAcceptCall {
                prepared_apply: PreparedQueueAcceptApply {
                    candidate: QueueAcceptCandidate {
                        key: FeeQueueKey::new("acct", SeqProxy::sequence(5)),
                        tx_id: Uint256::from_u64(5),
                        fee_level: 300,
                        retries_remaining: 10,
                        flags: ApplyFlags::NONE,
                    },
                    required_fee_level: 256,
                    queue_nearly_full: false,
                    candidate_index: 1,
                    account_retry_penalty: false,
                    account_drop_penalty: false,
                    account_txn_count: 2,
                    order: OrderCandidates::new(Uint256::from_u64(11)),
                },
                metrics_snapshot: metrics.snapshot(),
                call_state: QueueAcceptCallState::new(0, Some(10), Uint256::from_u64(13)),
                previous_parent_hash_comp: Uint256::from_u64(11),
                loop_messages: QueueAcceptLoopLogMessages {
                    trace: vec![
                        format_queue_accept_skip_not_first_trace_message(
                            Uint256::from_u64(9),
                            "acct",
                        ),
                        format_queue_accept_fee_trace_message(
                            Uint256::from_u64(5),
                            "acct",
                            300,
                            256,
                        ),
                        format_queue_accept_apply_trace_message(Uint256::from_u64(5)),
                    ],
                    debug: vec![],
                    info: vec![],
                },
                ledger_changed: false,
                processed_candidates: 2,
                applied_count: 0,
            })
        );
    }

    #[test]
    fn prepared_accept_call_state_resume_matches_direct_wrapper_behavior() {
        let build_views = || {
            let mut account_a = TxQAccount::new("a");
            account_a.add(
                SeqProxy::sequence(5),
                MaybeTxCore::new(
                    queued("a", SeqProxy::sequence(5), 5, 5_000),
                    TxConsequences::new(1, SeqProxy::sequence(5)),
                ),
            );

            let mut account_b = TxQAccount::new("b");
            account_b.add(
                SeqProxy::sequence(7),
                MaybeTxCore::new(
                    queued("b", SeqProxy::sequence(7), 7, 1_000),
                    TxConsequences::new(1, SeqProxy::sequence(7)),
                ),
            );

            QueueViews::new(
                BTreeMap::from([("a", account_a), ("b", account_b)]),
                vec![
                    FeeQueueEntry::new(
                        FeeQueueKey::new("a", SeqProxy::sequence(5)),
                        queue_candidate(SeqProxy::sequence(5), 5, 5_000),
                    ),
                    FeeQueueEntry::new(
                        FeeQueueKey::new("b", SeqProxy::sequence(7)),
                        queue_candidate(SeqProxy::sequence(7), 7, 1_000),
                    ),
                ],
            )
        };

        let metrics = metrics_state_with(1, 1_000);
        let call_state = QueueAcceptCallState::new(1, Some(10), Uint256::from_u64(3));

        let mut prepared_views = build_views();
        let mut prepared_owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(0));
        let prepared = match prepare_queue_accept_with_call_state(
            &mut prepared_views,
            &metrics,
            &mut prepared_owner_state,
            call_state,
        ) {
            QueueAcceptPreparedCallStep::Ready(prepared) => prepared,
            QueueAcceptPreparedCallStep::Complete(result) => {
                panic!("expected ready accept step, got {result:?}")
            }
        };
        let prepared_final = match resume_prepared_queue_accept_with_call_state(
            &mut prepared_views,
            &mut prepared_owner_state,
            prepared,
            ApplyResult::new(Ter::TES_SUCCESS, true, false),
        ) {
            QueueAcceptPreparedCallStep::Complete(result) => result,
            QueueAcceptPreparedCallStep::Ready(next) => {
                panic!("expected complete result, got {next:?}")
            }
        };

        let mut direct_views = build_views();
        let mut direct_owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(0));
        let direct_result = run_queue_accept_with_call_state(
            &mut direct_views,
            &metrics,
            &mut direct_owner_state,
            call_state,
            |_queued| ApplyResult::new(Ter::TES_SUCCESS, true, false),
        );

        assert_eq!(prepared_final, direct_result);
        assert_eq!(prepared_views, direct_views);
        assert_eq!(prepared_owner_state, direct_owner_state);
    }

    #[test]
    fn caller_prepared_accept_wrapper_matches_direct_call_state_behavior() {
        let build_views = || {
            let mut account_a = TxQAccount::new("a");
            account_a.add(
                SeqProxy::sequence(5),
                MaybeTxCore::new(
                    queued("a", SeqProxy::sequence(5), 5, 5_000),
                    TxConsequences::new(1, SeqProxy::sequence(5)),
                ),
            );

            let mut account_b = TxQAccount::new("b");
            account_b.add(
                SeqProxy::sequence(7),
                MaybeTxCore::new(
                    queued("b", SeqProxy::sequence(7), 7, 1_000),
                    TxConsequences::new(1, SeqProxy::sequence(7)),
                ),
            );

            QueueViews::new(
                BTreeMap::from([("a", account_a), ("b", account_b)]),
                vec![
                    FeeQueueEntry::new(
                        FeeQueueKey::new("a", SeqProxy::sequence(5)),
                        queue_candidate(SeqProxy::sequence(5), 5, 5_000),
                    ),
                    FeeQueueEntry::new(
                        FeeQueueKey::new("b", SeqProxy::sequence(7)),
                        queue_candidate(SeqProxy::sequence(7), 7, 1_000),
                    ),
                ],
            )
        };

        let metrics = metrics_state_with(1, 1_000);
        let call_state = QueueAcceptCallState::new(1, Some(10), Uint256::from_u64(3));

        let mut wrapped_views = build_views();
        let mut wrapped_owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(0));
        let seen_prepared_tx_ids = RefCell::new(Vec::new());
        let wrapped_result = run_queue_accept_with_caller_prepared_apply(
            &mut wrapped_views,
            &metrics,
            &mut wrapped_owner_state,
            call_state,
            |views, owner_state, prepared| {
                seen_prepared_tx_ids
                    .borrow_mut()
                    .push(prepared.prepared_apply.candidate.tx_id);
                run_prepared_queue_accept_call(views, owner_state, prepared, |_queued| {
                    ApplyResult::new(Ter::TES_SUCCESS, true, false)
                })
            },
        );

        let mut direct_views = build_views();
        let mut direct_owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(0));
        let direct_result = run_queue_accept_with_call_state(
            &mut direct_views,
            &metrics,
            &mut direct_owner_state,
            call_state,
            |_queued| ApplyResult::new(Ter::TES_SUCCESS, true, false),
        );

        assert_eq!(wrapped_result, direct_result);
        assert_eq!(wrapped_views, direct_views);
        assert_eq!(wrapped_owner_state, direct_owner_state);
        assert_eq!(
            seen_prepared_tx_ids.into_inner(),
            vec![Uint256::from_u64(5)]
        );
    }

    #[test]
    fn caller_prepared_accept_sink_wrapper_matches_direct_call_state_sink_behavior() {
        let mut account = TxQAccount::new("acct");
        account.drop_penalty = true;
        account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                queued("acct", SeqProxy::sequence(5), 5, 300),
                TxConsequences::new(1, SeqProxy::sequence(5)),
            ),
        );
        account.add(
            SeqProxy::sequence(9),
            MaybeTxCore::new(
                queued("acct", SeqProxy::sequence(9), 9, 60),
                TxConsequences::new(1, SeqProxy::sequence(9)),
            ),
        );

        let build_views = || {
            QueueViews::new(
                BTreeMap::from([("acct", account.clone())]),
                vec![
                    FeeQueueEntry::new(
                        FeeQueueKey::new("acct", SeqProxy::sequence(5)),
                        queue_candidate(SeqProxy::sequence(5), 5, 300),
                    ),
                    FeeQueueEntry::new(
                        FeeQueueKey::new("acct", SeqProxy::sequence(9)),
                        queue_candidate(SeqProxy::sequence(9), 9, 60),
                    ),
                ],
            )
        };

        let metrics = metrics_state();
        let call_state = QueueAcceptCallState::new(32, Some(2), Uint256::from_u64(9));

        let mut wrapped_views = build_views();
        let mut wrapped_owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(9));
        let wrapped_emitted = RefCell::new(Vec::new());
        let wrapped_result = run_queue_accept_with_caller_prepared_apply_and_log_sinks(
            &mut wrapped_views,
            &metrics,
            &mut wrapped_owner_state,
            call_state,
            |message| {
                wrapped_emitted
                    .borrow_mut()
                    .push(format!("trace:{message}"))
            },
            |message| {
                wrapped_emitted
                    .borrow_mut()
                    .push(format!("debug:{message}"))
            },
            |message| wrapped_emitted.borrow_mut().push(format!("info:{message}")),
            |message| wrapped_emitted.borrow_mut().push(format!("warn:{message}")),
            |views, owner_state, prepared| {
                run_prepared_queue_accept_call(views, owner_state, prepared, |_queued| {
                    ApplyResult::new(Ter::TER_RETRY, false, false)
                })
            },
        );

        let mut direct_views = build_views();
        let mut direct_owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(9));
        let direct_emitted = RefCell::new(Vec::new());
        let direct_result = run_queue_accept_with_call_state_and_log_sinks(
            &mut direct_views,
            &metrics,
            &mut direct_owner_state,
            call_state,
            |message| direct_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| direct_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| direct_emitted.borrow_mut().push(format!("info:{message}")),
            |message| direct_emitted.borrow_mut().push(format!("warn:{message}")),
            |_queued| ApplyResult::new(Ter::TER_RETRY, false, false),
        );

        assert_eq!(wrapped_result, direct_result);
        assert_eq!(wrapped_views, direct_views);
        assert_eq!(wrapped_owner_state, direct_owner_state);
        assert_eq!(wrapped_emitted.into_inner(), direct_emitted.into_inner());
    }

    #[test]
    fn accept_call_state_sink_wrapper_matches_collected_output() {
        let mut account = TxQAccount::new("acct");
        account.drop_penalty = true;
        account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                queued("acct", SeqProxy::sequence(5), 5, 300),
                TxConsequences::new(1, SeqProxy::sequence(5)),
            ),
        );
        account.add(
            SeqProxy::sequence(9),
            MaybeTxCore::new(
                queued("acct", SeqProxy::sequence(9), 9, 60),
                TxConsequences::new(1, SeqProxy::sequence(9)),
            ),
        );

        let mut views = QueueViews::new(
            BTreeMap::from([("acct", account)]),
            vec![
                FeeQueueEntry::new(
                    FeeQueueKey::new("acct", SeqProxy::sequence(5)),
                    queue_candidate(SeqProxy::sequence(5), 5, 300),
                ),
                FeeQueueEntry::new(
                    FeeQueueKey::new("acct", SeqProxy::sequence(9)),
                    queue_candidate(SeqProxy::sequence(9), 9, 60),
                ),
            ],
        );
        let metrics = metrics_state();
        let mut owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(9));
        let emitted = RefCell::new(Vec::new());

        let result = run_queue_accept_with_call_state_and_log_sinks(
            &mut views,
            &metrics,
            &mut owner_state,
            QueueAcceptCallState::new(32, Some(2), Uint256::from_u64(9)),
            |message| emitted.borrow_mut().push(format!("trace:{message}")),
            |message| emitted.borrow_mut().push(format!("debug:{message}")),
            |message| emitted.borrow_mut().push(format!("info:{message}")),
            |message| emitted.borrow_mut().push(format!("warn:{message}")),
            |_queued| ApplyResult::new(Ter::TER_RETRY, false, false),
        );

        assert_eq!(
            emitted.into_inner(),
            vec![
                format!(
                    "trace:{}",
                    format_queue_accept_fee_trace_message(Uint256::from_u64(5), "acct", 300, 256,)
                ),
                format!(
                    "trace:{}",
                    format_queue_accept_apply_trace_message(Uint256::from_u64(5))
                ),
                format!(
                    "debug:{}",
                    format_queue_accept_leave_in_queue_debug_message(
                        Uint256::from_u64(5),
                        Ter::TER_RETRY,
                        false,
                        ApplyFlags::NONE,
                    )
                ),
                format!(
                    "info:{}",
                    format_queue_accept_drop_last_info_message(
                        Uint256::from_u64(5),
                        Ter::TER_RETRY,
                        "acct",
                    )
                ),
                format!(
                    "warn:{}",
                    format_queue_accept_parent_hash_unchanged_warning(Uint256::from_u64(9))
                ),
            ]
        );
        assert_eq!(
            result.log_messages.warning,
            Some(format_queue_accept_parent_hash_unchanged_warning(
                Uint256::from_u64(9)
            ))
        );
    }
}
