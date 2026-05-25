use std::{cell::RefCell, collections::BTreeMap};

use basics::base_uint::Uint256;
use protocol::{Rules, SeqProxy, Ter};
use tx::{
    ApplyFlags, FeeQueueEntry, FeeQueueKey, MaybeTx, MaybeTxCore, OrderCandidates, PreflightResult,
    QueueAdvanceCandidate, QueueApplyFullQueueDecision, QueueApplyHoldFallback,
    QueueApplyQueueLogMessages, QueueApplyQueueStage, QueueViews, TxConsequences, TxQAccount,
    format_queue_apply_enqueue_debug_message, format_queue_apply_full_queue_evict_info_message,
    format_queue_apply_full_queue_lower_fee_info_message,
    format_queue_apply_full_queue_same_account_info_message, run_queue_apply_queue_stage,
    run_queue_apply_queue_stage_with_log_messages,
    run_queue_apply_queue_stage_with_log_messages_and_caller_enqueue,
    run_queue_apply_queue_stage_with_log_sinks,
};

fn make_preflight(
    tx: &'static str,
    seq_proxy: SeqProxy,
    flags: ApplyFlags,
    ter: Ter,
) -> PreflightResult<&'static str, TxConsequences, &'static str, &'static str> {
    PreflightResult::new(
        tx,
        None,
        Rules::new(std::iter::empty()),
        TxConsequences::new(1, seq_proxy),
        flags,
        "journal",
        ter,
    )
}

fn queued(
    account: &'static str,
    seq_proxy: SeqProxy,
    tx_id: u64,
    fee_level: u64,
    flags: ApplyFlags,
) -> MaybeTx<&'static str, &'static str, &'static str, &'static str> {
    MaybeTx::new(
        Uint256::from_u64(tx_id),
        fee_level,
        account,
        Some(200),
        seq_proxy,
        flags,
        make_preflight("tx", seq_proxy, flags, Ter::TES_SUCCESS),
    )
}

fn fee_entry(
    account: &'static str,
    seq_proxy: SeqProxy,
    tx_id: u64,
    fee_level: u64,
) -> FeeQueueEntry<&'static str> {
    FeeQueueEntry::new(
        FeeQueueKey::new(account, seq_proxy),
        QueueAdvanceCandidate {
            fee_level,
            tx_id: Uint256::from_u64(tx_id),
            seq_proxy,
        },
    )
}

#[test]
fn queue_apply_queue_log_formatters_match_current_cpp_strings() {
    assert_eq!(
        format_queue_apply_full_queue_same_account_info_message(Uint256::from_u64(9), "rAlice"),
        "Queue is full, and transaction 0000000000000000000000000000000000000000000000000000000000000009 would kick a transaction from the same account (rAlice) out of the queue."
    );
    assert_eq!(
        format_queue_apply_full_queue_evict_info_message("rBob", 50, Uint256::from_u64(9), 110),
        "Removing last item of account rBob from queue with average fee of 50 in favor of 0000000000000000000000000000000000000000000000000000000000000009 with fee of 110"
    );
    assert_eq!(
        format_queue_apply_full_queue_lower_fee_info_message(Uint256::from_u64(9)),
        "Queue is full, and transaction 0000000000000000000000000000000000000000000000000000000000000009 fee is lower than end item's account average fee"
    );
    assert_eq!(
        format_queue_apply_enqueue_debug_message(
            Uint256::from_u64(9),
            Ter::TES_SUCCESS,
            true,
            "rAlice",
            ApplyFlags::FAIL_HARD
        ),
        "Added transaction 0000000000000000000000000000000000000000000000000000000000000009 with result tesSUCCESS from existing account rAlice to queue. Flags: 16"
    );
}

#[test]
fn queue_apply_queue_stage_with_log_sinks_preserves_cpp_info_then_debug_order() {
    let mut account_a = TxQAccount::new("a");
    account_a.add(
        SeqProxy::sequence(5),
        MaybeTxCore::new(
            queued("a", SeqProxy::sequence(5), 5, 90, ApplyFlags::NONE),
            TxConsequences::new(1, SeqProxy::sequence(5)),
        ),
    );

    let mut account_b = TxQAccount::new("b");
    account_b.add(
        SeqProxy::sequence(8),
        MaybeTxCore::new(
            queued("b", SeqProxy::sequence(8), 8, 50, ApplyFlags::NONE),
            TxConsequences::new(1, SeqProxy::sequence(8)),
        ),
    );

    let mut views = QueueViews::new(
        BTreeMap::from([("a", account_a), ("b", account_b)]),
        vec![
            fee_entry("a", SeqProxy::sequence(5), 5, 90),
            fee_entry("b", SeqProxy::sequence(8), 8, 50),
        ],
    );

    let events = RefCell::new(Vec::new());
    let stage = run_queue_apply_queue_stage_with_log_sinks(
        &mut views,
        QueueApplyHoldFallback::AlreadyVerifiedByMultiTxn,
        QueueApplyFullQueueDecision::EvictCheapest {
            dropped: FeeQueueKey::new("b", SeqProxy::sequence(8)),
            end_effective_fee_level: 50,
        },
        None,
        "c",
        Uint256::from_u64(9),
        Some(250),
        SeqProxy::sequence(6),
        110,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        make_preflight(
            "tx",
            SeqProxy::sequence(6),
            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
            Ter::TES_SUCCESS,
        ),
        &OrderCandidates::new(Uint256::from_u64(0)),
        |message| events.borrow_mut().push(("debug", message)),
        |message| events.borrow_mut().push(("info", message)),
    );

    assert!(matches!(stage, QueueApplyQueueStage::Queued(_)));
    assert_eq!(
        events.into_inner(),
        vec![
            (
                "info",
                format_queue_apply_full_queue_evict_info_message(
                    "b",
                    50,
                    Uint256::from_u64(9),
                    110
                ),
            ),
            (
                "debug",
                format_queue_apply_enqueue_debug_message(
                    Uint256::from_u64(9),
                    Ter::TES_SUCCESS,
                    false,
                    "c",
                    ApplyFlags::FAIL_HARD
                ),
            ),
        ]
    );
}

#[test]
fn queue_apply_queue_stage_with_log_messages_captures_same_account_full_reject() {
    let mut account_a = TxQAccount::new("a");
    account_a.add(
        SeqProxy::sequence(5),
        MaybeTxCore::new(
            queued("a", SeqProxy::sequence(5), 5, 90, ApplyFlags::NONE),
            TxConsequences::new(1, SeqProxy::sequence(5)),
        ),
    );

    let mut views = QueueViews::new(
        BTreeMap::from([("a", account_a)]),
        vec![fee_entry("a", SeqProxy::sequence(5), 5, 90)],
    );

    let result = run_queue_apply_queue_stage_with_log_messages(
        &mut views,
        QueueApplyHoldFallback::HoldAllowed,
        QueueApplyFullQueueDecision::RejectFullSameAccount,
        None,
        "a",
        Uint256::from_u64(9),
        Some(250),
        SeqProxy::sequence(6),
        110,
        ApplyFlags::NONE,
        make_preflight(
            "tx",
            SeqProxy::sequence(6),
            ApplyFlags::NONE,
            Ter::TES_SUCCESS,
        ),
        &OrderCandidates::new(Uint256::from_u64(0)),
    );

    assert_eq!(result.stage, QueueApplyQueueStage::RejectFull);
    assert_eq!(
        result.log_messages,
        QueueApplyQueueLogMessages {
            trace: vec![],
            debug: vec![],
            info: vec![format_queue_apply_full_queue_same_account_info_message(
                Uint256::from_u64(9),
                "a",
            )],
        }
    );
}

#[test]
fn queue_apply_queue_stage_with_log_messages_captures_lower_fee_full_reject() {
    let mut views = QueueViews::new(BTreeMap::<&str, _>::new(), Vec::new());

    let result = run_queue_apply_queue_stage_with_log_messages(
        &mut views,
        QueueApplyHoldFallback::HoldAllowed,
        QueueApplyFullQueueDecision::RejectFullLowerFee {
            end_effective_fee_level: 50,
        },
        None,
        "a",
        Uint256::from_u64(9),
        Some(250),
        SeqProxy::sequence(6),
        110,
        ApplyFlags::NONE,
        make_preflight(
            "tx",
            SeqProxy::sequence(6),
            ApplyFlags::NONE,
            Ter::TES_SUCCESS,
        ),
        &OrderCandidates::new(Uint256::from_u64(0)),
    );

    assert_eq!(result.stage, QueueApplyQueueStage::RejectFull);
    assert_eq!(
        result.log_messages,
        QueueApplyQueueLogMessages {
            trace: vec![],
            debug: vec![],
            info: vec![format_queue_apply_full_queue_lower_fee_info_message(
                Uint256::from_u64(9),
            )],
        }
    );
}

#[test]
fn plain_queue_stage_still_matches_log_wrapper_stage_result() {
    let mut views = QueueViews::new(BTreeMap::<&str, TxQAccount<&str, _>>::new(), Vec::new());
    let plain = run_queue_apply_queue_stage(
        &mut views,
        QueueApplyHoldFallback::HoldAllowed,
        QueueApplyFullQueueDecision::RejectFullLowerFee {
            end_effective_fee_level: 50,
        },
        None,
        "a",
        Uint256::from_u64(9),
        Some(250),
        SeqProxy::sequence(6),
        110,
        ApplyFlags::NONE,
        make_preflight(
            "tx",
            SeqProxy::sequence(6),
            ApplyFlags::NONE,
            Ter::TES_SUCCESS,
        ),
        &OrderCandidates::new(Uint256::from_u64(0)),
    );

    let wrapped = run_queue_apply_queue_stage_with_log_messages(
        &mut QueueViews::new(BTreeMap::<&str, TxQAccount<&str, _>>::new(), Vec::new()),
        QueueApplyHoldFallback::HoldAllowed,
        QueueApplyFullQueueDecision::RejectFullLowerFee {
            end_effective_fee_level: 50,
        },
        None,
        "a",
        Uint256::from_u64(9),
        Some(250),
        SeqProxy::sequence(6),
        110,
        ApplyFlags::NONE,
        make_preflight(
            "tx",
            SeqProxy::sequence(6),
            ApplyFlags::NONE,
            Ter::TES_SUCCESS,
        ),
        &OrderCandidates::new(Uint256::from_u64(0)),
    );

    assert_eq!(plain, wrapped.stage);
}

#[test]
fn queue_apply_queue_stage_with_log_messages_and_caller_enqueue_keeps_cpp_log_order() {
    let mut account_a = TxQAccount::new("a");
    account_a.add(
        SeqProxy::sequence(5),
        MaybeTxCore::new(
            queued("a", SeqProxy::sequence(5), 5, 90, ApplyFlags::NONE),
            TxConsequences::new(1, SeqProxy::sequence(5)),
        ),
    );

    let mut account_b = TxQAccount::new("b");
    account_b.add(
        SeqProxy::sequence(8),
        MaybeTxCore::new(
            queued("b", SeqProxy::sequence(8), 8, 50, ApplyFlags::NONE),
            TxConsequences::new(1, SeqProxy::sequence(8)),
        ),
    );

    let mut views = QueueViews::new(
        BTreeMap::from([("a", account_a), ("b", account_b)]),
        vec![
            fee_entry("a", SeqProxy::sequence(5), 5, 90),
            fee_entry("b", SeqProxy::sequence(8), 8, 50),
        ],
    );
    let preflight = make_preflight(
        "tx",
        SeqProxy::sequence(6),
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        Ter::TES_SUCCESS,
    );
    let order = OrderCandidates::new(Uint256::from_u64(0));

    let result = run_queue_apply_queue_stage_with_log_messages_and_caller_enqueue(
        &mut views,
        QueueApplyHoldFallback::AlreadyVerifiedByMultiTxn,
        QueueApplyFullQueueDecision::EvictCheapest {
            dropped: FeeQueueKey::new("b", SeqProxy::sequence(8)),
            end_effective_fee_level: 50,
        },
        None,
        "c",
        Uint256::from_u64(9),
        Some(250),
        SeqProxy::sequence(6),
        110,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        preflight.clone(),
        &order,
        |views, prepared| {
            assert_eq!(
                prepared.full_queue_decision,
                QueueApplyFullQueueDecision::EvictCheapest {
                    dropped: FeeQueueKey::new("b", SeqProxy::sequence(8)),
                    end_effective_fee_level: 50,
                }
            );
            assert_eq!(prepared.pf_result, preflight.clone());
            assert!(std::ptr::eq(prepared.order, &order));
            tx::apply_queue_apply_full_queue_decision(views, prepared.full_queue_decision)
        },
        |_views, prepared| {
            assert_eq!(prepared.account, "c");
            assert_eq!(prepared.tx_id, Uint256::from_u64(9));
            assert_eq!(prepared.seq_proxy, SeqProxy::sequence(6));
            assert_eq!(prepared.fee_level, 110);
            assert_eq!(prepared.flags, ApplyFlags::FAIL_HARD | ApplyFlags::RETRY);
            assert_eq!(prepared.pf_result, preflight);
            assert!(std::ptr::eq(prepared.order, &order));

            tx::QueueApplyEnqueueResult {
                queued: FeeQueueKey::new("c", SeqProxy::sequence(6)),
                removed_replacement: None,
                account_created: true,
                stored_flags: ApplyFlags::FAIL_HARD,
            }
        },
    );

    assert_eq!(
        result.stage,
        QueueApplyQueueStage::Queued(tx::QueueApplyEnqueueResult {
            queued: FeeQueueKey::new("c", SeqProxy::sequence(6)),
            removed_replacement: None,
            account_created: true,
            stored_flags: ApplyFlags::FAIL_HARD,
        })
    );
    assert_eq!(
        result.log_messages,
        QueueApplyQueueLogMessages {
            trace: vec![],
            debug: vec![format_queue_apply_enqueue_debug_message(
                Uint256::from_u64(9),
                Ter::TER_QUEUED,
                false,
                "c",
                ApplyFlags::FAIL_HARD,
            )],
            info: vec![format_queue_apply_full_queue_evict_info_message(
                "b",
                50,
                Uint256::from_u64(9),
                110,
            )],
        }
    );
}
