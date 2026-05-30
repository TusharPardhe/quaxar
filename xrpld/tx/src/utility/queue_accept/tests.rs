use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

use basics::base_uint::Uint256;
use protocol::{Rules, SeqProxy, Ter};

use super::{
    PreparedQueueAcceptApply, PreparedQueueAcceptCall, QueueAcceptCallState, QueueAcceptCandidate,
    QueueAcceptIteration, QueueAcceptLogMessages, QueueAcceptLoopLogMessages,
    QueueAcceptOwnerResult, QueueAcceptOwnerState, QueueAcceptPreparedCallStep,
    QueueAcceptPreparedIteration, QueueAcceptRebuildResult, QueueAcceptStageResult,
    QueueAcceptStageWithLogMessagesResult, QueueAcceptStageWithRebuildResult,
    QueueAcceptWithMetricsResult, format_queue_accept_apply_trace_message,
    format_queue_accept_drop_last_info_message, format_queue_accept_fee_trace_message,
    format_queue_accept_leave_in_queue_debug_message,
    format_queue_accept_parent_hash_unchanged_warning,
    format_queue_accept_skip_not_first_trace_message, prepare_queue_accept_iteration,
    prepare_queue_accept_with_call_state, queue_accept_is_nearly_full,
    rebuild_queue_accept_fee_order, resume_prepared_queue_accept_with_call_state,
    run_prepared_queue_accept_apply, run_prepared_queue_accept_call, run_queue_accept_iteration,
    run_queue_accept_stage, run_queue_accept_stage_with_log_messages,
    run_queue_accept_stage_with_rebuild, run_queue_accept_with_call_state,
    run_queue_accept_with_call_state_and_log_sinks, run_queue_accept_with_caller_prepared_apply,
    run_queue_accept_with_caller_prepared_apply_and_log_sinks,
    run_queue_accept_with_log_sinks_and_owner_state, run_queue_accept_with_metrics_and_owner_state,
    run_queue_accept_with_owner_state,
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
                    format_queue_accept_skip_not_first_trace_message(Uint256::from_u64(7), "acct",),
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
    let expected_warning = format_queue_accept_parent_hash_unchanged_warning(Uint256::from_u64(9));

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
                    format_queue_accept_skip_not_first_trace_message(Uint256::from_u64(9), "acct",),
                    format_queue_accept_fee_trace_message(Uint256::from_u64(5), "acct", 300, 256,),
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
