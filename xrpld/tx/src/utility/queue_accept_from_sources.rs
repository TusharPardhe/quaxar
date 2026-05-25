//! Entry helper for callers that already have the live accept-call view facts
//! in hand and want to enter the landed
//! `TxQ::accept(...)` call-state wrapper without manually rebuilding each
//! intermediate carrier.
//!
//! This module lowers the current open-ledger count, parent hash, and queue
//! max-size facts into `QueueAcceptCallState`.

use std::fmt::Display;

use basics::base_uint::Uint256;

use crate::{
    ApplyResult, MaybeTx, QueueAcceptCallState, QueueAcceptOwnerState, QueueAcceptPreparedCallStep,
    QueueAcceptWithMetricsResult, QueueFeeMetricsState, QueueViews,
    prepare_queue_accept_with_call_state, run_queue_accept_with_call_state,
    run_queue_accept_with_call_state_and_log_sinks, run_queue_accept_with_caller_prepared_apply,
    run_queue_accept_with_caller_prepared_apply_and_log_sinks,
};

pub trait QueueAcceptObservedViewSource {
    fn open_ledger_tx_count(&self) -> usize;
    fn parent_hash(&self) -> Uint256;
}

pub trait QueueAcceptObservedQueueSource {
    fn current_max_size(&self) -> Option<usize>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueAcceptObservedViewInputs {
    pub open_ledger_tx_count: usize,
    pub parent_hash: Uint256,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueAcceptObservedQueueInputs {
    pub current_max_size: Option<usize>,
}

pub fn build_queue_accept_observed_view_inputs_from_source<Source>(
    source: &Source,
) -> QueueAcceptObservedViewInputs
where
    Source: QueueAcceptObservedViewSource,
{
    QueueAcceptObservedViewInputs {
        open_ledger_tx_count: source.open_ledger_tx_count(),
        parent_hash: source.parent_hash(),
    }
}

pub fn build_queue_accept_observed_queue_inputs_from_source<Source>(
    source: &Source,
) -> QueueAcceptObservedQueueInputs
where
    Source: QueueAcceptObservedQueueSource,
{
    QueueAcceptObservedQueueInputs {
        current_max_size: source.current_max_size(),
    }
}

pub fn build_queue_accept_call_state_from_observed(
    view_inputs: QueueAcceptObservedViewInputs,
    queue_inputs: QueueAcceptObservedQueueInputs,
) -> QueueAcceptCallState {
    QueueAcceptCallState::new(
        view_inputs.open_ledger_tx_count,
        queue_inputs.current_max_size,
        view_inputs.parent_hash,
    )
}

pub fn build_queue_accept_call_state_from_sources<ViewSource, QueueSource>(
    view_source: &ViewSource,
    queue_source: &QueueSource,
) -> QueueAcceptCallState
where
    ViewSource: QueueAcceptObservedViewSource,
    QueueSource: QueueAcceptObservedQueueSource,
{
    build_queue_accept_call_state_from_observed(
        build_queue_accept_observed_view_inputs_from_source(view_source),
        build_queue_accept_observed_queue_inputs_from_source(queue_source),
    )
}

pub fn run_queue_accept_from_sources<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    ViewSource,
    QueueSource,
    ApplyFn,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    metrics: &QueueFeeMetricsState,
    owner_state: &mut QueueAcceptOwnerState,
    view_source: &ViewSource,
    queue_source: &QueueSource,
    apply: ApplyFn,
) -> QueueAcceptWithMetricsResult<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    ViewSource: QueueAcceptObservedViewSource,
    QueueSource: QueueAcceptObservedQueueSource,
    ApplyFn: FnMut(&mut MaybeTx<Tx, Account, Journal, ParentBatchId>) -> ApplyResult,
{
    run_queue_accept_with_call_state(
        views,
        metrics,
        owner_state,
        build_queue_accept_call_state_from_sources(view_source, queue_source),
        apply,
    )
}

pub fn prepare_queue_accept_from_sources<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    ViewSource,
    QueueSource,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    metrics: &QueueFeeMetricsState,
    owner_state: &mut QueueAcceptOwnerState,
    view_source: &ViewSource,
    queue_source: &QueueSource,
) -> QueueAcceptPreparedCallStep<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    ViewSource: QueueAcceptObservedViewSource,
    QueueSource: QueueAcceptObservedQueueSource,
{
    prepare_queue_accept_with_call_state(
        views,
        metrics,
        owner_state,
        build_queue_accept_call_state_from_sources(view_source, queue_source),
    )
}

pub fn run_queue_accept_from_sources_with_caller_prepared_apply<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    ViewSource,
    QueueSource,
    RunPreparedApply,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    metrics: &QueueFeeMetricsState,
    owner_state: &mut QueueAcceptOwnerState,
    view_source: &ViewSource,
    queue_source: &QueueSource,
    run_prepared_apply: RunPreparedApply,
) -> QueueAcceptWithMetricsResult<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    ViewSource: QueueAcceptObservedViewSource,
    QueueSource: QueueAcceptObservedQueueSource,
    RunPreparedApply: FnMut(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        &mut QueueAcceptOwnerState,
        crate::PreparedQueueAcceptCall<Account>,
    ) -> crate::QueueAcceptPreparedCallStep<Account>,
{
    run_queue_accept_with_caller_prepared_apply(
        views,
        metrics,
        owner_state,
        build_queue_accept_call_state_from_sources(view_source, queue_source),
        run_prepared_apply,
    )
}

pub fn run_queue_accept_from_sources_with_caller_prepared_apply_and_log_sinks<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    ViewSource,
    QueueSource,
    RunPreparedApply,
    TraceFn,
    DebugFn,
    InfoFn,
    WarnFn,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    metrics: &QueueFeeMetricsState,
    owner_state: &mut QueueAcceptOwnerState,
    view_source: &ViewSource,
    queue_source: &QueueSource,
    trace: TraceFn,
    debug: DebugFn,
    info: InfoFn,
    warn: WarnFn,
    run_prepared_apply: RunPreparedApply,
) -> QueueAcceptWithMetricsResult<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    ViewSource: QueueAcceptObservedViewSource,
    QueueSource: QueueAcceptObservedQueueSource,
    RunPreparedApply: FnMut(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        &mut QueueAcceptOwnerState,
        crate::PreparedQueueAcceptCall<Account>,
    ) -> crate::QueueAcceptPreparedCallStep<Account>,
    TraceFn: FnMut(&str),
    DebugFn: FnMut(&str),
    InfoFn: FnMut(&str),
    WarnFn: FnMut(&str),
{
    run_queue_accept_with_caller_prepared_apply_and_log_sinks(
        views,
        metrics,
        owner_state,
        build_queue_accept_call_state_from_sources(view_source, queue_source),
        trace,
        debug,
        info,
        warn,
        run_prepared_apply,
    )
}

pub fn run_queue_accept_from_sources_and_log_sinks<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    ViewSource,
    QueueSource,
    ApplyFn,
    TraceFn,
    DebugFn,
    InfoFn,
    WarnFn,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    metrics: &QueueFeeMetricsState,
    owner_state: &mut QueueAcceptOwnerState,
    view_source: &ViewSource,
    queue_source: &QueueSource,
    trace: TraceFn,
    debug: DebugFn,
    info: InfoFn,
    warn: WarnFn,
    apply: ApplyFn,
) -> QueueAcceptWithMetricsResult<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    ViewSource: QueueAcceptObservedViewSource,
    QueueSource: QueueAcceptObservedQueueSource,
    ApplyFn: FnMut(&mut MaybeTx<Tx, Account, Journal, ParentBatchId>) -> ApplyResult,
    TraceFn: FnMut(&str),
    DebugFn: FnMut(&str),
    InfoFn: FnMut(&str),
    WarnFn: FnMut(&str),
{
    run_queue_accept_with_call_state_and_log_sinks(
        views,
        metrics,
        owner_state,
        build_queue_accept_call_state_from_sources(view_source, queue_source),
        trace,
        debug,
        info,
        warn,
        apply,
    )
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::collections::BTreeMap;

    use basics::base_uint::Uint256;
    use protocol::{Rules, SeqProxy, Ter};

    use super::{
        QueueAcceptObservedQueueInputs, QueueAcceptObservedQueueSource,
        QueueAcceptObservedViewInputs, QueueAcceptObservedViewSource,
        build_queue_accept_call_state_from_observed,
        build_queue_accept_observed_queue_inputs_from_source,
        build_queue_accept_observed_view_inputs_from_source, prepare_queue_accept_from_sources,
        run_queue_accept_from_sources, run_queue_accept_from_sources_and_log_sinks,
        run_queue_accept_from_sources_with_caller_prepared_apply,
        run_queue_accept_from_sources_with_caller_prepared_apply_and_log_sinks,
    };
    use crate::{
        ApplyFlags, ApplyResult, MaybeTx, MaybeTxCore, PreflightResult, PreparedQueueAcceptCall,
        QueueAcceptCallState, QueueAcceptOwnerState, QueueAcceptPreparedCallStep,
        QueueFeeMetricsConfig, QueueFeeMetricsState, QueueViews, TXQ_BASE_LEVEL, TxConsequences,
        TxQAccount, format_queue_accept_apply_trace_message,
        format_queue_accept_drop_last_info_message, format_queue_accept_fee_trace_message,
        format_queue_accept_leave_in_queue_debug_message,
        format_queue_accept_parent_hash_unchanged_warning, run_prepared_queue_accept_call,
        run_queue_accept_with_call_state,
    };

    #[derive(Debug, Clone, Copy)]
    struct TestViewSource {
        open_ledger_tx_count: usize,
        parent_hash: Uint256,
    }

    impl QueueAcceptObservedViewSource for TestViewSource {
        fn open_ledger_tx_count(&self) -> usize {
            self.open_ledger_tx_count
        }

        fn parent_hash(&self) -> Uint256 {
            self.parent_hash
        }
    }

    #[derive(Debug, Clone, Copy)]
    struct TestQueueSource {
        current_max_size: Option<usize>,
    }

    impl QueueAcceptObservedQueueSource for TestQueueSource {
        fn current_max_size(&self) -> Option<usize> {
            self.current_max_size
        }
    }

    fn queued(
        account: &'static str,
        seq_proxy: SeqProxy,
        tx_id: u64,
        fee_level: u64,
    ) -> MaybeTx<&'static str, &'static str, &'static str, &'static str> {
        MaybeTx::new(
            Uint256::from_u64(tx_id),
            fee_level,
            account,
            Some(200),
            seq_proxy,
            ApplyFlags::NONE,
            PreflightResult::new(
                "tx",
                None::<&str>,
                Rules::new(std::iter::empty()),
                TxConsequences::new(1, seq_proxy),
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            ),
        )
    }

    fn queue_candidate(
        seq_proxy: SeqProxy,
        tx_id: u64,
        fee_level: u64,
    ) -> crate::QueueAdvanceCandidate {
        crate::QueueAdvanceCandidate {
            fee_level,
            tx_id: Uint256::from_u64(tx_id),
            seq_proxy,
        }
    }

    fn metrics_state() -> QueueFeeMetricsState {
        QueueFeeMetricsState::new(QueueFeeMetricsConfig {
            ledgers_in_queue: 3,
            queue_size_min: 20,
            minimum_escalation_multiplier: TXQ_BASE_LEVEL * 500,
            minimum_txn_in_ledger: 32,
            target_txn_in_ledger: 256,
            maximum_txn_in_ledger: Some(400),
            normal_consensus_increase_percent: 20,
            slow_consensus_decrease_percent: 50,
        })
    }

    fn metrics_state_with(
        minimum_txn_in_ledger: usize,
        minimum_escalation_multiplier: u64,
    ) -> QueueFeeMetricsState {
        QueueFeeMetricsState::new(QueueFeeMetricsConfig {
            ledgers_in_queue: 3,
            queue_size_min: 20,
            minimum_escalation_multiplier,
            minimum_txn_in_ledger,
            target_txn_in_ledger: minimum_txn_in_ledger,
            maximum_txn_in_ledger: Some(minimum_txn_in_ledger),
            normal_consensus_increase_percent: 20,
            slow_consensus_decrease_percent: 50,
        })
    }

    #[test]
    fn accept_observed_input_builders_read_current_accept_source_facts() {
        let view_source = TestViewSource {
            open_ledger_tx_count: 32,
            parent_hash: Uint256::from_u64(9),
        };
        let queue_source = TestQueueSource {
            current_max_size: Some(15),
        };

        assert_eq!(
            build_queue_accept_observed_view_inputs_from_source(&view_source),
            QueueAcceptObservedViewInputs {
                open_ledger_tx_count: 32,
                parent_hash: Uint256::from_u64(9),
            }
        );
        assert_eq!(
            build_queue_accept_observed_queue_inputs_from_source(&queue_source),
            QueueAcceptObservedQueueInputs {
                current_max_size: Some(15),
            }
        );
        assert_eq!(
            build_queue_accept_call_state_from_observed(
                build_queue_accept_observed_view_inputs_from_source(&view_source),
                build_queue_accept_observed_queue_inputs_from_source(&queue_source),
            ),
            QueueAcceptCallState::new(32, Some(15), Uint256::from_u64(9))
        );
    }

    #[test]
    fn accept_source_wrapper_reuses_landed_call_state_behavior() {
        let mut account_a = TxQAccount::new("a");
        account_a.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                queued("a", SeqProxy::sequence(5), 5, 5_000),
                TxConsequences::new(1, SeqProxy::sequence(5)),
            ),
        );

        let mut account_b = TxQAccount::new("b");
        account_b.add(
            SeqProxy::sequence(7),
            MaybeTxCore::new(
                queued("b", SeqProxy::sequence(7), 7, 1_000),
                TxConsequences::new(1, SeqProxy::sequence(7)),
            ),
        );

        let build_views = || {
            QueueViews::new(
                BTreeMap::from([("a", account_a.clone()), ("b", account_b.clone())]),
                vec![
                    crate::FeeQueueEntry::new(
                        crate::FeeQueueKey::new("a", SeqProxy::sequence(5)),
                        queue_candidate(SeqProxy::sequence(5), 5, 5_000),
                    ),
                    crate::FeeQueueEntry::new(
                        crate::FeeQueueKey::new("b", SeqProxy::sequence(7)),
                        queue_candidate(SeqProxy::sequence(7), 7, 1_000),
                    ),
                ],
            )
        };

        let metrics = metrics_state_with(1, 1_000);
        let view_source = TestViewSource {
            open_ledger_tx_count: 1,
            parent_hash: Uint256::from_u64(3),
        };
        let queue_source = TestQueueSource {
            current_max_size: Some(10),
        };

        let mut source_views = build_views();
        let mut source_owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(0));
        let source_apply_calls = Cell::new(0_usize);
        let source_result = run_queue_accept_from_sources(
            &mut source_views,
            &metrics,
            &mut source_owner_state,
            &view_source,
            &queue_source,
            |_queued| {
                source_apply_calls.set(source_apply_calls.get() + 1);
                ApplyResult::new(Ter::TES_SUCCESS, true, false)
            },
        );

        let mut manual_views = build_views();
        let mut manual_owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(0));
        let manual_apply_calls = Cell::new(0_usize);
        let manual_result = run_queue_accept_with_call_state(
            &mut manual_views,
            &metrics,
            &mut manual_owner_state,
            QueueAcceptCallState::new(1, Some(10), Uint256::from_u64(3)),
            |_queued| {
                manual_apply_calls.set(manual_apply_calls.get() + 1);
                ApplyResult::new(Ter::TES_SUCCESS, true, false)
            },
        );

        assert_eq!(source_apply_calls.get(), 1);
        assert_eq!(manual_apply_calls.get(), 1);
        assert_eq!(source_result, manual_result);
        assert_eq!(source_views, manual_views);
        assert_eq!(source_owner_state, manual_owner_state);
    }

    #[test]
    fn accept_source_sink_wrapper_emits_same_messages_as_landed_call_state_wrapper() {
        let mut account = TxQAccount::new("acct");
        account.drop_penalty = true;
        account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                queued("acct", SeqProxy::sequence(5), 5, 300),
                TxConsequences::new(1, SeqProxy::sequence(5)),
            ),
        );
        account.add(
            SeqProxy::sequence(9),
            MaybeTxCore::new(
                queued("acct", SeqProxy::sequence(9), 9, 60),
                TxConsequences::new(1, SeqProxy::sequence(9)),
            ),
        );

        let mut views = QueueViews::new(
            BTreeMap::from([("acct", account)]),
            vec![
                crate::FeeQueueEntry::new(
                    crate::FeeQueueKey::new("acct", SeqProxy::sequence(5)),
                    queue_candidate(SeqProxy::sequence(5), 5, 300),
                ),
                crate::FeeQueueEntry::new(
                    crate::FeeQueueKey::new("acct", SeqProxy::sequence(9)),
                    queue_candidate(SeqProxy::sequence(9), 9, 60),
                ),
            ],
        );
        let metrics = metrics_state();
        let mut owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(9));
        let view_source = TestViewSource {
            open_ledger_tx_count: 32,
            parent_hash: Uint256::from_u64(9),
        };
        let queue_source = TestQueueSource {
            current_max_size: Some(2),
        };
        let emitted = RefCell::new(Vec::new());

        let result = run_queue_accept_from_sources_and_log_sinks(
            &mut views,
            &metrics,
            &mut owner_state,
            &view_source,
            &queue_source,
            |message| emitted.borrow_mut().push(format!("trace:{message}")),
            |message| emitted.borrow_mut().push(format!("debug:{message}")),
            |message| emitted.borrow_mut().push(format!("info:{message}")),
            |message| emitted.borrow_mut().push(format!("warn:{message}")),
            |_queued| ApplyResult::new(Ter::TER_RETRY, false, false),
        );

        assert_eq!(
            emitted.into_inner(),
            vec![
                format!(
                    "trace:{}",
                    format_queue_accept_fee_trace_message(Uint256::from_u64(5), "acct", 300, 256)
                ),
                format!(
                    "trace:{}",
                    format_queue_accept_apply_trace_message(Uint256::from_u64(5))
                ),
                format!(
                    "debug:{}",
                    format_queue_accept_leave_in_queue_debug_message(
                        Uint256::from_u64(5),
                        Ter::TER_RETRY,
                        false,
                        ApplyFlags::NONE,
                    )
                ),
                format!(
                    "info:{}",
                    format_queue_accept_drop_last_info_message(
                        Uint256::from_u64(5),
                        Ter::TER_RETRY,
                        "acct",
                    )
                ),
                format!(
                    "warn:{}",
                    format_queue_accept_parent_hash_unchanged_warning(Uint256::from_u64(9))
                ),
            ]
        );
        assert_eq!(
            result.log_messages.warning,
            Some(format_queue_accept_parent_hash_unchanged_warning(
                Uint256::from_u64(9)
            ))
        );
    }

    #[test]
    fn prepare_accept_from_sources_reuses_prepared_call_state_wrapper() {
        let mut account = TxQAccount::new("acct");
        account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                queued("acct", SeqProxy::sequence(5), 5, 300),
                TxConsequences::new(1, SeqProxy::sequence(5)),
            ),
        );
        let mut views = QueueViews::new(
            BTreeMap::from([("acct", account)]),
            vec![crate::FeeQueueEntry::new(
                crate::FeeQueueKey::new("acct", SeqProxy::sequence(5)),
                queue_candidate(SeqProxy::sequence(5), 5, 300),
            )],
        );
        let metrics = metrics_state();
        let mut owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(4));

        assert_eq!(
            prepare_queue_accept_from_sources(
                &mut views,
                &metrics,
                &mut owner_state,
                &TestViewSource {
                    open_ledger_tx_count: 0,
                    parent_hash: Uint256::from_u64(6),
                },
                &TestQueueSource {
                    current_max_size: Some(10),
                },
            ),
            QueueAcceptPreparedCallStep::Ready(PreparedQueueAcceptCall {
                prepared_apply: crate::PreparedQueueAcceptApply {
                    candidate: crate::QueueAcceptCandidate {
                        key: crate::FeeQueueKey::new("acct", SeqProxy::sequence(5)),
                        tx_id: Uint256::from_u64(5),
                        fee_level: 300,
                        retries_remaining: 10,
                        flags: ApplyFlags::NONE,
                    },
                    required_fee_level: 256,
                    queue_nearly_full: false,
                    candidate_index: 0,
                    account_retry_penalty: false,
                    account_drop_penalty: false,
                    account_txn_count: 1,
                    order: crate::OrderCandidates::new(Uint256::from_u64(4)),
                },
                metrics_snapshot: metrics.snapshot(),
                call_state: QueueAcceptCallState::new(0, Some(10), Uint256::from_u64(6)),
                previous_parent_hash_comp: Uint256::from_u64(4),
                loop_messages: crate::QueueAcceptLoopLogMessages {
                    trace: vec![
                        crate::format_queue_accept_fee_trace_message(
                            Uint256::from_u64(5),
                            "acct",
                            300,
                            256,
                        ),
                        crate::format_queue_accept_apply_trace_message(Uint256::from_u64(5)),
                    ],
                    debug: vec![],
                    info: vec![],
                },
                ledger_changed: false,
                processed_candidates: 1,
                applied_count: 0,
            })
        );
    }

    #[test]
    fn source_wrapper_with_caller_prepared_apply_matches_direct_source_wrapper() {
        let build_views = || {
            let mut account = TxQAccount::new("acct");
            account.add(
                SeqProxy::sequence(5),
                MaybeTxCore::new(
                    queued("acct", SeqProxy::sequence(5), 5, 300),
                    TxConsequences::new(1, SeqProxy::sequence(5)),
                ),
            );
            QueueViews::new(
                BTreeMap::from([("acct", account)]),
                vec![crate::FeeQueueEntry::new(
                    crate::FeeQueueKey::new("acct", SeqProxy::sequence(5)),
                    queue_candidate(SeqProxy::sequence(5), 5, 300),
                )],
            )
        };

        let metrics = metrics_state();
        let view_source = TestViewSource {
            open_ledger_tx_count: 0,
            parent_hash: Uint256::from_u64(6),
        };
        let queue_source = TestQueueSource {
            current_max_size: Some(10),
        };

        let mut wrapped_views = build_views();
        let mut wrapped_owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(4));
        let wrapped_result = run_queue_accept_from_sources_with_caller_prepared_apply(
            &mut wrapped_views,
            &metrics,
            &mut wrapped_owner_state,
            &view_source,
            &queue_source,
            |views, owner_state, prepared| {
                run_prepared_queue_accept_call(views, owner_state, prepared, |_queued| {
                    ApplyResult::new(Ter::TES_SUCCESS, true, false)
                })
            },
        );

        let mut direct_views = build_views();
        let mut direct_owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(4));
        let direct_result = run_queue_accept_from_sources(
            &mut direct_views,
            &metrics,
            &mut direct_owner_state,
            &view_source,
            &queue_source,
            |_queued| ApplyResult::new(Ter::TES_SUCCESS, true, false),
        );

        assert_eq!(wrapped_result, direct_result);
        assert_eq!(wrapped_views, direct_views);
        assert_eq!(wrapped_owner_state, direct_owner_state);
    }

    #[test]
    fn source_sink_wrapper_with_caller_prepared_apply_matches_direct_source_sink_wrapper() {
        let mut account = TxQAccount::new("acct");
        account.drop_penalty = true;
        account.add(
            SeqProxy::sequence(5),
            MaybeTxCore::new(
                queued("acct", SeqProxy::sequence(5), 5, 300),
                TxConsequences::new(1, SeqProxy::sequence(5)),
            ),
        );
        account.add(
            SeqProxy::sequence(9),
            MaybeTxCore::new(
                queued("acct", SeqProxy::sequence(9), 9, 60),
                TxConsequences::new(1, SeqProxy::sequence(9)),
            ),
        );

        let build_views = || {
            QueueViews::new(
                BTreeMap::from([("acct", account.clone())]),
                vec![
                    crate::FeeQueueEntry::new(
                        crate::FeeQueueKey::new("acct", SeqProxy::sequence(5)),
                        queue_candidate(SeqProxy::sequence(5), 5, 300),
                    ),
                    crate::FeeQueueEntry::new(
                        crate::FeeQueueKey::new("acct", SeqProxy::sequence(9)),
                        queue_candidate(SeqProxy::sequence(9), 9, 60),
                    ),
                ],
            )
        };

        let metrics = metrics_state();
        let view_source = TestViewSource {
            open_ledger_tx_count: 32,
            parent_hash: Uint256::from_u64(9),
        };
        let queue_source = TestQueueSource {
            current_max_size: Some(2),
        };

        let mut wrapped_views = build_views();
        let mut wrapped_owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(9));
        let wrapped_emitted = RefCell::new(Vec::new());
        let wrapped_result = run_queue_accept_from_sources_with_caller_prepared_apply_and_log_sinks(
            &mut wrapped_views,
            &metrics,
            &mut wrapped_owner_state,
            &view_source,
            &queue_source,
            |message| {
                wrapped_emitted
                    .borrow_mut()
                    .push(format!("trace:{message}"))
            },
            |message| {
                wrapped_emitted
                    .borrow_mut()
                    .push(format!("debug:{message}"))
            },
            |message| wrapped_emitted.borrow_mut().push(format!("info:{message}")),
            |message| wrapped_emitted.borrow_mut().push(format!("warn:{message}")),
            |views, owner_state, prepared| {
                run_prepared_queue_accept_call(views, owner_state, prepared, |_queued| {
                    ApplyResult::new(Ter::TER_RETRY, false, false)
                })
            },
        );

        let mut direct_views = build_views();
        let mut direct_owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(9));
        let direct_emitted = RefCell::new(Vec::new());
        let direct_result = run_queue_accept_from_sources_and_log_sinks(
            &mut direct_views,
            &metrics,
            &mut direct_owner_state,
            &view_source,
            &queue_source,
            |message| direct_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| direct_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| direct_emitted.borrow_mut().push(format!("info:{message}")),
            |message| direct_emitted.borrow_mut().push(format!("warn:{message}")),
            |_queued| ApplyResult::new(Ter::TER_RETRY, false, false),
        );

        assert_eq!(wrapped_result, direct_result);
        assert_eq!(wrapped_views, direct_views);
        assert_eq!(wrapped_owner_state, direct_owner_state);
        assert_eq!(wrapped_emitted.into_inner(), direct_emitted.into_inner());
    }
}
