use std::cell::RefCell;
use std::collections::BTreeMap;

use basics::base_uint::Uint256;
use protocol::{Rules, SeqProxy, Ter};
use tx::{
    ApplyFlags, ApplyResult, FeeQueueEntry, FeeQueueKey, MaybeTx, MaybeTxCore, PreflightResult,
    QueueAcceptAppRuntime, QueueAcceptAppSource, QueueAcceptLedgerViewSource,
    QueueAcceptLiveApplyRuntime, QueueAcceptLiveOwner, QueueAcceptOwnerState,
    QueueAdvanceCandidate, QueueFeeMetricsState, QueueViews, TxConsequences, TxQAccount, TxQSetup,
    prepare_queue_accept_with_app_view, prepare_queue_accept_with_live_owner,
    run_queue_accept_with_app_view, run_queue_accept_with_app_view_and_log_sinks,
    run_queue_accept_with_live_owner, run_queue_accept_with_live_owner_and_log_sinks,
};

#[derive(Debug)]
struct TestAcceptApp {
    metrics: QueueFeeMetricsState,
    current_max_size: Option<usize>,
    apply_result: ApplyResult,
    apply_calls: usize,
}

#[derive(Debug)]
struct TestAcceptApplyOnlyApp {
    apply_result: ApplyResult,
    apply_calls: usize,
}

#[derive(Debug, Clone, Copy)]
struct TestLedgerView {
    open_ledger_tx_count: usize,
    parent_hash: Uint256,
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

impl QueueAcceptLiveApplyRuntime<&'static str, &'static str, &'static str, &'static str>
    for TestAcceptApplyOnlyApp
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

fn test_metrics() -> QueueFeeMetricsState {
    test_setup().fee_metrics_state()
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

fn build_live_owner(
    current_max_size: Option<usize>,
    parent_hash_comp: Uint256,
) -> QueueAcceptLiveOwner<&'static str, &'static str, &'static str, &'static str> {
    QueueAcceptLiveOwner::new_from_setup(
        &test_setup(),
        current_max_size,
        QueueAcceptOwnerState::new(parent_hash_comp),
        build_views(),
    )
}

#[test]
fn queue_accept_live_owner_wrapper_matches_app_view_wrapper() {
    let mut app_view_app = TestAcceptApp {
        metrics: test_metrics(),
        current_max_size: Some(10),
        apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, false),
        apply_calls: 0,
    };
    let mut live_owner_app = TestAcceptApplyOnlyApp {
        apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, false),
        apply_calls: 0,
    };
    let view = TestLedgerView {
        open_ledger_tx_count: 0,
        parent_hash: Uint256::from_u64(6),
    };

    let mut app_view_views = build_views();
    let mut app_view_owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(4));
    let app_view_result = run_queue_accept_with_app_view(
        &mut app_view_views,
        &mut app_view_owner_state,
        &mut app_view_app,
        &view,
    );

    let mut live_owner = build_live_owner(Some(10), Uint256::from_u64(4));
    let live_owner_result =
        run_queue_accept_with_live_owner(&mut live_owner, &mut live_owner_app, &view);

    assert_eq!(live_owner_result, app_view_result);
    assert_eq!(live_owner.views(), &app_view_views);
    assert_eq!(live_owner.owner_state(), app_view_owner_state);
    assert_eq!(app_view_app.apply_calls, 1);
    assert_eq!(live_owner_app.apply_calls, 1);
}

#[test]
fn queue_accept_live_owner_prepare_matches_app_view_prepare_boundary() {
    let app_view_app = TestAcceptApp {
        metrics: test_metrics(),
        current_max_size: Some(2),
        apply_result: ApplyResult::new(Ter::TER_RETRY, false, false),
        apply_calls: 0,
    };
    let view = TestLedgerView {
        open_ledger_tx_count: 32,
        parent_hash: Uint256::from_u64(9),
    };

    let mut app_view_views = build_views();
    let mut app_view_owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(9));
    let app_view_prepared = prepare_queue_accept_with_app_view(
        &mut app_view_views,
        &mut app_view_owner_state,
        &app_view_app,
        &view,
    );

    let mut live_owner = build_live_owner(Some(2), Uint256::from_u64(9));
    let live_owner_prepared = prepare_queue_accept_with_live_owner(&mut live_owner, &view);

    assert_eq!(live_owner_prepared, app_view_prepared);
    assert_eq!(live_owner.views(), &app_view_views);
    assert_eq!(live_owner.owner_state(), app_view_owner_state);
}

#[test]
fn queue_accept_live_owner_sink_wrapper_matches_app_view_sink_wrapper() {
    let mut app_view_app = TestAcceptApp {
        metrics: test_metrics(),
        current_max_size: Some(2),
        apply_result: ApplyResult::new(Ter::TER_RETRY, false, false),
        apply_calls: 0,
    };
    let mut live_owner_app = TestAcceptApplyOnlyApp {
        apply_result: ApplyResult::new(Ter::TER_RETRY, false, false),
        apply_calls: 0,
    };
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
        &mut app_view_app,
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

    let mut live_owner = build_live_owner(Some(2), Uint256::from_u64(9));
    let live_owner_emitted = RefCell::new(Vec::new());
    let live_owner_result = run_queue_accept_with_live_owner_and_log_sinks(
        &mut live_owner,
        &mut live_owner_app,
        &view,
        |message| {
            live_owner_emitted
                .borrow_mut()
                .push(format!("trace:{message}"))
        },
        |message| {
            live_owner_emitted
                .borrow_mut()
                .push(format!("debug:{message}"))
        },
        |message| {
            live_owner_emitted
                .borrow_mut()
                .push(format!("info:{message}"))
        },
        |message| {
            live_owner_emitted
                .borrow_mut()
                .push(format!("warn:{message}"))
        },
    );

    assert_eq!(live_owner_result, app_view_result);
    assert_eq!(live_owner.views(), &app_view_views);
    assert_eq!(live_owner.owner_state(), app_view_owner_state);
    assert_eq!(
        live_owner_emitted.into_inner(),
        app_view_emitted.into_inner()
    );
    assert_eq!(app_view_app.apply_calls, 1);
    assert_eq!(live_owner_app.apply_calls, 1);
}
