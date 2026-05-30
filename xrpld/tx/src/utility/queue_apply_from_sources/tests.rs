use std::{cell::Cell, cell::RefCell, collections::BTreeMap};

use basics::base_uint::Uint256;
use protocol::{Rules, SeqProxy, Ter};

use super::{
    QueueApplyAfterPreflightSourceInputs, QueueApplyTopFromSourcesInputs,
    build_queue_apply_top_with_queued_stage_inputs_from_sources,
    build_queue_apply_top_with_queued_stage_inputs_from_sources_after_preflight,
    run_queue_apply_after_preflight_from_sources,
    run_queue_apply_after_preflight_with_acquired_direct_apply_from_sources,
    run_queue_apply_after_preflight_with_caller_direct_apply_and_caller_queued_stage_from_sources,
    run_queue_apply_after_preflight_with_caller_direct_apply_from_sources,
    run_queue_apply_after_preflight_with_direct_apply_from_sources,
    run_queue_apply_top_with_caller_direct_apply_and_caller_queued_stage_from_sources,
    run_queue_apply_top_with_caller_direct_apply_and_queued_stage_from_sources,
    run_queue_apply_top_with_queued_stage_from_sources,
};
use crate::{
    ApplyFlags, ApplyResult, DirectApplyAttemptResult, DirectApplyExecution, MaybeTx, MaybeTxCore,
    OrderCandidates, PreclaimResult, PreflightResult, PreparedDirectApply,
    QueueApplyFeeContextInputs, QueueApplyObservedAccountLookup, QueueApplyObservedQueue,
    QueueApplyObservedTicketLookup, QueueApplyObservedTxSource, QueueApplyObservedViewSource,
    QueueApplyPreflightStage, QueueApplyPreparedQueuedStageInputs, QueueApplyQueuedStage,
    QueueFeeMetricsSnapshot, QueueHoldPreflight, QueueViews, TXQ_BASE_LEVEL, TxConsequences,
    TxConsequencesCategory, TxQAccount, build_queue_apply_top_read_inputs_from_sources,
    build_queue_apply_top_with_queued_stage_inputs, evaluate_queue_apply_fee_context,
};

#[derive(Debug)]
struct TestObservedTxSource<'a> {
    account: &'a String,
    transaction_id: &'static str,
    tx_id: Uint256,
    tx_seq_proxy: SeqProxy,
}

impl QueueApplyObservedTxSource for TestObservedTxSource<'_> {
    type Account = String;
    type TransactionId = &'static str;

    fn account(&self) -> &Self::Account {
        self.account
    }

    fn transaction_id(&self) -> Self::TransactionId {
        self.transaction_id
    }

    fn tx_id(&self) -> Uint256 {
        self.tx_id
    }

    fn tx_seq_proxy(&self) -> SeqProxy {
        self.tx_seq_proxy
    }
}

#[derive(Debug)]
struct TestObservedViewSource {
    rules: Rules,
    account_lookup: QueueApplyObservedAccountLookup,
    ticket_lookup: QueueApplyObservedTicketLookup,
    calculated_base_fee_drops: i64,
    fee_paid_drops: i64,
    default_base_fee_drops: i64,
    metrics_snapshot: QueueFeeMetricsSnapshot,
    open_ledger_tx_count: usize,
    open_ledger_seq: u32,
    reserve_drops: u64,
    base_fee_drops: u64,
}

impl QueueApplyObservedViewSource<String> for TestObservedViewSource {
    fn rules(&self) -> &Rules {
        &self.rules
    }

    fn account_lookup(&self, _account: &String) -> QueueApplyObservedAccountLookup {
        self.account_lookup
    }

    fn ticket_lookup(
        &self,
        _account: &String,
        _tx_seq_proxy: SeqProxy,
    ) -> QueueApplyObservedTicketLookup {
        self.ticket_lookup
    }

    fn calculated_base_fee_drops(&self) -> i64 {
        self.calculated_base_fee_drops
    }

    fn fee_paid_drops(&self) -> i64 {
        self.fee_paid_drops
    }

    fn default_base_fee_drops(&self) -> i64 {
        self.default_base_fee_drops
    }

    fn metrics_snapshot(&self) -> QueueFeeMetricsSnapshot {
        self.metrics_snapshot
    }

    fn open_ledger_tx_count(&self) -> usize {
        self.open_ledger_tx_count
    }

    fn open_ledger_seq(&self) -> u32 {
        self.open_ledger_seq
    }

    fn reserve_drops(&self) -> u64 {
        self.reserve_drops
    }

    fn base_fee_drops(&self) -> u64 {
        self.base_fee_drops
    }
}

#[test]
fn source_builder_matches_manual_lowering_chain() {
    let rules = Rules::new(std::iter::empty());
    let order = OrderCandidates::new(Uint256::from_u64(0));
    let account = String::from("acct");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(8),
    };
    let view_source = TestObservedViewSource {
        rules: rules.clone(),
        account_lookup: QueueApplyObservedAccountLookup::Present {
            sequence: 8,
            balance_drops: 1_000,
        },
        ticket_lookup: QueueApplyObservedTicketLookup::Present,
        calculated_base_fee_drops: 10,
        fee_paid_drops: 20,
        default_base_fee_drops: 10,
        metrics_snapshot: QueueFeeMetricsSnapshot {
            txns_expected: 32,
            escalation_multiplier: TXQ_BASE_LEVEL * 500,
        },
        open_ledger_tx_count: 4,
        open_ledger_seq: 100,
        reserve_drops: 200,
        base_fee_drops: 10,
    };
    let inputs = QueueApplyTopFromSourcesInputs::new(
        QueueHoldPreflight::new(false, false, ApplyFlags::NONE, Some(250)),
        ApplyFlags::NONE,
        TxConsequences::with_sequences_consumed(1, SeqProxy::sequence(8), 1),
        QueueApplyObservedQueue {
            minimum_last_ledger_buffer: 2,
            maximum_txn_per_account: 10,
            retry_sequence_percent: 25,
            queue_is_full: false,
            can_be_held_result: Ter::TES_SUCCESS,
            order: &order,
        },
    );

    let built = build_queue_apply_top_with_queued_stage_inputs_from_sources(
        &tx_source,
        &view_source,
        inputs,
    );
    let manual = build_queue_apply_top_with_queued_stage_inputs(
        build_queue_apply_top_read_inputs_from_sources(
            &tx_source,
            &view_source,
            inputs.preflight,
            inputs.flags,
            inputs.consequences,
            inputs.queue,
        ),
    );

    assert_eq!(
        built.direct_apply.top.call.account_exists,
        manual.direct_apply.top.call.account_exists
    );
    assert_eq!(
        built.direct_apply.top.call.account_seq_proxy,
        manual.direct_apply.top.call.account_seq_proxy
    );
    assert_eq!(
        built.direct_apply.top.call.tx_seq_proxy,
        manual.direct_apply.top.call.tx_seq_proxy
    );
    assert_eq!(
        built.direct_apply.top.call.ticket_exists,
        manual.direct_apply.top.call.ticket_exists
    );
    assert_eq!(
        built
            .direct_apply
            .top
            .fee_context_inputs
            .calculated_base_fee_drops,
        manual
            .direct_apply
            .top
            .fee_context_inputs
            .calculated_base_fee_drops
    );
    assert_eq!(
        built.direct_apply.top.fee_context_inputs.fee_paid_drops,
        manual.direct_apply.top.fee_context_inputs.fee_paid_drops
    );
    assert_eq!(
        built
            .direct_apply
            .top
            .fee_context_inputs
            .default_base_fee_drops,
        manual
            .direct_apply
            .top
            .fee_context_inputs
            .default_base_fee_drops
    );
    assert_eq!(
        built.direct_apply.top.fee_context_inputs.metrics_snapshot,
        manual.direct_apply.top.fee_context_inputs.metrics_snapshot
    );
    assert_eq!(
        built.direct_apply.transaction_id,
        manual.direct_apply.transaction_id
    );
    assert_eq!(built.queued.account, manual.queued.account);
    assert_eq!(built.queued.balance_drops, manual.queued.balance_drops);
    assert_eq!(built.queued.last_valid, manual.queued.last_valid);
    assert_eq!(built.queued.tx_id, manual.queued.tx_id);
}

#[test]
fn source_runner_executes_existing_top_chain_without_rewiring_order() {
    let rules = Rules::new(std::iter::empty());
    let order = OrderCandidates::new(Uint256::from_u64(0));
    let account = String::from("acct");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(6),
    };
    let view_source = TestObservedViewSource {
        rules: rules.clone(),
        account_lookup: QueueApplyObservedAccountLookup::Present {
            sequence: 5,
            balance_drops: 1_000,
        },
        ticket_lookup: QueueApplyObservedTicketLookup::Present,
        calculated_base_fee_drops: 10,
        fee_paid_drops: 1,
        default_base_fee_drops: 10,
        metrics_snapshot: QueueFeeMetricsSnapshot {
            txns_expected: 32,
            escalation_multiplier: TXQ_BASE_LEVEL * 500,
        },
        open_ledger_tx_count: 4,
        open_ledger_seq: 100,
        reserve_drops: 200,
        base_fee_drops: 10,
    };

    let queued_account_id = String::from("acct");
    let mut queued_account = TxQAccount::new(queued_account_id.clone());
    queued_account.add(
        SeqProxy::sequence(5),
        MaybeTxCore::new(
            MaybeTx::new(
                Uint256::from_u64(5),
                90,
                queued_account_id.clone(),
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

    let mut views = QueueViews::new(
        BTreeMap::from([(queued_account_id, queued_account)]),
        vec![],
    );
    let try_clear_called = Cell::new(false);
    let sandbox_called = Cell::new(false);

    let stage = run_queue_apply_top_with_queued_stage_from_sources(
        &mut views,
        &tx_source,
        &view_source,
        QueueApplyTopFromSourcesInputs::new(
            QueueHoldPreflight::new(false, false, ApplyFlags::NONE, Some(250)),
            ApplyFlags::NONE,
            TxConsequences::with_category(
                1,
                SeqProxy::sequence(6),
                TxConsequencesCategory::Blocker,
            ),
            QueueApplyObservedQueue {
                minimum_last_ledger_buffer: 2,
                maximum_txn_per_account: 10,
                retry_sequence_percent: 25,
                queue_is_full: false,
                can_be_held_result: Ter::TES_SUCCESS,
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
            try_clear_called.set(true);
            unreachable!("queue-stage account rejection should happen before try-clear")
        },
        || sandbox_called.set(true),
    );

    assert_eq!(
        stage,
        QueueApplyPreflightStage::Entry(crate::QueueApplyEntryStage::Queued(
            QueueApplyQueuedStage::Account(crate::QueueApplyAccountStage::RejectBlockerAdmission(
                crate::BlockerQueueAdmission::RejectsNonReplacementOfLoneEntry,
            )),
        ))
    );
    assert!(!try_clear_called.get());
    assert!(!sandbox_called.get());
}

#[test]
fn source_runner_with_caller_direct_apply_exposes_prepared_execution_boundary() {
    let rules = Rules::new(std::iter::empty());
    let order = OrderCandidates::new(Uint256::from_u64(0));
    let account = String::from("acct");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(8),
    };
    let view_source = TestObservedViewSource {
        rules: rules.clone(),
        account_lookup: QueueApplyObservedAccountLookup::Present {
            sequence: 8,
            balance_drops: 1_000,
        },
        ticket_lookup: QueueApplyObservedTicketLookup::Present,
        calculated_base_fee_drops: 10,
        fee_paid_drops: 20,
        default_base_fee_drops: 10,
        metrics_snapshot: QueueFeeMetricsSnapshot {
            txns_expected: 32,
            escalation_multiplier: TXQ_BASE_LEVEL * 500,
        },
        open_ledger_tx_count: 0,
        open_ledger_seq: 100,
        reserve_drops: 200,
        base_fee_drops: 10,
    };
    let mut views =
        QueueViews::<String, MaybeTx<&'static str, String, &'static str, &'static str>>::new(
            BTreeMap::new(),
            vec![],
        );
    let direct_called = Cell::new(false);
    let try_clear_called = Cell::new(false);
    let sandbox_called = Cell::new(false);

    let stage = run_queue_apply_top_with_caller_direct_apply_and_queued_stage_from_sources(
        &mut views,
        &tx_source,
        &view_source,
        QueueApplyTopFromSourcesInputs::new(
            QueueHoldPreflight::new(false, false, ApplyFlags::NONE, Some(250)),
            ApplyFlags::NONE,
            TxConsequences::with_sequences_consumed(1, SeqProxy::sequence(8), 1),
            QueueApplyObservedQueue {
                minimum_last_ledger_buffer: 2,
                maximum_txn_per_account: 10,
                retry_sequence_percent: 25,
                queue_is_full: false,
                can_be_held_result: Ter::TES_SUCCESS,
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
        QueueApplyPreflightStage::Entry(crate::QueueApplyEntryStage::DirectApplied(_))
    ));
}

#[test]
fn source_runner_with_caller_direct_apply_and_caller_queued_stage_exposes_prepared_queue_boundary()
{
    let rules = Rules::new(std::iter::empty());
    let order = OrderCandidates::new(Uint256::from_u64(0));
    let account = String::from("acct");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(6),
    };
    let view_source = TestObservedViewSource {
        rules: rules.clone(),
        account_lookup: QueueApplyObservedAccountLookup::Present {
            sequence: 5,
            balance_drops: 1_000,
        },
        ticket_lookup: QueueApplyObservedTicketLookup::Present,
        calculated_base_fee_drops: 10,
        fee_paid_drops: 20,
        default_base_fee_drops: 10,
        metrics_snapshot: QueueFeeMetricsSnapshot {
            txns_expected: 32,
            escalation_multiplier: TXQ_BASE_LEVEL * 500,
        },
        open_ledger_tx_count: 0,
        open_ledger_seq: 100,
        reserve_drops: 200,
        base_fee_drops: 10,
    };
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

    let stage = run_queue_apply_top_with_caller_direct_apply_and_caller_queued_stage_from_sources(
        &mut views,
        &tx_source,
        &view_source,
        QueueApplyTopFromSourcesInputs::new(
            QueueHoldPreflight::new(false, false, ApplyFlags::NONE, Some(250)),
            ApplyFlags::NONE,
            expected_consequences.clone(),
            QueueApplyObservedQueue {
                minimum_last_ledger_buffer: 2,
                maximum_txn_per_account: 10,
                retry_sequence_percent: 25,
                queue_is_full: false,
                can_be_held_result: Ter::TES_SUCCESS,
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
        QueueApplyPreflightStage::Entry(crate::QueueApplyEntryStage::Queued(
            QueueApplyQueuedStage::MultiTxn(crate::QueueApplyMultiTxnStage::RejectPath(
                Ter::TER_PRE_SEQ,
            )),
        ))
    );
}

#[test]
fn after_preflight_source_builder_derives_flags_and_consequences_from_preflight_result() {
    let rules = Rules::new(std::iter::empty());
    let order = OrderCandidates::new(Uint256::from_u64(0));
    let account = String::from("acct");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(8),
    };
    let view_source = TestObservedViewSource {
        rules: rules.clone(),
        account_lookup: QueueApplyObservedAccountLookup::Present {
            sequence: 8,
            balance_drops: 1_000,
        },
        ticket_lookup: QueueApplyObservedTicketLookup::Present,
        calculated_base_fee_drops: 10,
        fee_paid_drops: 20,
        default_base_fee_drops: 10,
        metrics_snapshot: QueueFeeMetricsSnapshot {
            txns_expected: 32,
            escalation_multiplier: TXQ_BASE_LEVEL * 500,
        },
        open_ledger_tx_count: 4,
        open_ledger_seq: 100,
        reserve_drops: 200,
        base_fee_drops: 10,
    };
    let preflight_result = PreflightResult::new(
        "tx",
        None::<&str>,
        rules.clone(),
        TxConsequences::with_category(1, SeqProxy::sequence(8), TxConsequencesCategory::Blocker),
        ApplyFlags::FAIL_HARD,
        "journal",
        Ter::TES_SUCCESS,
    );

    let built = build_queue_apply_top_with_queued_stage_inputs_from_sources_after_preflight(
        &tx_source,
        &view_source,
        &preflight_result,
        QueueApplyAfterPreflightSourceInputs::new(
            QueueHoldPreflight::new(false, false, ApplyFlags::NONE, Some(250)),
            QueueApplyObservedQueue {
                minimum_last_ledger_buffer: 2,
                maximum_txn_per_account: 10,
                retry_sequence_percent: 25,
                queue_is_full: false,
                can_be_held_result: Ter::TES_SUCCESS,
                order: &order,
            },
        ),
    );

    assert_eq!(
        built.direct_apply.top.fee_context_inputs.flags,
        ApplyFlags::FAIL_HARD
    );
    assert!(built.queued.is_blocker);
    assert_eq!(built.queued.last_valid, Some(250));
}

#[test]
fn after_preflight_source_runner_reuses_existing_top_chain_without_rewiring_order() {
    let rules = Rules::new(std::iter::empty());
    let order = OrderCandidates::new(Uint256::from_u64(0));
    let account = String::from("acct");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(6),
    };
    let view_source = TestObservedViewSource {
        rules: rules.clone(),
        account_lookup: QueueApplyObservedAccountLookup::Present {
            sequence: 5,
            balance_drops: 1_000,
        },
        ticket_lookup: QueueApplyObservedTicketLookup::Present,
        calculated_base_fee_drops: 10,
        fee_paid_drops: 1,
        default_base_fee_drops: 10,
        metrics_snapshot: QueueFeeMetricsSnapshot {
            txns_expected: 32,
            escalation_multiplier: TXQ_BASE_LEVEL * 500,
        },
        open_ledger_tx_count: 4,
        open_ledger_seq: 100,
        reserve_drops: 200,
        base_fee_drops: 10,
    };

    let queued_account_id = String::from("acct");
    let mut queued_account = TxQAccount::new(queued_account_id.clone());
    queued_account.add(
        SeqProxy::sequence(5),
        MaybeTxCore::new(
            MaybeTx::new(
                Uint256::from_u64(5),
                90,
                queued_account_id.clone(),
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

    let mut views = QueueViews::new(
        BTreeMap::from([(queued_account_id, queued_account)]),
        vec![],
    );

    let stage = run_queue_apply_after_preflight_from_sources(
        &mut views,
        &tx_source,
        &view_source,
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
        QueueApplyAfterPreflightSourceInputs::new(
            QueueHoldPreflight::new(false, false, ApplyFlags::NONE, Some(250)),
            QueueApplyObservedQueue {
                minimum_last_ledger_buffer: 2,
                maximum_txn_per_account: 10,
                retry_sequence_percent: 25,
                queue_is_full: false,
                can_be_held_result: Ter::TES_SUCCESS,
                order: &order,
            },
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
        QueueApplyPreflightStage::Entry(crate::QueueApplyEntryStage::Queued(
            QueueApplyQueuedStage::Account(crate::QueueApplyAccountStage::RejectBlockerAdmission(
                crate::BlockerQueueAdmission::RejectsNonReplacementOfLoneEntry,
            )),
        ))
    );
}

#[test]
fn after_preflight_source_runner_with_direct_apply_bypasses_queue_hooks() {
    let prepare_called = Cell::new(false);
    let preclaim_called = Cell::new(false);
    let try_clear_called = Cell::new(false);
    let sandbox_called = Cell::new(false);
    let traces = RefCell::new(Vec::new());

    let rules = Rules::new(std::iter::empty());
    let order = OrderCandidates::new(Uint256::from_u64(0));
    let account = String::from("acct");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(8),
    };
    let view_source = TestObservedViewSource {
        rules: rules.clone(),
        account_lookup: QueueApplyObservedAccountLookup::Present {
            sequence: 8,
            balance_drops: 1_000,
        },
        ticket_lookup: QueueApplyObservedTicketLookup::Present,
        calculated_base_fee_drops: 10,
        fee_paid_drops: 20,
        default_base_fee_drops: 10,
        metrics_snapshot: QueueFeeMetricsSnapshot {
            txns_expected: 32,
            escalation_multiplier: TXQ_BASE_LEVEL * 500,
        },
        open_ledger_tx_count: 4,
        open_ledger_seq: 100,
        reserve_drops: 200,
        base_fee_drops: 10,
    };
    let mut views = QueueViews::new(BTreeMap::new(), vec![]);

    let stage = run_queue_apply_after_preflight_with_direct_apply_from_sources(
        &mut views,
        &tx_source,
        &view_source,
        &PreflightResult::new(
            "tx",
            None::<&str>,
            rules.clone(),
            TxConsequences::new(1, SeqProxy::sequence(8)),
            ApplyFlags::NONE,
            "journal",
            Ter::TES_SUCCESS,
        ),
        QueueApplyAfterPreflightSourceInputs::new(
            QueueHoldPreflight::new(false, false, ApplyFlags::NONE, Some(250)),
            QueueApplyObservedQueue {
                minimum_last_ledger_buffer: 2,
                maximum_txn_per_account: 10,
                retry_sequence_percent: 25,
                queue_is_full: false,
                can_be_held_result: Ter::TES_SUCCESS,
                order: &order,
            },
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
        QueueApplyPreflightStage::Entry(crate::QueueApplyEntryStage::DirectApplied(_))
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
fn after_preflight_source_runner_with_caller_direct_apply_exposes_prepared_boundary() {
    let prepare_called = Cell::new(false);
    let preclaim_called = Cell::new(false);
    let try_clear_called = Cell::new(false);
    let sandbox_called = Cell::new(false);
    let direct_called = Cell::new(false);

    let rules = Rules::new(std::iter::empty());
    let order = OrderCandidates::new(Uint256::from_u64(0));
    let account = String::from("acct");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(8),
    };
    let view_source = TestObservedViewSource {
        rules: rules.clone(),
        account_lookup: QueueApplyObservedAccountLookup::Present {
            sequence: 8,
            balance_drops: 1_000,
        },
        ticket_lookup: QueueApplyObservedTicketLookup::Present,
        calculated_base_fee_drops: 10,
        fee_paid_drops: 20,
        default_base_fee_drops: 10,
        metrics_snapshot: QueueFeeMetricsSnapshot {
            txns_expected: 32,
            escalation_multiplier: TXQ_BASE_LEVEL * 500,
        },
        open_ledger_tx_count: 4,
        open_ledger_seq: 100,
        reserve_drops: 200,
        base_fee_drops: 10,
    };
    let mut views = QueueViews::new(BTreeMap::new(), vec![]);

    let stage = run_queue_apply_after_preflight_with_caller_direct_apply_from_sources(
        &mut views,
        &tx_source,
        &view_source,
        &PreflightResult::new(
            "tx",
            None::<&str>,
            rules.clone(),
            TxConsequences::new(1, SeqProxy::sequence(8)),
            ApplyFlags::NONE,
            "journal",
            Ter::TES_SUCCESS,
        ),
        QueueApplyAfterPreflightSourceInputs::new(
            QueueHoldPreflight::new(false, false, ApplyFlags::NONE, Some(250)),
            QueueApplyObservedQueue {
                minimum_last_ledger_buffer: 2,
                maximum_txn_per_account: 10,
                retry_sequence_percent: 25,
                queue_is_full: false,
                can_be_held_result: Ter::TES_SUCCESS,
                order: &order,
            },
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
        QueueApplyPreflightStage::Entry(crate::QueueApplyEntryStage::DirectApplied(_))
    ));
}

#[test]
fn after_preflight_source_runner_with_caller_direct_apply_and_caller_queued_stage_exposes_prepared_queue_boundary()
 {
    let rules = Rules::new(std::iter::empty());
    let order = OrderCandidates::new(Uint256::from_u64(0));
    let account = String::from("acct");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(6),
    };
    let view_source = TestObservedViewSource {
        rules: rules.clone(),
        account_lookup: QueueApplyObservedAccountLookup::Present {
            sequence: 5,
            balance_drops: 1_000,
        },
        ticket_lookup: QueueApplyObservedTicketLookup::Present,
        calculated_base_fee_drops: 10,
        fee_paid_drops: 20,
        default_base_fee_drops: 10,
        metrics_snapshot: QueueFeeMetricsSnapshot {
            txns_expected: 32,
            escalation_multiplier: TXQ_BASE_LEVEL * 500,
        },
        open_ledger_tx_count: 0,
        open_ledger_seq: 100,
        reserve_drops: 200,
        base_fee_drops: 10,
    };
    let mut views = QueueViews::new(BTreeMap::new(), vec![]);
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

    let stage = run_queue_apply_after_preflight_with_caller_direct_apply_and_caller_queued_stage_from_sources(
            &mut views,
            &tx_source,
            &view_source,
            &preflight_result,
            QueueApplyAfterPreflightSourceInputs::new(
                QueueHoldPreflight::new(false, false, ApplyFlags::NONE, Some(250)),
                QueueApplyObservedQueue {
                    minimum_last_ledger_buffer: 2,
                    maximum_txn_per_account: 10,
                    retry_sequence_percent: 25,
                    queue_is_full: false,
                    can_be_held_result: Ter::TES_SUCCESS,
                    order: &order,
                },
            ),
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
        QueueApplyPreflightStage::Entry(crate::QueueApplyEntryStage::Queued(
            QueueApplyQueuedStage::MultiTxn(crate::QueueApplyMultiTxnStage::RejectPath(
                Ter::TER_PRE_SEQ,
            )),
        ))
    );
}

#[test]
fn after_preflight_source_runner_with_acquired_direct_apply_bypasses_queue_hooks() {
    let prepare_called = Cell::new(false);
    let preclaim_called = Cell::new(false);
    let try_clear_called = Cell::new(false);
    let sandbox_called = Cell::new(false);

    let rules = Rules::new(std::iter::empty());
    let order = OrderCandidates::new(Uint256::from_u64(0));
    let account = String::from("acct");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(8),
    };
    let view_source = TestObservedViewSource {
        rules: rules.clone(),
        account_lookup: QueueApplyObservedAccountLookup::Present {
            sequence: 8,
            balance_drops: 1_000,
        },
        ticket_lookup: QueueApplyObservedTicketLookup::Present,
        calculated_base_fee_drops: 10,
        fee_paid_drops: 20,
        default_base_fee_drops: 10,
        metrics_snapshot: QueueFeeMetricsSnapshot {
            txns_expected: 32,
            escalation_multiplier: TXQ_BASE_LEVEL * 500,
        },
        open_ledger_tx_count: 4,
        open_ledger_seq: 100,
        reserve_drops: 200,
        base_fee_drops: 10,
    };
    let mut views = QueueViews::new(BTreeMap::new(), vec![]);
    let direct = crate::DirectApplyExecution {
        transaction_id: "ABC123",
        attempt: crate::DirectApplyAttemptResult {
            apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, true),
            removed_replacement: None::<crate::FeeQueueKey<String>>,
        },
    };

    let stage = run_queue_apply_after_preflight_with_acquired_direct_apply_from_sources(
        &mut views,
        &tx_source,
        &view_source,
        &PreflightResult::new(
            "tx",
            None::<&str>,
            rules.clone(),
            TxConsequences::new(1, SeqProxy::sequence(8)),
            ApplyFlags::NONE,
            "journal",
            Ter::TES_SUCCESS,
        ),
        QueueApplyAfterPreflightSourceInputs::new(
            QueueHoldPreflight::new(false, false, ApplyFlags::NONE, Some(250)),
            QueueApplyObservedQueue {
                minimum_last_ledger_buffer: 2,
                maximum_txn_per_account: 10,
                retry_sequence_percent: 25,
                queue_is_full: false,
                can_be_held_result: Ter::TES_SUCCESS,
                order: &order,
            },
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
        QueueApplyPreflightStage::Entry(crate::QueueApplyEntryStage::DirectApplied(direct))
    );
    assert!(!prepare_called.get());
    assert!(!preclaim_called.get());
    assert!(!try_clear_called.get());
    assert!(!sandbox_called.get());
}
