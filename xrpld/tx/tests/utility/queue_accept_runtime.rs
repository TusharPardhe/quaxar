use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

use basics::base_uint::Uint256;
use protocol::{Rules, SeqProxy, Ter, feature_single_asset_vault};
use tx::{
    ApplyFlags, ApplyResult, FeeQueueEntry, FeeQueueKey, MAYBE_TX_RETRIES_ALLOWED, MaybeTx,
    MaybeTxCore, PreclaimResult, PreflightResult, QueueAcceptApplyRuntime, QueueAcceptOwnerState,
    QueueAcceptRuntimeEnvelope, QueueAdvanceCandidate, QueueFeeMetricsState, QueueViews,
    TxConsequences, TxQAccount, TxQSetup, prepare_queue_accept_top_with_runtime_source,
    prepare_queue_accept_with_runtime, run_queue_accept_top_with_runtime_source,
    run_queue_accept_top_with_runtime_source_with_log_sinks, run_queue_accept_with_runtime,
    run_queue_accept_with_runtime_and_log_sinks, snapshot_queue_accept_runtime,
};

#[derive(Debug)]
struct TestAcceptRuntime {
    metrics: QueueFeeMetricsState,
    open_ledger_tx_count: usize,
    parent_hash: Uint256,
    current_max_size: Option<usize>,
    apply_result: ApplyResult,
    apply_calls: usize,
}

#[derive(Debug)]
struct SnapshotCountingAcceptRuntime {
    metrics: QueueFeeMetricsState,
    metrics_calls: Cell<usize>,
    open_ledger_tx_count: usize,
    parent_hash: Uint256,
    current_max_size: Option<usize>,
    apply_result: ApplyResult,
    apply_calls: usize,
}

impl tx::QueueAcceptObservedViewSource for TestAcceptRuntime {
    fn open_ledger_tx_count(&self) -> usize {
        self.open_ledger_tx_count
    }

    fn parent_hash(&self) -> Uint256 {
        self.parent_hash
    }
}

impl tx::QueueAcceptObservedQueueSource for TestAcceptRuntime {
    fn current_max_size(&self) -> Option<usize> {
        self.current_max_size
    }
}

impl tx::QueueAcceptRuntimeSource for TestAcceptRuntime {
    fn metrics(&self) -> &QueueFeeMetricsState {
        &self.metrics
    }
}

impl tx::QueueAcceptObservedViewSource for SnapshotCountingAcceptRuntime {
    fn open_ledger_tx_count(&self) -> usize {
        self.open_ledger_tx_count
    }

    fn parent_hash(&self) -> Uint256 {
        self.parent_hash
    }
}

impl tx::QueueAcceptObservedQueueSource for SnapshotCountingAcceptRuntime {
    fn current_max_size(&self) -> Option<usize> {
        self.current_max_size
    }
}

impl tx::QueueAcceptRuntimeSource for SnapshotCountingAcceptRuntime {
    fn metrics(&self) -> &QueueFeeMetricsState {
        self.metrics_calls.set(self.metrics_calls.get() + 1);
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

impl QueueAcceptApplyRuntime<&'static str, &'static str, &'static str, &'static str>
    for SnapshotCountingAcceptRuntime
{
    fn apply_queued(
        &mut self,
        _queued: &mut MaybeTx<&'static str, &'static str, &'static str, &'static str>,
    ) -> ApplyResult {
        self.apply_calls += 1;
        self.apply_result.clone()
    }
}

fn test_runtime(
    open_ledger_tx_count: usize,
    parent_hash: Uint256,
    current_max_size: Option<usize>,
    apply_result: ApplyResult,
) -> TestAcceptRuntime {
    TestAcceptRuntime {
        metrics: test_setup().fee_metrics_state(),
        open_ledger_tx_count,
        parent_hash,
        current_max_size,
        apply_result,
        apply_calls: 0,
    }
}

fn snapshot_counting_runtime(apply_result: ApplyResult) -> SnapshotCountingAcceptRuntime {
    SnapshotCountingAcceptRuntime {
        metrics: test_setup().fee_metrics_state(),
        metrics_calls: Cell::new(0),
        open_ledger_tx_count: 4,
        parent_hash: Uint256::from_u64(9),
        current_max_size: Some(2),
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

fn ledger_rules(seed: u8) -> Rules {
    Rules::from_ledger(
        [feature_single_asset_vault()],
        Uint256::from_array([seed; 32]),
        std::iter::empty(),
    )
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
fn queue_accept_runtime_wrapper_matches_top_runtime_source_wrapper_and_uses_runtime_apply() {
    let mut runtime = test_runtime(
        0,
        Uint256::from_u64(6),
        Some(10),
        ApplyResult::new(Ter::TES_SUCCESS, true, false),
    );

    let mut runtime_views = build_views();
    let mut runtime_owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(4));
    let runtime_result =
        run_queue_accept_with_runtime(&mut runtime_views, &mut runtime_owner_state, &mut runtime);

    let snapshot = snapshot_queue_accept_runtime(&runtime);
    let mut top_views = build_views();
    let mut top_owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(4));
    let top_accept = run_queue_accept_top_with_runtime_source(
        &mut top_views,
        &mut top_owner_state,
        &snapshot,
        |_queued| ApplyResult::new(Ter::TES_SUCCESS, true, false),
    );
    let top_result = tx::QueueAcceptEntryResult {
        ledger_changed: top_accept.ledger_changed(),
        accept: top_accept,
    };

    assert_eq!(runtime_result, top_result);
    assert_eq!(runtime_views, top_views);
    assert_eq!(runtime_owner_state, top_owner_state);
    assert_eq!(runtime.apply_calls, 1);
}

#[test]
fn queue_accept_runtime_prepare_matches_top_runtime_source_prepare_boundary() {
    let runtime = test_runtime(
        32,
        Uint256::from_u64(9),
        Some(2),
        ApplyResult::new(Ter::TER_RETRY, false, false),
    );

    let mut runtime_views = build_views();
    let mut runtime_owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(9));
    let runtime_prepared =
        prepare_queue_accept_with_runtime(&mut runtime_views, &mut runtime_owner_state, &runtime);

    let snapshot = snapshot_queue_accept_runtime(&runtime);
    let mut top_views = build_views();
    let mut top_owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(9));
    let top_prepared = prepare_queue_accept_top_with_runtime_source(
        &mut top_views,
        &mut top_owner_state,
        &snapshot,
    );

    assert_eq!(runtime_prepared, top_prepared);
    assert_eq!(runtime_views, top_views);
    assert_eq!(runtime_owner_state, top_owner_state);
}

#[test]
fn queue_accept_runtime_sink_wrapper_matches_top_runtime_source_sink_wrapper() {
    let mut runtime = test_runtime(
        32,
        Uint256::from_u64(9),
        Some(2),
        ApplyResult::new(Ter::TER_RETRY, false, false),
    );

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

    let snapshot = snapshot_queue_accept_runtime(&runtime);
    let mut top_views = build_views();
    let mut top_owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(9));
    let top_emitted = RefCell::new(Vec::new());
    let top_accept = run_queue_accept_top_with_runtime_source_with_log_sinks(
        &mut top_views,
        &mut top_owner_state,
        &snapshot,
        |message| top_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| top_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| top_emitted.borrow_mut().push(format!("info:{message}")),
        |message| top_emitted.borrow_mut().push(format!("warn:{message}")),
        |_queued| ApplyResult::new(Ter::TER_RETRY, false, false),
    );
    let top_result = tx::QueueAcceptEntryResult {
        ledger_changed: top_accept.ledger_changed(),
        accept: top_accept,
    };

    assert_eq!(runtime_result, top_result);
    assert_eq!(runtime_views, top_views);
    assert_eq!(runtime_owner_state, top_owner_state);
    assert_eq!(runtime_emitted.into_inner(), top_emitted.into_inner());
    assert_eq!(runtime.apply_calls, 1);
}

#[test]
fn queue_accept_runtime_envelope_prepare_matches_top_runtime_source_prepare_and_samples_metrics_once()
 {
    let mut runtime = snapshot_counting_runtime(ApplyResult::new(Ter::TER_RETRY, false, false));
    let expected_snapshot = tx::QueueAcceptRuntimeSnapshot::new(
        runtime.metrics.clone(),
        runtime.open_ledger_tx_count,
        runtime.parent_hash,
        runtime.current_max_size,
    );

    let mut runtime_views = build_views();
    let mut runtime_owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(9));
    let runtime_prepared = {
        let envelope = QueueAcceptRuntimeEnvelope::new(&mut runtime);
        envelope.prepare_accept(&mut runtime_views, &mut runtime_owner_state)
    };

    let mut top_views = build_views();
    let mut top_owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(9));
    let top_prepared = prepare_queue_accept_top_with_runtime_source(
        &mut top_views,
        &mut top_owner_state,
        &expected_snapshot,
    );

    assert_eq!(runtime_prepared, top_prepared);
    assert_eq!(runtime_views, top_views);
    assert_eq!(runtime_owner_state, top_owner_state);
    assert_eq!(runtime.metrics_calls.get(), 1);
    assert_eq!(runtime.apply_calls, 0);
}

#[test]
fn queue_accept_runtime_envelope_accept_matches_top_runtime_source_wrapper_and_samples_metrics_once()
 {
    let mut runtime = snapshot_counting_runtime(ApplyResult::new(Ter::TES_SUCCESS, true, false));
    let expected_snapshot = tx::QueueAcceptRuntimeSnapshot::new(
        runtime.metrics.clone(),
        runtime.open_ledger_tx_count,
        runtime.parent_hash,
        runtime.current_max_size,
    );

    let mut runtime_views = build_views();
    let mut runtime_owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(9));
    let runtime_result = {
        let mut envelope = QueueAcceptRuntimeEnvelope::new(&mut runtime);
        envelope.accept(&mut runtime_views, &mut runtime_owner_state)
    };

    let mut top_views = build_views();
    let mut top_owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(9));
    let top_accept = run_queue_accept_top_with_runtime_source(
        &mut top_views,
        &mut top_owner_state,
        &expected_snapshot,
        |_queued| ApplyResult::new(Ter::TES_SUCCESS, true, false),
    );
    let top_result = tx::QueueAcceptEntryResult {
        ledger_changed: top_accept.ledger_changed(),
        accept: top_accept,
    };

    assert_eq!(runtime_result, top_result);
    assert_eq!(runtime_views, top_views);
    assert_eq!(runtime_owner_state, top_owner_state);
    assert_eq!(runtime.metrics_calls.get(), 1);
    assert_eq!(runtime.apply_calls, 1);
}

#[test]
fn queue_accept_runtime_envelope_log_sinks_match_top_runtime_source_wrapper_and_samples_metrics_once()
 {
    let mut runtime = snapshot_counting_runtime(ApplyResult::new(Ter::TER_RETRY, false, false));
    let expected_snapshot = tx::QueueAcceptRuntimeSnapshot::new(
        runtime.metrics.clone(),
        runtime.open_ledger_tx_count,
        runtime.parent_hash,
        runtime.current_max_size,
    );

    let mut runtime_views = build_views();
    let mut runtime_owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(9));
    let runtime_emitted = RefCell::new(Vec::new());
    let runtime_result = {
        let mut envelope = QueueAcceptRuntimeEnvelope::new(&mut runtime);
        envelope.accept_with_log_sinks(
            &mut runtime_views,
            &mut runtime_owner_state,
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
        )
    };

    let mut top_views = build_views();
    let mut top_owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(9));
    let top_emitted = RefCell::new(Vec::new());
    let top_accept = run_queue_accept_top_with_runtime_source_with_log_sinks(
        &mut top_views,
        &mut top_owner_state,
        &expected_snapshot,
        |message| top_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| top_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| top_emitted.borrow_mut().push(format!("info:{message}")),
        |message| top_emitted.borrow_mut().push(format!("warn:{message}")),
        |_queued| ApplyResult::new(Ter::TER_RETRY, false, false),
    );
    let top_result = tx::QueueAcceptEntryResult {
        ledger_changed: top_accept.ledger_changed(),
        accept: top_accept,
    };

    assert_eq!(runtime_result, top_result);
    assert_eq!(runtime_views, top_views);
    assert_eq!(runtime_owner_state, top_owner_state);
    assert_eq!(runtime_emitted.into_inner(), top_emitted.into_inner());
    assert_eq!(runtime.metrics_calls.get(), 1);
    assert_eq!(runtime.apply_calls, 1);
}

#[derive(Debug)]
struct ReflightAcceptRuntime {
    metrics: QueueFeeMetricsState,
    open_ledger_tx_count: usize,
    parent_hash: Uint256,
    current_max_size: Option<usize>,
    current_rules: Rules,
    apply_calls: usize,
}

impl tx::QueueAcceptObservedViewSource for ReflightAcceptRuntime {
    fn open_ledger_tx_count(&self) -> usize {
        self.open_ledger_tx_count
    }

    fn parent_hash(&self) -> Uint256 {
        self.parent_hash
    }
}

impl tx::QueueAcceptObservedQueueSource for ReflightAcceptRuntime {
    fn current_max_size(&self) -> Option<usize> {
        self.current_max_size
    }
}

impl tx::QueueAcceptRuntimeSource for ReflightAcceptRuntime {
    fn metrics(&self) -> &QueueFeeMetricsState {
        &self.metrics
    }
}

impl QueueAcceptApplyRuntime<&'static str, &'static str, &'static str, &'static str>
    for ReflightAcceptRuntime
{
    fn apply_queued(
        &mut self,
        queued: &mut MaybeTx<&'static str, &'static str, &'static str, &'static str>,
    ) -> ApplyResult {
        self.apply_calls += 1;
        let seq_proxy = queued.seq_proxy;
        let refreshed_rules = self.current_rules.clone();

        queued.apply_with_current_rules(
            &self.current_rules,
            |_| {},
            move |tx, flags, journal| {
                PreflightResult::new(
                    tx,
                    None::<&str>,
                    refreshed_rules.clone(),
                    TxConsequences::new(1, seq_proxy),
                    flags,
                    journal,
                    Ter::TES_SUCCESS,
                )
            },
            |preflight_result| {
                PreclaimResult::new(
                    0,
                    preflight_result.tx,
                    preflight_result.parent_batch_id,
                    preflight_result.flags,
                    preflight_result.journal,
                    Ter::TER_RETRY,
                )
            },
            |_preclaim_result| ApplyResult::new(Ter::TER_RETRY, false, false),
        )
    }
}

#[test]
fn queue_accept_runtime_apply_boundary_can_refresh_queued_preflight_in_place() {
    let old_rules = ledger_rules(0x11);
    let new_rules = ledger_rules(0x22);
    let mut views = build_views();
    views
        .accounts
        .get_mut("acct")
        .expect("account exists")
        .transactions
        .get_mut(&SeqProxy::sequence(5))
        .expect("candidate exists")
        .payload
        .pf_result
        .rules = old_rules;

    let mut runtime = ReflightAcceptRuntime {
        metrics: test_setup().fee_metrics_state(),
        open_ledger_tx_count: 32,
        parent_hash: Uint256::from_u64(9),
        current_max_size: Some(2),
        current_rules: new_rules.clone(),
        apply_calls: 0,
    };
    let mut owner_state = QueueAcceptOwnerState::new(Uint256::from_u64(9));

    let result = run_queue_accept_with_runtime(&mut views, &mut owner_state, &mut runtime);
    let queued = &views
        .accounts
        .get("acct")
        .expect("account exists")
        .transactions
        .get(&SeqProxy::sequence(5))
        .expect("candidate retained")
        .payload;

    assert!(!result.ledger_changed);
    assert_eq!(queued.pf_result.rules, new_rules);
    assert_eq!(queued.last_result, Some(Ter::TER_RETRY));
    assert_eq!(queued.retries_remaining, MAYBE_TX_RETRIES_ALLOWED - 1);
    assert_eq!(runtime.apply_calls, 1);
}
