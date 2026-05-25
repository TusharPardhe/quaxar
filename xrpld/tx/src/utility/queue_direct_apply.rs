//! Deterministic admission half of `TxQ::tryDirectApply(...)`.
//!
//! This ports only the account-present, sequence-match, and fee-threshold
//! gating logic before `xrpl::apply(...)`, plus the first wrapper layers
//! immediately around a supplied apply result.

use std::fmt::Display;

use protocol::{SeqProxy, trans_token};

use crate::{ApplyResult, FeeLevel64, FeeQueueKey, QueueViews};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectApplyEligibility {
    MissingAccount,
    SequenceMismatch,
    InsufficientFee,
    Eligible,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectApplyAttemptResult<Account> {
    pub apply_result: ApplyResult,
    pub removed_replacement: Option<FeeQueueKey<Account>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectApplyExecution<Account, TxId> {
    pub transaction_id: TxId,
    pub attempt: DirectApplyAttemptResult<Account>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectApplyLogMessages {
    pub start: String,
    pub finish: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedDirectApply<'a, Account, TxId> {
    pub transaction_id: TxId,
    pub applied_account: &'a Account,
    pub applied_seq_proxy: SeqProxy,
}

pub fn format_direct_apply_start_log_message<TxId>(transaction_id: TxId) -> String
where
    TxId: Display,
{
    format!("Applying transaction {} to open ledger.", transaction_id)
}

pub fn format_direct_apply_finish_log_message<Account, TxId>(
    execution: &DirectApplyExecution<Account, TxId>,
) -> String
where
    TxId: Display,
{
    let finish_verb = if execution.attempt.apply_result.applied {
        "applied successfully with"
    } else {
        "failed with"
    };

    format!(
        "New transaction {} {} {}",
        execution.transaction_id,
        finish_verb,
        trans_token(execution.attempt.apply_result.ter)
    )
}

pub fn evaluate_direct_apply_eligibility(
    account_exists: bool,
    account_seq_proxy: SeqProxy,
    tx_seq_proxy: SeqProxy,
    fee_level_paid: FeeLevel64,
    required_fee_level: FeeLevel64,
) -> DirectApplyEligibility {
    if !account_exists {
        return DirectApplyEligibility::MissingAccount;
    }

    if tx_seq_proxy.is_seq() && tx_seq_proxy != account_seq_proxy {
        return DirectApplyEligibility::SequenceMismatch;
    }

    if fee_level_paid < required_fee_level {
        return DirectApplyEligibility::InsufficientFee;
    }

    DirectApplyEligibility::Eligible
}

pub fn prepare_direct_apply_if_eligible<'a, Account, TxId>(
    transaction_id: TxId,
    account_exists: bool,
    account_seq_proxy: SeqProxy,
    tx_seq_proxy: SeqProxy,
    fee_level_paid: FeeLevel64,
    required_fee_level: FeeLevel64,
    applied_account: &'a Account,
) -> Option<PreparedDirectApply<'a, Account, TxId>> {
    let eligibility = evaluate_direct_apply_eligibility(
        account_exists,
        account_seq_proxy,
        tx_seq_proxy,
        fee_level_paid,
        required_fee_level,
    );

    (eligibility == DirectApplyEligibility::Eligible).then_some(PreparedDirectApply {
        transaction_id,
        applied_account,
        applied_seq_proxy: tx_seq_proxy,
    })
}

pub fn finalize_direct_apply_attempt<Account, T>(
    views: &mut QueueViews<Account, T>,
    eligibility: DirectApplyEligibility,
    applied_account: &Account,
    applied_seq_proxy: SeqProxy,
    apply_result: ApplyResult,
) -> Option<DirectApplyAttemptResult<Account>>
where
    Account: Clone + Ord + PartialEq,
{
    if eligibility != DirectApplyEligibility::Eligible {
        return None;
    }

    let removed_replacement = apply_result
        .applied
        .then(|| {
            views
                .cleanup_direct_apply_success(applied_account, applied_seq_proxy)
                .removed
        })
        .flatten();

    Some(DirectApplyAttemptResult {
        apply_result,
        removed_replacement,
    })
}

pub fn finalize_prepared_direct_apply<Account, T, TxId>(
    views: &mut QueueViews<Account, T>,
    prepared: PreparedDirectApply<'_, Account, TxId>,
    apply_result: ApplyResult,
) -> DirectApplyExecution<Account, TxId>
where
    Account: Clone + Ord + PartialEq,
{
    let attempt = finalize_direct_apply_attempt(
        views,
        DirectApplyEligibility::Eligible,
        prepared.applied_account,
        prepared.applied_seq_proxy,
        apply_result,
    )
    .expect("prepared direct apply must stay eligible");

    DirectApplyExecution {
        transaction_id: prepared.transaction_id,
        attempt,
    }
}

pub fn run_prepared_direct_apply<Account, T, TxId, ApplyFn>(
    views: &mut QueueViews<Account, T>,
    prepared: PreparedDirectApply<'_, Account, TxId>,
    apply: ApplyFn,
) -> DirectApplyExecution<Account, TxId>
where
    Account: Clone + Ord + PartialEq,
    ApplyFn: FnOnce() -> ApplyResult,
{
    finalize_prepared_direct_apply(views, prepared, apply())
}

pub fn run_prepared_direct_apply_with_trace<Account, T, TxId, TraceFn, ApplyFn>(
    views: &mut QueueViews<Account, T>,
    prepared: PreparedDirectApply<'_, Account, TxId>,
    mut trace: TraceFn,
    apply: ApplyFn,
) -> DirectApplyExecution<Account, TxId>
where
    Account: Clone + Ord + PartialEq,
    TxId: Clone + Display,
    TraceFn: FnMut(&str),
    ApplyFn: FnOnce() -> ApplyResult,
{
    let start = format_direct_apply_start_log_message(prepared.transaction_id.clone());
    trace(&start);

    let execution = run_prepared_direct_apply(views, prepared, apply);

    let result = trans_token(execution.attempt.apply_result.ter);
    tracing::debug!(target: "tx", tx_type = "direct", hash = %execution.transaction_id, result = %result, "Transaction processed");

    let finish = format_direct_apply_finish_log_message(&execution);
    trace(&finish);

    execution
}

pub fn run_direct_apply_if_eligible<Account, T, ApplyFn>(
    views: &mut QueueViews<Account, T>,
    account_exists: bool,
    account_seq_proxy: SeqProxy,
    tx_seq_proxy: SeqProxy,
    fee_level_paid: FeeLevel64,
    required_fee_level: FeeLevel64,
    applied_account: &Account,
    apply: ApplyFn,
) -> Option<DirectApplyAttemptResult<Account>>
where
    Account: Clone + Ord + PartialEq,
    ApplyFn: FnOnce() -> ApplyResult,
{
    prepare_direct_apply_if_eligible(
        (),
        account_exists,
        account_seq_proxy,
        tx_seq_proxy,
        fee_level_paid,
        required_fee_level,
        applied_account,
    )
    .map(|prepared| run_prepared_direct_apply(views, prepared, apply).attempt)
}

pub fn run_direct_apply_with_tx_id_if_eligible<Account, T, TxId, ApplyFn>(
    views: &mut QueueViews<Account, T>,
    transaction_id: TxId,
    account_exists: bool,
    account_seq_proxy: SeqProxy,
    tx_seq_proxy: SeqProxy,
    fee_level_paid: FeeLevel64,
    required_fee_level: FeeLevel64,
    applied_account: &Account,
    apply: ApplyFn,
) -> Option<DirectApplyExecution<Account, TxId>>
where
    Account: Clone + Ord + PartialEq,
    ApplyFn: FnOnce() -> ApplyResult,
{
    prepare_direct_apply_if_eligible(
        transaction_id,
        account_exists,
        account_seq_proxy,
        tx_seq_proxy,
        fee_level_paid,
        required_fee_level,
        applied_account,
    )
    .map(|prepared| run_prepared_direct_apply(views, prepared, apply))
}

pub fn run_direct_apply_with_trace_if_eligible<Account, T, TxId, TraceFn, ApplyFn>(
    views: &mut QueueViews<Account, T>,
    transaction_id: TxId,
    account_exists: bool,
    account_seq_proxy: SeqProxy,
    tx_seq_proxy: SeqProxy,
    fee_level_paid: FeeLevel64,
    required_fee_level: FeeLevel64,
    applied_account: &Account,
    trace: TraceFn,
    apply: ApplyFn,
) -> Option<DirectApplyExecution<Account, TxId>>
where
    Account: Clone + Ord + PartialEq,
    TxId: Clone + Display,
    TraceFn: FnMut(&str),
    ApplyFn: FnOnce() -> ApplyResult,
{
    prepare_direct_apply_if_eligible(
        transaction_id,
        account_exists,
        account_seq_proxy,
        tx_seq_proxy,
        fee_level_paid,
        required_fee_level,
        applied_account,
    )
    .map(|prepared| run_prepared_direct_apply_with_trace(views, prepared, trace, apply))
}

pub fn format_direct_apply_log_messages<Account, TxId>(
    execution: &DirectApplyExecution<Account, TxId>,
) -> DirectApplyLogMessages
where
    TxId: Display,
{
    DirectApplyLogMessages {
        start: format_direct_apply_start_log_message(&execution.transaction_id),
        finish: format_direct_apply_finish_log_message(execution),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    use basics::base_uint::Uint256;
    use protocol::SeqProxy;
    use protocol::Ter;

    use super::{
        DirectApplyAttemptResult, DirectApplyEligibility, DirectApplyExecution,
        DirectApplyLogMessages, PreparedDirectApply, evaluate_direct_apply_eligibility,
        finalize_direct_apply_attempt, finalize_prepared_direct_apply,
        format_direct_apply_finish_log_message, format_direct_apply_log_messages,
        format_direct_apply_start_log_message, prepare_direct_apply_if_eligible,
        run_direct_apply_if_eligible, run_direct_apply_with_trace_if_eligible,
        run_direct_apply_with_tx_id_if_eligible, run_prepared_direct_apply_with_trace,
    };
    use crate::{
        ApplyResult, FeeQueueEntry, FeeQueueKey, MaybeTxCore, QueueAdvanceCandidate, QueueViews,
        TxConsequences, TxQAccount,
    };

    fn candidate(seq_proxy: SeqProxy, tx_id: u64, fee_level: u64) -> QueueAdvanceCandidate {
        QueueAdvanceCandidate {
            fee_level,
            tx_id: Uint256::from_u64(tx_id),
            seq_proxy,
        }
    }

    #[test]
    fn direct_apply_requires_account_to_exist_in_the_ledger() {
        let result = evaluate_direct_apply_eligibility(
            false,
            SeqProxy::sequence(5),
            SeqProxy::sequence(5),
            100,
            100,
        );

        assert_eq!(result, DirectApplyEligibility::MissingAccount);
    }

    #[test]
    fn direct_apply_requires_sequence_transactions_to_match_account_sequence() {
        let result = evaluate_direct_apply_eligibility(
            true,
            SeqProxy::sequence(5),
            SeqProxy::sequence(6),
            100,
            100,
        );

        assert_eq!(result, DirectApplyEligibility::SequenceMismatch);
    }

    #[test]
    fn direct_apply_tickets_bypass_sequence_match_gate() {
        let result = evaluate_direct_apply_eligibility(
            true,
            SeqProxy::sequence(5),
            SeqProxy::ticket(9),
            100,
            100,
        );

        assert_eq!(result, DirectApplyEligibility::Eligible);
    }

    #[test]
    fn direct_apply_requires_fee_level_to_meet_the_current_threshold() {
        let result = evaluate_direct_apply_eligibility(
            true,
            SeqProxy::sequence(5),
            SeqProxy::sequence(5),
            99,
            100,
        );

        assert_eq!(result, DirectApplyEligibility::InsufficientFee);
    }

    #[test]
    fn direct_apply_is_eligible_when_all_current_gates_pass() {
        let result = evaluate_direct_apply_eligibility(
            true,
            SeqProxy::sequence(5),
            SeqProxy::sequence(5),
            100,
            100,
        );

        assert_eq!(result, DirectApplyEligibility::Eligible);
    }

    #[test]
    fn finalize_direct_apply_attempt_returns_none_for_ineligible_attempts() {
        let mut account_a = TxQAccount::new("a");
        account_a.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new("a5", TxConsequences::new(1, SeqProxy::sequence(5))),
        );
        let mut views = QueueViews::new(
            BTreeMap::from([("a", account_a)]),
            vec![FeeQueueEntry::new(
                FeeQueueKey::new("a", SeqProxy::sequence(5)),
                candidate(SeqProxy::sequence(5), 5, 100),
            )],
        );

        let result = finalize_direct_apply_attempt(
            &mut views,
            DirectApplyEligibility::InsufficientFee,
            &"a",
            SeqProxy::sequence(5),
            ApplyResult::new(Ter::TES_SUCCESS, true, true),
        );

        assert_eq!(result, None);
        assert_eq!(views.fee_order.len(), 1);
        assert_eq!(views.accounts["a"].get_txn_count(), 1);
    }

    #[test]
    fn finalize_direct_apply_attempt_cleans_up_replacement_only_when_apply_succeeds() {
        let mut success_account = TxQAccount::new("a");
        success_account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new("a5", TxConsequences::new(1, SeqProxy::sequence(5))),
        );
        let mut success_views = QueueViews::new(
            BTreeMap::from([("a", success_account)]),
            vec![FeeQueueEntry::new(
                FeeQueueKey::new("a", SeqProxy::sequence(5)),
                candidate(SeqProxy::sequence(5), 5, 100),
            )],
        );

        let success = finalize_direct_apply_attempt(
            &mut success_views,
            DirectApplyEligibility::Eligible,
            &"a",
            SeqProxy::sequence(5),
            ApplyResult::new(Ter::TES_SUCCESS, true, true),
        );

        assert_eq!(
            success,
            Some(DirectApplyAttemptResult {
                apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, true),
                removed_replacement: Some(FeeQueueKey::new("a", SeqProxy::sequence(5))),
            })
        );
        assert!(success_views.fee_order.is_empty());
        assert!(success_views.accounts["a"].empty());

        let mut failure_account = TxQAccount::new("a");
        failure_account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new("a5", TxConsequences::new(1, SeqProxy::sequence(5))),
        );
        let mut failure_views = QueueViews::new(
            BTreeMap::from([("a", failure_account)]),
            vec![FeeQueueEntry::new(
                FeeQueueKey::new("a", SeqProxy::sequence(5)),
                candidate(SeqProxy::sequence(5), 5, 100),
            )],
        );

        let failure = finalize_direct_apply_attempt(
            &mut failure_views,
            DirectApplyEligibility::Eligible,
            &"a",
            SeqProxy::sequence(5),
            ApplyResult::new(Ter::TEF_EXCEPTION, false, false),
        );

        assert_eq!(
            failure,
            Some(DirectApplyAttemptResult {
                apply_result: ApplyResult::new(Ter::TEF_EXCEPTION, false, false),
                removed_replacement: None,
            })
        );
        assert_eq!(failure_views.fee_order.len(), 1);
        assert_eq!(failure_views.accounts["a"].get_txn_count(), 1);
    }

    #[test]
    fn run_direct_apply_if_eligible_skips_the_apply_step_when_gates_fail() {
        let invoked = Cell::new(false);
        let mut views = QueueViews::<&str, &str>::default();

        let result = run_direct_apply_if_eligible(
            &mut views,
            false,
            SeqProxy::sequence(5),
            SeqProxy::sequence(5),
            100,
            100,
            &"a",
            || {
                invoked.set(true);
                ApplyResult::new(Ter::TES_SUCCESS, true, true)
            },
        );

        assert_eq!(result, None);
        assert!(!invoked.get());
    }

    #[test]
    fn prepare_direct_apply_if_eligible_returns_execution_token_only_for_eligible_paths() {
        let account = "a";

        let prepared = prepare_direct_apply_if_eligible(
            "ABC123",
            true,
            SeqProxy::sequence(5),
            SeqProxy::sequence(5),
            200,
            100,
            &account,
        );

        assert_eq!(
            prepared,
            Some(PreparedDirectApply {
                transaction_id: "ABC123",
                applied_account: &account,
                applied_seq_proxy: SeqProxy::sequence(5),
            })
        );

        let rejected = prepare_direct_apply_if_eligible(
            "ABC123",
            false,
            SeqProxy::sequence(5),
            SeqProxy::sequence(5),
            200,
            100,
            &account,
        );
        assert_eq!(rejected, None);
    }

    #[test]
    fn run_direct_apply_if_eligible_invokes_apply_once_and_routes_cleanup() {
        let invoked = Cell::new(0_u32);
        let mut account = TxQAccount::new("a");
        account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new("a5", TxConsequences::new(1, SeqProxy::sequence(5))),
        );
        let mut views = QueueViews::new(
            BTreeMap::from([("a", account)]),
            vec![FeeQueueEntry::new(
                FeeQueueKey::new("a", SeqProxy::sequence(5)),
                candidate(SeqProxy::sequence(5), 5, 100),
            )],
        );

        let result = run_direct_apply_if_eligible(
            &mut views,
            true,
            SeqProxy::sequence(5),
            SeqProxy::sequence(5),
            100,
            100,
            &"a",
            || {
                invoked.set(invoked.get() + 1);
                ApplyResult::new(Ter::TES_SUCCESS, true, true)
            },
        );

        assert_eq!(invoked.get(), 1);
        assert_eq!(
            result,
            Some(DirectApplyAttemptResult {
                apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, true),
                removed_replacement: Some(FeeQueueKey::new("a", SeqProxy::sequence(5))),
            })
        );
        assert!(views.fee_order.is_empty());
        assert!(views.accounts["a"].empty());
    }

    #[test]
    fn finalize_prepared_direct_apply_runs_cleanup_after_caller_owned_apply_result() {
        let mut account = TxQAccount::new("a");
        account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new("a5", TxConsequences::new(1, SeqProxy::sequence(5))),
        );
        let mut views = QueueViews::new(
            BTreeMap::from([("a", account)]),
            vec![FeeQueueEntry::new(
                FeeQueueKey::new("a", SeqProxy::sequence(5)),
                QueueAdvanceCandidate {
                    fee_level: 100,
                    tx_id: Uint256::from_u64(5),
                    seq_proxy: SeqProxy::sequence(5),
                },
            )],
        );

        let execution = finalize_prepared_direct_apply(
            &mut views,
            PreparedDirectApply {
                transaction_id: "ABC123",
                applied_account: &"a",
                applied_seq_proxy: SeqProxy::sequence(5),
            },
            ApplyResult::new(Ter::TES_SUCCESS, true, true),
        );

        assert_eq!(
            execution,
            DirectApplyExecution {
                transaction_id: "ABC123",
                attempt: DirectApplyAttemptResult {
                    apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, true),
                    removed_replacement: Some(FeeQueueKey::new("a", SeqProxy::sequence(5))),
                },
            }
        );
        assert!(views.fee_order.is_empty());
        assert!(views.accounts["a"].empty());
    }

    #[test]
    fn run_prepared_direct_apply_with_trace_leaves_apply_execution_to_the_caller_step() {
        let mut account = TxQAccount::new("a");
        account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new("a5", TxConsequences::new(1, SeqProxy::sequence(5))),
        );
        let mut views = QueueViews::new(
            BTreeMap::from([("a", account)]),
            vec![FeeQueueEntry::new(
                FeeQueueKey::new("a", SeqProxy::sequence(5)),
                QueueAdvanceCandidate {
                    fee_level: 100,
                    tx_id: Uint256::from_u64(5),
                    seq_proxy: SeqProxy::sequence(5),
                },
            )],
        );
        let applied = Cell::new(false);
        let trace_messages = RefCell::new(Vec::new());

        let execution = run_prepared_direct_apply_with_trace(
            &mut views,
            PreparedDirectApply {
                transaction_id: "ABC123",
                applied_account: &"a",
                applied_seq_proxy: SeqProxy::sequence(5),
            },
            |message| trace_messages.borrow_mut().push(message.to_string()),
            || {
                applied.set(true);
                assert_eq!(
                    trace_messages.borrow().as_slice(),
                    ["Applying transaction ABC123 to open ledger."]
                );
                ApplyResult::new(Ter::TES_SUCCESS, true, true)
            },
        );

        assert!(applied.get());
        assert_eq!(
            execution,
            DirectApplyExecution {
                transaction_id: "ABC123",
                attempt: DirectApplyAttemptResult {
                    apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, true),
                    removed_replacement: Some(FeeQueueKey::new("a", SeqProxy::sequence(5))),
                },
            }
        );
        assert_eq!(
            trace_messages.into_inner(),
            vec![
                "Applying transaction ABC123 to open ledger.".to_string(),
                "New transaction ABC123 applied successfully with tesSUCCESS".to_string(),
            ]
        );
    }

    #[test]
    fn run_direct_apply_with_tx_id_if_eligible_preserves_transaction_identity() {
        let mut account = TxQAccount::new("a");
        account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new("a5", TxConsequences::new(1, SeqProxy::sequence(5))),
        );
        let mut views = QueueViews::new(
            BTreeMap::from([("a", account)]),
            vec![FeeQueueEntry::new(
                FeeQueueKey::new("a", SeqProxy::sequence(5)),
                candidate(SeqProxy::sequence(5), 5, 100),
            )],
        );

        let result = run_direct_apply_with_tx_id_if_eligible(
            &mut views,
            "tx-5",
            true,
            SeqProxy::sequence(5),
            SeqProxy::sequence(5),
            100,
            100,
            &"a",
            || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        );

        assert_eq!(
            result,
            Some(DirectApplyExecution {
                transaction_id: "tx-5",
                attempt: DirectApplyAttemptResult {
                    apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, true),
                    removed_replacement: Some(FeeQueueKey::new("a", SeqProxy::sequence(5))),
                },
            })
        );
    }

    #[test]
    fn format_direct_apply_start_log_message_matches_current_cpp_wording_shape() {
        assert_eq!(
            format_direct_apply_start_log_message("ABC123"),
            "Applying transaction ABC123 to open ledger."
        );
    }

    #[test]
    fn format_direct_apply_finish_log_message_matches_current_cpp_wording_shape() {
        let success = format_direct_apply_finish_log_message(&DirectApplyExecution {
            transaction_id: "ABC123",
            attempt: DirectApplyAttemptResult::<&str> {
                apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, true),
                removed_replacement: None,
            },
        });
        assert_eq!(
            success,
            "New transaction ABC123 applied successfully with tesSUCCESS"
        );

        let failure = format_direct_apply_finish_log_message(&DirectApplyExecution {
            transaction_id: "ABC123",
            attempt: DirectApplyAttemptResult::<&str> {
                apply_result: ApplyResult::new(Ter::TEF_EXCEPTION, false, false),
                removed_replacement: None,
            },
        });
        assert_eq!(failure, "New transaction ABC123 failed with tefEXCEPTION");
    }

    #[test]
    fn run_direct_apply_with_trace_if_eligible_emitsed_trace_messages() {
        let mut account = TxQAccount::new("a");
        account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new("a5", TxConsequences::new(1, SeqProxy::sequence(5))),
        );
        let mut views = QueueViews::new(
            BTreeMap::from([("a", account)]),
            vec![FeeQueueEntry::new(
                FeeQueueKey::new("a", SeqProxy::sequence(5)),
                candidate(SeqProxy::sequence(5), 5, 100),
            )],
        );
        let trace_messages = RefCell::new(Vec::new());

        let execution = run_direct_apply_with_trace_if_eligible(
            &mut views,
            "ABC123",
            true,
            SeqProxy::sequence(5),
            SeqProxy::sequence(5),
            100,
            100,
            &"a",
            |message| trace_messages.borrow_mut().push(message.to_string()),
            || {
                assert_eq!(
                    trace_messages.borrow().as_slice(),
                    ["Applying transaction ABC123 to open ledger."]
                );
                ApplyResult::new(Ter::TES_SUCCESS, true, true)
            },
        );

        assert_eq!(
            execution,
            Some(DirectApplyExecution {
                transaction_id: "ABC123",
                attempt: DirectApplyAttemptResult {
                    apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, true),
                    removed_replacement: Some(FeeQueueKey::new("a", SeqProxy::sequence(5))),
                },
            })
        );
        assert_eq!(
            trace_messages.into_inner(),
            vec![
                "Applying transaction ABC123 to open ledger.".to_string(),
                "New transaction ABC123 applied successfully with tesSUCCESS".to_string(),
            ]
        );
    }

    #[test]
    fn run_direct_apply_with_trace_if_eligible_skips_trace_when_gates_fail() {
        let invoked = Cell::new(false);
        let trace_messages = RefCell::new(Vec::new());
        let mut views = QueueViews::<&str, &str>::default();

        let result = run_direct_apply_with_trace_if_eligible(
            &mut views,
            "ABC123",
            false,
            SeqProxy::sequence(5),
            SeqProxy::sequence(5),
            100,
            100,
            &"a",
            |message| trace_messages.borrow_mut().push(message.to_string()),
            || {
                invoked.set(true);
                ApplyResult::new(Ter::TES_SUCCESS, true, true)
            },
        );

        assert_eq!(result, None);
        assert!(!invoked.get());
        assert!(trace_messages.into_inner().is_empty());
    }

    #[test]
    fn format_direct_apply_log_messages_matches_current_cpp_wording_shape() {
        let success = format_direct_apply_log_messages(&DirectApplyExecution {
            transaction_id: "ABC123",
            attempt: DirectApplyAttemptResult::<&str> {
                apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, true),
                removed_replacement: None,
            },
        });
        assert_eq!(
            success,
            DirectApplyLogMessages {
                start: "Applying transaction ABC123 to open ledger.".to_string(),
                finish: "New transaction ABC123 applied successfully with tesSUCCESS".to_string(),
            }
        );

        let failure = format_direct_apply_log_messages(&DirectApplyExecution {
            transaction_id: "ABC123",
            attempt: DirectApplyAttemptResult::<&str> {
                apply_result: ApplyResult::new(Ter::TEF_EXCEPTION, false, false),
                removed_replacement: None,
            },
        });
        assert_eq!(
            failure,
            DirectApplyLogMessages {
                start: "Applying transaction ABC123 to open ledger.".to_string(),
                finish: "New transaction ABC123 failed with tefEXCEPTION".to_string(),
            }
        );
    }
}
