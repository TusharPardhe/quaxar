use std::cell::RefCell;
use std::collections::BTreeMap;

use basics::base_uint::Uint256;
use protocol::{Rules, SeqProxy, Ter};
use tx::{
    ApplyFlags, ApplyResult, FeeQueueEntry, FeeQueueKey, MaybeTx, MaybeTxCore, PreflightResult,
    QueueAcceptObservedQueueSource, QueueAcceptObservedViewSource, QueueAcceptOwnerState,
    QueueAcceptRuntimeSource, QueueAdvanceCandidate, QueueFeeMetricsState, QueueViews,
    TxConsequences, TxQAccount, TxQSetup, prepare_queue_accept_entry,
    prepare_queue_accept_top_with_runtime_source, run_prepared_queue_accept_call,
    run_queue_accept_entry, run_queue_accept_entry_with_caller_prepared_apply_and_log_sinks,
    run_queue_accept_top_with_runtime_source,
    run_queue_accept_top_with_runtime_source_with_caller_prepared_apply_and_log_sinks,
};

#[derive(Debug)]
struct TestRuntimeSource {
    metrics: QueueFeeMetricsState,
    open_ledger_tx_count: usize,
    parent_hash: Uint256,
    current_max_size: Option<usize>,
}

impl QueueAcceptObservedViewSource for TestRuntimeSource {
    fn open_ledger_tx_count(&self) -> usize {
        self.open_ledger_tx_count
    }

    fn parent_hash(&self) -> Uint256 {
        self.parent_hash
    }
}

impl QueueAcceptObservedQueueSource for TestRuntimeSource {
    fn current_max_size(&self) -> Option<usize> {
        self.current_max_size
    }
}

impl QueueAcceptRuntimeSource for TestRuntimeSource {
    fn metrics(&self) -> &QueueFeeMetricsState {
        &self.metrics
    }
}

fn test_runtime_source(
    open_ledger_tx_count: usize,
    parent_hash: Uint256,
    current_max_size: Option<usize>,
) -> TestRuntimeSource {
    TestRuntimeSource {
        metrics: test_setup().fee_metrics_state(),
        open_ledger_tx_count,
        parent_hash,
        current_max_size,
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
fn queue_accept_entry_matches_top_wrapper_and_public_bool() {
    let runtime = test_runtime_source(0, Uint256::from_u64(6), Some(10));

    let mut top_views = build_views();
    let mut top_owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(4));
    let top_result = run_queue_accept_top_with_runtime_source(
        &mut top_views,
        &mut top_owner_state,
        &runtime,
        |_queued| ApplyResult::new(Ter::TES_SUCCESS, true, false),
    );

    let mut entry_views = build_views();
    let mut entry_owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(4));
    let entry_result = run_queue_accept_entry(
        &mut entry_views,
        &mut entry_owner_state,
        &runtime,
        |_queued| ApplyResult::new(Ter::TES_SUCCESS, true, false),
    );

    assert_eq!(entry_result.accept, top_result);
    assert_eq!(entry_result.ledger_changed, top_result.ledger_changed());
    assert!(entry_result.ledger_changed);
    assert_eq!(entry_views, top_views);
    assert_eq!(entry_owner_state, top_owner_state);
}

#[test]
fn queue_accept_entry_prepare_matches_top_prepare_boundary() {
    let runtime = test_runtime_source(32, Uint256::from_u64(9), Some(2));

    let mut top_views = build_views();
    let mut top_owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(9));
    let top_prepared = prepare_queue_accept_top_with_runtime_source(
        &mut top_views,
        &mut top_owner_state,
        &runtime,
    );

    let mut entry_views = build_views();
    let mut entry_owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(9));
    let entry_prepared =
        prepare_queue_accept_entry(&mut entry_views, &mut entry_owner_state, &runtime);

    assert_eq!(entry_prepared, top_prepared);
    assert_eq!(entry_views, top_views);
    assert_eq!(entry_owner_state, top_owner_state);
}

#[test]
fn queue_accept_entry_caller_prepared_sink_wrapper_matches_top_wrapper() {
    let runtime = test_runtime_source(32, Uint256::from_u64(9), Some(2));

    let mut top_views = build_views();
    let mut top_owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(9));
    let top_emitted = RefCell::new(Vec::new());
    let top_result =
        run_queue_accept_top_with_runtime_source_with_caller_prepared_apply_and_log_sinks(
            &mut top_views,
            &mut top_owner_state,
            &runtime,
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

    let mut entry_views = build_views();
    let mut entry_owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(9));
    let entry_emitted = RefCell::new(Vec::new());
    let entry_result = run_queue_accept_entry_with_caller_prepared_apply_and_log_sinks(
        &mut entry_views,
        &mut entry_owner_state,
        &runtime,
        |message| entry_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| entry_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| entry_emitted.borrow_mut().push(format!("info:{message}")),
        |message| entry_emitted.borrow_mut().push(format!("warn:{message}")),
        |views, owner_state, prepared| {
            run_prepared_queue_accept_call(views, owner_state, prepared, |_queued| {
                ApplyResult::new(Ter::TER_RETRY, false, false)
            })
        },
    );

    assert_eq!(entry_result.accept, top_result);
    assert_eq!(entry_result.ledger_changed, top_result.ledger_changed());
    assert_eq!(entry_views, top_views);
    assert_eq!(entry_owner_state, top_owner_state);
    assert_eq!(entry_emitted.into_inner(), top_emitted.into_inner());
}
