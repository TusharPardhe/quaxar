use std::{cell::RefCell, collections::BTreeMap};

use basics::base_uint::Uint256;
use protocol::{Rules, SeqProxy, Ter};
use tx::{
    ApplyFlags, ApplyResult, BlockerQueueAdmission, DirectApplyAttemptResult, DirectApplyExecution,
    MaybeTx, MaybeTxCore, OrderCandidates, PreclaimResult, PreflightResult, QueueApplyAccountStage,
    QueueApplyCallEnvelope, QueueApplyCurrentPreclaimClearRuntime, QueueApplyEntryStage,
    QueueApplyExecutionRuntime, QueueApplyHoldPreflightTxSource, QueueApplyLiveOwner,
    QueueApplyObservedAccountLookup, QueueApplyObservedTicketLookup, QueueApplyObservedTxSource,
    QueueApplyObservedViewSource, QueueApplyOwnerShell, QueueApplyPreclaimStage,
    QueueApplyPreflightStage, QueueApplyPreparedPreclaimInputs, QueueApplyQueuedStage,
    QueueApplyRuntimeEnvelope, QueueApplyViewAdjustment, QueueFeeMetricsSnapshot,
    QueueHoldPreflight, QueueViews, TxConsequences, TxConsequencesCategory, TxQAccount,
    derive_queue_hold_preflight_from_tx_source, format_direct_apply_finish_log_message,
    format_direct_apply_start_log_message, run_queue_apply_after_preflight_with_app_view,
    run_queue_apply_after_preflight_with_app_view_and_caller_preclaim,
    run_queue_apply_after_preflight_with_app_view_and_caller_preclaim_and_log_sinks,
    run_queue_apply_after_preflight_with_app_view_and_derived_hold_preflight,
    run_queue_apply_after_preflight_with_app_view_and_log_messages,
    run_queue_apply_after_preflight_with_app_view_and_log_sinks, run_queue_apply_with_app_view,
    run_queue_apply_with_app_view_and_caller_preclaim,
    run_queue_apply_with_app_view_and_caller_preclaim_and_log_sinks,
    run_queue_apply_with_app_view_and_derived_hold_preflight,
    run_queue_apply_with_app_view_and_derived_preflight_facts,
    run_queue_apply_with_app_view_and_log_messages, run_queue_apply_with_app_view_and_log_sinks,
    snapshot_queue_apply_app_view,
};

#[derive(Debug)]
struct TestObservedTxSource<'a> {
    account: &'a String,
    transaction_id: &'static str,
    tx_id: Uint256,
    tx_seq_proxy: SeqProxy,
}

#[derive(Debug, Clone)]
struct TestLedgerView {
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

#[derive(Debug, Default)]
struct TestApplyApp {
    trace_messages: Vec<String>,
    preflight_calls: usize,
    direct_apply_calls: usize,
    prepare_multitxn_calls: usize,
    preclaim_calls: usize,
    try_clear_calls: usize,
    apply_sandbox_calls: usize,
}

#[derive(Debug, Default)]
struct FlowApplyApp {
    trace_messages: Vec<String>,
    preflight_calls: usize,
    direct_apply_calls: usize,
    prepare_multitxn_calls: usize,
    preclaim_calls: usize,
    try_clear_calls: usize,
    apply_sandbox_calls: usize,
}

#[derive(Debug)]
struct TestLogSinkApp {
    preflight_result: PreflightResult<&'static str, TxConsequences, &'static str, &'static str>,
    apply_result: ApplyResult,
    trace_messages: Vec<String>,
    preflight_calls: usize,
    direct_apply_calls: usize,
    prepare_multitxn_calls: usize,
    preclaim_calls: usize,
    try_clear_calls: usize,
    apply_sandbox_calls: usize,
}

fn structured_try_clear_success() -> tx::TryClearAccountResult {
    tx::TryClearAccountResult::ClearQueue {
        plan: tx::TryClearAccountPlan {
            queued_seq_proxies: vec![SeqProxy::sequence(5)],
            queued_count: 1,
            target_was_already_queued: false,
            total_fee_level_paid: 50,
        },
        required_total_fee_level: 40,
        execution: tx::TryClearExecution::CurrentTx(tx::TryClearFinalization {
            current_result: ApplyResult::new(Ter::TES_SUCCESS, true, true),
            cleanup: None,
        }),
    }
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

impl QueueApplyHoldPreflightTxSource for TestObservedTxSource<'_> {
    fn has_previous_txn_id(&self) -> bool {
        false
    }

    fn has_account_txn_id(&self) -> bool {
        false
    }

    fn last_valid_ledger(&self) -> Option<u32> {
        Some(250)
    }
}

impl QueueApplyObservedViewSource<String> for TestLedgerView {
    fn rules(&self) -> &Rules {
        &self.rules
    }

    fn account_lookup(&self, account: &String) -> QueueApplyObservedAccountLookup {
        if account == "acct" {
            self.account_lookup
        } else {
            QueueApplyObservedAccountLookup::Missing
        }
    }

    fn ticket_lookup(
        &self,
        _account: &String,
        tx_seq_proxy: SeqProxy,
    ) -> QueueApplyObservedTicketLookup {
        if tx_seq_proxy == SeqProxy::sequence(6) {
            self.ticket_lookup
        } else {
            QueueApplyObservedTicketLookup::Missing
        }
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

impl QueueApplyExecutionRuntime<&'static str, &'static str, &'static str> for TestApplyApp {
    fn run_preflight(
        &mut self,
    ) -> PreflightResult<&'static str, TxConsequences, &'static str, &'static str> {
        self.preflight_calls += 1;
        PreflightResult::new(
            "tx",
            None::<&str>,
            rules(),
            blocker_consequences(SeqProxy::sequence(6)),
            ApplyFlags::NONE,
            "journal",
            Ter::TES_SUCCESS,
        )
    }

    fn trace(&mut self, message: &str) {
        self.trace_messages.push(message.to_owned());
    }

    fn direct_apply(&mut self) -> ApplyResult {
        self.direct_apply_calls += 1;
        ApplyResult::new(Ter::TES_SUCCESS, true, true)
    }

    fn prepare_multitxn(&mut self, _adjustment: QueueApplyViewAdjustment) -> bool {
        self.prepare_multitxn_calls += 1;
        true
    }

    fn run_preclaim(
        &mut self,
        view_source: tx::QueueApplyPreclaimViewSource,
    ) -> PreclaimResult<&'static str, &'static str, &'static str> {
        self.preclaim_calls += 1;
        assert!(!view_source.has_multi_txn());
        PreclaimResult::new(
            100,
            "tx",
            None::<&str>,
            ApplyFlags::NONE,
            "journal",
            Ter::TES_SUCCESS,
        )
    }

    fn run_try_clear(&mut self) -> ApplyResult {
        self.try_clear_calls += 1;
        ApplyResult::new(Ter::TES_SUCCESS, true, true)
    }

    fn apply_sandbox(&mut self) {
        self.apply_sandbox_calls += 1;
    }
}

impl QueueApplyCurrentPreclaimClearRuntime<&'static str, &'static str, &'static str>
    for TestApplyApp
{
    fn run_try_clear_with_current_preclaim(&mut self) -> tx::TryClearAccountResult {
        structured_try_clear_success()
    }
}

impl QueueApplyExecutionRuntime<&'static str, &'static str, &'static str> for FlowApplyApp {
    fn run_preflight(
        &mut self,
    ) -> PreflightResult<&'static str, TxConsequences, &'static str, &'static str> {
        self.preflight_calls += 1;
        flow_preflight_result()
    }

    fn trace(&mut self, message: &str) {
        self.trace_messages.push(message.to_owned());
    }

    fn direct_apply(&mut self) -> ApplyResult {
        self.direct_apply_calls += 1;
        ApplyResult::new(Ter::TES_SUCCESS, true, true)
    }

    fn prepare_multitxn(&mut self, _adjustment: QueueApplyViewAdjustment) -> bool {
        self.prepare_multitxn_calls += 1;
        true
    }

    fn run_preclaim(
        &mut self,
        _view_source: tx::QueueApplyPreclaimViewSource,
    ) -> PreclaimResult<&'static str, &'static str, &'static str> {
        self.preclaim_calls += 1;
        panic!("caller-preclaim boundary should bypass runtime preclaim");
    }

    fn run_try_clear(&mut self) -> ApplyResult {
        self.try_clear_calls += 1;
        ApplyResult::new(Ter::TES_SUCCESS, true, true)
    }

    fn apply_sandbox(&mut self) {
        self.apply_sandbox_calls += 1;
    }
}

impl QueueApplyCurrentPreclaimClearRuntime<&'static str, &'static str, &'static str>
    for FlowApplyApp
{
    fn run_try_clear_with_current_preclaim(&mut self) -> tx::TryClearAccountResult {
        structured_try_clear_success()
    }
}

impl QueueApplyExecutionRuntime<&'static str, &'static str, &'static str> for TestLogSinkApp {
    fn run_preflight(
        &mut self,
    ) -> PreflightResult<&'static str, TxConsequences, &'static str, &'static str> {
        self.preflight_calls += 1;
        self.preflight_result.clone()
    }

    fn trace(&mut self, message: &str) {
        self.trace_messages.push(message.to_owned());
    }

    fn direct_apply(&mut self) -> ApplyResult {
        self.direct_apply_calls += 1;
        self.apply_result.clone()
    }

    fn prepare_multitxn(&mut self, _adjustment: QueueApplyViewAdjustment) -> bool {
        self.prepare_multitxn_calls += 1;
        true
    }

    fn run_preclaim(
        &mut self,
        _view_source: tx::QueueApplyPreclaimViewSource,
    ) -> PreclaimResult<&'static str, &'static str, &'static str> {
        self.preclaim_calls += 1;
        PreclaimResult::new(
            100,
            "tx",
            None::<&str>,
            ApplyFlags::NONE,
            "journal",
            Ter::TES_SUCCESS,
        )
    }

    fn run_try_clear(&mut self) -> ApplyResult {
        self.try_clear_calls += 1;
        ApplyResult::new(Ter::TES_SUCCESS, true, true)
    }

    fn apply_sandbox(&mut self) {
        self.apply_sandbox_calls += 1;
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

fn flow_view() -> TestLedgerView {
    TestLedgerView {
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

fn flow_preflight_result()
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

fn log_sink_preflight_result(
    seq_proxy: SeqProxy,
) -> PreflightResult<&'static str, TxConsequences, &'static str, &'static str> {
    PreflightResult::new(
        "tx",
        None::<&str>,
        rules(),
        TxConsequences::new(1, seq_proxy),
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        "journal",
        Ter::TES_SUCCESS,
    )
}

fn view() -> TestLedgerView {
    TestLedgerView {
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

fn direct_view() -> TestLedgerView {
    TestLedgerView {
        rules: rules(),
        account_lookup: QueueApplyObservedAccountLookup::Present {
            sequence: 5,
            balance_drops: 1_000,
        },
        ticket_lookup: QueueApplyObservedTicketLookup::Missing,
        calculated_base_fee_drops: 10,
        fee_paid_drops: 100,
        default_base_fee_drops: 10,
        metrics_snapshot: QueueFeeMetricsSnapshot {
            txns_expected: 32,
            escalation_multiplier: tx::TXQ_BASE_LEVEL,
        },
        open_ledger_tx_count: 4,
        open_ledger_seq: 100,
        reserve_drops: 200,
        base_fee_drops: 10,
    }
}

fn direct_tx_source<'a>(account: &'a String) -> TestObservedTxSource<'a> {
    TestObservedTxSource {
        account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
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

fn build_owner_shell(
    current_max_size: Option<usize>,
) -> QueueApplyOwnerShell<String, &'static str, &'static str, &'static str> {
    QueueApplyOwnerShell::new(QueueApplyLiveOwner::new(
        2,
        10,
        25,
        current_max_size,
        OrderCandidates::new(Uint256::from_u64(0)),
        build_views(),
    ))
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

fn build_flow_owner_shell(
    current_max_size: Option<usize>,
) -> QueueApplyOwnerShell<String, &'static str, &'static str, &'static str> {
    QueueApplyOwnerShell::new(QueueApplyLiveOwner::new(
        2,
        10,
        25,
        current_max_size,
        OrderCandidates::new(Uint256::from_u64(0)),
        build_flow_views(),
    ))
}

fn build_direct_owner_shell()
-> QueueApplyOwnerShell<String, &'static str, &'static str, &'static str> {
    QueueApplyOwnerShell::new(QueueApplyLiveOwner::new(
        2,
        10,
        25,
        Some(10),
        OrderCandidates::new(Uint256::from_u64(0)),
        QueueViews::new(BTreeMap::new(), Vec::new()),
    ))
}

fn log_sink_app(
    preflight_result: PreflightResult<&'static str, TxConsequences, &'static str, &'static str>,
) -> TestLogSinkApp {
    TestLogSinkApp {
        preflight_result,
        apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, true),
        trace_messages: Vec::new(),
        preflight_calls: 0,
        direct_apply_calls: 0,
        prepare_multitxn_calls: 0,
        preclaim_calls: 0,
        try_clear_calls: 0,
        apply_sandbox_calls: 0,
    }
}

fn expected_stage()
-> QueueApplyPreflightStage<String, &'static str, &'static str, &'static str, &'static str> {
    QueueApplyPreflightStage::Entry(QueueApplyEntryStage::Queued(
        QueueApplyQueuedStage::Account(QueueApplyAccountStage::RejectBlockerAdmission(
            BlockerQueueAdmission::RejectsNonReplacementOfLoneEntry,
        )),
    ))
}

#[test]
fn queue_apply_app_view_wrapper_matches_runtime_envelope_apply() {
    let account = String::from("acct");
    let tx_source = tx_source(&account);
    let view = view();
    let snapshot = snapshot_queue_apply_app_view(&tx_source, &view);
    let call = QueueApplyCallEnvelope::new(&tx_source, &snapshot);

    let mut direct_app = TestApplyApp::default();
    let mut direct_shell = build_owner_shell(Some(10));
    let direct_stage = QueueApplyRuntimeEnvelope::new(&mut direct_app).apply(
        &mut direct_shell,
        &call,
        hold_preflight(),
        ApplyFlags::NONE,
        blocker_consequences(SeqProxy::sequence(6)),
        Ter::TES_SUCCESS,
    );

    let mut app = TestApplyApp::default();
    let mut app_view_shell = build_owner_shell(Some(10));
    let app_view_stage = run_queue_apply_with_app_view(
        &mut app_view_shell,
        &mut app,
        &view,
        &tx_source,
        hold_preflight(),
        ApplyFlags::NONE,
        blocker_consequences(SeqProxy::sequence(6)),
        Ter::TES_SUCCESS,
    );

    assert_eq!(app_view_stage, direct_stage);
    assert_eq!(app_view_shell.owner(), direct_shell.owner());
    assert_eq!(app_view_stage, expected_stage());
    assert_eq!(app.preflight_calls, 1);
    assert_eq!(app.direct_apply_calls, 0);
    assert_eq!(app.prepare_multitxn_calls, 0);
    assert_eq!(app.preclaim_calls, 0);
    assert_eq!(app.try_clear_calls, 0);
    assert_eq!(app.apply_sandbox_calls, 0);
    assert!(app.trace_messages.is_empty());
}

#[test]
fn queue_apply_app_view_can_derive_hold_preflight_from_tx_source() {
    let account = String::from("acct");
    let tx_source = tx_source(&account);
    let view = view();

    let mut explicit_app = TestApplyApp::default();
    let mut explicit_shell = build_owner_shell(Some(10));
    let explicit_stage = run_queue_apply_with_app_view(
        &mut explicit_shell,
        &mut explicit_app,
        &view,
        &tx_source,
        hold_preflight(),
        ApplyFlags::NONE,
        blocker_consequences(SeqProxy::sequence(6)),
        Ter::TES_SUCCESS,
    );

    let mut derived_app = TestApplyApp::default();
    let mut derived_shell = build_owner_shell(Some(10));
    let derived_stage = run_queue_apply_with_app_view_and_derived_hold_preflight(
        &mut derived_shell,
        &mut derived_app,
        &view,
        &tx_source,
        ApplyFlags::NONE,
        blocker_consequences(SeqProxy::sequence(6)),
        Ter::TES_SUCCESS,
    );

    assert_eq!(derived_stage, explicit_stage);
    assert_eq!(derived_shell.owner(), explicit_shell.owner());
    assert_eq!(
        derive_queue_hold_preflight_from_tx_source(&tx_source, ApplyFlags::NONE),
        hold_preflight()
    );
    assert_eq!(derived_app.preflight_calls, 1);
}

#[test]
fn queue_apply_app_view_can_derive_preflight_facts_from_runtime() {
    let account = String::from("acct");
    let tx_source = tx_source(&account);
    let view = view();

    let mut explicit_app = TestApplyApp::default();
    let mut explicit_shell = build_owner_shell(Some(10));
    let explicit_stage = run_queue_apply_with_app_view(
        &mut explicit_shell,
        &mut explicit_app,
        &view,
        &tx_source,
        hold_preflight(),
        ApplyFlags::NONE,
        blocker_consequences(SeqProxy::sequence(6)),
        Ter::TES_SUCCESS,
    );

    let mut derived_app = TestApplyApp::default();
    let mut derived_shell = build_owner_shell(Some(10));
    let derived_stage = run_queue_apply_with_app_view_and_derived_preflight_facts(
        &mut derived_shell,
        &mut derived_app,
        &view,
        &tx_source,
        Ter::TES_SUCCESS,
    );

    assert_eq!(derived_stage, explicit_stage);
    assert_eq!(derived_shell.owner(), explicit_shell.owner());
    assert_eq!(derived_app.preflight_calls, 1);
}

#[test]
fn queue_apply_app_view_wrapper_matches_runtime_envelope_after_preflight() {
    let account = String::from("acct");
    let tx_source = tx_source(&account);
    let view = view();
    let snapshot = snapshot_queue_apply_app_view(&tx_source, &view);
    let call = QueueApplyCallEnvelope::new(&tx_source, &snapshot);
    let preflight = preflight_result(SeqProxy::sequence(6));

    let mut direct_app = TestApplyApp::default();
    let mut direct_shell = build_owner_shell(Some(10));
    let direct_stage = QueueApplyRuntimeEnvelope::new(&mut direct_app).apply_after_preflight(
        &mut direct_shell,
        &call,
        &preflight,
        hold_preflight(),
        Ter::TES_SUCCESS,
    );

    let mut app = TestApplyApp::default();
    let mut app_view_shell = build_owner_shell(Some(10));
    let app_view_stage = run_queue_apply_after_preflight_with_app_view(
        &mut app_view_shell,
        &mut app,
        &view,
        &tx_source,
        &preflight,
        hold_preflight(),
        Ter::TES_SUCCESS,
    );

    assert_eq!(app_view_stage, direct_stage);
    assert_eq!(app_view_shell.owner(), direct_shell.owner());
    assert_eq!(app_view_stage, expected_stage());
    assert_eq!(app.preflight_calls, 0);
    assert_eq!(app.direct_apply_calls, 0);
    assert_eq!(app.prepare_multitxn_calls, 0);
    assert_eq!(app.preclaim_calls, 0);
    assert_eq!(app.try_clear_calls, 0);
    assert_eq!(app.apply_sandbox_calls, 0);
    assert!(app.trace_messages.is_empty());
}

#[test]
fn queue_apply_app_view_after_preflight_can_derive_hold_preflight_from_tx_source() {
    let account = String::from("acct");
    let tx_source = tx_source(&account);
    let view = view();
    let preflight = preflight_result(SeqProxy::sequence(6));

    let mut explicit_app = TestApplyApp::default();
    let mut explicit_shell = build_owner_shell(Some(10));
    let explicit_stage = run_queue_apply_after_preflight_with_app_view(
        &mut explicit_shell,
        &mut explicit_app,
        &view,
        &tx_source,
        &preflight,
        hold_preflight(),
        Ter::TES_SUCCESS,
    );

    let mut derived_app = TestApplyApp::default();
    let mut derived_shell = build_owner_shell(Some(10));
    let derived_stage = run_queue_apply_after_preflight_with_app_view_and_derived_hold_preflight(
        &mut derived_shell,
        &mut derived_app,
        &view,
        &tx_source,
        &preflight,
        Ter::TES_SUCCESS,
    );

    assert_eq!(derived_stage, explicit_stage);
    assert_eq!(derived_shell.owner(), explicit_shell.owner());
    assert_eq!(derived_app.preflight_calls, 0);
}

#[test]
fn queue_apply_app_view_log_sinks_match_runtime_envelope_on_direct_path() {
    let account = String::from("acct");
    let tx_source = direct_tx_source(&account);
    let view = direct_view();
    let snapshot = snapshot_queue_apply_app_view(&tx_source, &view);
    let call = QueueApplyCallEnvelope::new(&tx_source, &snapshot);

    let mut direct_app = log_sink_app(log_sink_preflight_result(SeqProxy::sequence(5)));
    let mut direct_shell = build_direct_owner_shell();
    let direct_emitted = RefCell::new(Vec::new());
    let direct_stage = QueueApplyRuntimeEnvelope::new(&mut direct_app).apply_with_log_sinks(
        &mut direct_shell,
        &call,
        hold_preflight(),
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(5)),
        Ter::TES_SUCCESS,
        |message| direct_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| direct_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| direct_emitted.borrow_mut().push(format!("info:{message}")),
    );

    let mut app = log_sink_app(log_sink_preflight_result(SeqProxy::sequence(5)));
    let mut app_view_shell = build_direct_owner_shell();
    let app_view_emitted = RefCell::new(Vec::new());
    let app_view_stage = run_queue_apply_with_app_view_and_log_sinks(
        &mut app_view_shell,
        &mut app,
        &view,
        &tx_source,
        hold_preflight(),
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(5)),
        Ter::TES_SUCCESS,
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
    );

    let expected_emitted = vec![
        format!("trace:{}", format_direct_apply_start_log_message("ABC123")),
        format!(
            "trace:{}",
            format_direct_apply_finish_log_message(&DirectApplyExecution::<String, _> {
                transaction_id: "ABC123",
                attempt: DirectApplyAttemptResult {
                    apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, true),
                    removed_replacement: None,
                },
            })
        ),
    ];
    let direct_emitted = direct_emitted.into_inner();
    let app_view_emitted = app_view_emitted.into_inner();

    assert_eq!(app_view_stage, direct_stage);
    assert_eq!(app_view_shell.owner(), direct_shell.owner());
    assert_eq!(app_view_emitted, direct_emitted);
    assert_eq!(app.trace_messages, Vec::<String>::new());
    assert_eq!(
        direct_emitted, expected_emitted,
        "runtime envelope should preserve direct-path trace sinks"
    );
    assert_eq!(app.preflight_calls, 1);
    assert_eq!(app.direct_apply_calls, 1);
    assert_eq!(app.prepare_multitxn_calls, 0);
    assert_eq!(app.preclaim_calls, 0);
    assert_eq!(app.try_clear_calls, 0);
    assert_eq!(app.apply_sandbox_calls, 0);
}

#[test]
fn queue_apply_app_view_log_messages_match_runtime_envelope_on_direct_path() {
    let account = String::from("acct");
    let tx_source = direct_tx_source(&account);
    let view = direct_view();
    let snapshot = snapshot_queue_apply_app_view(&tx_source, &view);
    let call = QueueApplyCallEnvelope::new(&tx_source, &snapshot);

    let mut direct_app = log_sink_app(log_sink_preflight_result(SeqProxy::sequence(5)));
    let mut direct_shell = build_direct_owner_shell();
    let direct_stage = QueueApplyRuntimeEnvelope::new(&mut direct_app).apply_with_log_messages(
        &mut direct_shell,
        &call,
        hold_preflight(),
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(5)),
        Ter::TES_SUCCESS,
    );

    let mut app = log_sink_app(log_sink_preflight_result(SeqProxy::sequence(5)));
    let mut app_view_shell = build_direct_owner_shell();
    let app_view_stage = run_queue_apply_with_app_view_and_log_messages(
        &mut app_view_shell,
        &mut app,
        &view,
        &tx_source,
        hold_preflight(),
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(5)),
        Ter::TES_SUCCESS,
    );

    assert_eq!(app_view_stage, direct_stage);
    assert_eq!(app_view_shell.owner(), direct_shell.owner());
    assert_eq!(
        app.trace_messages,
        vec![
            format_direct_apply_start_log_message("ABC123"),
            format_direct_apply_finish_log_message(&DirectApplyExecution::<String, _> {
                transaction_id: "ABC123",
                attempt: DirectApplyAttemptResult {
                    apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, true),
                    removed_replacement: None,
                },
            }),
        ]
    );
    assert_eq!(app.preflight_calls, 1);
    assert_eq!(app.direct_apply_calls, 1);
    assert_eq!(app.prepare_multitxn_calls, 0);
    assert_eq!(app.preclaim_calls, 0);
    assert_eq!(app.try_clear_calls, 0);
    assert_eq!(app.apply_sandbox_calls, 0);
}

#[test]
fn queue_apply_app_view_after_preflight_log_sinks_match_runtime_envelope_on_direct_path() {
    let account = String::from("acct");
    let tx_source = direct_tx_source(&account);
    let view = direct_view();
    let snapshot = snapshot_queue_apply_app_view(&tx_source, &view);
    let call = QueueApplyCallEnvelope::new(&tx_source, &snapshot);
    let preflight = log_sink_preflight_result(SeqProxy::sequence(5));

    let mut direct_app = log_sink_app(log_sink_preflight_result(SeqProxy::sequence(5)));
    let mut direct_shell = build_direct_owner_shell();
    let direct_emitted = RefCell::new(Vec::new());
    let direct_stage = QueueApplyRuntimeEnvelope::new(&mut direct_app)
        .apply_after_preflight_with_log_sinks(
            &mut direct_shell,
            &call,
            &preflight,
            hold_preflight(),
            Ter::TES_SUCCESS,
            |message| direct_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| direct_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| direct_emitted.borrow_mut().push(format!("info:{message}")),
        );

    let mut app = log_sink_app(log_sink_preflight_result(SeqProxy::sequence(5)));
    let mut app_view_shell = build_direct_owner_shell();
    let app_view_emitted = RefCell::new(Vec::new());
    let app_view_stage = run_queue_apply_after_preflight_with_app_view_and_log_sinks(
        &mut app_view_shell,
        &mut app,
        &view,
        &tx_source,
        &preflight,
        hold_preflight(),
        Ter::TES_SUCCESS,
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
    );

    let expected_emitted = vec![
        format!("trace:{}", format_direct_apply_start_log_message("ABC123")),
        format!(
            "trace:{}",
            format_direct_apply_finish_log_message(&DirectApplyExecution::<String, _> {
                transaction_id: "ABC123",
                attempt: DirectApplyAttemptResult {
                    apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, true),
                    removed_replacement: None,
                },
            })
        ),
    ];
    let direct_emitted = direct_emitted.into_inner();
    let app_view_emitted = app_view_emitted.into_inner();

    assert_eq!(app_view_stage, direct_stage);
    assert_eq!(app_view_shell.owner(), direct_shell.owner());
    assert_eq!(app_view_emitted, direct_emitted);
    assert_eq!(app.trace_messages, Vec::<String>::new());
    assert_eq!(
        direct_emitted, expected_emitted,
        "runtime envelope should preserve after-preflight direct-path trace sinks"
    );
    assert_eq!(app.preflight_calls, 0);
    assert_eq!(app.direct_apply_calls, 1);
    assert_eq!(app.prepare_multitxn_calls, 0);
    assert_eq!(app.preclaim_calls, 0);
    assert_eq!(app.try_clear_calls, 0);
    assert_eq!(app.apply_sandbox_calls, 0);
}

#[test]
fn queue_apply_app_view_after_preflight_log_messages_match_runtime_envelope_on_direct_path() {
    let account = String::from("acct");
    let tx_source = direct_tx_source(&account);
    let view = direct_view();
    let snapshot = snapshot_queue_apply_app_view(&tx_source, &view);
    let call = QueueApplyCallEnvelope::new(&tx_source, &snapshot);
    let preflight = log_sink_preflight_result(SeqProxy::sequence(5));

    let mut direct_app = log_sink_app(log_sink_preflight_result(SeqProxy::sequence(5)));
    let mut direct_shell = build_direct_owner_shell();
    let direct_stage = QueueApplyRuntimeEnvelope::new(&mut direct_app)
        .apply_after_preflight_with_log_messages(
            &mut direct_shell,
            &call,
            &preflight,
            hold_preflight(),
            Ter::TES_SUCCESS,
        );

    let mut app = log_sink_app(log_sink_preflight_result(SeqProxy::sequence(5)));
    let mut app_view_shell = build_direct_owner_shell();
    let app_view_stage = run_queue_apply_after_preflight_with_app_view_and_log_messages(
        &mut app_view_shell,
        &mut app,
        &view,
        &tx_source,
        &preflight,
        hold_preflight(),
        Ter::TES_SUCCESS,
    );

    assert_eq!(app_view_stage, direct_stage);
    assert_eq!(app_view_shell.owner(), direct_shell.owner());
    assert_eq!(
        app.trace_messages,
        vec![
            format_direct_apply_start_log_message("ABC123"),
            format_direct_apply_finish_log_message(&DirectApplyExecution::<String, _> {
                transaction_id: "ABC123",
                attempt: DirectApplyAttemptResult {
                    apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, true),
                    removed_replacement: None,
                },
            }),
        ]
    );
    assert_eq!(app.preflight_calls, 0);
    assert_eq!(app.direct_apply_calls, 1);
    assert_eq!(app.prepare_multitxn_calls, 0);
    assert_eq!(app.preclaim_calls, 0);
    assert_eq!(app.try_clear_calls, 0);
    assert_eq!(app.apply_sandbox_calls, 0);
}

#[test]
fn queue_apply_app_view_with_caller_preclaim_matches_runtime_envelope() {
    let account = String::from("acct");
    let tx_source = tx_source(&account);
    let view = flow_view();
    let snapshot = snapshot_queue_apply_app_view(&tx_source, &view);
    let call = QueueApplyCallEnvelope::new(&tx_source, &snapshot);
    let expected_prepared = RefCell::new(None::<QueueApplyPreparedPreclaimInputs<String>>);

    let mut direct_app = FlowApplyApp::default();
    let mut direct_shell = build_flow_owner_shell(Some(10));
    let direct_stage = QueueApplyRuntimeEnvelope::new(&mut direct_app).apply_with_caller_preclaim(
        &mut direct_shell,
        &call,
        hold_preflight(),
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        flow_preflight_result().consequences,
        Ter::TES_SUCCESS,
        |prepared| {
            expected_prepared.replace(Some(prepared.clone()));
            Ok(QueueApplyPreclaimStage {
                view_source: prepared.view_source,
                trace_message: "trace".to_string(),
                preclaim_result: flow_preflight_result().to_preclaim(9, Ter::TES_SUCCESS),
            })
        },
    );

    let mut app = FlowApplyApp::default();
    let mut app_view_shell = build_flow_owner_shell(Some(10));
    let app_view_stage = run_queue_apply_with_app_view_and_caller_preclaim(
        &mut app_view_shell,
        &mut app,
        &view,
        &tx_source,
        hold_preflight(),
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        flow_preflight_result().consequences,
        Ter::TES_SUCCESS,
        |prepared| {
            assert_eq!(Some(prepared.clone()), *expected_prepared.borrow());
            Ok(QueueApplyPreclaimStage {
                view_source: prepared.view_source,
                trace_message: "trace".to_string(),
                preclaim_result: flow_preflight_result().to_preclaim(9, Ter::TES_SUCCESS),
            })
        },
    );

    assert_eq!(app_view_stage, direct_stage);
    assert_eq!(app_view_shell.owner(), direct_shell.owner());
    assert_eq!(app.preflight_calls, 1);
    assert_eq!(app.preclaim_calls, 0);
    assert_eq!(app.direct_apply_calls, 0);
    assert_eq!(app.prepare_multitxn_calls, 1);
}

#[test]
fn queue_apply_app_view_after_preflight_with_caller_preclaim_matches_runtime_envelope() {
    let account = String::from("acct");
    let tx_source = tx_source(&account);
    let view = flow_view();
    let snapshot = snapshot_queue_apply_app_view(&tx_source, &view);
    let call = QueueApplyCallEnvelope::new(&tx_source, &snapshot);
    let preflight = flow_preflight_result();
    let expected_prepared = RefCell::new(None::<QueueApplyPreparedPreclaimInputs<String>>);

    let mut direct_app = FlowApplyApp::default();
    let mut direct_shell = build_flow_owner_shell(Some(10));
    let direct_stage = QueueApplyRuntimeEnvelope::new(&mut direct_app)
        .apply_after_preflight_with_caller_preclaim(
            &mut direct_shell,
            &call,
            &preflight,
            hold_preflight(),
            Ter::TES_SUCCESS,
            |prepared| {
                expected_prepared.replace(Some(prepared.clone()));
                Ok(QueueApplyPreclaimStage {
                    view_source: prepared.view_source,
                    trace_message: "trace".to_string(),
                    preclaim_result: flow_preflight_result().to_preclaim(9, Ter::TES_SUCCESS),
                })
            },
        );

    let mut app = FlowApplyApp::default();
    let mut app_view_shell = build_flow_owner_shell(Some(10));
    let app_view_stage = run_queue_apply_after_preflight_with_app_view_and_caller_preclaim(
        &mut app_view_shell,
        &mut app,
        &view,
        &tx_source,
        &preflight,
        hold_preflight(),
        Ter::TES_SUCCESS,
        |prepared| {
            assert_eq!(Some(prepared.clone()), *expected_prepared.borrow());
            Ok(QueueApplyPreclaimStage {
                view_source: prepared.view_source,
                trace_message: "trace".to_string(),
                preclaim_result: flow_preflight_result().to_preclaim(9, Ter::TES_SUCCESS),
            })
        },
    );

    assert_eq!(app_view_stage, direct_stage);
    assert_eq!(app_view_shell.owner(), direct_shell.owner());
    assert_eq!(app.preflight_calls, 0);
    assert_eq!(app.preclaim_calls, 0);
    assert_eq!(app.direct_apply_calls, 0);
    assert_eq!(app.prepare_multitxn_calls, 1);
}

#[test]
fn queue_apply_app_view_caller_preclaim_log_sinks_match_runtime_envelope() {
    let account = String::from("acct");
    let tx_source = tx_source(&account);
    let view = flow_view();
    let snapshot = snapshot_queue_apply_app_view(&tx_source, &view);
    let call = QueueApplyCallEnvelope::new(&tx_source, &snapshot);
    let expected_prepared = RefCell::new(None::<QueueApplyPreparedPreclaimInputs<String>>);

    let mut direct_app = FlowApplyApp::default();
    let mut direct_shell = build_flow_owner_shell(Some(10));
    let direct_emitted = RefCell::new(Vec::new());
    let direct_stage = QueueApplyRuntimeEnvelope::new(&mut direct_app)
        .apply_with_caller_preclaim_and_log_sinks(
            &mut direct_shell,
            &call,
            hold_preflight(),
            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
            flow_preflight_result().consequences,
            Ter::TES_SUCCESS,
            |prepared| {
                expected_prepared.replace(Some(prepared.clone()));
                Ok(QueueApplyPreclaimStage {
                    view_source: prepared.view_source,
                    trace_message: "trace".to_string(),
                    preclaim_result: flow_preflight_result().to_preclaim(9, Ter::TES_SUCCESS),
                })
            },
            |message| direct_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| direct_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| direct_emitted.borrow_mut().push(format!("info:{message}")),
        );

    let mut app = FlowApplyApp::default();
    let mut app_view_shell = build_flow_owner_shell(Some(10));
    let app_view_emitted = RefCell::new(Vec::new());
    let app_view_stage = run_queue_apply_with_app_view_and_caller_preclaim_and_log_sinks(
        &mut app_view_shell,
        &mut app,
        &view,
        &tx_source,
        hold_preflight(),
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        flow_preflight_result().consequences,
        Ter::TES_SUCCESS,
        |prepared| {
            assert_eq!(Some(prepared.clone()), *expected_prepared.borrow());
            Ok(QueueApplyPreclaimStage {
                view_source: prepared.view_source,
                trace_message: "trace".to_string(),
                preclaim_result: flow_preflight_result().to_preclaim(9, Ter::TES_SUCCESS),
            })
        },
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
    );

    assert_eq!(app_view_stage, direct_stage);
    assert_eq!(app_view_shell.owner(), direct_shell.owner());
    assert_eq!(app_view_emitted.into_inner(), direct_emitted.into_inner());
    assert_eq!(app.preflight_calls, 1);
    assert_eq!(app.preclaim_calls, 0);
    assert_eq!(app.direct_apply_calls, 0);
    assert_eq!(app.prepare_multitxn_calls, 1);
}

#[test]
fn queue_apply_app_view_after_preflight_caller_preclaim_log_sinks_match_runtime_envelope() {
    let account = String::from("acct");
    let tx_source = tx_source(&account);
    let view = flow_view();
    let snapshot = snapshot_queue_apply_app_view(&tx_source, &view);
    let call = QueueApplyCallEnvelope::new(&tx_source, &snapshot);
    let preflight = flow_preflight_result();
    let expected_prepared = RefCell::new(None::<QueueApplyPreparedPreclaimInputs<String>>);

    let mut direct_app = FlowApplyApp::default();
    let mut direct_shell = build_flow_owner_shell(Some(10));
    let direct_emitted = RefCell::new(Vec::new());
    let direct_stage = QueueApplyRuntimeEnvelope::new(&mut direct_app)
        .apply_after_preflight_with_caller_preclaim_and_log_sinks(
            &mut direct_shell,
            &call,
            &preflight,
            hold_preflight(),
            Ter::TES_SUCCESS,
            |prepared| {
                expected_prepared.replace(Some(prepared.clone()));
                Ok(QueueApplyPreclaimStage {
                    view_source: prepared.view_source,
                    trace_message: "trace".to_string(),
                    preclaim_result: flow_preflight_result().to_preclaim(9, Ter::TES_SUCCESS),
                })
            },
            |message| direct_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| direct_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| direct_emitted.borrow_mut().push(format!("info:{message}")),
        );

    let mut app = FlowApplyApp::default();
    let mut app_view_shell = build_flow_owner_shell(Some(10));
    let app_view_emitted = RefCell::new(Vec::new());
    let app_view_stage =
        run_queue_apply_after_preflight_with_app_view_and_caller_preclaim_and_log_sinks(
            &mut app_view_shell,
            &mut app,
            &view,
            &tx_source,
            &preflight,
            hold_preflight(),
            Ter::TES_SUCCESS,
            |prepared| {
                assert_eq!(Some(prepared.clone()), *expected_prepared.borrow());
                Ok(QueueApplyPreclaimStage {
                    view_source: prepared.view_source,
                    trace_message: "trace".to_string(),
                    preclaim_result: flow_preflight_result().to_preclaim(9, Ter::TES_SUCCESS),
                })
            },
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
        );

    assert_eq!(app_view_stage, direct_stage);
    assert_eq!(app_view_shell.owner(), direct_shell.owner());
    assert_eq!(app_view_emitted.into_inner(), direct_emitted.into_inner());
    assert_eq!(app.preflight_calls, 0);
    assert_eq!(app.preclaim_calls, 0);
    assert_eq!(app.direct_apply_calls, 0);
    assert_eq!(app.prepare_multitxn_calls, 1);
}

#[test]
fn queue_apply_app_view_snapshot_captures_tx_specific_view_facts() {
    let account = String::from("acct");
    let tx_source = tx_source(&account);
    let view = view();

    let snapshot = snapshot_queue_apply_app_view(&tx_source, &view);

    assert_eq!(
        snapshot.account_lookup(&account),
        QueueApplyObservedAccountLookup::Present {
            sequence: 5,
            balance_drops: 1_000,
        }
    );
    assert_eq!(
        snapshot.ticket_lookup(&account, SeqProxy::sequence(6)),
        QueueApplyObservedTicketLookup::Present
    );
    assert_eq!(snapshot.calculated_base_fee_drops, 10);
    assert_eq!(snapshot.fee_paid_drops, 1);
    assert_eq!(snapshot.default_base_fee_drops, 10);
    assert_eq!(
        snapshot.metrics_snapshot,
        QueueFeeMetricsSnapshot {
            txns_expected: 32,
            escalation_multiplier: tx::TXQ_BASE_LEVEL * 500,
        }
    );
    assert_eq!(snapshot.open_ledger_tx_count, 4);
    assert_eq!(snapshot.open_ledger_seq, 100);
    assert_eq!(snapshot.reserve_drops, 200);
    assert_eq!(snapshot.base_fee_drops, 10);
}
