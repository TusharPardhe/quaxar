//! Public top-level Rust carrier for the current `xrpld`
//! `TxQ::accept(...)` seam.
//!
//! This top layer now owns both currently needed source shapes:
//! 1. split metrics/view/queue sources,
//! 2. one runtime-style source that exposes those same facts together.
//!
//! The landed top-level accept carrier preserves the current higher caller
//! order directly from either of those source shapes.

use std::fmt::Display;

use crate::{
    ApplyResult, MaybeTx, PreparedQueueAcceptCall, QueueAcceptObservedQueueSource,
    QueueAcceptObservedViewSource, QueueAcceptOwnerState, QueueAcceptPreparedCallStep,
    QueueAcceptRuntimeSource, QueueAcceptWithMetricsResult, QueueFeeMetricsState, QueueViews,
    prepare_queue_accept_from_sources, run_queue_accept_from_sources,
    run_queue_accept_from_sources_and_log_sinks,
    run_queue_accept_from_sources_with_caller_prepared_apply,
    run_queue_accept_from_sources_with_caller_prepared_apply_and_log_sinks,
};

#[derive(Debug, Clone, Copy)]
pub struct QueueAcceptTopInputs<'a, ViewSource, QueueSource> {
    pub metrics: &'a QueueFeeMetricsState,
    pub view_source: &'a ViewSource,
    pub queue_source: &'a QueueSource,
}

impl<'a, ViewSource, QueueSource> QueueAcceptTopInputs<'a, ViewSource, QueueSource> {
    pub const fn new(
        metrics: &'a QueueFeeMetricsState,
        view_source: &'a ViewSource,
        queue_source: &'a QueueSource,
    ) -> Self {
        Self {
            metrics,
            view_source,
            queue_source,
        }
    }
}

pub fn build_queue_accept_top_inputs_from_runtime_source<Source>(
    source: &Source,
) -> QueueAcceptTopInputs<'_, Source, Source>
where
    Source: QueueAcceptRuntimeSource,
{
    QueueAcceptTopInputs::new(source.metrics(), source, source)
}

pub fn run_queue_accept_top<Account, Tx, Journal, ParentBatchId, ViewSource, QueueSource, ApplyFn>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    owner_state: &mut QueueAcceptOwnerState,
    inputs: QueueAcceptTopInputs<'_, ViewSource, QueueSource>,
    apply: ApplyFn,
) -> QueueAcceptWithMetricsResult<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    ViewSource: QueueAcceptObservedViewSource,
    QueueSource: QueueAcceptObservedQueueSource,
    ApplyFn: FnMut(&mut MaybeTx<Tx, Account, Journal, ParentBatchId>) -> ApplyResult,
{
    run_queue_accept_from_sources(
        views,
        inputs.metrics,
        owner_state,
        inputs.view_source,
        inputs.queue_source,
        apply,
    )
}

pub fn prepare_queue_accept_top<Account, Tx, Journal, ParentBatchId, ViewSource, QueueSource>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    owner_state: &mut QueueAcceptOwnerState,
    inputs: QueueAcceptTopInputs<'_, ViewSource, QueueSource>,
) -> QueueAcceptPreparedCallStep<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    ViewSource: QueueAcceptObservedViewSource,
    QueueSource: QueueAcceptObservedQueueSource,
{
    prepare_queue_accept_from_sources(
        views,
        inputs.metrics,
        owner_state,
        inputs.view_source,
        inputs.queue_source,
    )
}

pub fn run_queue_accept_top_with_runtime_source<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    Source,
    ApplyFn,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    owner_state: &mut QueueAcceptOwnerState,
    runtime: &Source,
    apply: ApplyFn,
) -> QueueAcceptWithMetricsResult<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    Source: QueueAcceptRuntimeSource,
    ApplyFn: FnMut(&mut MaybeTx<Tx, Account, Journal, ParentBatchId>) -> ApplyResult,
{
    run_queue_accept_top(
        views,
        owner_state,
        build_queue_accept_top_inputs_from_runtime_source(runtime),
        apply,
    )
}

pub fn prepare_queue_accept_top_with_runtime_source<Account, Tx, Journal, ParentBatchId, Source>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    owner_state: &mut QueueAcceptOwnerState,
    runtime: &Source,
) -> QueueAcceptPreparedCallStep<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    Source: QueueAcceptRuntimeSource,
{
    prepare_queue_accept_top(
        views,
        owner_state,
        build_queue_accept_top_inputs_from_runtime_source(runtime),
    )
}

pub fn run_queue_accept_top_with_caller_prepared_apply<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    ViewSource,
    QueueSource,
    RunPreparedApply,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    owner_state: &mut QueueAcceptOwnerState,
    inputs: QueueAcceptTopInputs<'_, ViewSource, QueueSource>,
    run_prepared_apply: RunPreparedApply,
) -> QueueAcceptWithMetricsResult<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    ViewSource: QueueAcceptObservedViewSource,
    QueueSource: QueueAcceptObservedQueueSource,
    RunPreparedApply: FnMut(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        &mut QueueAcceptOwnerState,
        PreparedQueueAcceptCall<Account>,
    ) -> QueueAcceptPreparedCallStep<Account>,
{
    run_queue_accept_from_sources_with_caller_prepared_apply(
        views,
        inputs.metrics,
        owner_state,
        inputs.view_source,
        inputs.queue_source,
        run_prepared_apply,
    )
}

pub fn run_queue_accept_top_with_runtime_source_with_caller_prepared_apply<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    Source,
    RunPreparedApply,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    owner_state: &mut QueueAcceptOwnerState,
    runtime: &Source,
    run_prepared_apply: RunPreparedApply,
) -> QueueAcceptWithMetricsResult<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    Source: QueueAcceptRuntimeSource,
    RunPreparedApply: FnMut(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        &mut QueueAcceptOwnerState,
        PreparedQueueAcceptCall<Account>,
    ) -> QueueAcceptPreparedCallStep<Account>,
{
    run_queue_accept_top_with_caller_prepared_apply(
        views,
        owner_state,
        build_queue_accept_top_inputs_from_runtime_source(runtime),
        run_prepared_apply,
    )
}

pub fn run_queue_accept_top_with_log_sinks<
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
    owner_state: &mut QueueAcceptOwnerState,
    inputs: QueueAcceptTopInputs<'_, ViewSource, QueueSource>,
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
    run_queue_accept_from_sources_and_log_sinks(
        views,
        inputs.metrics,
        owner_state,
        inputs.view_source,
        inputs.queue_source,
        trace,
        debug,
        info,
        warn,
        apply,
    )
}

pub fn run_queue_accept_top_with_runtime_source_with_log_sinks<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    Source,
    ApplyFn,
    TraceFn,
    DebugFn,
    InfoFn,
    WarnFn,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    owner_state: &mut QueueAcceptOwnerState,
    runtime: &Source,
    trace: TraceFn,
    debug: DebugFn,
    info: InfoFn,
    warn: WarnFn,
    apply: ApplyFn,
) -> QueueAcceptWithMetricsResult<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    Source: QueueAcceptRuntimeSource,
    ApplyFn: FnMut(&mut MaybeTx<Tx, Account, Journal, ParentBatchId>) -> ApplyResult,
    TraceFn: FnMut(&str),
    DebugFn: FnMut(&str),
    InfoFn: FnMut(&str),
    WarnFn: FnMut(&str),
{
    run_queue_accept_top_with_log_sinks(
        views,
        owner_state,
        build_queue_accept_top_inputs_from_runtime_source(runtime),
        trace,
        debug,
        info,
        warn,
        apply,
    )
}

pub fn run_queue_accept_top_with_caller_prepared_apply_and_log_sinks<
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
    owner_state: &mut QueueAcceptOwnerState,
    inputs: QueueAcceptTopInputs<'_, ViewSource, QueueSource>,
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
        PreparedQueueAcceptCall<Account>,
    ) -> QueueAcceptPreparedCallStep<Account>,
    TraceFn: FnMut(&str),
    DebugFn: FnMut(&str),
    InfoFn: FnMut(&str),
    WarnFn: FnMut(&str),
{
    run_queue_accept_from_sources_with_caller_prepared_apply_and_log_sinks(
        views,
        inputs.metrics,
        owner_state,
        inputs.view_source,
        inputs.queue_source,
        trace,
        debug,
        info,
        warn,
        run_prepared_apply,
    )
}

pub fn run_queue_accept_top_with_runtime_source_with_caller_prepared_apply_and_log_sinks<
    Account,
    Tx,
    Journal,
    ParentBatchId,
    Source,
    RunPreparedApply,
    TraceFn,
    DebugFn,
    InfoFn,
    WarnFn,
>(
    views: &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
    owner_state: &mut QueueAcceptOwnerState,
    runtime: &Source,
    trace: TraceFn,
    debug: DebugFn,
    info: InfoFn,
    warn: WarnFn,
    run_prepared_apply: RunPreparedApply,
) -> QueueAcceptWithMetricsResult<Account>
where
    Account: Clone + Display + Ord + PartialEq,
    Source: QueueAcceptRuntimeSource,
    RunPreparedApply: FnMut(
        &mut QueueViews<Account, MaybeTx<Tx, Account, Journal, ParentBatchId>>,
        &mut QueueAcceptOwnerState,
        PreparedQueueAcceptCall<Account>,
    ) -> QueueAcceptPreparedCallStep<Account>,
    TraceFn: FnMut(&str),
    DebugFn: FnMut(&str),
    InfoFn: FnMut(&str),
    WarnFn: FnMut(&str),
{
    run_queue_accept_top_with_caller_prepared_apply_and_log_sinks(
        views,
        owner_state,
        build_queue_accept_top_inputs_from_runtime_source(runtime),
        trace,
        debug,
        info,
        warn,
        run_prepared_apply,
    )
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    use basics::base_uint::Uint256;
    use protocol::{Rules, SeqProxy, Ter};

    use super::{
        QueueAcceptTopInputs, prepare_queue_accept_top, run_queue_accept_top,
        run_queue_accept_top_with_caller_prepared_apply_and_log_sinks,
    };
    use crate::{
        ApplyFlags, ApplyResult, MaybeTx, MaybeTxCore, PreflightResult, QueueAcceptOwnerState,
        QueueViews, TxConsequences, TxQAccount, prepare_queue_accept_from_sources,
        run_prepared_queue_accept_call, run_queue_accept_from_sources,
        run_queue_accept_from_sources_and_log_sinks,
    };

    #[derive(Debug, Clone, Copy)]
    struct TestViewSource {
        open_ledger_tx_count: usize,
        parent_hash: Uint256,
    }

    impl crate::QueueAcceptObservedViewSource for TestViewSource {
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

    impl crate::QueueAcceptObservedQueueSource for TestQueueSource {
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

    fn metrics_state() -> crate::QueueFeeMetricsState {
        crate::QueueFeeMetricsState::new(crate::QueueFeeMetricsConfig {
            ledgers_in_queue: 3,
            queue_size_min: 20,
            minimum_escalation_multiplier: crate::TXQ_BASE_LEVEL * 500,
            minimum_txn_in_ledger: 32,
            target_txn_in_ledger: 256,
            maximum_txn_in_ledger: Some(400),
            normal_consensus_increase_percent: 20,
            slow_consensus_decrease_percent: 50,
        })
    }

    #[test]
    fn accept_top_wrapper_reuses_source_wrapper_behavior() {
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
                    crate::QueueAdvanceCandidate {
                        fee_level: 300,
                        tx_id: Uint256::from_u64(5),
                        seq_proxy: SeqProxy::sequence(5),
                    },
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

        let mut top_views = build_views();
        let mut top_owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(4));
        let top_result = run_queue_accept_top(
            &mut top_views,
            &mut top_owner_state,
            QueueAcceptTopInputs::new(&metrics, &view_source, &queue_source),
            |_queued| ApplyResult::new(Ter::TES_SUCCESS, true, false),
        );

        let mut source_views = build_views();
        let mut source_owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(4));
        let source_result = run_queue_accept_from_sources(
            &mut source_views,
            &metrics,
            &mut source_owner_state,
            &view_source,
            &queue_source,
            |_queued| ApplyResult::new(Ter::TES_SUCCESS, true, false),
        );

        assert_eq!(top_result, source_result);
        assert_eq!(top_views, source_views);
        assert_eq!(top_owner_state, source_owner_state);
    }

    #[test]
    fn prepare_accept_top_reuses_prepared_source_wrapper() {
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
                    crate::QueueAdvanceCandidate {
                        fee_level: 300,
                        tx_id: Uint256::from_u64(5),
                        seq_proxy: SeqProxy::sequence(5),
                    },
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

        let mut top_views = build_views();
        let mut top_owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(4));
        let top_prepared = prepare_queue_accept_top(
            &mut top_views,
            &mut top_owner_state,
            QueueAcceptTopInputs::new(&metrics, &view_source, &queue_source),
        );

        let mut source_views = build_views();
        let mut source_owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(4));
        let source_prepared = prepare_queue_accept_from_sources(
            &mut source_views,
            &metrics,
            &mut source_owner_state,
            &view_source,
            &queue_source,
        );

        assert_eq!(top_prepared, source_prepared);
        assert_eq!(top_views, source_views);
        assert_eq!(top_owner_state, source_owner_state);
    }

    #[test]
    fn accept_top_caller_prepared_sink_wrapper_matches_source_sink_wrapper() {
        let build_views = || {
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
            QueueViews::new(
                BTreeMap::from([("acct", account)]),
                vec![
                    crate::FeeQueueEntry::new(
                        crate::FeeQueueKey::new("acct", SeqProxy::sequence(5)),
                        crate::QueueAdvanceCandidate {
                            fee_level: 300,
                            tx_id: Uint256::from_u64(5),
                            seq_proxy: SeqProxy::sequence(5),
                        },
                    ),
                    crate::FeeQueueEntry::new(
                        crate::FeeQueueKey::new("acct", SeqProxy::sequence(9)),
                        crate::QueueAdvanceCandidate {
                            fee_level: 60,
                            tx_id: Uint256::from_u64(9),
                            seq_proxy: SeqProxy::sequence(9),
                        },
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

        let mut top_views = build_views();
        let mut top_owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(9));
        let top_emitted = RefCell::new(Vec::new());
        let top_result = run_queue_accept_top_with_caller_prepared_apply_and_log_sinks(
            &mut top_views,
            &mut top_owner_state,
            QueueAcceptTopInputs::new(&metrics, &view_source, &queue_source),
            |message| top_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| top_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| top_emitted.borrow_mut().push(format!("info:{message}")),
            |message| top_emitted.borrow_mut().push(format!("warn:{message}")),
            |views, owner_state, prepared| {
                run_prepared_queue_accept_call(views, owner_state, prepared, |_queued| {
                    ApplyResult::new(Ter::TER_RETRY, false, false)
                })
            },
        );

        let mut source_views = build_views();
        let mut source_owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(9));
        let source_emitted = RefCell::new(Vec::new());
        let source_result = run_queue_accept_from_sources_and_log_sinks(
            &mut source_views,
            &metrics,
            &mut source_owner_state,
            &view_source,
            &queue_source,
            |message| source_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| source_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| source_emitted.borrow_mut().push(format!("info:{message}")),
            |message| source_emitted.borrow_mut().push(format!("warn:{message}")),
            |_queued| ApplyResult::new(Ter::TER_RETRY, false, false),
        );

        assert_eq!(top_result, source_result);
        assert_eq!(top_views, source_views);
        assert_eq!(top_owner_state, source_owner_state);
        assert_eq!(top_emitted.into_inner(), source_emitted.into_inner());
    }
}
