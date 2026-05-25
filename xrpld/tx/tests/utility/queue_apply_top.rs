use std::collections::BTreeMap;

use basics::base_uint::Uint256;
use protocol::{Rules, SeqProxy, Ter};
use tx::{
    ApplyFlags, ApplyResult, MaybeTx, QueueApplyCallInputs, QueueApplyEntryStage,
    QueueApplyFlowStage, QueueApplyPreclaimStage, QueueApplyPreclaimViewSource,
    QueueApplyQueueLogMessages, QueueApplyQueueStage, QueueApplyQueuedStage,
    QueueApplyQueuedStageWithLogMessagesResult, QueueApplyTopWithDirectApplyInputs,
    QueueApplyTopWithFeeContextInputs, QueueApplyTopWithLogMessagesResult,
    QueueApplyTopWithQueuedStageInputs, QueueApplyTryClearStage, QueueViews, TxConsequences,
    format_queue_apply_enqueue_debug_message, format_queue_apply_full_queue_evict_info_message,
    run_queue_apply_top_with_caller_direct_apply_and_caller_queued_stage_and_log_messages,
    run_queue_apply_top_with_log_messages,
};

#[test]
fn top_with_log_messages_keeps_empty_logs_on_preflight_reject() {
    let rules = Rules::new(std::iter::empty());

    let result = run_queue_apply_top_with_log_messages::<
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        _,
        _,
        _,
    >(
        QueueApplyCallInputs::new(
            &rules,
            true,
            SeqProxy::sequence(8),
            SeqProxy::sequence(8),
            true,
        ),
        || {
            tx::PreflightResult::new(
                "tx",
                None::<&str>,
                rules.clone(),
                "normal",
                ApplyFlags::NONE,
                "journal",
                Ter::TER_RETRY,
            )
        },
        || unreachable!("preflight reject should return before direct apply"),
        || unreachable!("preflight reject should return before queued stage"),
    );

    assert_eq!(
        result,
        QueueApplyTopWithLogMessagesResult {
            stage: tx::QueueApplyPreflightStage::RejectPreflight(ApplyResult::new(
                Ter::TER_RETRY,
                false,
                false,
            )),
            queue_log_messages: QueueApplyQueueLogMessages::default(),
        }
    );
}

#[test]
fn top_with_caller_direct_apply_and_caller_queued_stage_and_log_messages_preserves_queue_logs() {
    let rules = Rules::new(std::iter::empty());
    let order = tx::OrderCandidates::new(Uint256::from_u64(0));
    let account = String::from("acct");
    let mut views =
        QueueViews::<String, MaybeTx<&'static str, String, &'static str, &'static str>>::new(
            BTreeMap::new(),
            vec![],
        );
    let queue_log_messages = QueueApplyQueueLogMessages {
        trace: vec![],
        debug: vec![format_queue_apply_enqueue_debug_message(
            Uint256::from_u64(9),
            Ter::TER_QUEUED,
            false,
            "acct",
            ApplyFlags::FAIL_HARD,
        )],
        info: vec![format_queue_apply_full_queue_evict_info_message(
            "b",
            50,
            Uint256::from_u64(9),
            110,
        )],
    };

    let result =
        run_queue_apply_top_with_caller_direct_apply_and_caller_queued_stage_and_log_messages(
            &mut views,
            QueueApplyTopWithQueuedStageInputs::new(
                QueueApplyTopWithDirectApplyInputs::new(
                    QueueApplyTopWithFeeContextInputs::new(
                        QueueApplyCallInputs::new(
                            &rules,
                            true,
                            SeqProxy::sequence(5),
                            SeqProxy::sequence(6),
                            true,
                        ),
                        tx::QueueApplyFeeContextInputs {
                            calculated_base_fee_drops: 10,
                            fee_paid_drops: 1,
                            default_base_fee_drops: 10,
                            metrics_snapshot: tx::QueueFeeMetricsSnapshot {
                                txns_expected: 32,
                                escalation_multiplier: tx::TXQ_BASE_LEVEL * 500,
                            },
                            open_ledger_tx_count: 0,
                            flags: ApplyFlags::NONE,
                        },
                    ),
                    "ABC123",
                    &account,
                ),
                tx::QueueApplyQueuedWithFeeContextInputs {
                    account: account.clone(),
                    preflight: tx::QueueHoldPreflight::new(
                        false,
                        false,
                        ApplyFlags::NONE,
                        Some(250),
                    ),
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
                    order: &order,
                },
            ),
            || {
                tx::PreflightResult::new(
                    "tx",
                    None::<&str>,
                    rules.clone(),
                    TxConsequences::new(1, SeqProxy::sequence(6)),
                    ApplyFlags::NONE,
                    "journal",
                    Ter::TES_SUCCESS,
                )
            },
            |_views, _prepared| unreachable!("sequence mismatch should skip direct apply"),
            |_views, queued| {
                assert_eq!(queued.account_seq_proxy, SeqProxy::sequence(5));
                assert_eq!(queued.tx_seq_proxy, SeqProxy::sequence(6));
                assert_eq!(queued.queued.account, account);

                QueueApplyQueuedStageWithLogMessagesResult {
                    stage: QueueApplyQueuedStage::Flow {
                        window: tx::AccountQueueWindow {
                            account_is_in_queue: true,
                            first_relevant_seq_proxy: Some(SeqProxy::sequence(5)),
                            relevant_tx_count: 2,
                            replaces_existing: false,
                            front_is_blocker: false,
                        },
                        replacement_decision: None,
                        path: tx::QueueApplyPath::QueuedAccount {
                            requires_multi_txn: true,
                        },
                        view_adjustment: Some(tx::QueueApplyViewAdjustment {
                            potential_total_spend_drops: 185,
                            adjusted_balance_drops: 815,
                            applied_sequence_value: 6,
                        }),
                        flow: QueueApplyFlowStage::QueueOutcome {
                            preclaim: QueueApplyPreclaimStage {
                                view_source: QueueApplyPreclaimViewSource::MultiTxnOpenView,
                                trace_message: "trace".to_string(),
                                preclaim_result: tx::PreflightResult::new(
                                    "tx",
                                    None::<&str>,
                                    Rules::new(std::iter::empty()),
                                    TxConsequences::new(1, SeqProxy::sequence(6)),
                                    ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
                                    "journal",
                                    Ter::TES_SUCCESS,
                                )
                                .to_preclaim(9, Ter::TES_SUCCESS),
                            },
                            try_clear: QueueApplyTryClearStage::ContinueAfterAttempt,
                            queue: QueueApplyQueueStage::Queued(tx::QueueApplyEnqueueResult {
                                queued: tx::FeeQueueKey::new(
                                    account.clone(),
                                    SeqProxy::sequence(6),
                                ),
                                removed_replacement: None,
                                account_created: true,
                                stored_flags: ApplyFlags::FAIL_HARD,
                            }),
                        },
                    },
                    queue_log_messages: queue_log_messages.clone(),
                }
            },
        );

    assert_eq!(
        result,
        QueueApplyTopWithLogMessagesResult {
            stage: tx::QueueApplyPreflightStage::Entry(QueueApplyEntryStage::Queued(
                QueueApplyQueuedStage::Flow {
                    window: tx::AccountQueueWindow {
                        account_is_in_queue: true,
                        first_relevant_seq_proxy: Some(SeqProxy::sequence(5)),
                        relevant_tx_count: 2,
                        replaces_existing: false,
                        front_is_blocker: false,
                    },
                    replacement_decision: None,
                    path: tx::QueueApplyPath::QueuedAccount {
                        requires_multi_txn: true,
                    },
                    view_adjustment: Some(tx::QueueApplyViewAdjustment {
                        potential_total_spend_drops: 185,
                        adjusted_balance_drops: 815,
                        applied_sequence_value: 6,
                    }),
                    flow: QueueApplyFlowStage::QueueOutcome {
                        preclaim: QueueApplyPreclaimStage {
                            view_source: QueueApplyPreclaimViewSource::MultiTxnOpenView,
                            trace_message: "trace".to_string(),
                            preclaim_result: tx::PreflightResult::new(
                                "tx",
                                None::<&str>,
                                Rules::new(std::iter::empty()),
                                TxConsequences::new(1, SeqProxy::sequence(6)),
                                ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
                                "journal",
                                Ter::TES_SUCCESS,
                            )
                            .to_preclaim(9, Ter::TES_SUCCESS),
                        },
                        try_clear: QueueApplyTryClearStage::ContinueAfterAttempt,
                        queue: QueueApplyQueueStage::Queued(tx::QueueApplyEnqueueResult {
                            queued: tx::FeeQueueKey::new(account, SeqProxy::sequence(6)),
                            removed_replacement: None,
                            account_created: true,
                            stored_flags: ApplyFlags::FAIL_HARD,
                        }),
                    },
                },
            )),
            queue_log_messages,
        }
    );
}
