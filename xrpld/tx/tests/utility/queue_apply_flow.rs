use std::collections::BTreeMap;

use basics::base_uint::Uint256;
use protocol::{Rules, SeqProxy, Ter};
use tx::{
    AccountQueueWindow, ApplyFlags, ApplyResult, MAYBE_TX_RETRIES_ALLOWED, MaybeTx,
    OrderCandidates, PreclaimResult, PreflightResult, QueueApplyFlowStage,
    QueueApplyFlowStageWithLogMessagesResult, QueueApplyFullQueueDecision, QueueApplyHoldFallback,
    QueueApplyLiveOwner, QueueApplyPath, QueueApplyPreclaimStage, QueueApplyPreclaimViewSource,
    QueueApplyPreparedFlowInputs, QueueApplyPreparedPostPreclaimInputs,
    QueueApplyPreparedQueuedFlowStage, QueueApplyPreparedTryClearInputs,
    QueueApplyQueueLogMessages, QueueApplyQueueStage, QueueApplyQueueStageWithLogMessagesResult,
    QueueApplyTryClearGate, QueueApplyTryClearStage, QueueViews, TryClearAccountPlan,
    TryClearAccountResult, TxConsequences, derive_queue_apply_prepared_post_preclaim_inputs,
    format_queue_apply_enqueue_debug_message, format_queue_apply_full_queue_evict_info_message,
    run_prepared_queue_apply_post_preclaim_stage_with_caller_queue,
    run_prepared_queue_apply_post_preclaim_stage_with_log_messages_and_caller_queue,
    run_queue_apply_flow_stage,
    run_queue_apply_flow_stage_with_log_messages_and_caller_preclaim_and_caller_try_clear_and_caller_queue,
};

fn make_preclaim(
    likely_to_claim_fee: bool,
    ter: Ter,
) -> PreclaimResult<&'static str, &'static str, &'static str> {
    let mut result = PreclaimResult::new(7, "tx", None::<&str>, ApplyFlags::NONE, "journal", ter);
    result.likely_to_claim_fee = likely_to_claim_fee;
    result
}

fn make_preflight(
    flags: ApplyFlags,
) -> PreflightResult<&'static str, TxConsequences, &'static str, &'static str> {
    PreflightResult::new(
        "tx",
        None,
        Rules::new(std::iter::empty()),
        TxConsequences::new(1, SeqProxy::sequence(6)),
        flags,
        "journal",
        Ter::TES_SUCCESS,
    )
}

fn make_preclaim_stage() -> QueueApplyPreclaimStage<&'static str, &'static str, &'static str> {
    QueueApplyPreclaimStage {
        view_source: QueueApplyPreclaimViewSource::MultiTxnOpenView,
        trace_message: "trace".to_string(),
        preclaim_result: make_preclaim(true, Ter::TES_SUCCESS),
    }
}

#[test]
fn prepared_post_preclaim_inputs_preserve_queue_handoff_and_derive_try_clear_gate() {
    let preclaim = make_preclaim_stage();
    let order = OrderCandidates::new(Uint256::from_u64(0));
    let preflight = make_preflight(ApplyFlags::RETRY);

    let prepared = derive_queue_apply_prepared_post_preclaim_inputs(
        SeqProxy::sequence(6),
        Some(MAYBE_TX_RETRIES_ALLOWED),
        110,
        50,
        100,
        QueueApplyHoldFallback::AlreadyVerifiedByMultiTxn,
        QueueApplyFullQueueDecision::Bypass,
        None::<tx::FeeQueueKey<&'static str>>,
        "acct",
        Uint256::from_u64(9),
        Some(250),
        ApplyFlags::RETRY,
        preflight.clone(),
        &order,
        preclaim.clone(),
    );

    assert_eq!(
        prepared,
        QueueApplyPreparedPostPreclaimInputs::new(
            QueueApplyPreparedTryClearInputs::new(
                preclaim,
                QueueApplyTryClearGate::AttemptClearAhead,
            ),
            QueueApplyHoldFallback::AlreadyVerifiedByMultiTxn,
            QueueApplyFullQueueDecision::Bypass,
            None,
            "acct",
            Uint256::from_u64(9),
            Some(250),
            SeqProxy::sequence(6),
            110,
            ApplyFlags::RETRY,
            preflight,
            &order,
        )
    );
}

#[test]
fn prepared_post_preclaim_stage_exposes_try_clear_and_queue_boundaries() {
    let preclaim = make_preclaim_stage();
    let preflight = make_preflight(ApplyFlags::RETRY);
    let order = OrderCandidates::new(Uint256::from_u64(0));
    let prepared = derive_queue_apply_prepared_post_preclaim_inputs(
        SeqProxy::sequence(6),
        Some(MAYBE_TX_RETRIES_ALLOWED),
        110,
        50,
        100,
        QueueApplyHoldFallback::AlreadyVerifiedByMultiTxn,
        QueueApplyFullQueueDecision::Bypass,
        None::<tx::FeeQueueKey<&'static str>>,
        "acct",
        Uint256::from_u64(9),
        Some(250),
        ApplyFlags::RETRY,
        preflight.clone(),
        &order,
        preclaim.clone(),
    );
    let mut views = QueueViews::<
        &'static str,
        MaybeTx<&'static str, &'static str, &'static str, &'static str>,
    >::new(BTreeMap::new(), vec![]);

    let stage = run_prepared_queue_apply_post_preclaim_stage_with_caller_queue(
        &mut views,
        prepared,
        |prepared_try_clear| {
            assert_eq!(
                prepared_try_clear,
                QueueApplyPreparedTryClearInputs::new(
                    preclaim.clone(),
                    QueueApplyTryClearGate::AttemptClearAhead,
                )
            );

            QueueApplyTryClearStage::ContinueAfterAttempt
        },
        |_views, prepared_queue| {
            assert_eq!(prepared_queue.preclaim, preclaim);
            assert_eq!(
                prepared_queue.try_clear,
                QueueApplyTryClearStage::ContinueAfterAttempt
            );
            assert_eq!(
                prepared_queue.hold_fallback,
                QueueApplyHoldFallback::AlreadyVerifiedByMultiTxn
            );
            assert_eq!(
                prepared_queue.full_queue_decision,
                QueueApplyFullQueueDecision::Bypass
            );
            assert_eq!(prepared_queue.account, "acct");
            assert_eq!(prepared_queue.tx_id, Uint256::from_u64(9));
            assert_eq!(prepared_queue.last_valid, Some(250));
            assert_eq!(prepared_queue.seq_proxy, SeqProxy::sequence(6));
            assert_eq!(prepared_queue.fee_level, 110);
            assert_eq!(prepared_queue.flags, ApplyFlags::RETRY);
            assert_eq!(prepared_queue.pf_result, preflight);
            assert!(std::ptr::eq(prepared_queue.order, &order));

            QueueApplyQueueStage::RejectFull
        },
    );

    assert_eq!(
        stage,
        QueueApplyFlowStage::QueueOutcome {
            preclaim,
            try_clear: QueueApplyTryClearStage::ContinueAfterAttempt,
            queue: QueueApplyQueueStage::RejectFull,
        }
    );
    assert_eq!(
        stage.apply_result(),
        ApplyResult::new(Ter::TEL_CAN_NOT_QUEUE_FULL, false, false)
    );
}

#[test]
fn prepared_post_preclaim_stage_with_log_messages_preserves_queue_log_messages() {
    let preclaim = make_preclaim_stage();
    let preflight = make_preflight(ApplyFlags::FAIL_HARD | ApplyFlags::RETRY);
    let order = OrderCandidates::new(Uint256::from_u64(0));
    let prepared = derive_queue_apply_prepared_post_preclaim_inputs(
        SeqProxy::sequence(6),
        Some(MAYBE_TX_RETRIES_ALLOWED),
        110,
        50,
        100,
        QueueApplyHoldFallback::AlreadyVerifiedByMultiTxn,
        QueueApplyFullQueueDecision::EvictCheapest {
            dropped: tx::FeeQueueKey::new("b", SeqProxy::sequence(8)),
            end_effective_fee_level: 50,
        },
        None::<tx::FeeQueueKey<&'static str>>,
        "acct",
        Uint256::from_u64(9),
        Some(250),
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        preflight.clone(),
        &order,
        preclaim.clone(),
    );
    let mut views = QueueViews::<
        &'static str,
        MaybeTx<&'static str, &'static str, &'static str, &'static str>,
    >::new(BTreeMap::new(), vec![]);
    let queued = tx::QueueApplyEnqueueResult {
        queued: tx::FeeQueueKey::new("acct", SeqProxy::sequence(6)),
        removed_replacement: None,
        account_created: true,
        stored_flags: ApplyFlags::FAIL_HARD,
    };
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

    let result = run_prepared_queue_apply_post_preclaim_stage_with_log_messages_and_caller_queue(
        &mut views,
        prepared,
        |prepared_try_clear| {
            assert_eq!(
                prepared_try_clear,
                QueueApplyPreparedTryClearInputs::new(
                    preclaim.clone(),
                    QueueApplyTryClearGate::AttemptClearAhead,
                )
            );

            QueueApplyTryClearStage::ContinueAfterAttempt
        },
        |_views, prepared_queue| {
            assert_eq!(prepared_queue.preclaim, preclaim);
            assert_eq!(
                prepared_queue.try_clear,
                QueueApplyTryClearStage::ContinueAfterAttempt
            );
            assert_eq!(
                prepared_queue.hold_fallback,
                QueueApplyHoldFallback::AlreadyVerifiedByMultiTxn
            );
            assert_eq!(
                prepared_queue.full_queue_decision,
                QueueApplyFullQueueDecision::EvictCheapest {
                    dropped: tx::FeeQueueKey::new("b", SeqProxy::sequence(8)),
                    end_effective_fee_level: 50,
                }
            );
            assert_eq!(prepared_queue.account, "acct");
            assert_eq!(prepared_queue.tx_id, Uint256::from_u64(9));
            assert_eq!(prepared_queue.last_valid, Some(250));
            assert_eq!(prepared_queue.seq_proxy, SeqProxy::sequence(6));
            assert_eq!(prepared_queue.fee_level, 110);
            assert_eq!(
                prepared_queue.flags,
                ApplyFlags::FAIL_HARD | ApplyFlags::RETRY
            );
            assert_eq!(prepared_queue.pf_result, preflight);
            assert!(std::ptr::eq(prepared_queue.order, &order));

            QueueApplyQueueStageWithLogMessagesResult {
                stage: QueueApplyQueueStage::Queued(queued.clone()),
                log_messages: queue_log_messages.clone(),
            }
        },
    );

    assert_eq!(
        result,
        QueueApplyFlowStageWithLogMessagesResult {
            stage: QueueApplyFlowStage::QueueOutcome {
                preclaim,
                try_clear: QueueApplyTryClearStage::ContinueAfterAttempt,
                queue: QueueApplyQueueStage::Queued(queued),
            },
            queue_log_messages,
        }
    );
}

#[test]
fn live_owner_post_preclaim_helper_reduces_prepared_flow_to_lower_post_preclaim_inputs() {
    let preclaim = QueueApplyPreclaimStage {
        view_source: QueueApplyPreclaimViewSource::CurrentView,
        trace_message: "trace".to_string(),
        preclaim_result: make_preclaim(true, Ter::TES_SUCCESS),
    };
    let preflight = make_preflight(ApplyFlags::RETRY);
    let order = OrderCandidates::new(Uint256::from_u64(0));
    let prepared_flow = QueueApplyPreparedQueuedFlowStage::Flow {
        window: AccountQueueWindow::default(),
        replacement_decision: None,
        path: QueueApplyPath::OpenLedger,
        view_adjustment: None,
        prepared: QueueApplyPreparedFlowInputs::new(
            QueueApplyPreclaimViewSource::CurrentView,
            SeqProxy::sequence(6),
            None,
            110,
            50,
            100,
            4,
            QueueApplyHoldFallback::HoldAllowed,
            QueueApplyFullQueueDecision::Bypass,
            None,
            "acct",
            Uint256::from_u64(9),
            Some(250),
            ApplyFlags::RETRY,
            preflight.clone(),
            &order,
        ),
    };

    assert_eq!(
        QueueApplyLiveOwner::<&'static str, &'static str, &'static str, &'static str>::derive_prepared_post_preclaim_inputs(
            prepared_flow,
            preclaim.clone(),
        ),
        Some(QueueApplyPreparedPostPreclaimInputs::new(
            QueueApplyPreparedTryClearInputs::new(preclaim, QueueApplyTryClearGate::Bypass),
            QueueApplyHoldFallback::HoldAllowed,
            QueueApplyFullQueueDecision::Bypass,
            None,
            "acct",
            Uint256::from_u64(9),
            Some(250),
            SeqProxy::sequence(6),
            110,
            ApplyFlags::RETRY,
            preflight,
            &order,
        ))
    );
}

#[test]
fn flow_stage_accepts_structured_try_clear_result_without_widening_queue_path_shape() {
    let mut views = QueueViews::<
        &'static str,
        MaybeTx<&'static str, &'static str, &'static str, &'static str>,
    >::new(BTreeMap::new(), vec![]);

    let stage = run_queue_apply_flow_stage(
        &mut views,
        QueueApplyPreclaimViewSource::MultiTxnOpenView,
        SeqProxy::sequence(6),
        Some(MAYBE_TX_RETRIES_ALLOWED),
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
        make_preflight(ApplyFlags::NONE),
        &OrderCandidates::new(Uint256::from_u64(0)),
        |_| make_preclaim(true, Ter::TES_SUCCESS),
        || TryClearAccountResult::InsufficientFee {
            plan: TryClearAccountPlan {
                queued_seq_proxies: vec![SeqProxy::sequence(5)],
                queued_count: 1,
                target_was_already_queued: false,
                total_fee_level_paid: 50,
            },
            required_total_fee_level: 60,
        },
        || unreachable!("sandbox should not apply for insufficient fee fallback"),
    );

    assert!(matches!(
        stage,
        QueueApplyFlowStage::QueueOutcome {
            try_clear: QueueApplyTryClearStage::ContinueAfterAttempt,
            ..
        }
    ));
    assert_eq!(
        stage.apply_result(),
        ApplyResult::new(Ter::TER_QUEUED, false, false)
    );
}

#[test]
fn flow_stage_with_log_messages_and_caller_queue_preserves_cpp_info_then_debug_order() {
    let mut views = QueueViews::<
        &'static str,
        MaybeTx<&'static str, &'static str, &'static str, &'static str>,
    >::new(BTreeMap::new(), vec![]);
    let preclaim = make_preclaim_stage();
    let try_clear = QueueApplyTryClearStage::ContinueAfterAttempt;
    let preflight = make_preflight(ApplyFlags::FAIL_HARD | ApplyFlags::RETRY);
    let order = OrderCandidates::new(Uint256::from_u64(0));
    let queued = tx::QueueApplyEnqueueResult {
        queued: tx::FeeQueueKey::new("acct", SeqProxy::sequence(6)),
        removed_replacement: None,
        account_created: true,
        stored_flags: ApplyFlags::FAIL_HARD,
    };
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
        run_queue_apply_flow_stage_with_log_messages_and_caller_preclaim_and_caller_try_clear_and_caller_queue(
            &mut views,
            SeqProxy::sequence(6),
            110,
            QueueApplyHoldFallback::AlreadyVerifiedByMultiTxn,
            QueueApplyFullQueueDecision::EvictCheapest {
                dropped: tx::FeeQueueKey::new("b", SeqProxy::sequence(8)),
                end_effective_fee_level: 50,
            },
            None,
            "acct",
            Uint256::from_u64(9),
            Some(250),
            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
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
                    QueueApplyFullQueueDecision::EvictCheapest {
                        dropped: tx::FeeQueueKey::new("b", SeqProxy::sequence(8)),
                        end_effective_fee_level: 50,
                    }
                );
                assert_eq!(prepared.account, "acct");
                assert_eq!(prepared.tx_id, Uint256::from_u64(9));
                assert_eq!(prepared.last_valid, Some(250));
                assert_eq!(prepared.seq_proxy, SeqProxy::sequence(6));
                assert_eq!(prepared.fee_level, 110);
                assert_eq!(
                    prepared.flags,
                    ApplyFlags::FAIL_HARD | ApplyFlags::RETRY
                );
                assert_eq!(prepared.pf_result, preflight);
                assert!(std::ptr::eq(prepared.order, &order));

                QueueApplyQueueStageWithLogMessagesResult {
                    stage: QueueApplyQueueStage::Queued(queued.clone()),
                    log_messages: queue_log_messages.clone(),
                }
            },
        );

    assert_eq!(
        result,
        QueueApplyFlowStageWithLogMessagesResult {
            stage: QueueApplyFlowStage::QueueOutcome {
                preclaim,
                try_clear,
                queue: QueueApplyQueueStage::Queued(queued),
            },
            queue_log_messages,
        }
    );
}
