use std::collections::BTreeMap;

use basics::base_uint::Uint256;
use protocol::{Rules, SeqProxy, Ter};
use tx::{
    ApplyFlags, ApplyResult, QueueApplyEntryStage, QueueApplyFlowStage, QueueApplyPreclaimStage,
    QueueApplyPreclaimViewSource, QueueApplyPreflightStage,
    QueueApplyPreflightStageWithLogMessagesResult, QueueApplyQueueLogMessages,
    QueueApplyQueueStage, QueueApplyQueuedStage, QueueApplyQueuedStageWithLogMessagesResult,
    QueueApplyTryClearStage, QueueViews, TxConsequences, format_queue_apply_enqueue_debug_message,
    format_queue_apply_full_queue_evict_info_message,
    run_queue_apply_preflight_stage_with_log_messages,
};

fn preflight(
    ter: Ter,
) -> tx::PreflightResult<&'static str, &'static str, &'static str, &'static str> {
    tx::PreflightResult::new(
        "tx",
        None,
        Rules::new(std::iter::empty()),
        "normal",
        ApplyFlags::NONE,
        "journal",
        ter,
    )
}

#[test]
fn preflight_stage_with_log_messages_keeps_empty_logs_on_preflight_reject() {
    let result = run_queue_apply_preflight_stage_with_log_messages::<
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        _,
        _,
    >(
        &preflight(Ter::TER_RETRY),
        true,
        SeqProxy::sequence(8),
        SeqProxy::sequence(8),
        false,
        || None,
        || unreachable!("preflight reject should return before entry stage"),
    );

    assert_eq!(
        result,
        QueueApplyPreflightStageWithLogMessagesResult {
            stage: QueueApplyPreflightStage::RejectPreflight(ApplyResult::new(
                Ter::TER_RETRY,
                false,
                false,
            )),
            queue_log_messages: QueueApplyQueueLogMessages::default(),
        }
    );
}

#[test]
fn preflight_stage_with_log_messages_preserves_queued_stage_log_messages() {
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
    let _views = QueueViews::<
        &'static str,
        tx::MaybeTx<&'static str, &'static str, &'static str, &'static str>,
    >::new(BTreeMap::new(), vec![]);

    let result = run_queue_apply_preflight_stage_with_log_messages(
        &preflight(Ter::TES_SUCCESS),
        true,
        SeqProxy::sequence(5),
        SeqProxy::sequence(6),
        true,
        || None::<tx::DirectApplyExecution<&'static str, &'static str>>,
        || QueueApplyQueuedStageWithLogMessagesResult {
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
                        queued: tx::FeeQueueKey::new("acct", SeqProxy::sequence(6)),
                        removed_replacement: None,
                        account_created: true,
                        stored_flags: ApplyFlags::FAIL_HARD,
                    }),
                },
            },
            queue_log_messages: queue_log_messages.clone(),
        },
    );

    assert_eq!(
        result,
        QueueApplyPreflightStageWithLogMessagesResult {
            stage: QueueApplyPreflightStage::Entry(QueueApplyEntryStage::Queued(
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
                            queued: tx::FeeQueueKey::new("acct", SeqProxy::sequence(6)),
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
