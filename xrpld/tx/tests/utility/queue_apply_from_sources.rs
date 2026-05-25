#![allow(clippy::clone_on_copy)]

use std::{cell::RefCell, collections::BTreeMap};

use basics::base_uint::Uint256;
use protocol::{Rules, SeqProxy, Ter};
use tx::{
    ApplyFlags, ApplyResult, MaybeTx, MaybeTxCore, OrderCandidates, PreflightResult,
    QueueApplyAfterPreflightSourceInputs, QueueApplyObservedAccountLookup, QueueApplyObservedQueue,
    QueueApplyObservedTicketLookup, QueueApplyObservedTxSource, QueueApplyObservedViewSource,
    QueueApplyPreclaimStage, QueueApplyPreparedPreclaimInputs, QueueFeeMetricsSnapshot,
    QueueHoldPreflight, QueueViews, TxConsequences, TxQAccount,
    derive_queue_apply_prepared_flow_stage, derive_queue_apply_prepared_post_preclaim_inputs,
    run_prepared_queue_apply_post_preclaim_stage_with_caller_queue,
    run_prepared_queue_apply_queued_flow_stage, run_queue_apply_after_preflight_from_sources,
    run_queue_apply_after_preflight_with_acquired_direct_apply_and_caller_queued_stage_from_sources,
    run_queue_apply_after_preflight_with_acquired_direct_apply_from_sources,
    run_queue_apply_after_preflight_with_caller_direct_apply_and_caller_queued_stage_from_sources,
    run_queue_apply_after_preflight_with_caller_direct_apply_from_sources,
    run_queue_apply_after_preflight_with_caller_preclaim_and_log_sinks_from_sources,
    run_queue_apply_after_preflight_with_caller_preclaim_from_sources,
    run_queue_apply_after_preflight_with_direct_apply_and_caller_queued_stage_and_log_messages,
    run_queue_apply_after_preflight_with_direct_apply_and_caller_queued_stage_from_sources,
    run_queue_apply_after_preflight_with_log_messages_from_sources,
    run_queue_apply_after_preflight_with_log_sinks_from_sources,
    run_queue_apply_flow_stage_with_caller_preclaim, run_queue_apply_queue_stage_with_log_sinks,
    run_queue_apply_queued_stage_with_fee_context,
    run_queue_apply_queued_stage_with_fee_context_and_log_sinks,
    run_queue_apply_queued_stage_with_log_messages,
    run_queue_apply_top_with_caller_direct_apply_and_caller_queued_stage_from_sources,
    run_queue_apply_top_with_caller_direct_apply_and_queued_stage_from_sources,
    run_queue_apply_top_with_caller_preclaim_and_log_sinks_from_sources,
    run_queue_apply_top_with_caller_preclaim_from_sources,
    run_queue_apply_top_with_log_messages_from_sources,
    run_queue_apply_top_with_log_sinks_from_sources,
    run_queue_apply_top_with_queued_stage_from_sources, run_queue_apply_try_clear_stage,
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

fn observed_queue<'a>(order: &'a OrderCandidates) -> QueueApplyObservedQueue<'a> {
    let setup = tx::TxQSetup::default();
    QueueApplyObservedQueue {
        minimum_last_ledger_buffer: setup.minimum_last_ledger_buffer,
        maximum_txn_per_account: usize::try_from(setup.maximum_txn_per_account)
            .unwrap_or(usize::MAX),
        retry_sequence_percent: setup.retry_sequence_percent,
        queue_is_full: false,
        can_be_held_result: Ter::TES_SUCCESS,
        order,
    }
}

#[test]
fn queue_apply_from_sources_caller_preclaim_matches_manual_top_lowering() {
    let account = String::from("acct");
    let tx_source = flow_tx_source(&account);
    let view_source = flow_view_source();
    let order = OrderCandidates::new(Uint256::from_u64(0));
    let preflight = flow_after_preflight_result();
    let expected_prepared = RefCell::new(None::<QueueApplyPreparedPreclaimInputs<String>>);

    let mut manual_views = build_flow_views();
    let manual_stage =
        run_queue_apply_top_with_caller_direct_apply_and_caller_queued_stage_from_sources(
            &mut manual_views,
            &tx_source,
            &view_source,
            tx::QueueApplyTopFromSourcesInputs::new(
                hold_preflight(),
                ApplyFlags::NONE,
                preflight.clone().consequences,
                observed_queue(&order),
            ),
            || preflight.clone(),
            |views, prepared| {
                tx::run_prepared_direct_apply_with_trace(
                    views,
                    prepared,
                    |_| unreachable!("direct apply should fall through without tracing"),
                    || ApplyResult::new(Ter::TES_SUCCESS, true, true),
                )
            },
            |views, queued| {
                run_prepared_queue_apply_queued_flow_stage(
                    views,
                    derive_queue_apply_prepared_flow_stage(
                        &*views,
                        queued.account_seq_proxy,
                        queued.tx_seq_proxy,
                        queued.queued,
                        queued.fee_context,
                        queued.preflight_result,
                        |_| true,
                    ),
                    |views, prepared| {
                        run_queue_apply_flow_stage_with_caller_preclaim(
                            views,
                            prepared.tx_seq_proxy,
                            prepared.first_relevant_retries_remaining,
                            prepared.hold_fallback,
                            prepared.full_queue_decision,
                            prepared.replaced,
                            prepared.last_valid,
                            prepared.flags,
                            prepared.pf_result,
                            prepared.order,
                            QueueApplyPreparedPreclaimInputs::new(
                                prepared.preclaim_view_source,
                                prepared.fee_level_paid,
                                prepared.base_level,
                                prepared.required_fee_level,
                                prepared.open_ledger_tx_count,
                                prepared.tx_id,
                                prepared.account,
                            ),
                            |prepared| {
                                expected_prepared.replace(Some(prepared.clone()));
                                Ok(QueueApplyPreclaimStage {
                                    view_source: prepared.view_source,
                                    trace_message: "trace".to_string(),
                                    preclaim_result: flow_after_preflight_result()
                                        .to_preclaim(9, Ter::TES_SUCCESS),
                                })
                            },
                            || ApplyResult::new(Ter::TES_SUCCESS, true, false),
                            || {},
                        )
                    },
                )
            },
        );

    let mut helper_views = build_flow_views();
    let helper_stage = run_queue_apply_top_with_caller_preclaim_from_sources(
        &mut helper_views,
        &tx_source,
        &view_source,
        tx::QueueApplyTopFromSourcesInputs::new(
            hold_preflight(),
            ApplyFlags::NONE,
            preflight.clone().consequences,
            observed_queue(&order),
        ),
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

    assert_eq!(helper_stage, manual_stage);
    assert_eq!(helper_views, manual_views);
}

#[test]
fn queue_apply_from_sources_plain_matches_manual_top_lowering() {
    let account = String::from("acct");
    let tx_source = flow_tx_source(&account);
    let view_source = flow_view_source();
    let order = OrderCandidates::new(Uint256::from_u64(0));
    let preflight = flow_after_preflight_result();

    let mut manual_views = build_flow_views();
    let manual_stage =
        run_queue_apply_top_with_caller_direct_apply_and_caller_queued_stage_from_sources(
            &mut manual_views,
            &tx_source,
            &view_source,
            tx::QueueApplyTopFromSourcesInputs::new(
                hold_preflight(),
                ApplyFlags::NONE,
                preflight.clone().consequences,
                observed_queue(&order),
            ),
            || preflight.clone(),
            |views, prepared| {
                tx::run_prepared_direct_apply_with_trace(
                    views,
                    prepared,
                    |_| unreachable!("direct apply should fall through without tracing"),
                    || ApplyResult::new(Ter::TES_SUCCESS, true, true),
                )
            },
            |views, queued| {
                run_queue_apply_queued_stage_with_fee_context(
                    views,
                    queued.account_seq_proxy,
                    queued.tx_seq_proxy,
                    queued.queued,
                    queued.fee_context,
                    queued.preflight_result,
                    |_| true,
                    |_| flow_after_preflight_result().to_preclaim(9, Ter::TES_SUCCESS),
                    || ApplyResult::new(Ter::TES_SUCCESS, true, false),
                    || {},
                )
            },
        );

    let mut helper_views = build_flow_views();
    let helper_stage = run_queue_apply_top_with_queued_stage_from_sources(
        &mut helper_views,
        &tx_source,
        &view_source,
        tx::QueueApplyTopFromSourcesInputs::new(
            hold_preflight(),
            ApplyFlags::NONE,
            preflight.clone().consequences,
            observed_queue(&order),
        ),
        || preflight.clone(),
        |_| unreachable!("direct apply should fall through without tracing"),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |_| flow_after_preflight_result().to_preclaim(9, Ter::TES_SUCCESS),
        || ApplyResult::new(Ter::TES_SUCCESS, true, false),
        || {},
    );

    assert_eq!(helper_stage, manual_stage);
    assert_eq!(helper_views, manual_views);
}

#[test]
fn queue_apply_from_sources_after_preflight_caller_preclaim_matches_manual_lowering() {
    let account = String::from("acct");
    let tx_source = flow_tx_source(&account);
    let view_source = flow_view_source();
    let order = OrderCandidates::new(Uint256::from_u64(0));
    let preflight = flow_after_preflight_result();
    let expected_prepared = RefCell::new(None::<QueueApplyPreparedPreclaimInputs<String>>);

    let mut manual_views = build_flow_views();
    let manual_stage =
        run_queue_apply_after_preflight_with_caller_direct_apply_and_caller_queued_stage_from_sources(
            &mut manual_views,
            &tx_source,
            &view_source,
            &preflight,
            QueueApplyAfterPreflightSourceInputs::new(hold_preflight(), observed_queue(&order)),
            |views, prepared| {
                tx::run_prepared_direct_apply_with_trace(
                    views,
                    prepared,
                    |_| unreachable!("direct apply should fall through without tracing"),
                    || ApplyResult::new(Ter::TES_SUCCESS, true, true),
                )
            },
            |views, queued| {
                run_prepared_queue_apply_queued_flow_stage(
                    views,
                    derive_queue_apply_prepared_flow_stage(
                        &*views,
                        queued.account_seq_proxy,
                        queued.tx_seq_proxy,
                        queued.queued,
                        queued.fee_context,
                        queued.preflight_result,
                        |_| true,
                    ),
                    |views, prepared| {
                        run_queue_apply_flow_stage_with_caller_preclaim(
                            views,
                            prepared.tx_seq_proxy,
                            prepared.first_relevant_retries_remaining,
                            prepared.hold_fallback,
                            prepared.full_queue_decision,
                            prepared.replaced,
                            prepared.last_valid,
                            prepared.flags,
                            prepared.pf_result,
                            prepared.order,
                            QueueApplyPreparedPreclaimInputs::new(
                                prepared.preclaim_view_source,
                                prepared.fee_level_paid,
                                prepared.base_level,
                                prepared.required_fee_level,
                                prepared.open_ledger_tx_count,
                                prepared.tx_id,
                                prepared.account,
                            ),
                            |prepared| {
                                expected_prepared.replace(Some(prepared.clone()));
                                Ok(QueueApplyPreclaimStage {
                                    view_source: prepared.view_source,
                                    trace_message: "trace".to_string(),
                                    preclaim_result: flow_after_preflight_result()
                                        .to_preclaim(9, Ter::TES_SUCCESS),
                                })
                            },
                            || ApplyResult::new(Ter::TES_SUCCESS, true, false),
                            || {},
                        )
                    },
                )
            },
        );

    let mut helper_views = build_flow_views();
    let helper_stage = run_queue_apply_after_preflight_with_caller_preclaim_from_sources(
        &mut helper_views,
        &tx_source,
        &view_source,
        &preflight,
        QueueApplyAfterPreflightSourceInputs::new(hold_preflight(), observed_queue(&order)),
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

    assert_eq!(helper_stage, manual_stage);
    assert_eq!(helper_views, manual_views);
}

#[test]
fn queue_apply_from_sources_after_preflight_plain_matches_manual_lowering() {
    let account = String::from("acct");
    let tx_source = flow_tx_source(&account);
    let view_source = flow_view_source();
    let order = OrderCandidates::new(Uint256::from_u64(0));
    let preflight = flow_after_preflight_result();

    let mut manual_views = build_flow_views();
    let manual_stage =
        run_queue_apply_after_preflight_with_direct_apply_and_caller_queued_stage_from_sources(
            &mut manual_views,
            &tx_source,
            &view_source,
            &preflight,
            QueueApplyAfterPreflightSourceInputs::new(hold_preflight(), observed_queue(&order)),
            |_| unreachable!("direct apply should fall through without tracing"),
            || ApplyResult::new(Ter::TES_SUCCESS, true, true),
            |views, queued| {
                run_queue_apply_queued_stage_with_fee_context(
                    views,
                    queued.account_seq_proxy,
                    queued.tx_seq_proxy,
                    queued.queued,
                    queued.fee_context,
                    queued.preflight_result,
                    |_| true,
                    |_| flow_after_preflight_result().to_preclaim(9, Ter::TES_SUCCESS),
                    || ApplyResult::new(Ter::TES_SUCCESS, true, false),
                    || {},
                )
            },
        );

    let mut helper_views = build_flow_views();
    let helper_stage = run_queue_apply_after_preflight_from_sources(
        &mut helper_views,
        &tx_source,
        &view_source,
        &preflight,
        QueueApplyAfterPreflightSourceInputs::new(hold_preflight(), observed_queue(&order)),
        |_| unreachable!("direct apply should fall through without tracing"),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |_| flow_after_preflight_result().to_preclaim(9, Ter::TES_SUCCESS),
        || ApplyResult::new(Ter::TES_SUCCESS, true, false),
        || {},
    );

    assert_eq!(helper_stage, manual_stage);
    assert_eq!(helper_views, manual_views);
}

#[test]
fn queue_apply_from_sources_caller_preclaim_log_sinks_match_manual_top_lowering() {
    let account = String::from("acct");
    let tx_source = flow_tx_source(&account);
    let view_source = flow_view_source();
    let order = OrderCandidates::new(Uint256::from_u64(0));
    let preflight = flow_after_preflight_result();
    let expected_prepared = RefCell::new(None::<QueueApplyPreparedPreclaimInputs<String>>);

    let mut manual_debug = Vec::new();
    let mut manual_info = Vec::new();
    let mut manual_views = build_flow_views();
    let manual_stage =
        run_queue_apply_top_with_caller_direct_apply_and_caller_queued_stage_from_sources(
            &mut manual_views,
            &tx_source,
            &view_source,
            tx::QueueApplyTopFromSourcesInputs::new(
                hold_preflight(),
                ApplyFlags::NONE,
                preflight.clone().consequences,
                observed_queue(&order),
            ),
            || preflight.clone(),
            |views, prepared| {
                tx::run_prepared_direct_apply_with_trace(
                    views,
                    prepared,
                    |_| unreachable!("direct apply should fall through without tracing"),
                    || ApplyResult::new(Ter::TES_SUCCESS, true, true),
                )
            },
            |views, queued| {
                run_prepared_queue_apply_queued_flow_stage(
                    views,
                    derive_queue_apply_prepared_flow_stage(
                        &*views,
                        queued.account_seq_proxy,
                        queued.tx_seq_proxy,
                        queued.queued,
                        queued.fee_context,
                        queued.preflight_result,
                        |_| true,
                    ),
                    |views, prepared| {
                        let prepared_preclaim = QueueApplyPreparedPreclaimInputs::new(
                            prepared.preclaim_view_source,
                            prepared.fee_level_paid,
                            prepared.base_level,
                            prepared.required_fee_level,
                            prepared.open_ledger_tx_count,
                            prepared.tx_id,
                            prepared.account,
                        );
                        let QueueApplyPreparedPreclaimInputs {
                            view_source,
                            fee_level_paid,
                            base_level,
                            required_fee_level,
                            open_ledger_tx_count,
                            tx_id,
                            account,
                        } = prepared_preclaim;

                        let preclaim = {
                            let prepared = QueueApplyPreparedPreclaimInputs::new(
                                view_source,
                                fee_level_paid,
                                base_level,
                                required_fee_level,
                                open_ledger_tx_count,
                                tx_id.clone(),
                                account.clone(),
                            );
                            expected_prepared.replace(Some(prepared.clone()));
                            QueueApplyPreclaimStage {
                                view_source: prepared.view_source,
                                trace_message: "trace".to_string(),
                                preclaim_result: flow_after_preflight_result()
                                    .to_preclaim(9, Ter::TES_SUCCESS),
                            }
                        };

                        run_prepared_queue_apply_post_preclaim_stage_with_caller_queue(
                            views,
                            derive_queue_apply_prepared_post_preclaim_inputs(
                                prepared.tx_seq_proxy,
                                prepared.first_relevant_retries_remaining,
                                fee_level_paid,
                                base_level,
                                required_fee_level,
                                prepared.hold_fallback,
                                prepared.full_queue_decision,
                                prepared.replaced,
                                account,
                                tx_id,
                                prepared.last_valid,
                                prepared.flags,
                                prepared.pf_result,
                                prepared.order,
                                preclaim,
                            ),
                            |prepared| {
                                run_queue_apply_try_clear_stage(
                                    prepared.gate,
                                    || ApplyResult::new(Ter::TES_SUCCESS, true, false),
                                    || {},
                                )
                            },
                            |views, prepared| {
                                run_queue_apply_queue_stage_with_log_sinks(
                                    views,
                                    prepared.hold_fallback,
                                    prepared.full_queue_decision,
                                    prepared.replaced,
                                    prepared.account,
                                    prepared.tx_id,
                                    prepared.last_valid,
                                    prepared.seq_proxy,
                                    prepared.fee_level,
                                    prepared.flags,
                                    prepared.pf_result,
                                    prepared.order,
                                    |message| manual_debug.push(message),
                                    |message| manual_info.push(message),
                                )
                            },
                        )
                    },
                )
            },
        );

    let mut helper_debug = Vec::new();
    let mut helper_info = Vec::new();
    let mut helper_views = build_flow_views();
    let helper_stage = run_queue_apply_top_with_caller_preclaim_and_log_sinks_from_sources(
        &mut helper_views,
        &tx_source,
        &view_source,
        tx::QueueApplyTopFromSourcesInputs::new(
            hold_preflight(),
            ApplyFlags::NONE,
            preflight.clone().consequences,
            observed_queue(&order),
        ),
        || preflight.clone(),
        |_| unreachable!("direct apply should fall through without tracing"),
        |message| helper_debug.push(message),
        |message| helper_info.push(message),
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

    assert_eq!(helper_stage, manual_stage);
    assert_eq!(helper_views, manual_views);
    assert_eq!(helper_debug, manual_debug);
    assert_eq!(helper_info, manual_info);
}

#[test]
fn queue_apply_from_sources_plain_log_sinks_match_manual_top_lowering() {
    let account = String::from("acct");
    let tx_source = flow_tx_source(&account);
    let view_source = flow_view_source();
    let order = OrderCandidates::new(Uint256::from_u64(0));
    let preflight = flow_after_preflight_result();

    let mut manual_debug = Vec::new();
    let mut manual_info = Vec::new();
    let mut manual_views = build_flow_views();
    let manual_stage =
        run_queue_apply_top_with_caller_direct_apply_and_caller_queued_stage_from_sources(
            &mut manual_views,
            &tx_source,
            &view_source,
            tx::QueueApplyTopFromSourcesInputs::new(
                hold_preflight(),
                ApplyFlags::NONE,
                preflight.clone().consequences,
                observed_queue(&order),
            ),
            || preflight.clone(),
            |views, prepared| {
                tx::run_prepared_direct_apply_with_trace(
                    views,
                    prepared,
                    |_| unreachable!("direct apply should fall through without tracing"),
                    || ApplyResult::new(Ter::TES_SUCCESS, true, true),
                )
            },
            |views, queued| {
                run_queue_apply_queued_stage_with_fee_context_and_log_sinks(
                    views,
                    queued.account_seq_proxy,
                    queued.tx_seq_proxy,
                    queued.queued,
                    queued.fee_context,
                    queued.preflight_result,
                    |_| true,
                    |_| flow_after_preflight_result().to_preclaim(9, Ter::TES_SUCCESS),
                    || ApplyResult::new(Ter::TES_SUCCESS, true, false),
                    || {},
                    |message| manual_debug.push(message),
                    |message| manual_info.push(message),
                )
            },
        );

    let mut helper_debug = Vec::new();
    let mut helper_info = Vec::new();
    let mut helper_views = build_flow_views();
    let helper_stage = run_queue_apply_top_with_log_sinks_from_sources(
        &mut helper_views,
        &tx_source,
        &view_source,
        tx::QueueApplyTopFromSourcesInputs::new(
            hold_preflight(),
            ApplyFlags::NONE,
            preflight.clone().consequences,
            observed_queue(&order),
        ),
        || preflight.clone(),
        |_| unreachable!("direct apply should fall through without tracing"),
        |message| helper_debug.push(message),
        |message| helper_info.push(message),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |_| flow_after_preflight_result().to_preclaim(9, Ter::TES_SUCCESS),
        || ApplyResult::new(Ter::TES_SUCCESS, true, false),
        || {},
    );

    assert_eq!(helper_stage, manual_stage);
    assert_eq!(helper_views, manual_views);
    assert_eq!(helper_debug, manual_debug);
    assert_eq!(helper_info, manual_info);
}

#[test]
fn queue_apply_from_sources_plain_log_messages_match_manual_top_lowering() {
    let account = String::from("acct");
    let tx_source = flow_tx_source(&account);
    let view_source = flow_view_source();
    let order = OrderCandidates::new(Uint256::from_u64(0));
    let preflight = flow_after_preflight_result();

    let mut manual_views = build_flow_views();
    let manual_stage =
        tx::run_queue_apply_top_with_caller_direct_apply_and_caller_queued_stage_and_log_messages(
            &mut manual_views,
            tx::build_queue_apply_top_with_queued_stage_inputs_from_sources(
                &tx_source,
                &view_source,
                tx::QueueApplyTopFromSourcesInputs::new(
                    hold_preflight(),
                    ApplyFlags::NONE,
                    preflight.clone().consequences,
                    observed_queue(&order),
                ),
            ),
            || preflight.clone(),
            |_, _| unreachable!("direct apply should fall through without tracing"),
            |views, queued| {
                run_queue_apply_queued_stage_with_log_messages(
                    views,
                    queued.queued.account,
                    queued.account_seq_proxy,
                    queued.tx_seq_proxy,
                    queued.queued.preflight,
                    queued.queued.is_blocker,
                    queued.queued.open_ledger_seq,
                    queued.queued.minimum_last_ledger_buffer,
                    queued.queued.maximum_txn_per_account,
                    queued.queued.retry_sequence_percent,
                    queued.queued.queue_is_full,
                    queued.fee_context.fee_level_paid,
                    queued.fee_context.required_fee_level,
                    queued.fee_context.base_level,
                    queued.queued.balance_drops,
                    queued.queued.reserve_drops,
                    queued.queued.base_fee_drops,
                    queued.queued.can_be_held_result,
                    queued.queued.open_ledger_tx_count,
                    queued.queued.tx_id,
                    queued.queued.last_valid,
                    queued.queued.flags,
                    queued.preflight_result,
                    queued.queued.order,
                    |_| true,
                    |_| flow_after_preflight_result().to_preclaim(9, Ter::TES_SUCCESS),
                    || ApplyResult::new(Ter::TES_SUCCESS, true, false),
                    || {},
                )
            },
        );

    let mut helper_views = build_flow_views();
    let helper_stage = run_queue_apply_top_with_log_messages_from_sources(
        &mut helper_views,
        &tx_source,
        &view_source,
        tx::QueueApplyTopFromSourcesInputs::new(
            hold_preflight(),
            ApplyFlags::NONE,
            preflight.clone().consequences,
            observed_queue(&order),
        ),
        || preflight.clone(),
        |_| unreachable!("direct apply should fall through without tracing"),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |_| flow_after_preflight_result().to_preclaim(9, Ter::TES_SUCCESS),
        || ApplyResult::new(Ter::TES_SUCCESS, true, false),
        || {},
    );

    assert_eq!(helper_stage, manual_stage);
    assert_eq!(helper_views, manual_views);
}

#[test]
fn queue_apply_from_sources_after_preflight_caller_preclaim_log_sinks_match_manual_lowering() {
    let account = String::from("acct");
    let tx_source = flow_tx_source(&account);
    let view_source = flow_view_source();
    let order = OrderCandidates::new(Uint256::from_u64(0));
    let preflight = flow_after_preflight_result();
    let expected_prepared = RefCell::new(None::<QueueApplyPreparedPreclaimInputs<String>>);

    let mut manual_debug = Vec::new();
    let mut manual_info = Vec::new();
    let mut manual_views = build_flow_views();
    let manual_stage =
        run_queue_apply_after_preflight_with_caller_direct_apply_and_caller_queued_stage_from_sources(
            &mut manual_views,
            &tx_source,
            &view_source,
            &preflight,
            QueueApplyAfterPreflightSourceInputs::new(hold_preflight(), observed_queue(&order)),
            |views, prepared| {
                tx::run_prepared_direct_apply_with_trace(
                    views,
                    prepared,
                    |_| unreachable!("direct apply should fall through without tracing"),
                    || ApplyResult::new(Ter::TES_SUCCESS, true, true),
                )
            },
            |views, queued| {
                run_prepared_queue_apply_queued_flow_stage(
                    views,
                    derive_queue_apply_prepared_flow_stage(
                        &*views,
                        queued.account_seq_proxy,
                        queued.tx_seq_proxy,
                        queued.queued,
                        queued.fee_context,
                        queued.preflight_result,
                        |_| true,
                    ),
                    |views, prepared| {
                        let prepared_preclaim = QueueApplyPreparedPreclaimInputs::new(
                            prepared.preclaim_view_source,
                            prepared.fee_level_paid,
                            prepared.base_level,
                            prepared.required_fee_level,
                            prepared.open_ledger_tx_count,
                            prepared.tx_id,
                            prepared.account,
                        );
                        let QueueApplyPreparedPreclaimInputs {
                            view_source,
                            fee_level_paid,
                            base_level,
                            required_fee_level,
                            open_ledger_tx_count,
                            tx_id,
                            account,
                        } = prepared_preclaim;

                        let preclaim = {
                            let prepared = QueueApplyPreparedPreclaimInputs::new(
                                view_source,
                                fee_level_paid,
                                base_level,
                                required_fee_level,
                                open_ledger_tx_count,
                                tx_id.clone(),
                                account.clone(),
                            );
                            expected_prepared.replace(Some(prepared.clone()));
                            QueueApplyPreclaimStage {
                                view_source: prepared.view_source,
                                trace_message: "trace".to_string(),
                                preclaim_result: flow_after_preflight_result()
                                    .to_preclaim(9, Ter::TES_SUCCESS),
                            }
                        };

                        run_prepared_queue_apply_post_preclaim_stage_with_caller_queue(
                            views,
                            derive_queue_apply_prepared_post_preclaim_inputs(
                                prepared.tx_seq_proxy,
                                prepared.first_relevant_retries_remaining,
                                fee_level_paid,
                                base_level,
                                required_fee_level,
                                prepared.hold_fallback,
                                prepared.full_queue_decision,
                                prepared.replaced,
                                account,
                                tx_id,
                                prepared.last_valid,
                                prepared.flags,
                                prepared.pf_result,
                                prepared.order,
                                preclaim,
                            ),
                            |prepared| {
                                run_queue_apply_try_clear_stage(
                                    prepared.gate,
                                    || ApplyResult::new(Ter::TES_SUCCESS, true, false),
                                    || {},
                                )
                            },
                            |views, prepared| {
                                run_queue_apply_queue_stage_with_log_sinks(
                                    views,
                                    prepared.hold_fallback,
                                    prepared.full_queue_decision,
                                    prepared.replaced,
                                    prepared.account,
                                    prepared.tx_id,
                                    prepared.last_valid,
                                    prepared.seq_proxy,
                                    prepared.fee_level,
                                    prepared.flags,
                                    prepared.pf_result,
                                    prepared.order,
                                    |message| manual_debug.push(message),
                                    |message| manual_info.push(message),
                                )
                            },
                        )
                    },
                )
            },
        );

    let mut helper_debug = Vec::new();
    let mut helper_info = Vec::new();
    let mut helper_views = build_flow_views();
    let helper_stage =
        run_queue_apply_after_preflight_with_caller_preclaim_and_log_sinks_from_sources(
            &mut helper_views,
            &tx_source,
            &view_source,
            &preflight,
            QueueApplyAfterPreflightSourceInputs::new(hold_preflight(), observed_queue(&order)),
            |_| unreachable!("direct apply should fall through without tracing"),
            |message| helper_debug.push(message),
            |message| helper_info.push(message),
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

    assert_eq!(helper_stage, manual_stage);
    assert_eq!(helper_views, manual_views);
    assert_eq!(helper_debug, manual_debug);
    assert_eq!(helper_info, manual_info);
}

#[test]
fn queue_apply_from_sources_after_preflight_plain_log_sinks_match_manual_lowering() {
    let account = String::from("acct");
    let tx_source = flow_tx_source(&account);
    let view_source = flow_view_source();
    let order = OrderCandidates::new(Uint256::from_u64(0));
    let preflight = flow_after_preflight_result();

    let mut manual_debug = Vec::new();
    let mut manual_info = Vec::new();
    let mut manual_views = build_flow_views();
    let manual_stage =
        run_queue_apply_after_preflight_with_direct_apply_and_caller_queued_stage_from_sources(
            &mut manual_views,
            &tx_source,
            &view_source,
            &preflight,
            QueueApplyAfterPreflightSourceInputs::new(hold_preflight(), observed_queue(&order)),
            |_| unreachable!("direct apply should fall through without tracing"),
            || ApplyResult::new(Ter::TES_SUCCESS, true, true),
            |views, queued| {
                run_queue_apply_queued_stage_with_fee_context_and_log_sinks(
                    views,
                    queued.account_seq_proxy,
                    queued.tx_seq_proxy,
                    queued.queued,
                    queued.fee_context,
                    queued.preflight_result,
                    |_| true,
                    |_| flow_after_preflight_result().to_preclaim(9, Ter::TES_SUCCESS),
                    || ApplyResult::new(Ter::TES_SUCCESS, true, false),
                    || {},
                    |message| manual_debug.push(message),
                    |message| manual_info.push(message),
                )
            },
        );

    let mut helper_debug = Vec::new();
    let mut helper_info = Vec::new();
    let mut helper_views = build_flow_views();
    let helper_stage = run_queue_apply_after_preflight_with_log_sinks_from_sources(
        &mut helper_views,
        &tx_source,
        &view_source,
        &preflight,
        QueueApplyAfterPreflightSourceInputs::new(hold_preflight(), observed_queue(&order)),
        |_| unreachable!("direct apply should fall through without tracing"),
        |message| helper_debug.push(message),
        |message| helper_info.push(message),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |_| flow_after_preflight_result().to_preclaim(9, Ter::TES_SUCCESS),
        || ApplyResult::new(Ter::TES_SUCCESS, true, false),
        || {},
    );

    assert_eq!(helper_stage, manual_stage);
    assert_eq!(helper_views, manual_views);
    assert_eq!(helper_debug, manual_debug);
    assert_eq!(helper_info, manual_info);
}

#[test]
fn queue_apply_from_sources_after_preflight_plain_log_messages_match_manual_lowering() {
    let account = String::from("acct");
    let tx_source = flow_tx_source(&account);
    let view_source = flow_view_source();
    let order = OrderCandidates::new(Uint256::from_u64(0));
    let preflight = flow_after_preflight_result();

    let mut manual_views = build_flow_views();
    let manual_stage =
        run_queue_apply_after_preflight_with_direct_apply_and_caller_queued_stage_and_log_messages(
            &mut manual_views,
            tx::build_queue_apply_top_with_queued_stage_inputs_from_sources_after_preflight(
                &tx_source,
                &view_source,
                &preflight,
                QueueApplyAfterPreflightSourceInputs::new(hold_preflight(), observed_queue(&order)),
            ),
            &preflight,
            |_| unreachable!("direct apply should fall through without tracing"),
            || ApplyResult::new(Ter::TES_SUCCESS, true, true),
            |views, queued| {
                run_queue_apply_queued_stage_with_log_messages(
                    views,
                    queued.queued.account,
                    queued.account_seq_proxy,
                    queued.tx_seq_proxy,
                    queued.queued.preflight,
                    queued.queued.is_blocker,
                    queued.queued.open_ledger_seq,
                    queued.queued.minimum_last_ledger_buffer,
                    queued.queued.maximum_txn_per_account,
                    queued.queued.retry_sequence_percent,
                    queued.queued.queue_is_full,
                    queued.fee_context.fee_level_paid,
                    queued.fee_context.required_fee_level,
                    queued.fee_context.base_level,
                    queued.queued.balance_drops,
                    queued.queued.reserve_drops,
                    queued.queued.base_fee_drops,
                    queued.queued.can_be_held_result,
                    queued.queued.open_ledger_tx_count,
                    queued.queued.tx_id,
                    queued.queued.last_valid,
                    queued.queued.flags,
                    queued.preflight_result,
                    queued.queued.order,
                    |_| true,
                    |_| flow_after_preflight_result().to_preclaim(9, Ter::TES_SUCCESS),
                    || ApplyResult::new(Ter::TES_SUCCESS, true, false),
                    || {},
                )
            },
        );

    let mut helper_views = build_flow_views();
    let helper_stage = run_queue_apply_after_preflight_with_log_messages_from_sources(
        &mut helper_views,
        &tx_source,
        &view_source,
        &preflight,
        QueueApplyAfterPreflightSourceInputs::new(hold_preflight(), observed_queue(&order)),
        |_| unreachable!("direct apply should fall through without tracing"),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |_| flow_after_preflight_result().to_preclaim(9, Ter::TES_SUCCESS),
        || ApplyResult::new(Ter::TES_SUCCESS, true, false),
        || {},
    );

    assert_eq!(helper_stage, manual_stage);
    assert_eq!(helper_views, manual_views);
}

#[test]
fn queue_apply_from_sources_caller_direct_apply_matches_manual_top_lowering() {
    let account = String::from("acct");
    let tx_source = flow_tx_source(&account);
    let view_source = flow_view_source();
    let order = OrderCandidates::new(Uint256::from_u64(0));
    let preflight = flow_after_preflight_result();

    let mut manual_views = build_flow_views();
    let manual_stage =
        run_queue_apply_top_with_caller_direct_apply_and_caller_queued_stage_from_sources(
            &mut manual_views,
            &tx_source,
            &view_source,
            tx::QueueApplyTopFromSourcesInputs::new(
                hold_preflight(),
                ApplyFlags::NONE,
                preflight.clone().consequences,
                observed_queue(&order),
            ),
            || preflight.clone(),
            |_views, _prepared| unreachable!("sequence mismatch should skip direct apply"),
            |views, queued| {
                run_queue_apply_queued_stage_with_fee_context(
                    views,
                    queued.account_seq_proxy,
                    queued.tx_seq_proxy,
                    queued.queued,
                    queued.fee_context,
                    queued.preflight_result,
                    |_| true,
                    |_| flow_after_preflight_result().to_preclaim(9, Ter::TES_SUCCESS),
                    || ApplyResult::new(Ter::TES_SUCCESS, true, false),
                    || {},
                )
            },
        );

    let mut helper_views = build_flow_views();
    let helper_stage = run_queue_apply_top_with_caller_direct_apply_and_queued_stage_from_sources(
        &mut helper_views,
        &tx_source,
        &view_source,
        tx::QueueApplyTopFromSourcesInputs::new(
            hold_preflight(),
            ApplyFlags::NONE,
            preflight.clone().consequences,
            observed_queue(&order),
        ),
        || preflight.clone(),
        |_views, _prepared| unreachable!("sequence mismatch should skip direct apply"),
        |_| true,
        |_| flow_after_preflight_result().to_preclaim(9, Ter::TES_SUCCESS),
        || ApplyResult::new(Ter::TES_SUCCESS, true, false),
        || {},
    );

    assert_eq!(helper_stage, manual_stage);
    assert_eq!(helper_views, manual_views);
}

#[test]
fn queue_apply_from_sources_after_preflight_caller_direct_apply_matches_manual_lowering() {
    let account = String::from("acct");
    let tx_source = flow_tx_source(&account);
    let view_source = flow_view_source();
    let order = OrderCandidates::new(Uint256::from_u64(0));
    let preflight = flow_after_preflight_result();

    let mut manual_views = build_flow_views();
    let manual_stage =
        run_queue_apply_after_preflight_with_caller_direct_apply_and_caller_queued_stage_from_sources(
            &mut manual_views,
            &tx_source,
            &view_source,
            &preflight,
            QueueApplyAfterPreflightSourceInputs::new(hold_preflight(), observed_queue(&order)),
            |_views, _prepared| unreachable!("sequence mismatch should skip direct apply"),
            |views, queued| {
                run_queue_apply_queued_stage_with_fee_context(
                    views,
                    queued.account_seq_proxy,
                    queued.tx_seq_proxy,
                    queued.queued,
                    queued.fee_context,
                    queued.preflight_result,
                    |_| true,
                    |_| flow_after_preflight_result().to_preclaim(9, Ter::TES_SUCCESS),
                    || ApplyResult::new(Ter::TES_SUCCESS, true, false),
                    || {},
                )
            },
        );

    let mut helper_views = build_flow_views();
    let helper_stage = run_queue_apply_after_preflight_with_caller_direct_apply_from_sources(
        &mut helper_views,
        &tx_source,
        &view_source,
        &preflight,
        QueueApplyAfterPreflightSourceInputs::new(hold_preflight(), observed_queue(&order)),
        |_views, _prepared| unreachable!("sequence mismatch should skip direct apply"),
        |_| true,
        |_| flow_after_preflight_result().to_preclaim(9, Ter::TES_SUCCESS),
        || ApplyResult::new(Ter::TES_SUCCESS, true, false),
        || {},
    );

    assert_eq!(helper_stage, manual_stage);
    assert_eq!(helper_views, manual_views);
}

#[test]
fn queue_apply_from_sources_after_preflight_acquired_direct_apply_matches_manual_lowering() {
    let account = String::from("acct");
    let tx_source = flow_tx_source(&account);
    let view_source = flow_view_source();
    let order = OrderCandidates::new(Uint256::from_u64(0));
    let preflight = flow_after_preflight_result();

    let mut manual_views = build_flow_views();
    let manual_stage =
        run_queue_apply_after_preflight_with_acquired_direct_apply_and_caller_queued_stage_from_sources(
            &mut manual_views,
            &tx_source,
            &view_source,
            &preflight,
            QueueApplyAfterPreflightSourceInputs::new(hold_preflight(), observed_queue(&order)),
            None::<tx::DirectApplyExecution<String, &'static str>>,
            |views, queued| {
                run_queue_apply_queued_stage_with_fee_context(
                    views,
                    queued.account_seq_proxy,
                    queued.tx_seq_proxy,
                    queued.queued,
                    queued.fee_context,
                    queued.preflight_result,
                    |_| true,
                    |_| flow_after_preflight_result().to_preclaim(9, Ter::TES_SUCCESS),
                    || ApplyResult::new(Ter::TES_SUCCESS, true, false),
                    || {},
                )
            },
        );

    let mut helper_views = build_flow_views();
    let helper_stage = run_queue_apply_after_preflight_with_acquired_direct_apply_from_sources(
        &mut helper_views,
        &tx_source,
        &view_source,
        &preflight,
        QueueApplyAfterPreflightSourceInputs::new(hold_preflight(), observed_queue(&order)),
        None::<tx::DirectApplyExecution<String, &'static str>>,
        |_| true,
        |_| flow_after_preflight_result().to_preclaim(9, Ter::TES_SUCCESS),
        || ApplyResult::new(Ter::TES_SUCCESS, true, false),
        || {},
    );

    assert_eq!(helper_stage, manual_stage);
    assert_eq!(helper_views, manual_views);
}
