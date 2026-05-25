use std::cell::RefCell;
use std::collections::BTreeMap;

use basics::base_uint::Uint256;
use protocol::{Rules, SeqProxy, Ter};
use tx::{
    ApplyFlags, ApplyResult, FeeQueueEntry, FeeQueueKey, MaybeTx, MaybeTxCore, PreflightResult,
    QueueAcceptAppRuntime, QueueAcceptAppSource, QueueAcceptApplyRuntime,
    QueueAcceptLedgerViewSource, QueueAcceptObservedQueueSource, QueueAcceptObservedViewSource,
    QueueAcceptOwnerState, QueueAcceptRuntimeSource, QueueAdvanceCandidate, QueueFeeMetricsState,
    QueueViews, TxConsequences, TxQAccount, TxQSetup, prepare_queue_accept_with_app_view,
    run_queue_accept_with_app_view, run_queue_accept_with_app_view_and_log_sinks,
    run_queue_accept_with_runtime, run_queue_accept_with_runtime_and_log_sinks,
    snapshot_queue_accept_app_view,
};

#[derive(Debug)]
struct TestAcceptApp {
    metrics: QueueFeeMetricsState,
    current_max_size: Option<usize>,
    apply_result: ApplyResult,
    apply_calls: usize,
}

#[derive(Debug, Clone, Copy)]
struct TestLedgerView {
    open_ledger_tx_count: usize,
    parent_hash: Uint256,
}

#[derive(Debug)]
struct TestAcceptRuntime {
    metrics: QueueFeeMetricsState,
    open_ledger_tx_count: usize,
    parent_hash: Uint256,
    current_max_size: Option<usize>,
    apply_result: ApplyResult,
    apply_calls: usize,
}

impl QueueAcceptAppSource for TestAcceptApp {
    fn metrics(&self) -> &QueueFeeMetricsState {
        &self.metrics
    }

    fn current_max_size(&self) -> Option<usize> {
        self.current_max_size
    }
}

impl QueueAcceptAppRuntime<&'static str, &'static str, &'static str, &'static str>
    for TestAcceptApp
{
    fn apply_queued(
        &mut self,
        _queued: &mut MaybeTx<&'static str, &'static str, &'static str, &'static str>,
    ) -> ApplyResult {
        self.apply_calls += 1;
        self.apply_result.clone()
    }
}

impl QueueAcceptLedgerViewSource for TestLedgerView {
    fn open_ledger_tx_count(&self) -> usize {
        self.open_ledger_tx_count
    }

    fn parent_hash(&self) -> Uint256 {
        self.parent_hash
    }
}

impl QueueAcceptObservedViewSource for TestAcceptRuntime {
    fn open_ledger_tx_count(&self) -> usize {
        self.open_ledger_tx_count
    }

    fn parent_hash(&self) -> Uint256 {
        self.parent_hash
    }
}

impl QueueAcceptObservedQueueSource for TestAcceptRuntime {
    fn current_max_size(&self) -> Option<usize> {
        self.current_max_size
    }
}

impl QueueAcceptRuntimeSource for TestAcceptRuntime {
    fn metrics(&self) -> &QueueFeeMetricsState {
        &self.metrics
    }
}

impl QueueAcceptApplyRuntime<&'static str, &'static str, &'static str, &'static str>
    for TestAcceptRuntime
{
    fn apply_queued(
        &mut self,
        _queued: &mut MaybeTx<&'static str, &'static str, &'static str, &'static str>,
    ) -> ApplyResult {
        self.apply_calls += 1;
        self.apply_result.clone()
    }
}

fn test_app(current_max_size: Option<usize>, apply_result: ApplyResult) -> TestAcceptApp {
    TestAcceptApp {
        metrics: test_setup().fee_metrics_state(),
        current_max_size,
        apply_result,
        apply_calls: 0,
    }
}

fn test_setup() -> TxQSetup {
    TxQSetup {
        ledgers_in_queue: 3,
        queue_size_min: 20,
        maximum_txn_in_ledger: Some(400),
        ..TxQSetup::default()
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

fn build_views()
-> QueueViews<&'static str, MaybeTx<&'static str, &'static str, &'static str, &'static str>> {
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
            FeeQueueEntry::new(
                FeeQueueKey::new("acct", SeqProxy::sequence(5)),
                QueueAdvanceCandidate {
                    fee_level: 300,
                    tx_id: Uint256::from_u64(5),
                    seq_proxy: SeqProxy::sequence(5),
                },
            ),
            FeeQueueEntry::new(
                FeeQueueKey::new("acct", SeqProxy::sequence(9)),
                QueueAdvanceCandidate {
                    fee_level: 60,
                    tx_id: Uint256::from_u64(9),
                    seq_proxy: SeqProxy::sequence(9),
                },
            ),
        ],
    )
}

#[test]
fn queue_accept_app_view_wrapper_matches_runtime_wrapper() {
    let mut app = test_app(Some(10), ApplyResult::new(Ter::TES_SUCCESS, true, false));
    let view = TestLedgerView {
        open_ledger_tx_count: 0,
        parent_hash: Uint256::from_u64(6),
    };

    let mut app_view_views = build_views();
    let mut app_view_owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(4));
    let app_view_result = run_queue_accept_with_app_view(
        &mut app_view_views,
        &mut app_view_owner_state,
        &mut app,
        &view,
    );

    let snapshot = snapshot_queue_accept_app_view(&app, &view);
    let mut runtime = TestAcceptRuntime {
        metrics: snapshot.metrics.clone(),
        open_ledger_tx_count: snapshot.open_ledger_tx_count,
        parent_hash: snapshot.parent_hash,
        current_max_size: snapshot.current_max_size,
        apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, false),
        apply_calls: 0,
    };
    let mut runtime_views = build_views();
    let mut runtime_owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(4));
    let runtime_result =
        run_queue_accept_with_runtime(&mut runtime_views, &mut runtime_owner_state, &mut runtime);

    assert_eq!(app_view_result, runtime_result);
    assert_eq!(app_view_views, runtime_views);
    assert_eq!(app_view_owner_state, runtime_owner_state);
    assert_eq!(app.apply_calls, 1);
    assert_eq!(runtime.apply_calls, 1);
}

#[test]
fn queue_accept_app_view_prepare_matches_runtime_prepare_boundary() {
    let app = test_app(Some(2), ApplyResult::new(Ter::TER_RETRY, false, false));
    let view = TestLedgerView {
        open_ledger_tx_count: 32,
        parent_hash: Uint256::from_u64(9),
    };

    let mut app_view_views = build_views();
    let mut app_view_owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(9));
    let app_view_prepared = prepare_queue_accept_with_app_view(
        &mut app_view_views,
        &mut app_view_owner_state,
        &app,
        &view,
    );

    let snapshot = snapshot_queue_accept_app_view(&app, &view);
    let mut runtime_views = build_views();
    let mut runtime_owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(9));
    let runtime_prepared = tx::prepare_queue_accept_with_runtime(
        &mut runtime_views,
        &mut runtime_owner_state,
        &TestAcceptRuntime {
            metrics: snapshot.metrics.clone(),
            open_ledger_tx_count: snapshot.open_ledger_tx_count,
            parent_hash: snapshot.parent_hash,
            current_max_size: snapshot.current_max_size,
            apply_result: ApplyResult::new(Ter::TER_RETRY, false, false),
            apply_calls: 0,
        },
    );

    assert_eq!(app_view_prepared, runtime_prepared);
    assert_eq!(app_view_views, runtime_views);
    assert_eq!(app_view_owner_state, runtime_owner_state);
}

#[test]
fn queue_accept_app_view_sink_wrapper_matches_runtime_sink_wrapper() {
    let mut app = test_app(Some(2), ApplyResult::new(Ter::TER_RETRY, false, false));
    let view = TestLedgerView {
        open_ledger_tx_count: 32,
        parent_hash: Uint256::from_u64(9),
    };

    let mut app_view_views = build_views();
    let mut app_view_owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(9));
    let app_view_emitted = RefCell::new(Vec::new());
    let app_view_result = run_queue_accept_with_app_view_and_log_sinks(
        &mut app_view_views,
        &mut app_view_owner_state,
        &mut app,
        &view,
        |message| {
            app_view_emitted
                .borrow_mut()
                .push(format!("trace:{message}"))
        },
        |message| {
            app_view_emitted
                .borrow_mut()
                .push(format!("debug:{message}"))
        },
        |message| {
            app_view_emitted
                .borrow_mut()
                .push(format!("info:{message}"))
        },
        |message| {
            app_view_emitted
                .borrow_mut()
                .push(format!("warn:{message}"))
        },
    );

    let snapshot = snapshot_queue_accept_app_view(&app, &view);
    let mut runtime = TestAcceptRuntime {
        metrics: snapshot.metrics.clone(),
        open_ledger_tx_count: snapshot.open_ledger_tx_count,
        parent_hash: snapshot.parent_hash,
        current_max_size: snapshot.current_max_size,
        apply_result: ApplyResult::new(Ter::TER_RETRY, false, false),
        apply_calls: 0,
    };
    let mut runtime_views = build_views();
    let mut runtime_owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(9));
    let runtime_emitted = RefCell::new(Vec::new());
    let runtime_result = run_queue_accept_with_runtime_and_log_sinks(
        &mut runtime_views,
        &mut runtime_owner_state,
        &mut runtime,
        |message| {
            runtime_emitted
                .borrow_mut()
                .push(format!("trace:{message}"))
        },
        |message| {
            runtime_emitted
                .borrow_mut()
                .push(format!("debug:{message}"))
        },
        |message| runtime_emitted.borrow_mut().push(format!("info:{message}")),
        |message| runtime_emitted.borrow_mut().push(format!("warn:{message}")),
    );

    assert_eq!(app_view_result, runtime_result);
    assert_eq!(app_view_views, runtime_views);
    assert_eq!(app_view_owner_state, runtime_owner_state);
    assert_eq!(app_view_emitted.into_inner(), runtime_emitted.into_inner());
    assert_eq!(app.apply_calls, 1);
    assert_eq!(runtime.apply_calls, 1);
}
