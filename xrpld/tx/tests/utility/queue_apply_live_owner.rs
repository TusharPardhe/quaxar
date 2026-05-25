use std::{cell::RefCell, collections::BTreeMap};

use basics::base_uint::Uint256;
use protocol::{Rules, SeqProxy, Ter};
use tx::{
    ApplyFlags, ApplyResult, MaybeTx, MaybeTxCore, OrderCandidates, PreclaimResult,
    PreflightResult, QueueApplyAccountStage, QueueApplyAfterPreflightSourceInputs,
    QueueApplyEntryStage, QueueApplyFeeContextInputs, QueueApplyLiveOwner,
    QueueApplyLiveOwnerQueuedFrontStage, QueueApplyMultiTxnInputs, QueueApplyObservedAccountLookup,
    QueueApplyObservedTicketLookup, QueueApplyObservedTxSource, QueueApplyObservedViewSource,
    QueueApplyPreclaimStage, QueueApplyPreflightStage, QueueApplyPreparedPreclaimInputs,
    QueueApplyQueuedStage, QueueApplyQueuedWithFeeContextInputs, QueueFeeMetricsSnapshot,
    QueueHoldPreflight, QueueViews, TryClearAccountPlan, TryClearAccountResult, TxConsequences,
    TxConsequencesCategory, TxQAccount, TxQSetup, derive_queue_apply_prepared_flow_stage,
    evaluate_queue_apply_fee_context, run_queue_apply_account_stage,
    run_queue_apply_after_preflight_from_sources,
    run_queue_apply_after_preflight_with_caller_preclaim_and_log_sinks_from_sources,
    run_queue_apply_after_preflight_with_caller_preclaim_from_sources,
    run_queue_apply_after_preflight_with_live_owner_from_sources,
    run_queue_apply_after_preflight_with_live_owner_from_sources_and_caller_preclaim,
    run_queue_apply_after_preflight_with_live_owner_from_sources_and_caller_preclaim_and_log_sinks,
    run_queue_apply_after_preflight_with_live_owner_from_sources_and_log_messages,
    run_queue_apply_after_preflight_with_live_owner_from_sources_and_log_sinks,
    run_queue_apply_after_preflight_with_log_messages_from_sources,
    run_queue_apply_after_preflight_with_log_sinks_from_sources, run_queue_apply_multitxn_stage,
    run_queue_apply_top_with_caller_preclaim_and_log_sinks_from_sources,
    run_queue_apply_top_with_caller_preclaim_from_sources,
    run_queue_apply_top_with_log_messages_from_sources,
    run_queue_apply_top_with_log_sinks_from_sources,
    run_queue_apply_top_with_queued_stage_from_sources,
    run_queue_apply_with_live_owner_from_sources,
    run_queue_apply_with_live_owner_from_sources_and_caller_preclaim,
    run_queue_apply_with_live_owner_from_sources_and_caller_preclaim_and_log_sinks,
    run_queue_apply_with_live_owner_from_sources_and_log_messages,
    run_queue_apply_with_live_owner_from_sources_and_log_sinks,
};

#[derive(Debug)]
struct TestObservedTxSource<'a> {
    account: &'a String,
    transaction_id: &'static str,
    tx_id: Uint256,
    tx_seq_proxy: SeqProxy,
}

#[derive(Debug, Clone)]
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

fn rules() -> Rules {
    Rules::new(std::iter::empty())
}

fn hold_preflight() -> QueueHoldPreflight {
    QueueHoldPreflight::new(false, false, ApplyFlags::NONE, Some(250))
}

fn blocker_consequences(seq_proxy: SeqProxy) -> TxConsequences {
    TxConsequences::with_category(1, seq_proxy, TxConsequencesCategory::Blocker)
}

fn preflight_result(
    seq_proxy: SeqProxy,
) -> PreflightResult<&'static str, TxConsequences, &'static str, &'static str> {
    PreflightResult::new(
        "tx",
        None::<&str>,
        rules(),
        blocker_consequences(seq_proxy),
        ApplyFlags::NONE,
        "journal",
        Ter::TES_SUCCESS,
    )
}

fn tx_source<'a>(account: &'a String) -> TestObservedTxSource<'a> {
    TestObservedTxSource {
        account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(6),
    }
}

fn view_source() -> TestObservedViewSource {
    TestObservedViewSource {
        rules: rules(),
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
            escalation_multiplier: tx::TXQ_BASE_LEVEL * 500,
        },
        open_ledger_tx_count: 4,
        open_ledger_seq: 100,
        reserve_drops: 200,
        base_fee_drops: 10,
    }
}

fn build_views() -> QueueViews<String, MaybeTx<&'static str, String, &'static str, &'static str>> {
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
                preflight_result(SeqProxy::sequence(5)),
            ),
            blocker_consequences(SeqProxy::sequence(5)),
        ),
    );

    QueueViews::new(
        BTreeMap::from([(queued_account_id, queued_account)]),
        vec![],
    )
}

fn build_flow_views()
-> QueueViews<String, MaybeTx<&'static str, String, &'static str, &'static str>> {
    let queued_account_id = String::from("acct");
    let mut queued_account = TxQAccount::new(queued_account_id.clone());
    queued_account.add(
        SeqProxy::sequence(5),
        MaybeTxCore::new(
            MaybeTx::new(
                Uint256::from_u64(5),
                20,
                queued_account_id.clone(),
                Some(200),
                SeqProxy::sequence(5),
                ApplyFlags::NONE,
                PreflightResult::new(
                    "tx",
                    None::<&str>,
                    rules(),
                    TxConsequences::with_potential_spend(20, SeqProxy::sequence(5), 100),
                    ApplyFlags::NONE,
                    "journal",
                    Ter::TES_SUCCESS,
                ),
            ),
            TxConsequences::with_potential_spend(20, SeqProxy::sequence(5), 100),
        ),
    );
    queued_account.add(
        SeqProxy::sequence(7),
        MaybeTxCore::new(
            MaybeTx::new(
                Uint256::from_u64(7),
                15,
                queued_account_id.clone(),
                Some(200),
                SeqProxy::sequence(7),
                ApplyFlags::NONE,
                PreflightResult::new(
                    "tx",
                    None::<&str>,
                    rules(),
                    TxConsequences::with_potential_spend(15, SeqProxy::sequence(7), 50),
                    ApplyFlags::NONE,
                    "journal",
                    Ter::TES_SUCCESS,
                ),
            ),
            TxConsequences::with_potential_spend(15, SeqProxy::sequence(7), 50),
        ),
    );

    QueueViews::new(
        BTreeMap::from([(queued_account_id, queued_account)]),
        vec![],
    )
}

fn build_owner(
    current_max_size: Option<usize>,
) -> QueueApplyLiveOwner<String, &'static str, &'static str, &'static str> {
    QueueApplyLiveOwner::new_from_setup(
        &test_setup(),
        current_max_size,
        OrderCandidates::new(Uint256::from_u64(0)),
        build_views(),
    )
}

fn build_flow_owner(
    current_max_size: Option<usize>,
) -> QueueApplyLiveOwner<String, &'static str, &'static str, &'static str> {
    QueueApplyLiveOwner::new_from_setup(
        &test_setup(),
        current_max_size,
        OrderCandidates::new(Uint256::from_u64(0)),
        build_flow_views(),
    )
}

fn test_setup() -> TxQSetup {
    TxQSetup::default()
}

fn expected_stage()
-> QueueApplyPreflightStage<String, &'static str, &'static str, &'static str, &'static str> {
    QueueApplyPreflightStage::Entry(QueueApplyEntryStage::Queued(
        QueueApplyQueuedStage::Account(QueueApplyAccountStage::RejectBlockerAdmission(
            tx::BlockerQueueAdmission::RejectsNonReplacementOfLoneEntry,
        )),
    ))
}

fn direct_observed_queue<'a>(
    order: &'a OrderCandidates,
    queue_is_full: bool,
) -> tx::QueueApplyObservedQueue<'a> {
    let setup = test_setup();
    tx::QueueApplyObservedQueue {
        minimum_last_ledger_buffer: setup.minimum_last_ledger_buffer,
        maximum_txn_per_account: usize::try_from(setup.maximum_txn_per_account)
            .unwrap_or(usize::MAX),
        retry_sequence_percent: setup.retry_sequence_percent,
        queue_is_full,
        can_be_held_result: Ter::TES_SUCCESS,
        order,
    }
}

fn flow_tx_source<'a>(account: &'a String) -> TestObservedTxSource<'a> {
    TestObservedTxSource {
        account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(6),
    }
}

fn flow_view_source() -> TestObservedViewSource {
    TestObservedViewSource {
        rules: rules(),
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
            escalation_multiplier: tx::TXQ_BASE_LEVEL * 500,
        },
        open_ledger_tx_count: 4,
        open_ledger_seq: 100,
        reserve_drops: 200,
        base_fee_drops: 10,
    }
}

fn flow_after_preflight_result()
-> PreflightResult<&'static str, TxConsequences, &'static str, &'static str> {
    PreflightResult::new(
        "tx",
        None::<&str>,
        rules(),
        TxConsequences::with_potential_spend(50, SeqProxy::sequence(6), 500),
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        "journal",
        Ter::TES_SUCCESS,
    )
}

#[test]
fn queue_apply_live_owner_can_derive_hold_admission_from_live_owner_and_view_facts() {
    let account = String::from("acct");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(8),
    };
    let owner = QueueApplyLiveOwner::new(
        2,
        1,
        25,
        None,
        OrderCandidates::new(Uint256::from_u64(0)),
        build_views(),
    );

    assert_eq!(
        owner.derive_can_be_held_result(&tx_source, &view_source(), hold_preflight()),
        Ter::TEL_CAN_NOT_QUEUE_FULL
    );
}

#[test]
fn queue_apply_live_owner_can_derive_queued_front_stage_from_live_owner_and_view_facts() {
    let account = String::from("acct");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(8),
    };
    let view_source = flow_view_source();
    let owner = QueueApplyLiveOwner::new(
        2,
        10,
        25,
        Some(10),
        OrderCandidates::new(Uint256::from_u64(0)),
        QueueViews::new(
            BTreeMap::from([({
                let mut account = TxQAccount::new(String::from("acct"));
                account.add(
                    SeqProxy::sequence(5),
                    MaybeTxCore::new(
                        MaybeTx::new(
                            Uint256::from_u64(5),
                            90,
                            String::from("acct"),
                            Some(200),
                            SeqProxy::sequence(5),
                            ApplyFlags::NONE,
                            PreflightResult::new(
                                "tx",
                                None::<&str>,
                                rules(),
                                TxConsequences::with_sequences_consumed(
                                    1,
                                    SeqProxy::sequence(5),
                                    1,
                                ),
                                ApplyFlags::NONE,
                                "journal",
                                Ter::TES_SUCCESS,
                            ),
                        ),
                        TxConsequences::with_sequences_consumed(1, SeqProxy::sequence(5), 1),
                    ),
                );
                account.add(
                    SeqProxy::sequence(6),
                    MaybeTxCore::new(
                        MaybeTx::new(
                            Uint256::from_u64(6),
                            110,
                            String::from("acct"),
                            Some(200),
                            SeqProxy::sequence(6),
                            ApplyFlags::NONE,
                            PreflightResult::new(
                                "tx",
                                None::<&str>,
                                rules(),
                                TxConsequences::with_sequences_consumed(
                                    1,
                                    SeqProxy::sequence(6),
                                    1,
                                ),
                                ApplyFlags::NONE,
                                "journal",
                                Ter::TES_SUCCESS,
                            ),
                        ),
                        TxConsequences::with_sequences_consumed(1, SeqProxy::sequence(6), 1),
                    ),
                );
                (String::from("acct"), account)
            })]),
            vec![],
        ),
    );
    let consequences = TxConsequences::new(20, SeqProxy::sequence(8));
    let hold_preflight = hold_preflight();
    let fee_context = evaluate_queue_apply_fee_context(QueueApplyFeeContextInputs {
        calculated_base_fee_drops: view_source.calculated_base_fee_drops(),
        fee_paid_drops: view_source.fee_paid_drops(),
        default_base_fee_drops: view_source.default_base_fee_drops(),
        metrics_snapshot: view_source.metrics_snapshot(),
        open_ledger_tx_count: view_source.open_ledger_tx_count(),
        flags: ApplyFlags::NONE,
    });
    let account_stage = run_queue_apply_account_stage(
        owner.views(),
        &account,
        SeqProxy::sequence(5),
        SeqProxy::sequence(8),
        false,
        fee_context.fee_level_paid,
        25,
    );
    let account_context = match &account_stage {
        QueueApplyAccountStage::Ready(context) => context,
        _ => panic!("account stage should be ready"),
    };
    let expected_multitxn = run_queue_apply_multitxn_stage(
        owner.views().accounts.get(&account),
        account_context,
        QueueApplyMultiTxnInputs {
            preflight: hold_preflight,
            open_ledger_seq: view_source.open_ledger_seq(),
            minimum_last_ledger_buffer: 2,
            maximum_txn_per_account: 10,
            account_seq_proxy: SeqProxy::sequence(5),
            tx_seq_proxy: SeqProxy::sequence(8),
            balance_drops: 1_000,
            reserve_drops: view_source.reserve_drops(),
            base_fee_drops: view_source.base_fee_drops(),
            can_be_held_result: owner.derive_can_be_held_result(
                &tx_source,
                &view_source,
                hold_preflight,
            ),
            consequences,
        },
    );

    assert_eq!(
        owner.derive_queued_front_stage(
            &tx_source,
            &view_source,
            hold_preflight,
            ApplyFlags::NONE,
            consequences,
        ),
        QueueApplyLiveOwnerQueuedFrontStage::MultiTxn {
            account_seq_proxy: SeqProxy::sequence(5),
            fee_context,
            stage: expected_multitxn,
        }
    );
}

#[test]
fn queue_apply_live_owner_queued_front_stage_preserves_account_rejection_before_multitxn() {
    let account = String::from("acct");
    let owner = build_owner(Some(10));
    let view_source = view_source();
    let fee_context = evaluate_queue_apply_fee_context(QueueApplyFeeContextInputs {
        calculated_base_fee_drops: view_source.calculated_base_fee_drops(),
        fee_paid_drops: view_source.fee_paid_drops(),
        default_base_fee_drops: view_source.default_base_fee_drops(),
        metrics_snapshot: view_source.metrics_snapshot(),
        open_ledger_tx_count: view_source.open_ledger_tx_count(),
        flags: ApplyFlags::NONE,
    });

    assert_eq!(
        owner.derive_queued_front_stage(
            &tx_source(&account),
            &view_source,
            hold_preflight(),
            ApplyFlags::NONE,
            blocker_consequences(SeqProxy::sequence(6)),
        ),
        QueueApplyLiveOwnerQueuedFrontStage::Account {
            account_seq_proxy: SeqProxy::sequence(5),
            fee_context,
            stage: QueueApplyAccountStage::RejectBlockerAdmission(
                tx::BlockerQueueAdmission::RejectsNonReplacementOfLoneEntry,
            ),
        }
    );
}

#[test]
fn queue_apply_live_owner_can_derive_after_preflight_prepared_flow_stage() {
    let account = String::from("acct");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(8),
    };
    let view_source = view_source();
    let hold_preflight = hold_preflight();
    let preflight = PreflightResult::new(
        "tx",
        None::<&str>,
        rules(),
        TxConsequences::new(20, SeqProxy::sequence(7)),
        ApplyFlags::NONE,
        "journal",
        Ter::TES_SUCCESS,
    );
    let owner = QueueApplyLiveOwner::new(
        2,
        10,
        25,
        Some(10),
        OrderCandidates::new(Uint256::from_u64(0)),
        QueueViews::new(
            BTreeMap::from([({
                let mut account = TxQAccount::new(String::from("acct"));
                account.add(
                    SeqProxy::sequence(5),
                    MaybeTxCore::new(
                        MaybeTx::new(
                            Uint256::from_u64(5),
                            90,
                            String::from("acct"),
                            Some(200),
                            SeqProxy::sequence(5),
                            ApplyFlags::NONE,
                            PreflightResult::new(
                                "tx",
                                None::<&str>,
                                rules(),
                                TxConsequences::with_sequences_consumed(
                                    1,
                                    SeqProxy::sequence(5),
                                    1,
                                ),
                                ApplyFlags::NONE,
                                "journal",
                                Ter::TES_SUCCESS,
                            ),
                        ),
                        TxConsequences::with_sequences_consumed(1, SeqProxy::sequence(5), 1),
                    ),
                );
                account.add(
                    SeqProxy::sequence(6),
                    MaybeTxCore::new(
                        MaybeTx::new(
                            Uint256::from_u64(6),
                            110,
                            String::from("acct"),
                            Some(200),
                            SeqProxy::sequence(6),
                            ApplyFlags::NONE,
                            PreflightResult::new(
                                "tx",
                                None::<&str>,
                                rules(),
                                TxConsequences::with_sequences_consumed(
                                    1,
                                    SeqProxy::sequence(6),
                                    1,
                                ),
                                ApplyFlags::NONE,
                                "journal",
                                Ter::TES_SUCCESS,
                            ),
                        ),
                        TxConsequences::with_sequences_consumed(1, SeqProxy::sequence(6), 1),
                    ),
                );
                (String::from("acct"), account)
            })]),
            vec![],
        ),
    );
    let fee_context = evaluate_queue_apply_fee_context(QueueApplyFeeContextInputs {
        calculated_base_fee_drops: view_source.calculated_base_fee_drops(),
        fee_paid_drops: view_source.fee_paid_drops(),
        default_base_fee_drops: view_source.default_base_fee_drops(),
        metrics_snapshot: view_source.metrics_snapshot(),
        open_ledger_tx_count: view_source.open_ledger_tx_count(),
        flags: preflight.flags,
    });

    assert_eq!(
        owner.derive_after_preflight_prepared_flow_stage(
            &tx_source,
            &view_source,
            hold_preflight,
            preflight.clone(),
            |_| true,
        ),
        derive_queue_apply_prepared_flow_stage(
            owner.views(),
            SeqProxy::sequence(5),
            SeqProxy::sequence(8),
            QueueApplyQueuedWithFeeContextInputs {
                account: account.clone(),
                preflight: hold_preflight,
                is_blocker: false,
                open_ledger_seq: view_source.open_ledger_seq(),
                minimum_last_ledger_buffer: 2,
                maximum_txn_per_account: 10,
                retry_sequence_percent: 25,
                queue_is_full: false,
                balance_drops: 1_000,
                reserve_drops: view_source.reserve_drops(),
                base_fee_drops: view_source.base_fee_drops(),
                can_be_held_result: owner.derive_can_be_held_result(
                    &tx_source,
                    &view_source,
                    hold_preflight,
                ),
                open_ledger_tx_count: view_source.open_ledger_tx_count(),
                tx_id: Uint256::from_u64(9),
                last_valid: hold_preflight.last_valid_ledger,
                flags: preflight.flags,
                order: owner.order(),
            },
            fee_context,
            preflight,
            |_| true,
        )
    );
}

#[test]
fn queue_apply_live_owner_builds_observed_queue_from_owner_state() {
    let owner = QueueApplyLiveOwner::new_from_setup(
        &test_setup(),
        Some(1),
        OrderCandidates::new(Uint256::from_u64(0)),
        QueueViews::<String, MaybeTx<&'static str, String, &'static str, &'static str>>::new(
            BTreeMap::new(),
            vec![tx::FeeQueueEntry::new(
                tx::FeeQueueKey::new(String::from("acct"), SeqProxy::sequence(5)),
                tx::QueueAdvanceCandidate {
                    fee_level: 90,
                    tx_id: Uint256::from_u64(5),
                    seq_proxy: SeqProxy::sequence(5),
                },
            )],
        ),
    );
    let observed = owner.observed_queue(Ter::TES_SUCCESS);

    assert_eq!(observed.minimum_last_ledger_buffer, 2);
    assert_eq!(observed.maximum_txn_per_account, 10);
    assert_eq!(observed.retry_sequence_percent, 25);
    assert!(observed.queue_is_full);
    assert_eq!(observed.can_be_held_result, Ter::TES_SUCCESS);
    assert_eq!(*observed.order, OrderCandidates::new(Uint256::from_u64(0)));
}

#[test]
fn queue_apply_live_owner_wrapper_matches_source_wrapper() {
    let account = String::from("acct");
    let tx_source = tx_source(&account);
    let view_source = view_source();
    let order = OrderCandidates::new(Uint256::from_u64(0));

    let mut direct_views = build_views();
    let direct_stage = run_queue_apply_top_with_queued_stage_from_sources(
        &mut direct_views,
        &tx_source,
        &view_source,
        tx::QueueApplyTopFromSourcesInputs::new(
            hold_preflight(),
            ApplyFlags::NONE,
            blocker_consequences(SeqProxy::sequence(6)),
            direct_observed_queue(&order, false),
        ),
        || preflight_result(SeqProxy::sequence(6)),
        |_| unreachable!("direct apply should fall through without tracing"),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |preclaim_view| {
            assert!(!preclaim_view.has_multi_txn());
            PreclaimResult::new(
                100,
                "tx",
                None::<&str>,
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            )
        },
        || -> ApplyResult {
            unreachable!("queue-stage account rejection should happen before try-clear")
        },
        || unreachable!("queue-stage account rejection should happen before sandbox apply"),
    );

    let mut owner = build_owner(Some(10));
    let owner_stage = run_queue_apply_with_live_owner_from_sources(
        &mut owner,
        &tx_source,
        &view_source,
        hold_preflight(),
        ApplyFlags::NONE,
        blocker_consequences(SeqProxy::sequence(6)),
        Ter::TES_SUCCESS,
        || preflight_result(SeqProxy::sequence(6)),
        |_| unreachable!("direct apply should fall through without tracing"),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |preclaim_view| {
            assert!(!preclaim_view.has_multi_txn());
            PreclaimResult::new(
                100,
                "tx",
                None::<&str>,
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            )
        },
        || -> ApplyResult {
            unreachable!("queue-stage account rejection should happen before try-clear")
        },
        || unreachable!("queue-stage account rejection should happen before sandbox apply"),
    );

    assert_eq!(owner_stage, direct_stage);
    assert_eq!(owner_stage, expected_stage());
    assert_eq!(owner.views(), &direct_views);
}

#[test]
fn queue_apply_live_owner_after_preflight_wrapper_matches_source_wrapper() {
    let account = String::from("acct");
    let tx_source = tx_source(&account);
    let view_source = view_source();
    let preflight = preflight_result(SeqProxy::sequence(6));
    let order = OrderCandidates::new(Uint256::from_u64(0));

    let mut direct_views = build_views();
    let direct_stage = run_queue_apply_after_preflight_from_sources(
        &mut direct_views,
        &tx_source,
        &view_source,
        &preflight,
        QueueApplyAfterPreflightSourceInputs::new(
            hold_preflight(),
            direct_observed_queue(&order, false),
        ),
        |_| unreachable!("direct apply should fall through without tracing"),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |preclaim_view| {
            assert!(!preclaim_view.has_multi_txn());
            PreclaimResult::new(
                100,
                "tx",
                None::<&str>,
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            )
        },
        || -> ApplyResult {
            unreachable!("queue-stage account rejection should happen before try-clear")
        },
        || unreachable!("queue-stage account rejection should happen before sandbox apply"),
    );

    let mut owner = build_owner(Some(10));
    let owner_stage = run_queue_apply_after_preflight_with_live_owner_from_sources(
        &mut owner,
        &tx_source,
        &view_source,
        &preflight,
        hold_preflight(),
        Ter::TES_SUCCESS,
        |_| unreachable!("direct apply should fall through without tracing"),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |preclaim_view| {
            assert!(!preclaim_view.has_multi_txn());
            PreclaimResult::new(
                100,
                "tx",
                None::<&str>,
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            )
        },
        || -> ApplyResult {
            unreachable!("queue-stage account rejection should happen before try-clear")
        },
        || unreachable!("queue-stage account rejection should happen before sandbox apply"),
    );

    assert_eq!(owner_stage, direct_stage);
    assert_eq!(owner_stage, expected_stage());
    assert_eq!(owner.views(), &direct_views);
}

#[test]
fn queue_apply_live_owner_after_preflight_accepts_structured_try_clear_result() {
    let account = String::from("acct");
    let tx_source = flow_tx_source(&account);
    let view_source = flow_view_source();
    let preflight = flow_after_preflight_result();
    let order = OrderCandidates::new(Uint256::from_u64(0));

    let mut direct_views = build_flow_views();
    let direct_stage = run_queue_apply_after_preflight_from_sources(
        &mut direct_views,
        &tx_source,
        &view_source,
        &preflight,
        QueueApplyAfterPreflightSourceInputs::new(
            hold_preflight(),
            direct_observed_queue(&order, false),
        ),
        |_| unreachable!("direct apply should fall through without tracing"),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |_| {
            PreclaimResult::new(
                100,
                "tx",
                None::<&str>,
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            )
        },
        || TryClearAccountResult::InsufficientFee {
            plan: TryClearAccountPlan {
                queued_seq_proxies: vec![SeqProxy::sequence(5)],
                queued_count: 1,
                target_was_already_queued: false,
                total_fee_level_paid: 50,
            },
            required_total_fee_level: 60,
        },
        || unreachable!("insufficient-fee clear-ahead must not apply sandbox"),
    );

    let mut owner = build_flow_owner(Some(10));
    let owner_stage = run_queue_apply_after_preflight_with_live_owner_from_sources(
        &mut owner,
        &tx_source,
        &view_source,
        &preflight,
        hold_preflight(),
        Ter::TES_SUCCESS,
        |_| unreachable!("direct apply should fall through without tracing"),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |_| {
            PreclaimResult::new(
                100,
                "tx",
                None::<&str>,
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            )
        },
        || TryClearAccountResult::InsufficientFee {
            plan: TryClearAccountPlan {
                queued_seq_proxies: vec![SeqProxy::sequence(5)],
                queued_count: 1,
                target_was_already_queued: false,
                total_fee_level_paid: 50,
            },
            required_total_fee_level: 60,
        },
        || unreachable!("insufficient-fee clear-ahead must not apply sandbox"),
    );

    assert_eq!(owner_stage, direct_stage);
    assert_eq!(
        owner_stage.apply_result(),
        ApplyResult::new(Ter::TER_QUEUED, false, false)
    );
    assert_eq!(owner.views(), &direct_views);
}

#[test]
fn queue_apply_live_owner_wrapper_with_caller_preclaim_matches_lower_source_boundary() {
    let account = String::from("acct");
    let tx_source = flow_tx_source(&account);
    let view_source = flow_view_source();
    let order = OrderCandidates::new(Uint256::from_u64(0));
    let preflight = flow_after_preflight_result();
    let expected_prepared = RefCell::new(None::<QueueApplyPreparedPreclaimInputs<String>>);

    let mut direct_views = build_flow_views();
    let direct_stage = run_queue_apply_top_with_caller_preclaim_from_sources(
        &mut direct_views,
        &tx_source,
        &view_source,
        tx::QueueApplyTopFromSourcesInputs::new(
            hold_preflight(),
            ApplyFlags::NONE,
            preflight.clone().consequences,
            direct_observed_queue(&order, false),
        ),
        || preflight.clone(),
        |_| unreachable!("direct apply should fall through without tracing"),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |prepared| {
            expected_prepared.replace(Some(prepared.clone()));
            Ok(QueueApplyPreclaimStage {
                view_source: prepared.view_source,
                trace_message: "trace".to_string(),
                preclaim_result: flow_after_preflight_result().to_preclaim(9, Ter::TES_SUCCESS),
            })
        },
        || ApplyResult::new(Ter::TES_SUCCESS, true, false),
        || {},
    );

    let mut owner = build_flow_owner(Some(10));
    let owner_stage = run_queue_apply_with_live_owner_from_sources_and_caller_preclaim(
        &mut owner,
        &tx_source,
        &view_source,
        hold_preflight(),
        ApplyFlags::NONE,
        preflight.clone().consequences,
        Ter::TES_SUCCESS,
        || preflight.clone(),
        |_| unreachable!("direct apply should fall through without tracing"),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |prepared| {
            assert_eq!(Some(prepared.clone()), *expected_prepared.borrow());
            Ok(QueueApplyPreclaimStage {
                view_source: prepared.view_source,
                trace_message: "trace".to_string(),
                preclaim_result: flow_after_preflight_result().to_preclaim(9, Ter::TES_SUCCESS),
            })
        },
        || ApplyResult::new(Ter::TES_SUCCESS, true, false),
        || {},
    );

    assert_eq!(owner_stage, direct_stage);
    assert_eq!(owner.views(), &direct_views);
}

#[test]
fn queue_apply_live_owner_log_messages_match_lower_source_boundary() {
    let account = String::from("acct");
    let tx_source = flow_tx_source(&account);
    let view_source = flow_view_source();
    let order = OrderCandidates::new(Uint256::from_u64(0));
    let preflight = flow_after_preflight_result();

    let mut direct_views = build_flow_views();
    let direct_stage = run_queue_apply_top_with_log_messages_from_sources(
        &mut direct_views,
        &tx_source,
        &view_source,
        tx::QueueApplyTopFromSourcesInputs::new(
            hold_preflight(),
            ApplyFlags::NONE,
            preflight.clone().consequences,
            direct_observed_queue(&order, false),
        ),
        || preflight.clone(),
        |_| unreachable!("direct apply should fall through without tracing"),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |_| {
            PreclaimResult::new(
                100,
                "tx",
                None::<&str>,
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            )
        },
        || ApplyResult::new(Ter::TES_SUCCESS, true, false),
        || {},
    );

    let mut owner = build_flow_owner(Some(10));
    let owner_stage = run_queue_apply_with_live_owner_from_sources_and_log_messages(
        &mut owner,
        &tx_source,
        &view_source,
        hold_preflight(),
        ApplyFlags::NONE,
        preflight.clone().consequences,
        Ter::TES_SUCCESS,
        || preflight.clone(),
        |_| unreachable!("direct apply should fall through without tracing"),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |_| {
            PreclaimResult::new(
                100,
                "tx",
                None::<&str>,
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            )
        },
        || ApplyResult::new(Ter::TES_SUCCESS, true, false),
        || {},
    );

    assert_eq!(owner_stage, direct_stage);
    assert_eq!(owner.views(), &direct_views);
}

#[test]
fn queue_apply_live_owner_after_preflight_with_caller_preclaim_matches_lower_source_boundary() {
    let account = String::from("acct");
    let tx_source = flow_tx_source(&account);
    let view_source = flow_view_source();
    let order = OrderCandidates::new(Uint256::from_u64(0));
    let preflight = flow_after_preflight_result();
    let expected_prepared = RefCell::new(None::<QueueApplyPreparedPreclaimInputs<String>>);

    let mut direct_views = build_flow_views();
    let direct_stage = run_queue_apply_after_preflight_with_caller_preclaim_from_sources(
        &mut direct_views,
        &tx_source,
        &view_source,
        &preflight,
        QueueApplyAfterPreflightSourceInputs::new(
            hold_preflight(),
            direct_observed_queue(&order, false),
        ),
        |_| unreachable!("direct apply should fall through without tracing"),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |prepared| {
            expected_prepared.replace(Some(prepared.clone()));
            Ok(QueueApplyPreclaimStage {
                view_source: prepared.view_source,
                trace_message: "trace".to_string(),
                preclaim_result: flow_after_preflight_result().to_preclaim(9, Ter::TES_SUCCESS),
            })
        },
        || ApplyResult::new(Ter::TES_SUCCESS, true, false),
        || {},
    );

    let mut owner = build_flow_owner(Some(10));
    let owner_stage =
        run_queue_apply_after_preflight_with_live_owner_from_sources_and_caller_preclaim(
            &mut owner,
            &tx_source,
            &view_source,
            &preflight,
            hold_preflight(),
            Ter::TES_SUCCESS,
            |_| unreachable!("direct apply should fall through without tracing"),
            || ApplyResult::new(Ter::TES_SUCCESS, true, true),
            |_| true,
            |prepared| {
                assert_eq!(Some(prepared.clone()), *expected_prepared.borrow());
                Ok(QueueApplyPreclaimStage {
                    view_source: prepared.view_source,
                    trace_message: "trace".to_string(),
                    preclaim_result: flow_after_preflight_result().to_preclaim(9, Ter::TES_SUCCESS),
                })
            },
            || ApplyResult::new(Ter::TES_SUCCESS, true, false),
            || {},
        );

    assert_eq!(owner_stage, direct_stage);
    assert_eq!(owner.views(), &direct_views);
}

#[test]
fn queue_apply_live_owner_after_preflight_log_messages_match_lower_source_boundary() {
    let account = String::from("acct");
    let tx_source = flow_tx_source(&account);
    let view_source = flow_view_source();
    let order = OrderCandidates::new(Uint256::from_u64(0));
    let preflight = flow_after_preflight_result();

    let mut direct_views = build_flow_views();
    let direct_stage = run_queue_apply_after_preflight_with_log_messages_from_sources(
        &mut direct_views,
        &tx_source,
        &view_source,
        &preflight,
        QueueApplyAfterPreflightSourceInputs::new(
            hold_preflight(),
            direct_observed_queue(&order, false),
        ),
        |_| unreachable!("direct apply should fall through without tracing"),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |_| {
            PreclaimResult::new(
                100,
                "tx",
                None::<&str>,
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            )
        },
        || ApplyResult::new(Ter::TES_SUCCESS, true, false),
        || {},
    );

    let mut owner = build_flow_owner(Some(10));
    let owner_stage = run_queue_apply_after_preflight_with_live_owner_from_sources_and_log_messages(
        &mut owner,
        &tx_source,
        &view_source,
        &preflight,
        hold_preflight(),
        Ter::TES_SUCCESS,
        |_| unreachable!("direct apply should fall through without tracing"),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |_| {
            PreclaimResult::new(
                100,
                "tx",
                None::<&str>,
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            )
        },
        || ApplyResult::new(Ter::TES_SUCCESS, true, false),
        || {},
    );

    assert_eq!(owner_stage, direct_stage);
    assert_eq!(owner.views(), &direct_views);
}

#[test]
fn queue_apply_live_owner_log_sinks_match_lower_source_boundary() {
    let account = String::from("acct");
    let tx_source = flow_tx_source(&account);
    let view_source = flow_view_source();
    let order = OrderCandidates::new(Uint256::from_u64(0));
    let preflight = flow_after_preflight_result();

    let mut direct_views = build_flow_views();
    let direct_emitted = RefCell::new(Vec::new());
    let direct_stage = run_queue_apply_top_with_log_sinks_from_sources(
        &mut direct_views,
        &tx_source,
        &view_source,
        tx::QueueApplyTopFromSourcesInputs::new(
            hold_preflight(),
            ApplyFlags::NONE,
            preflight.clone().consequences,
            direct_observed_queue(&order, false),
        ),
        || preflight.clone(),
        |_| unreachable!("direct apply should fall through without tracing"),
        |message| direct_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| direct_emitted.borrow_mut().push(format!("info:{message}")),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |_| {
            PreclaimResult::new(
                100,
                "tx",
                None::<&str>,
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            )
        },
        || ApplyResult::new(Ter::TES_SUCCESS, true, false),
        || {},
    );

    let mut owner = build_flow_owner(Some(10));
    let owner_emitted = RefCell::new(Vec::new());
    let owner_stage = run_queue_apply_with_live_owner_from_sources_and_log_sinks(
        &mut owner,
        &tx_source,
        &view_source,
        hold_preflight(),
        ApplyFlags::NONE,
        preflight.clone().consequences,
        Ter::TES_SUCCESS,
        || preflight.clone(),
        |_| unreachable!("direct apply should fall through without tracing"),
        |message| owner_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| owner_emitted.borrow_mut().push(format!("info:{message}")),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |_| {
            PreclaimResult::new(
                100,
                "tx",
                None::<&str>,
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            )
        },
        || ApplyResult::new(Ter::TES_SUCCESS, true, false),
        || {},
    );

    assert_eq!(owner_stage, direct_stage);
    assert_eq!(owner.views(), &direct_views);
    assert_eq!(owner_emitted.into_inner(), direct_emitted.into_inner());
}

#[test]
fn queue_apply_live_owner_caller_preclaim_log_sinks_match_lower_source_boundary() {
    let account = String::from("acct");
    let tx_source = flow_tx_source(&account);
    let view_source = flow_view_source();
    let order = OrderCandidates::new(Uint256::from_u64(0));
    let preflight = flow_after_preflight_result();
    let expected_prepared = RefCell::new(None::<QueueApplyPreparedPreclaimInputs<String>>);

    let mut direct_views = build_flow_views();
    let direct_emitted = RefCell::new(Vec::new());
    let direct_stage = run_queue_apply_top_with_caller_preclaim_and_log_sinks_from_sources(
        &mut direct_views,
        &tx_source,
        &view_source,
        tx::QueueApplyTopFromSourcesInputs::new(
            hold_preflight(),
            ApplyFlags::NONE,
            preflight.clone().consequences,
            direct_observed_queue(&order, false),
        ),
        || preflight.clone(),
        |_| unreachable!("direct apply should fall through without tracing"),
        |message| direct_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| direct_emitted.borrow_mut().push(format!("info:{message}")),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |prepared| {
            expected_prepared.replace(Some(prepared.clone()));
            Ok(QueueApplyPreclaimStage {
                view_source: prepared.view_source,
                trace_message: "trace".to_string(),
                preclaim_result: flow_after_preflight_result().to_preclaim(9, Ter::TES_SUCCESS),
            })
        },
        || ApplyResult::new(Ter::TES_SUCCESS, true, false),
        || {},
    );

    let mut owner = build_flow_owner(Some(10));
    let owner_emitted = RefCell::new(Vec::new());
    let owner_stage =
        run_queue_apply_with_live_owner_from_sources_and_caller_preclaim_and_log_sinks(
            &mut owner,
            &tx_source,
            &view_source,
            hold_preflight(),
            ApplyFlags::NONE,
            preflight.clone().consequences,
            Ter::TES_SUCCESS,
            || preflight.clone(),
            |_| unreachable!("direct apply should fall through without tracing"),
            |message| owner_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| owner_emitted.borrow_mut().push(format!("info:{message}")),
            || ApplyResult::new(Ter::TES_SUCCESS, true, true),
            |_| true,
            |prepared| {
                assert_eq!(Some(prepared.clone()), *expected_prepared.borrow());
                Ok(QueueApplyPreclaimStage {
                    view_source: prepared.view_source,
                    trace_message: "trace".to_string(),
                    preclaim_result: flow_after_preflight_result().to_preclaim(9, Ter::TES_SUCCESS),
                })
            },
            || ApplyResult::new(Ter::TES_SUCCESS, true, false),
            || {},
        );

    assert_eq!(owner_stage, direct_stage);
    assert_eq!(owner.views(), &direct_views);
    assert_eq!(owner_emitted.into_inner(), direct_emitted.into_inner());
}

#[test]
fn queue_apply_live_owner_after_preflight_log_sinks_match_lower_source_boundary() {
    let account = String::from("acct");
    let tx_source = flow_tx_source(&account);
    let view_source = flow_view_source();
    let order = OrderCandidates::new(Uint256::from_u64(0));
    let preflight = flow_after_preflight_result();

    let mut direct_views = build_flow_views();
    let direct_emitted = RefCell::new(Vec::new());
    let direct_stage = run_queue_apply_after_preflight_with_log_sinks_from_sources(
        &mut direct_views,
        &tx_source,
        &view_source,
        &preflight,
        QueueApplyAfterPreflightSourceInputs::new(
            hold_preflight(),
            direct_observed_queue(&order, false),
        ),
        |_| unreachable!("direct apply should fall through without tracing"),
        |message| direct_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| direct_emitted.borrow_mut().push(format!("info:{message}")),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |_| {
            PreclaimResult::new(
                100,
                "tx",
                None::<&str>,
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            )
        },
        || ApplyResult::new(Ter::TES_SUCCESS, true, false),
        || {},
    );

    let mut owner = build_flow_owner(Some(10));
    let owner_emitted = RefCell::new(Vec::new());
    let owner_stage = run_queue_apply_after_preflight_with_live_owner_from_sources_and_log_sinks(
        &mut owner,
        &tx_source,
        &view_source,
        &preflight,
        hold_preflight(),
        Ter::TES_SUCCESS,
        |_| unreachable!("direct apply should fall through without tracing"),
        |message| owner_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| owner_emitted.borrow_mut().push(format!("info:{message}")),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |_| {
            PreclaimResult::new(
                100,
                "tx",
                None::<&str>,
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            )
        },
        || ApplyResult::new(Ter::TES_SUCCESS, true, false),
        || {},
    );

    assert_eq!(owner_stage, direct_stage);
    assert_eq!(owner.views(), &direct_views);
    assert_eq!(owner_emitted.into_inner(), direct_emitted.into_inner());
}

#[test]
fn queue_apply_live_owner_after_preflight_caller_preclaim_log_sinks_match_lower_source_boundary() {
    let account = String::from("acct");
    let tx_source = flow_tx_source(&account);
    let view_source = flow_view_source();
    let order = OrderCandidates::new(Uint256::from_u64(0));
    let preflight = flow_after_preflight_result();
    let expected_prepared = RefCell::new(None::<QueueApplyPreparedPreclaimInputs<String>>);

    let mut direct_views = build_flow_views();
    let direct_emitted = RefCell::new(Vec::new());
    let direct_stage =
        run_queue_apply_after_preflight_with_caller_preclaim_and_log_sinks_from_sources(
            &mut direct_views,
            &tx_source,
            &view_source,
            &preflight,
            QueueApplyAfterPreflightSourceInputs::new(
                hold_preflight(),
                direct_observed_queue(&order, false),
            ),
            |_| unreachable!("direct apply should fall through without tracing"),
            |message| direct_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| direct_emitted.borrow_mut().push(format!("info:{message}")),
            || ApplyResult::new(Ter::TES_SUCCESS, true, true),
            |_| true,
            |prepared| {
                expected_prepared.replace(Some(prepared.clone()));
                Ok(QueueApplyPreclaimStage {
                    view_source: prepared.view_source,
                    trace_message: "trace".to_string(),
                    preclaim_result: flow_after_preflight_result().to_preclaim(9, Ter::TES_SUCCESS),
                })
            },
            || ApplyResult::new(Ter::TES_SUCCESS, true, false),
            || {},
        );

    let mut owner = build_flow_owner(Some(10));
    let owner_emitted = RefCell::new(Vec::new());
    let owner_stage =
        run_queue_apply_after_preflight_with_live_owner_from_sources_and_caller_preclaim_and_log_sinks(
            &mut owner,
            &tx_source,
            &view_source,
            &preflight,
            hold_preflight(),
            Ter::TES_SUCCESS,
            |_| unreachable!("direct apply should fall through without tracing"),
            |message| owner_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| owner_emitted.borrow_mut().push(format!("info:{message}")),
            || ApplyResult::new(Ter::TES_SUCCESS, true, true),
            |_| true,
            |prepared| {
                assert_eq!(Some(prepared.clone()), *expected_prepared.borrow());
                Ok(QueueApplyPreclaimStage {
                    view_source: prepared.view_source,
                    trace_message: "trace".to_string(),
                    preclaim_result: flow_after_preflight_result().to_preclaim(9, Ter::TES_SUCCESS),
                })
            },
            || ApplyResult::new(Ter::TES_SUCCESS, true, false),
            || {},
        );

    assert_eq!(owner_stage, direct_stage);
    assert_eq!(owner.views(), &direct_views);
    assert_eq!(owner_emitted.into_inner(), direct_emitted.into_inner());
}
