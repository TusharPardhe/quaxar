use std::collections::BTreeMap;

use basics::base_uint::Uint256;
use protocol::{Rules, SeqProxy, Ter};
use tx::{
    ApplyFlags, ApplyResult, BlockerQueueAdmission, MAYBE_TX_RETRIES_ALLOWED, MaybeTx, MaybeTxCore,
    OrderCandidates, PreflightResult, QueueApplyAccountStage, QueueApplyFlowStage,
    QueueApplyFlowStageWithLogMessagesResult, QueueApplyHoldFallback, QueueApplyPath,
    QueueApplyPreclaimViewSource, QueueApplyPreparedFlowInputs, QueueApplyQueueLogMessages,
    QueueApplyQueueStage, QueueApplyQueuedStage, QueueApplyQueuedWithFeeContextInputs,
    QueueApplyTryClearStage, QueueApplyViewAdjustment, QueueHoldPreflight, QueueViews,
    TxConsequences, TxConsequencesCategory, TxQAccount, format_queue_apply_enqueue_debug_message,
    format_queue_apply_full_queue_evict_info_message,
    run_queue_apply_queued_stage_with_fee_context_and_log_messages,
    run_queue_apply_queued_stage_with_log_messages_and_caller_flow,
};

fn make_preflight(
    tx: &'static str,
    _seq_proxy: SeqProxy,
    flags: ApplyFlags,
    consequences: TxConsequences,
) -> PreflightResult<&'static str, TxConsequences, &'static str, &'static str> {
    PreflightResult::new(
        tx,
        None,
        Rules::new(std::iter::empty()),
        consequences,
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
    consequences: TxConsequences,
) -> MaybeTx<&'static str, &'static str, &'static str, &'static str> {
    MaybeTx::new(
        Uint256::from_u64(tx_id),
        fee_level,
        account,
        Some(200),
        seq_proxy,
        flags,
        make_preflight("tx", seq_proxy, flags, consequences),
    )
}

fn make_queued_inputs<'a>(
    account: &'static str,
    order: &'a OrderCandidates,
) -> QueueApplyQueuedWithFeeContextInputs<'a, &'static str> {
    QueueApplyQueuedWithFeeContextInputs {
        account,
        preflight: QueueHoldPreflight::new(false, false, ApplyFlags::NONE, None),
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
        order,
    }
}

#[test]
fn queued_stage_with_log_messages_keeps_empty_logs_before_flow() {
    let mut queued_account = TxQAccount::new("acct");
    queued_account.add(
        SeqProxy::sequence(5),
        MaybeTxCore::new(
            queued(
                "acct",
                SeqProxy::sequence(5),
                5,
                90,
                ApplyFlags::NONE,
                TxConsequences::with_category(
                    1,
                    SeqProxy::sequence(5),
                    TxConsequencesCategory::Blocker,
                ),
            ),
            TxConsequences::with_category(
                1,
                SeqProxy::sequence(5),
                TxConsequencesCategory::Blocker,
            ),
        ),
    );

    let mut views = QueueViews::new(BTreeMap::from([("acct", queued_account)]), vec![]);
    let order = OrderCandidates::new(Uint256::from_u64(0));
    let mut inputs = make_queued_inputs("acct", &order);
    inputs.is_blocker = true;

    let result = run_queue_apply_queued_stage_with_fee_context_and_log_messages(
        &mut views,
        SeqProxy::sequence(5),
        SeqProxy::sequence(6),
        inputs,
        tx::QueueApplyFeeContext {
            base_level: 50,
            fee_level_paid: 110,
            required_fee_level: 100,
        },
        make_preflight(
            "tx",
            SeqProxy::sequence(6),
            ApplyFlags::NONE,
            TxConsequences::with_category(
                1,
                SeqProxy::sequence(6),
                TxConsequencesCategory::Blocker,
            ),
        ),
        |_| true,
        |_| unreachable!("account-stage rejection should happen before preclaim"),
        || -> ApplyResult {
            unreachable!("account-stage rejection should happen before try-clear")
        },
        || unreachable!("account-stage rejection should happen before sandbox apply"),
    );

    assert_eq!(
        result.stage,
        QueueApplyQueuedStage::Account(QueueApplyAccountStage::RejectBlockerAdmission(
            BlockerQueueAdmission::RejectsNonReplacementOfLoneEntry,
        ))
    );
    assert_eq!(
        result.queue_log_messages,
        QueueApplyQueueLogMessages::default()
    );
}

#[test]
fn queued_stage_with_log_messages_and_caller_flow_preserves_queue_log_messages() {
    let mut queued_account = TxQAccount::new("acct");
    queued_account.add(
        SeqProxy::sequence(5),
        MaybeTxCore::new(
            queued(
                "acct",
                SeqProxy::sequence(5),
                5,
                20,
                ApplyFlags::NONE,
                TxConsequences::with_potential_spend(20, SeqProxy::sequence(5), 100),
            ),
            TxConsequences::with_potential_spend(20, SeqProxy::sequence(5), 100),
        ),
    );
    queued_account.add(
        SeqProxy::sequence(7),
        MaybeTxCore::new(
            queued(
                "acct",
                SeqProxy::sequence(7),
                7,
                15,
                ApplyFlags::NONE,
                TxConsequences::with_potential_spend(15, SeqProxy::sequence(7), 50),
            ),
            TxConsequences::with_potential_spend(15, SeqProxy::sequence(7), 50),
        ),
    );

    let mut views = QueueViews::new(BTreeMap::from([("acct", queued_account)]), vec![]);
    let order = OrderCandidates::new(Uint256::from_u64(0));
    let preflight = make_preflight(
        "tx",
        SeqProxy::sequence(6),
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::with_potential_spend(50, SeqProxy::sequence(6), 500),
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

    let result = run_queue_apply_queued_stage_with_log_messages_and_caller_flow(
        &mut views,
        "acct",
        SeqProxy::sequence(5),
        SeqProxy::sequence(6),
        QueueHoldPreflight::new(false, false, ApplyFlags::NONE, None),
        false,
        100,
        2,
        10,
        25,
        false,
        110,
        100,
        50,
        1_000,
        200,
        10,
        Ter::TES_SUCCESS,
        4,
        Uint256::from_u64(9),
        Some(250),
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        preflight.clone(),
        &order,
        |_| true,
        |_views, prepared| {
            let QueueApplyPreparedFlowInputs {
                preclaim_view_source,
                tx_seq_proxy,
                first_relevant_retries_remaining,
                fee_level_paid,
                base_level,
                required_fee_level,
                open_ledger_tx_count,
                hold_fallback,
                full_queue_decision,
                replaced,
                account,
                tx_id,
                last_valid,
                flags,
                pf_result,
                order: prepared_order,
            } = prepared;

            assert_eq!(
                preclaim_view_source,
                QueueApplyPreclaimViewSource::MultiTxnOpenView
            );
            assert_eq!(tx_seq_proxy, SeqProxy::sequence(6));
            assert_eq!(
                first_relevant_retries_remaining,
                Some(MAYBE_TX_RETRIES_ALLOWED)
            );
            assert_eq!(fee_level_paid, 110);
            assert_eq!(base_level, 50);
            assert_eq!(required_fee_level, 100);
            assert_eq!(open_ledger_tx_count, 4);
            assert_eq!(
                hold_fallback,
                QueueApplyHoldFallback::AlreadyVerifiedByMultiTxn
            );
            assert_eq!(full_queue_decision, tx::QueueApplyFullQueueDecision::Bypass);
            assert_eq!(replaced, None);
            assert_eq!(account, "acct");
            assert_eq!(tx_id, Uint256::from_u64(9));
            assert_eq!(last_valid, Some(250));
            assert_eq!(flags, ApplyFlags::FAIL_HARD | ApplyFlags::RETRY);
            assert_eq!(pf_result, preflight);
            assert!(std::ptr::eq(prepared_order, &order));

            QueueApplyFlowStageWithLogMessagesResult {
                stage: QueueApplyFlowStage::QueueOutcome {
                    preclaim: tx::QueueApplyPreclaimStage {
                        view_source: QueueApplyPreclaimViewSource::MultiTxnOpenView,
                        trace_message: "trace".to_string(),
                        preclaim_result: make_preflight(
                            "tx",
                            SeqProxy::sequence(6),
                            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
                            TxConsequences::with_potential_spend(50, SeqProxy::sequence(6), 500),
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
                queue_log_messages: queue_log_messages.clone(),
            }
        },
    );

    match result.stage {
        QueueApplyQueuedStage::Flow {
            window,
            replacement_decision,
            path,
            view_adjustment,
            flow,
        } => {
            assert_eq!(replacement_decision, None);
            assert_eq!(
                window,
                tx::AccountQueueWindow {
                    account_is_in_queue: true,
                    first_relevant_seq_proxy: Some(SeqProxy::sequence(5)),
                    relevant_tx_count: 2,
                    replaces_existing: false,
                    front_is_blocker: false,
                }
            );
            assert_eq!(
                path,
                QueueApplyPath::QueuedAccount {
                    requires_multi_txn: true
                }
            );
            assert_eq!(
                view_adjustment,
                Some(QueueApplyViewAdjustment {
                    potential_total_spend_drops: 185,
                    adjusted_balance_drops: 815,
                    applied_sequence_value: 6,
                })
            );
            assert!(matches!(
                flow,
                QueueApplyFlowStage::QueueOutcome {
                    try_clear: QueueApplyTryClearStage::ContinueAfterAttempt,
                    queue: QueueApplyQueueStage::Queued(_),
                    ..
                }
            ));
        }
        other => panic!("expected flow stage, got {other:?}"),
    }

    assert_eq!(result.queue_log_messages, queue_log_messages);
}
