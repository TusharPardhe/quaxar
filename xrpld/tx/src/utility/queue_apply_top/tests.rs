use std::{cell::Cell, cell::RefCell, collections::BTreeMap};

use basics::base_uint::Uint256;
use protocol::{Rules, SeqProxy, Ter};

use super::{
    QueueApplyCallInputs, QueueApplyPreparedQueuedStageInputs, QueueApplyTopWithDirectApplyInputs,
    QueueApplyTopWithFeeContextInputs, QueueApplyTopWithQueuedStageInputs,
    run_queue_apply_after_preflight_with_acquired_direct_apply_and_queued_stage,
    run_queue_apply_after_preflight_with_caller_direct_apply_and_caller_queued_stage,
    run_queue_apply_after_preflight_with_caller_direct_apply_and_queued_stage,
    run_queue_apply_after_preflight_with_direct_apply_and_queued_stage,
    run_queue_apply_after_preflight_with_queued_stage, run_queue_apply_top,
    run_queue_apply_top_with_caller_direct_apply_and_caller_queued_stage,
    run_queue_apply_top_with_caller_direct_apply_and_caller_queued_stage_and_log_messages,
    run_queue_apply_top_with_caller_direct_apply_and_queued_stage,
    run_queue_apply_top_with_direct_apply, run_queue_apply_top_with_fee_context,
    run_queue_apply_top_with_queued_stage,
};
use crate::{
    ApplyFlags, ApplyResult, BlockerQueueAdmission, DirectApplyAttemptResult, DirectApplyExecution,
    MaybeTx, MaybeTxCore, OrderCandidates, PreclaimResult, PreflightResult, PreparedDirectApply,
    QueueApplyEntryStage, QueueApplyFeeContextInputs, QueueApplyPreflightStage,
    QueueApplyPrerequisite, QueueApplyQueueLogMessages, QueueApplyQueuedStage,
    QueueApplyQueuedStageWithLogMessagesResult, QueueApplyQueuedWithFeeContextInputs,
    QueueApplyTopWithLogMessagesResult, QueueFeeMetricsSnapshot, QueueHoldPreflight, QueueViews,
    TXQ_BASE_LEVEL, TxConsequences, TxConsequencesCategory, TxQAccount,
    evaluate_queue_apply_fee_context,
};

#[test]
fn top_wrapper_forwards_structured_prerequisite_inputs() {
    let rules = Rules::new(std::iter::empty());
    let queued_called = Cell::new(false);

    let stage = run_queue_apply_top::<
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
            false,
            SeqProxy::sequence(8),
            SeqProxy::sequence(8),
            false,
        ),
        || {
            PreflightResult::new(
                "tx",
                None::<&str>,
                rules.clone(),
                "normal",
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            )
        },
        || None,
        || {
            queued_called.set(true);
            unreachable!("missing account should return before queued stage")
        },
    );

    assert_eq!(
        stage,
        QueueApplyPreflightStage::Entry(QueueApplyEntryStage::RejectPrerequisite(
            QueueApplyPrerequisite::MissingAccount
        ))
    );
    assert_eq!(
        stage.apply_result(),
        ApplyResult::new(Ter::TER_NO_ACCOUNT, false, false)
    );
    assert!(!queued_called.get());
}

#[test]
fn top_wrapper_preserves_direct_apply_priority_over_ticket_gate() {
    let rules = Rules::new(std::iter::empty());
    let queued_called = Cell::new(false);
    let direct = DirectApplyExecution {
        transaction_id: "ABC123",
        attempt: DirectApplyAttemptResult {
            apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, true),
            removed_replacement: None::<crate::FeeQueueKey<&'static str>>,
        },
    };

    let stage = run_queue_apply_top(
        QueueApplyCallInputs::new(
            &rules,
            true,
            SeqProxy::sequence(8),
            SeqProxy::ticket(7),
            false,
        ),
        || {
            PreflightResult::new(
                "tx",
                None::<&str>,
                rules.clone(),
                "normal",
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            )
        },
        || Some(direct.clone()),
        || {
            queued_called.set(true);
            unreachable!("direct apply should return before queued stage")
        },
    );

    assert_eq!(
        stage,
        QueueApplyPreflightStage::Entry(QueueApplyEntryStage::DirectApplied(direct))
    );
    assert_eq!(
        stage.apply_result(),
        ApplyResult::new(Ter::TES_SUCCESS, true, true)
    );
    assert!(!queued_called.get());
}

#[test]
fn top_wrapper_with_fee_context_keeps_preflight_ahead_of_fee_derivation() {
    let rules = Rules::new(std::iter::empty());

    let stage = run_queue_apply_top_with_fee_context::<
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
        QueueApplyTopWithFeeContextInputs::new(
            QueueApplyCallInputs::new(
                &rules,
                true,
                SeqProxy::sequence(8),
                SeqProxy::sequence(8),
                true,
            ),
            crate::QueueApplyFeeContextInputs {
                calculated_base_fee_drops: -1,
                fee_paid_drops: 0,
                default_base_fee_drops: 0,
                metrics_snapshot: QueueFeeMetricsSnapshot {
                    txns_expected: 32,
                    escalation_multiplier: TXQ_BASE_LEVEL * 500,
                },
                open_ledger_tx_count: 0,
                flags: ApplyFlags::NONE,
            },
        ),
        || {
            PreflightResult::new(
                "tx",
                None::<&str>,
                rules.clone(),
                "normal",
                ApplyFlags::NONE,
                "journal",
                Ter::TER_RETRY,
            )
        },
        |_| unreachable!("preflight rejection should return before direct apply"),
        |_| unreachable!("preflight rejection should return before queued stage"),
    );

    assert_eq!(
        stage,
        QueueApplyPreflightStage::RejectPreflight(ApplyResult::new(Ter::TER_RETRY, false, false,))
    );
}

#[test]
fn top_wrapper_with_fee_context_feeds_computed_fee_levels_into_direct_apply() {
    let rules = Rules::new(std::iter::empty());
    let queued_called = Cell::new(false);
    let direct = DirectApplyExecution {
        transaction_id: "ABC123",
        attempt: DirectApplyAttemptResult {
            apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, true),
            removed_replacement: None::<crate::FeeQueueKey<&'static str>>,
        },
    };

    let stage = run_queue_apply_top_with_fee_context(
        QueueApplyTopWithFeeContextInputs::new(
            QueueApplyCallInputs::new(
                &rules,
                true,
                SeqProxy::sequence(8),
                SeqProxy::sequence(8),
                true,
            ),
            crate::QueueApplyFeeContextInputs {
                calculated_base_fee_drops: 10,
                fee_paid_drops: 20,
                default_base_fee_drops: 10,
                metrics_snapshot: QueueFeeMetricsSnapshot {
                    txns_expected: 32,
                    escalation_multiplier: TXQ_BASE_LEVEL * 500,
                },
                open_ledger_tx_count: 33,
                flags: ApplyFlags::NONE,
            },
        ),
        || {
            PreflightResult::new(
                "tx",
                None::<&str>,
                rules.clone(),
                "normal",
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            )
        },
        |context| {
            assert_eq!(context.base_level, TXQ_BASE_LEVEL);
            assert_eq!(context.fee_level_paid, 512);
            assert_eq!(context.required_fee_level, 136_125);
            Some(direct.clone())
        },
        |_| {
            queued_called.set(true);
            QueueApplyQueuedStage::MultiTxn(crate::QueueApplyMultiTxnStage::RejectPath(
                Ter::TER_PRE_SEQ,
            ))
        },
    );

    assert_eq!(
        stage,
        QueueApplyPreflightStage::Entry(QueueApplyEntryStage::DirectApplied(direct))
    );
    assert!(!queued_called.get());
}

#[test]
fn top_wrapper_with_direct_apply_keeps_preflight_ahead_of_entry_and_fee_work() {
    let rules = Rules::new(std::iter::empty());
    let traces = RefCell::new(Vec::new());
    let mut views = QueueViews::<&'static str, ()>::new(BTreeMap::new(), vec![]);

    let stage = run_queue_apply_top_with_direct_apply::<
        &'static str,
        (),
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        _,
        _,
        _,
        _,
    >(
        &mut views,
        QueueApplyTopWithDirectApplyInputs::new(
            QueueApplyTopWithFeeContextInputs::new(
                QueueApplyCallInputs::new(
                    &rules,
                    true,
                    SeqProxy::sequence(8),
                    SeqProxy::sequence(8),
                    true,
                ),
                crate::QueueApplyFeeContextInputs {
                    calculated_base_fee_drops: -1,
                    fee_paid_drops: 0,
                    default_base_fee_drops: 0,
                    metrics_snapshot: QueueFeeMetricsSnapshot {
                        txns_expected: 32,
                        escalation_multiplier: TXQ_BASE_LEVEL * 500,
                    },
                    open_ledger_tx_count: 0,
                    flags: ApplyFlags::NONE,
                },
            ),
            "ABC123",
            &"acct",
        ),
        || {
            PreflightResult::new(
                "tx",
                None::<&str>,
                rules.clone(),
                "normal",
                ApplyFlags::NONE,
                "journal",
                Ter::TER_RETRY,
            )
        },
        |line| traces.borrow_mut().push(line.to_owned()),
        || unreachable!("preflight rejection should return before direct apply"),
        |_, _| unreachable!("preflight rejection should return before queued stage"),
    );

    assert_eq!(
        stage,
        QueueApplyPreflightStage::RejectPreflight(ApplyResult::new(Ter::TER_RETRY, false, false,))
    );
    assert!(traces.borrow().is_empty());
}

#[test]
fn top_wrapper_with_direct_apply_runs_runtime_then_direct_apply_before_queue() {
    let rules = Rules::new(std::iter::empty());
    let traces = RefCell::new(Vec::new());
    let mut views = QueueViews::<&'static str, ()>::new(BTreeMap::new(), vec![]);

    let stage = run_queue_apply_top_with_direct_apply(
        &mut views,
        QueueApplyTopWithDirectApplyInputs::new(
            QueueApplyTopWithFeeContextInputs::new(
                QueueApplyCallInputs::new(
                    &rules,
                    true,
                    SeqProxy::sequence(8),
                    SeqProxy::sequence(8),
                    true,
                ),
                crate::QueueApplyFeeContextInputs {
                    calculated_base_fee_drops: 10,
                    fee_paid_drops: 20,
                    default_base_fee_drops: 10,
                    metrics_snapshot: QueueFeeMetricsSnapshot {
                        txns_expected: 32,
                        escalation_multiplier: TXQ_BASE_LEVEL * 500,
                    },
                    open_ledger_tx_count: 0,
                    flags: ApplyFlags::NONE,
                },
            ),
            "ABC123",
            &"acct",
        ),
        || {
            PreflightResult::new(
                "tx",
                None::<&str>,
                rules.clone(),
                "normal",
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            )
        },
        |line| traces.borrow_mut().push(line.to_owned()),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_, _| {
            QueueApplyQueuedStage::<&'static str, &'static str, &'static str, &'static str>::MultiTxn(
                    crate::QueueApplyMultiTxnStage::RejectPath(Ter::TER_PRE_SEQ),
                )
        },
    );

    assert!(matches!(
        stage,
        QueueApplyPreflightStage::Entry(QueueApplyEntryStage::DirectApplied(_))
    ));
    assert_eq!(
        traces.into_inner(),
        vec![
            "Applying transaction ABC123 to open ledger.".to_owned(),
            "New transaction ABC123 applied successfully with tesSUCCESS".to_owned(),
        ]
    );
}

#[test]
fn top_wrapper_with_caller_direct_apply_and_queued_stage_exposes_prepared_boundary() {
    let rules = Rules::new(std::iter::empty());
    let order = OrderCandidates::new(Uint256::from_u64(0));
    let account = String::from("acct");
    let mut views =
        QueueViews::<String, MaybeTx<&'static str, String, &'static str, &'static str>>::new(
            BTreeMap::new(),
            vec![],
        );
    let try_clear_called = Cell::new(false);
    let sandbox_called = Cell::new(false);
    let direct_called = Cell::new(false);

    let stage = run_queue_apply_top_with_caller_direct_apply_and_queued_stage(
        &mut views,
        QueueApplyTopWithQueuedStageInputs::new(
            QueueApplyTopWithDirectApplyInputs::new(
                QueueApplyTopWithFeeContextInputs::new(
                    QueueApplyCallInputs::new(
                        &rules,
                        true,
                        SeqProxy::sequence(8),
                        SeqProxy::sequence(8),
                        true,
                    ),
                    QueueApplyFeeContextInputs {
                        calculated_base_fee_drops: 10,
                        fee_paid_drops: 20,
                        default_base_fee_drops: 10,
                        metrics_snapshot: QueueFeeMetricsSnapshot {
                            txns_expected: 32,
                            escalation_multiplier: TXQ_BASE_LEVEL * 500,
                        },
                        open_ledger_tx_count: 0,
                        flags: ApplyFlags::NONE,
                    },
                ),
                "ABC123",
                &account,
            ),
            QueueApplyQueuedWithFeeContextInputs {
                account: account.clone(),
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
                order: &order,
            },
        ),
        || {
            PreflightResult::new(
                "tx",
                None::<&str>,
                rules.clone(),
                TxConsequences::new(1, SeqProxy::sequence(8)),
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            )
        },
        |_views, prepared| {
            direct_called.set(true);
            assert_eq!(
                prepared,
                PreparedDirectApply {
                    transaction_id: "ABC123",
                    applied_account: &account,
                    applied_seq_proxy: SeqProxy::sequence(8),
                }
            );
            DirectApplyExecution {
                transaction_id: "ABC123",
                attempt: DirectApplyAttemptResult {
                    apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, true),
                    removed_replacement: None,
                },
            }
        },
        |_| true,
        |_| unreachable!("direct apply should return before preclaim"),
        || -> crate::ApplyResult {
            try_clear_called.set(true);
            unreachable!("direct apply should return before try-clear")
        },
        || sandbox_called.set(true),
    );

    assert!(direct_called.get());
    assert!(!try_clear_called.get());
    assert!(!sandbox_called.get());
    assert!(matches!(
        stage,
        QueueApplyPreflightStage::Entry(QueueApplyEntryStage::DirectApplied(_))
    ));
}

#[test]
fn top_wrapper_with_caller_direct_apply_and_caller_queued_stage_exposes_prepared_queue_boundary() {
    let rules = Rules::new(std::iter::empty());
    let order = OrderCandidates::new(Uint256::from_u64(0));
    let account = String::from("acct");
    let mut views =
        QueueViews::<String, MaybeTx<&'static str, String, &'static str, &'static str>>::new(
            BTreeMap::new(),
            vec![],
        );
    let direct_called = Cell::new(false);
    let queued_called = Cell::new(false);
    let fee_context_inputs = QueueApplyFeeContextInputs {
        calculated_base_fee_drops: 10,
        fee_paid_drops: 20,
        default_base_fee_drops: 10,
        metrics_snapshot: QueueFeeMetricsSnapshot {
            txns_expected: 32,
            escalation_multiplier: TXQ_BASE_LEVEL * 500,
        },
        open_ledger_tx_count: 0,
        flags: ApplyFlags::NONE,
    };
    let expected_fee_context = evaluate_queue_apply_fee_context(fee_context_inputs);
    let expected_consequences = TxConsequences::new(1, SeqProxy::sequence(6));

    let stage = run_queue_apply_top_with_caller_direct_apply_and_caller_queued_stage(
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
                    fee_context_inputs,
                ),
                "ABC123",
                &account,
            ),
            QueueApplyQueuedWithFeeContextInputs {
                account: account.clone(),
                preflight: QueueHoldPreflight::new(false, false, ApplyFlags::NONE, Some(250)),
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
            PreflightResult::new(
                "tx",
                None::<&str>,
                rules.clone(),
                expected_consequences.clone(),
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            )
        },
        |_views, _prepared| {
            direct_called.set(true);
            unreachable!("sequence mismatch should skip direct apply")
        },
        |_views, queued| {
            queued_called.set(true);
            let QueueApplyPreparedQueuedStageInputs {
                account_seq_proxy,
                tx_seq_proxy,
                queued,
                fee_context,
                preflight_result,
            } = queued;

            assert_eq!(account_seq_proxy, SeqProxy::sequence(5));
            assert_eq!(tx_seq_proxy, SeqProxy::sequence(6));
            assert_eq!(queued.account, account);
            assert_eq!(queued.last_valid, Some(250));
            assert_eq!(fee_context, expected_fee_context);
            assert_eq!(preflight_result.ter, Ter::TES_SUCCESS);
            assert_eq!(preflight_result.flags, ApplyFlags::NONE);
            assert_eq!(preflight_result.consequences, expected_consequences);

            QueueApplyQueuedStage::MultiTxn(crate::QueueApplyMultiTxnStage::RejectPath(
                Ter::TER_PRE_SEQ,
            ))
        },
    );

    assert!(!direct_called.get());
    assert!(queued_called.get());
    assert_eq!(
        stage,
        QueueApplyPreflightStage::Entry(QueueApplyEntryStage::Queued(
            QueueApplyQueuedStage::MultiTxn(crate::QueueApplyMultiTxnStage::RejectPath(
                Ter::TER_PRE_SEQ,
            )),
        ))
    );
}

#[test]
fn top_wrapper_with_caller_direct_apply_and_caller_queued_stage_and_log_messages_reuses_prepared_queue_boundary()
 {
    let rules = Rules::new(std::iter::empty());
    let order = OrderCandidates::new(Uint256::from_u64(0));
    let account = String::from("acct");
    let mut views =
        QueueViews::<String, MaybeTx<&'static str, String, &'static str, &'static str>>::new(
            BTreeMap::new(),
            vec![],
        );
    let direct_called = Cell::new(false);
    let queued_called = Cell::new(false);
    let fee_context_inputs = QueueApplyFeeContextInputs {
        calculated_base_fee_drops: 10,
        fee_paid_drops: 20,
        default_base_fee_drops: 10,
        metrics_snapshot: QueueFeeMetricsSnapshot {
            txns_expected: 32,
            escalation_multiplier: TXQ_BASE_LEVEL * 500,
        },
        open_ledger_tx_count: 0,
        flags: ApplyFlags::NONE,
    };
    let expected_fee_context = evaluate_queue_apply_fee_context(fee_context_inputs);
    let expected_consequences = TxConsequences::new(1, SeqProxy::sequence(6));

    let stage =
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
                        fee_context_inputs,
                    ),
                    "ABC123",
                    &account,
                ),
                QueueApplyQueuedWithFeeContextInputs {
                    account: account.clone(),
                    preflight: QueueHoldPreflight::new(false, false, ApplyFlags::NONE, Some(250)),
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
                PreflightResult::new(
                    "tx",
                    None::<&str>,
                    rules.clone(),
                    expected_consequences.clone(),
                    ApplyFlags::NONE,
                    "journal",
                    Ter::TES_SUCCESS,
                )
            },
            |_views, _prepared| {
                direct_called.set(true);
                unreachable!("sequence mismatch should skip direct apply")
            },
            |_views, queued| {
                queued_called.set(true);
                let QueueApplyPreparedQueuedStageInputs {
                    account_seq_proxy,
                    tx_seq_proxy,
                    queued,
                    fee_context,
                    preflight_result,
                } = queued;

                assert_eq!(account_seq_proxy, SeqProxy::sequence(5));
                assert_eq!(tx_seq_proxy, SeqProxy::sequence(6));
                assert_eq!(queued.account, account);
                assert_eq!(queued.last_valid, Some(250));
                assert_eq!(fee_context, expected_fee_context);
                assert_eq!(preflight_result.ter, Ter::TES_SUCCESS);
                assert_eq!(preflight_result.flags, ApplyFlags::NONE);
                assert_eq!(preflight_result.consequences, expected_consequences);

                QueueApplyQueuedStageWithLogMessagesResult {
                    stage: QueueApplyQueuedStage::MultiTxn(
                        crate::QueueApplyMultiTxnStage::RejectPath(Ter::TER_PRE_SEQ),
                    ),
                    queue_log_messages: QueueApplyQueueLogMessages::default(),
                }
            },
        );

    assert!(!direct_called.get());
    assert!(queued_called.get());
    assert_eq!(
        stage,
        QueueApplyTopWithLogMessagesResult {
            stage: QueueApplyPreflightStage::Entry(QueueApplyEntryStage::Queued(
                QueueApplyQueuedStage::MultiTxn(crate::QueueApplyMultiTxnStage::RejectPath(
                    Ter::TER_PRE_SEQ,
                )),
            )),
            queue_log_messages: QueueApplyQueueLogMessages::default(),
        }
    );
}

#[test]
fn top_wrapper_with_queued_stage_routes_direct_apply_fallthrough_into_landed_queue_stage() {
    let rules = Rules::new(std::iter::empty());
    let mut queued_account = TxQAccount::new("acct");
    queued_account.add(
        SeqProxy::sequence(5),
        crate::MaybeTxCore::new(
            crate::MaybeTx::new(
                Uint256::from_u64(5),
                90,
                "acct",
                Some(200),
                SeqProxy::sequence(5),
                ApplyFlags::NONE,
                PreflightResult::new(
                    "tx",
                    None::<&str>,
                    rules.clone(),
                    TxConsequences::with_category(
                        1,
                        SeqProxy::sequence(5),
                        TxConsequencesCategory::Blocker,
                    ),
                    ApplyFlags::NONE,
                    "journal",
                    Ter::TES_SUCCESS,
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

    let stage = run_queue_apply_top_with_queued_stage(
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
                    QueueApplyFeeContextInputs {
                        calculated_base_fee_drops: 10,
                        fee_paid_drops: 1,
                        default_base_fee_drops: 10,
                        metrics_snapshot: QueueFeeMetricsSnapshot {
                            txns_expected: 32,
                            escalation_multiplier: TXQ_BASE_LEVEL * 500,
                        },
                        open_ledger_tx_count: 0,
                        flags: ApplyFlags::NONE,
                    },
                ),
                "ABC123",
                &"acct",
            ),
            QueueApplyQueuedWithFeeContextInputs {
                account: "acct",
                preflight: QueueHoldPreflight::new(false, false, ApplyFlags::NONE, None),
                is_blocker: true,
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
            PreflightResult::new(
                "tx",
                None::<&str>,
                rules.clone(),
                TxConsequences::with_category(
                    1,
                    SeqProxy::sequence(6),
                    TxConsequencesCategory::Blocker,
                ),
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            )
        },
        |_| unreachable!("direct apply should fall through without tracing"),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |view_source| {
            assert!(!view_source.has_multi_txn());
            PreclaimResult::new(
                100,
                "tx",
                None::<&str>,
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            )
        },
        || -> crate::ApplyResult {
            unreachable!("queue-stage account rejection should happen before try-clear")
        },
        || unreachable!("queue-stage account rejection should happen before sandbox apply"),
    );

    assert_eq!(
        stage,
        QueueApplyPreflightStage::Entry(QueueApplyEntryStage::Queued(
            QueueApplyQueuedStage::Account(crate::QueueApplyAccountStage::RejectBlockerAdmission(
                BlockerQueueAdmission::RejectsNonReplacementOfLoneEntry,
            )),
        ))
    );
}

#[test]
fn after_preflight_wrapper_with_queued_stage_reuses_landed_queue_order() {
    let rules = Rules::new(std::iter::empty());
    let mut queued_account = TxQAccount::new("acct");
    queued_account.add(
        SeqProxy::sequence(5),
        MaybeTxCore::new(
            MaybeTx::new(
                Uint256::from_u64(5),
                90,
                "acct",
                Some(200),
                SeqProxy::sequence(5),
                ApplyFlags::NONE,
                PreflightResult::new(
                    "tx",
                    None::<&str>,
                    rules.clone(),
                    TxConsequences::with_category(
                        1,
                        SeqProxy::sequence(5),
                        TxConsequencesCategory::Blocker,
                    ),
                    ApplyFlags::NONE,
                    "journal",
                    Ter::TES_SUCCESS,
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

    let stage = run_queue_apply_after_preflight_with_queued_stage(
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
                    QueueApplyFeeContextInputs {
                        calculated_base_fee_drops: 10,
                        fee_paid_drops: 1,
                        default_base_fee_drops: 10,
                        metrics_snapshot: QueueFeeMetricsSnapshot {
                            txns_expected: 32,
                            escalation_multiplier: TXQ_BASE_LEVEL * 500,
                        },
                        open_ledger_tx_count: 0,
                        flags: ApplyFlags::NONE,
                    },
                ),
                "ABC123",
                &"acct",
            ),
            QueueApplyQueuedWithFeeContextInputs {
                account: "acct",
                preflight: QueueHoldPreflight::new(false, false, ApplyFlags::NONE, None),
                is_blocker: true,
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
        &PreflightResult::new(
            "tx",
            None::<&str>,
            rules.clone(),
            TxConsequences::with_category(
                1,
                SeqProxy::sequence(6),
                TxConsequencesCategory::Blocker,
            ),
            ApplyFlags::NONE,
            "journal",
            Ter::TES_SUCCESS,
        ),
        |_| unreachable!("direct apply should fall through without tracing"),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |view_source| {
            assert!(!view_source.has_multi_txn());
            PreclaimResult::new(
                100,
                "tx",
                None::<&str>,
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            )
        },
        || -> crate::ApplyResult {
            unreachable!("queue-stage account rejection should happen before try-clear")
        },
        || unreachable!("queue-stage account rejection should happen before sandbox apply"),
    );

    assert_eq!(
        stage,
        QueueApplyPreflightStage::Entry(QueueApplyEntryStage::Queued(
            QueueApplyQueuedStage::Account(crate::QueueApplyAccountStage::RejectBlockerAdmission(
                BlockerQueueAdmission::RejectsNonReplacementOfLoneEntry,
            )),
        ))
    );
}

#[test]
fn after_preflight_direct_apply_wrapper_with_queued_stage_returns_direct_apply_before_queue_hooks()
{
    let prepare_called = Cell::new(false);
    let preclaim_called = Cell::new(false);
    let try_clear_called = Cell::new(false);
    let sandbox_called = Cell::new(false);
    let traces = RefCell::new(Vec::new());

    let rules = Rules::new(std::iter::empty());
    let mut views = QueueViews::<
        &'static str,
        MaybeTx<&'static str, &'static str, &'static str, &'static str>,
    >::new(BTreeMap::new(), vec![]);
    let order = OrderCandidates::new(Uint256::from_u64(0));

    let stage = run_queue_apply_after_preflight_with_direct_apply_and_queued_stage(
        &mut views,
        QueueApplyTopWithQueuedStageInputs::new(
            QueueApplyTopWithDirectApplyInputs::new(
                QueueApplyTopWithFeeContextInputs::new(
                    QueueApplyCallInputs::new(
                        &rules,
                        true,
                        SeqProxy::sequence(8),
                        SeqProxy::sequence(8),
                        true,
                    ),
                    QueueApplyFeeContextInputs {
                        calculated_base_fee_drops: 10,
                        fee_paid_drops: 20,
                        default_base_fee_drops: 10,
                        metrics_snapshot: QueueFeeMetricsSnapshot {
                            txns_expected: 32,
                            escalation_multiplier: TXQ_BASE_LEVEL * 500,
                        },
                        open_ledger_tx_count: 0,
                        flags: ApplyFlags::NONE,
                    },
                ),
                "ABC123",
                &"acct",
            ),
            QueueApplyQueuedWithFeeContextInputs {
                account: "acct",
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
                order: &order,
            },
        ),
        &PreflightResult::new(
            "tx",
            None::<&str>,
            rules.clone(),
            TxConsequences::new(1, SeqProxy::sequence(8)),
            ApplyFlags::NONE,
            "journal",
            Ter::TES_SUCCESS,
        ),
        |line| traces.borrow_mut().push(line.to_owned()),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| {
            prepare_called.set(true);
            true
        },
        |_| {
            preclaim_called.set(true);
            PreclaimResult::new(
                100,
                "tx",
                None::<&str>,
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            )
        },
        || {
            try_clear_called.set(true);
            ApplyResult::new(Ter::TES_SUCCESS, true, true)
        },
        || sandbox_called.set(true),
    );

    assert!(matches!(
        stage,
        QueueApplyPreflightStage::Entry(QueueApplyEntryStage::DirectApplied(_))
    ));
    assert!(!prepare_called.get());
    assert!(!preclaim_called.get());
    assert!(!try_clear_called.get());
    assert!(!sandbox_called.get());
    assert_eq!(
        traces.into_inner(),
        vec![
            "Applying transaction ABC123 to open ledger.".to_owned(),
            "New transaction ABC123 applied successfully with tesSUCCESS".to_owned(),
        ]
    );
}

#[test]
fn after_preflight_caller_direct_apply_wrapper_with_queued_stage_exposes_prepared_boundary() {
    let prepare_called = Cell::new(false);
    let preclaim_called = Cell::new(false);
    let try_clear_called = Cell::new(false);
    let sandbox_called = Cell::new(false);
    let direct_called = Cell::new(false);

    let rules = Rules::new(std::iter::empty());
    let account = String::from("acct");
    let mut views =
        QueueViews::<String, MaybeTx<&'static str, String, &'static str, &'static str>>::new(
            BTreeMap::new(),
            vec![],
        );
    let order = OrderCandidates::new(Uint256::from_u64(0));

    let stage = run_queue_apply_after_preflight_with_caller_direct_apply_and_queued_stage(
        &mut views,
        QueueApplyTopWithQueuedStageInputs::new(
            QueueApplyTopWithDirectApplyInputs::new(
                QueueApplyTopWithFeeContextInputs::new(
                    QueueApplyCallInputs::new(
                        &rules,
                        true,
                        SeqProxy::sequence(8),
                        SeqProxy::sequence(8),
                        true,
                    ),
                    QueueApplyFeeContextInputs {
                        calculated_base_fee_drops: 10,
                        fee_paid_drops: 20,
                        default_base_fee_drops: 10,
                        metrics_snapshot: QueueFeeMetricsSnapshot {
                            txns_expected: 32,
                            escalation_multiplier: TXQ_BASE_LEVEL * 500,
                        },
                        open_ledger_tx_count: 0,
                        flags: ApplyFlags::NONE,
                    },
                ),
                "ABC123",
                &account,
            ),
            QueueApplyQueuedWithFeeContextInputs {
                account: account.clone(),
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
                order: &order,
            },
        ),
        &PreflightResult::new(
            "tx",
            None::<&str>,
            rules.clone(),
            TxConsequences::new(1, SeqProxy::sequence(8)),
            ApplyFlags::NONE,
            "journal",
            Ter::TES_SUCCESS,
        ),
        |_views, prepared| {
            direct_called.set(true);
            assert_eq!(
                prepared,
                PreparedDirectApply {
                    transaction_id: "ABC123",
                    applied_account: &account,
                    applied_seq_proxy: SeqProxy::sequence(8),
                }
            );
            DirectApplyExecution {
                transaction_id: "ABC123",
                attempt: DirectApplyAttemptResult {
                    apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, true),
                    removed_replacement: None,
                },
            }
        },
        |_| {
            prepare_called.set(true);
            true
        },
        |_| {
            preclaim_called.set(true);
            PreclaimResult::new(
                100,
                "tx",
                None::<&str>,
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            )
        },
        || {
            try_clear_called.set(true);
            ApplyResult::new(Ter::TES_SUCCESS, true, true)
        },
        || sandbox_called.set(true),
    );

    assert!(direct_called.get());
    assert!(!prepare_called.get());
    assert!(!preclaim_called.get());
    assert!(!try_clear_called.get());
    assert!(!sandbox_called.get());
    assert!(matches!(
        stage,
        QueueApplyPreflightStage::Entry(QueueApplyEntryStage::DirectApplied(_))
    ));
}

#[test]
fn after_preflight_caller_direct_apply_and_caller_queued_stage_exposes_prepared_queue_boundary() {
    let rules = Rules::new(std::iter::empty());
    let account = String::from("acct");
    let mut views =
        QueueViews::<String, MaybeTx<&'static str, String, &'static str, &'static str>>::new(
            BTreeMap::new(),
            vec![],
        );
    let order = OrderCandidates::new(Uint256::from_u64(0));
    let direct_called = Cell::new(false);
    let queued_called = Cell::new(false);
    let fee_context_inputs = QueueApplyFeeContextInputs {
        calculated_base_fee_drops: 10,
        fee_paid_drops: 20,
        default_base_fee_drops: 10,
        metrics_snapshot: QueueFeeMetricsSnapshot {
            txns_expected: 32,
            escalation_multiplier: TXQ_BASE_LEVEL * 500,
        },
        open_ledger_tx_count: 0,
        flags: ApplyFlags::NONE,
    };
    let expected_fee_context = evaluate_queue_apply_fee_context(fee_context_inputs);
    let preflight_result = PreflightResult::new(
        "tx",
        None::<&str>,
        rules.clone(),
        TxConsequences::new(1, SeqProxy::sequence(6)),
        ApplyFlags::NONE,
        "journal",
        Ter::TES_SUCCESS,
    );

    let stage = run_queue_apply_after_preflight_with_caller_direct_apply_and_caller_queued_stage(
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
                    fee_context_inputs,
                ),
                "ABC123",
                &account,
            ),
            QueueApplyQueuedWithFeeContextInputs {
                account: account.clone(),
                preflight: QueueHoldPreflight::new(false, false, ApplyFlags::NONE, Some(250)),
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
        &preflight_result,
        |_views, _prepared| {
            direct_called.set(true);
            unreachable!("sequence mismatch should skip direct apply")
        },
        |_views, queued| {
            queued_called.set(true);
            assert_eq!(queued.account_seq_proxy, SeqProxy::sequence(5));
            assert_eq!(queued.tx_seq_proxy, SeqProxy::sequence(6));
            assert_eq!(queued.queued.account, account);
            assert_eq!(queued.queued.last_valid, Some(250));
            assert_eq!(queued.fee_context, expected_fee_context);
            assert_eq!(queued.preflight_result, preflight_result);

            QueueApplyQueuedStage::MultiTxn(crate::QueueApplyMultiTxnStage::RejectPath(
                Ter::TER_PRE_SEQ,
            ))
        },
    );

    assert!(!direct_called.get());
    assert!(queued_called.get());
    assert_eq!(
        stage,
        QueueApplyPreflightStage::Entry(QueueApplyEntryStage::Queued(
            QueueApplyQueuedStage::MultiTxn(crate::QueueApplyMultiTxnStage::RejectPath(
                Ter::TER_PRE_SEQ,
            )),
        ))
    );
}

#[test]
fn acquired_direct_apply_wrapper_returns_supplied_execution_before_queue_flow() {
    let prepare_called = Cell::new(false);
    let preclaim_called = Cell::new(false);
    let try_clear_called = Cell::new(false);
    let sandbox_called = Cell::new(false);

    let rules = Rules::new(std::iter::empty());
    let mut views = QueueViews::<
        &'static str,
        MaybeTx<&'static str, &'static str, &'static str, &'static str>,
    >::new(BTreeMap::new(), vec![]);
    let order = OrderCandidates::new(Uint256::from_u64(0));
    let direct = DirectApplyExecution {
        transaction_id: "ABC123",
        attempt: DirectApplyAttemptResult {
            apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, true),
            removed_replacement: None::<crate::FeeQueueKey<&'static str>>,
        },
    };

    let stage = run_queue_apply_after_preflight_with_acquired_direct_apply_and_queued_stage(
        &mut views,
        QueueApplyTopWithQueuedStageInputs::new(
            QueueApplyTopWithDirectApplyInputs::new(
                QueueApplyTopWithFeeContextInputs::new(
                    QueueApplyCallInputs::new(
                        &rules,
                        true,
                        SeqProxy::sequence(8),
                        SeqProxy::sequence(8),
                        true,
                    ),
                    QueueApplyFeeContextInputs {
                        calculated_base_fee_drops: 10,
                        fee_paid_drops: 20,
                        default_base_fee_drops: 10,
                        metrics_snapshot: QueueFeeMetricsSnapshot {
                            txns_expected: 32,
                            escalation_multiplier: TXQ_BASE_LEVEL * 500,
                        },
                        open_ledger_tx_count: 0,
                        flags: ApplyFlags::NONE,
                    },
                ),
                "ABC123",
                &"acct",
            ),
            QueueApplyQueuedWithFeeContextInputs {
                account: "acct",
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
                order: &order,
            },
        ),
        &PreflightResult::new(
            "tx",
            None::<&str>,
            rules.clone(),
            TxConsequences::new(1, SeqProxy::sequence(8)),
            ApplyFlags::NONE,
            "journal",
            Ter::TES_SUCCESS,
        ),
        Some(direct.clone()),
        |_| {
            prepare_called.set(true);
            true
        },
        |_| {
            preclaim_called.set(true);
            PreclaimResult::new(
                100,
                "tx",
                None::<&str>,
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            )
        },
        || {
            try_clear_called.set(true);
            ApplyResult::new(Ter::TES_SUCCESS, true, true)
        },
        || sandbox_called.set(true),
    );

    assert_eq!(
        stage,
        QueueApplyPreflightStage::Entry(QueueApplyEntryStage::DirectApplied(direct))
    );
    assert!(!prepare_called.get());
    assert!(!preclaim_called.get());
    assert!(!try_clear_called.get());
    assert!(!sandbox_called.get());
}
