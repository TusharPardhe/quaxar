use std::collections::BTreeMap;

use basics::base_uint::Uint256;
use protocol::{SeqProxy, Ter};
use tx::{
    ApplyFlags, ApplyResult, DirectApplyAttemptResult, DirectApplyExecution, MaybeTx,
    QueueApplyEntryStage, QueueApplyEntryStageWithLogMessagesResult,
    QueueApplyEntryWithDirectApplyInputs, QueueApplyFeeContext, QueueApplyFlowStage,
    QueueApplyPreclaimStage, QueueApplyPreclaimViewSource, QueueApplyQueueLogMessages,
    QueueApplyQueueStage, QueueApplyQueuedStage, QueueApplyQueuedStageWithLogMessagesResult,
    QueueApplyTryClearStage, QueueViews, TxConsequences, format_queue_apply_enqueue_debug_message,
    format_queue_apply_full_queue_evict_info_message,
    run_queue_apply_entry_stage_with_log_messages,
    run_queue_apply_entry_with_direct_apply_and_log_messages,
};

#[test]
fn entry_stage_with_log_messages_keeps_empty_logs_before_queued_stage() {
    let direct = DirectApplyExecution {
        transaction_id: "tx",
        attempt: DirectApplyAttemptResult {
            apply_result: ApplyResult::new(Ter::TEF_NO_TICKET, false, false),
            removed_replacement: None::<tx::FeeQueueKey<&'static str>>,
        },
    };

    let result = run_queue_apply_entry_stage_with_log_messages::<
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
        || Some(direct.clone()),
        || unreachable!("direct apply should return before queued stage"),
    );

    assert_eq!(
        result,
        QueueApplyEntryStageWithLogMessagesResult {
            stage: QueueApplyEntryStage::DirectApplied(direct),
            queue_log_messages: QueueApplyQueueLogMessages::default(),
        }
    );
}

#[test]
fn entry_with_direct_apply_and_log_messages_preserves_queue_log_messages() {
    let mut views = QueueViews::<
        &'static str,
        MaybeTx<&'static str, &'static str, &'static str, &'static str>,
    >::new(BTreeMap::new(), vec![]);
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

    let result = run_queue_apply_entry_with_direct_apply_and_log_messages(
        &mut views,
        QueueApplyEntryWithDirectApplyInputs::new(
            "ABC123",
            true,
            SeqProxy::sequence(5),
            SeqProxy::sequence(6),
            true,
            QueueApplyFeeContext {
                base_level: 50,
                fee_level_paid: 110,
                required_fee_level: 100,
            },
            &"acct",
        ),
        |_message| unreachable!("direct apply should not run"),
        || unreachable!("direct apply should not run"),
        |_views, fee_context| {
            assert_eq!(
                fee_context,
                QueueApplyFeeContext {
                    base_level: 50,
                    fee_level_paid: 110,
                    required_fee_level: 100,
                }
            );

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
                                protocol::Rules::new(std::iter::empty()),
                                TxConsequences::new(1, SeqProxy::sequence(6)),
                                ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
                                "journal",
                                Ter::TES_SUCCESS,
                            )
                            .to_preclaim(9, Ter::TES_SUCCESS),
                        },
                        try_clear: QueueApplyTryClearStage::ContinueAfterAttempt,
                        queue: QueueApplyQueueStage::Queued(tx::QueueApplyEnqueueResult {
                            queued: tx::FeeQueueKey::new("acct", SeqProxy::sequence(6)),
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
        QueueApplyEntryStageWithLogMessagesResult {
            stage: QueueApplyEntryStage::Queued(QueueApplyQueuedStage::Flow {
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
                            protocol::Rules::new(std::iter::empty()),
                            TxConsequences::new(1, SeqProxy::sequence(6)),
                            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
                            "journal",
                            Ter::TES_SUCCESS,
                        )
                        .to_preclaim(9, Ter::TES_SUCCESS),
                    },
                    try_clear: QueueApplyTryClearStage::ContinueAfterAttempt,
                    queue: QueueApplyQueueStage::Queued(tx::QueueApplyEnqueueResult {
                        queued: tx::FeeQueueKey::new("acct", SeqProxy::sequence(6)),
                        removed_replacement: None,
                        account_created: true,
                        stored_flags: ApplyFlags::FAIL_HARD,
                    }),
                },
            }),
            queue_log_messages,
        }
    );
}
