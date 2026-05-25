use std::{cell::RefCell, collections::BTreeMap};

use basics::base_uint::Uint256;
use protocol::{Rules, SeqProxy, Ter};
use tx::{
    ApplyFlags, ApplyResult, BlockerQueueAdmission, FeeQueueEntry, FeeQueueKey, MaybeTx,
    MaybeTxCore, OrderCandidates, PreclaimResult, PreflightResult, QueueAdvanceCandidate,
    QueueApplyAccountStage, QueueApplyCallEnvelope, QueueApplyCurrentPreclaimClearRuntime,
    QueueApplyEntryStage, QueueApplyExecutionRuntime, QueueApplyHoldPreflightTxSource,
    QueueApplyLiveOwner, QueueApplyObservedAccountLookup, QueueApplyObservedTicketLookup,
    QueueApplyObservedTxSource, QueueApplyObservedViewSource, QueueApplyOwnerShell,
    QueueApplyPreflightStage, QueueApplyQueuedStage, QueueApplyRuntimeEnvelope,
    QueueApplyViewAdjustment, QueueFeeMetricsSnapshot, QueueHoldPreflight, QueueViews,
    TxConsequences, TxConsequencesCategory, TxQAccount, format_direct_apply_finish_log_message,
    format_direct_apply_start_log_message, format_queue_apply_enqueue_debug_message,
    format_queue_apply_full_queue_evict_info_message,
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

#[derive(Debug, Default)]
struct TestApplyRuntime {
    trace_messages: Vec<String>,
    preflight_calls: usize,
    direct_apply_calls: usize,
    prepare_multitxn_calls: usize,
    preclaim_calls: usize,
    try_clear_calls: usize,
    apply_sandbox_calls: usize,
}

#[derive(Debug)]
struct TestLogSinkRuntime {
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

impl QueueApplyExecutionRuntime<&'static str, &'static str, &'static str> for TestApplyRuntime {
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
    for TestApplyRuntime
{
    fn run_try_clear_with_current_preclaim(&mut self) -> tx::TryClearAccountResult {
        structured_try_clear_success()
    }
}

impl QueueApplyExecutionRuntime<&'static str, &'static str, &'static str> for TestLogSinkRuntime {
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

impl QueueApplyCurrentPreclaimClearRuntime<&'static str, &'static str, &'static str>
    for TestLogSinkRuntime
{
    fn run_try_clear_with_current_preclaim(&mut self) -> tx::TryClearAccountResult {
        structured_try_clear_success()
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

fn queueing_preflight_result(
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

fn queueing_view(account: &str) -> TestObservedViewSource {
    let _ = account;
    TestObservedViewSource {
        rules: rules(),
        account_lookup: QueueApplyObservedAccountLookup::Present {
            sequence: 5,
            balance_drops: 1_000,
        },
        ticket_lookup: QueueApplyObservedTicketLookup::Missing,
        calculated_base_fee_drops: 10,
        fee_paid_drops: 10,
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

fn direct_view(account: &str) -> TestObservedViewSource {
    let _ = account;
    TestObservedViewSource {
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

fn queued(
    account: String,
    seq_proxy: SeqProxy,
    tx_id: u64,
    fee_level: u64,
) -> MaybeTx<&'static str, String, &'static str, &'static str> {
    MaybeTx::new(
        Uint256::from_u64(tx_id),
        fee_level,
        account,
        Some(200),
        seq_proxy,
        ApplyFlags::NONE,
        queueing_preflight_result(seq_proxy),
    )
}

fn fee_entry(
    account: String,
    seq_proxy: SeqProxy,
    tx_id: u64,
    fee_level: u64,
) -> FeeQueueEntry<String> {
    FeeQueueEntry::new(
        FeeQueueKey::new(account, seq_proxy),
        QueueAdvanceCandidate {
            fee_level,
            tx_id: Uint256::from_u64(tx_id),
            seq_proxy,
        },
    )
}

fn build_full_queue_owner_shell()
-> QueueApplyOwnerShell<String, &'static str, &'static str, &'static str> {
    let account_a = String::from("a");
    let account_b = String::from("b");

    let mut queued_a = TxQAccount::new(account_a.clone());
    queued_a.add(
        SeqProxy::sequence(5),
        MaybeTxCore::new(
            queued(account_a.clone(), SeqProxy::sequence(5), 5, 90),
            TxConsequences::new(1, SeqProxy::sequence(5)),
        ),
    );

    let mut queued_b = TxQAccount::new(account_b.clone());
    queued_b.add(
        SeqProxy::sequence(8),
        MaybeTxCore::new(
            queued(account_b.clone(), SeqProxy::sequence(8), 8, 50),
            TxConsequences::new(1, SeqProxy::sequence(8)),
        ),
    );

    QueueApplyOwnerShell::new(QueueApplyLiveOwner::new(
        2,
        10,
        25,
        Some(2),
        OrderCandidates::new(Uint256::from_u64(0)),
        QueueViews::new(
            BTreeMap::from([(account_a.clone(), queued_a), (account_b.clone(), queued_b)]),
            vec![
                fee_entry(account_a, SeqProxy::sequence(5), 5, 90),
                fee_entry(account_b, SeqProxy::sequence(8), 8, 50),
            ],
        ),
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

fn queueing_runtime() -> TestLogSinkRuntime {
    TestLogSinkRuntime {
        preflight_result: queueing_preflight_result(SeqProxy::sequence(6)),
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

fn direct_runtime() -> TestLogSinkRuntime {
    TestLogSinkRuntime {
        preflight_result: queueing_preflight_result(SeqProxy::sequence(5)),
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
fn queue_apply_runtime_envelope_matches_call_envelope_apply() {
    let account = String::from("acct");
    let tx_source = tx_source(&account);
    let view_source = view_source();
    let call = QueueApplyCallEnvelope::new(&tx_source, &view_source);

    let mut direct_shell = build_owner_shell(Some(10));
    let direct_stage = call.apply(
        &mut direct_shell,
        hold_preflight(),
        ApplyFlags::NONE,
        blocker_consequences(SeqProxy::sequence(6)),
        Ter::TES_SUCCESS,
        || preflight_result(SeqProxy::sequence(6)),
        |_| unreachable!("direct apply should fall through"),
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

    let mut runtime = queueing_runtime();
    let mut runtime_shell = build_owner_shell(Some(10));
    let runtime_stage = QueueApplyRuntimeEnvelope::new(&mut runtime).apply(
        &mut runtime_shell,
        &call,
        hold_preflight(),
        ApplyFlags::NONE,
        blocker_consequences(SeqProxy::sequence(6)),
        Ter::TES_SUCCESS,
    );

    assert_eq!(runtime_stage, direct_stage);
    assert_eq!(runtime_shell.owner(), direct_shell.owner());
    assert_eq!(runtime_stage, expected_stage());
    assert_eq!(runtime.preflight_calls, 1);
    assert_eq!(runtime.direct_apply_calls, 0);
    assert_eq!(runtime.prepare_multitxn_calls, 0);
    assert_eq!(runtime.preclaim_calls, 0);
    assert_eq!(runtime.try_clear_calls, 0);
    assert_eq!(runtime.apply_sandbox_calls, 0);
    assert!(runtime.trace_messages.is_empty());
}

#[test]
fn queue_apply_runtime_envelope_matches_call_envelope_after_preflight() {
    let account = String::from("acct");
    let tx_source = tx_source(&account);
    let view_source = view_source();
    let call = QueueApplyCallEnvelope::new(&tx_source, &view_source);
    let preflight = preflight_result(SeqProxy::sequence(6));

    let mut direct_shell = build_owner_shell(Some(10));
    let direct_stage = call.apply_after_preflight(
        &mut direct_shell,
        &preflight,
        hold_preflight(),
        Ter::TES_SUCCESS,
        |_| unreachable!("direct apply should fall through"),
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

    let mut runtime = queueing_runtime();
    let mut runtime_shell = build_owner_shell(Some(10));
    let runtime_stage = QueueApplyRuntimeEnvelope::new(&mut runtime).apply_after_preflight(
        &mut runtime_shell,
        &call,
        &preflight,
        hold_preflight(),
        Ter::TES_SUCCESS,
    );

    assert_eq!(runtime_stage, direct_stage);
    assert_eq!(runtime_shell.owner(), direct_shell.owner());
    assert_eq!(runtime_stage, expected_stage());
    assert_eq!(runtime.preflight_calls, 0);
    assert_eq!(runtime.direct_apply_calls, 0);
    assert_eq!(runtime.prepare_multitxn_calls, 0);
    assert_eq!(runtime.preclaim_calls, 0);
    assert_eq!(runtime.try_clear_calls, 0);
    assert_eq!(runtime.apply_sandbox_calls, 0);
    assert!(runtime.trace_messages.is_empty());
}

#[test]
fn queue_apply_runtime_envelope_exposes_runtime_accessor() {
    let mut runtime = queueing_runtime();
    let mut envelope = QueueApplyRuntimeEnvelope::new(&mut runtime);

    envelope.runtime().trace("hello");

    assert_eq!(runtime.trace_messages, vec![String::from("hello")]);
}

#[test]
fn queue_apply_runtime_envelope_log_sinks_match_call_envelope_on_full_queue_path() {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(6),
    };
    let view_source = queueing_view(&account);
    let call = QueueApplyCallEnvelope::new(&tx_source, &view_source);

    let mut direct_shell = build_full_queue_owner_shell();
    let direct_emitted = RefCell::new(Vec::new());
    let direct_stage = call.apply_with_log_sinks(
        &mut direct_shell,
        hold_preflight(),
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(6)),
        Ter::TES_SUCCESS,
        || queueing_preflight_result(SeqProxy::sequence(6)),
        |message| direct_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| direct_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| direct_emitted.borrow_mut().push(format!("info:{message}")),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |_preclaim_view| {
            PreclaimResult::new(
                100,
                "tx",
                None::<&str>,
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            )
        },
        || -> ApplyResult { unreachable!("full queue path should not hit try-clear") },
        || unreachable!("full queue path should not hit sandbox apply"),
    );

    let mut runtime = queueing_runtime();
    let mut runtime_shell = build_full_queue_owner_shell();
    let runtime_emitted = RefCell::new(Vec::new());
    let runtime_stage = QueueApplyRuntimeEnvelope::new(&mut runtime).apply_with_log_sinks(
        &mut runtime_shell,
        &call,
        hold_preflight(),
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(6)),
        Ter::TES_SUCCESS,
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
    );

    let expected_emitted = vec![
        format!(
            "info:{}",
            format_queue_apply_full_queue_evict_info_message("b", 50, Uint256::from_u64(9), 256)
        ),
        format!(
            "debug:{}",
            format_queue_apply_enqueue_debug_message(
                Uint256::from_u64(9),
                Ter::TES_SUCCESS,
                true,
                "a",
                ApplyFlags::FAIL_HARD
            )
        ),
    ];
    let direct_emitted = direct_emitted.into_inner();
    let runtime_emitted = runtime_emitted.into_inner();

    assert_eq!(runtime_stage, direct_stage);
    assert_eq!(runtime_emitted, direct_emitted);
    assert_eq!(runtime_emitted, expected_emitted);
    assert_eq!(runtime.preflight_calls, 1);
    assert_eq!(runtime.direct_apply_calls, 0);
    assert_eq!(runtime.prepare_multitxn_calls, 1);
    assert_eq!(runtime.preclaim_calls, 1);
    assert_eq!(runtime.try_clear_calls, 0);
    assert_eq!(runtime.apply_sandbox_calls, 0);
    assert!(runtime.trace_messages.is_empty());
}

#[test]
fn queue_apply_runtime_envelope_returned_log_messages_match_call_envelope_on_full_queue_path() {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(6),
    };
    let view_source = queueing_view(&account);
    let call = QueueApplyCallEnvelope::new(&tx_source, &view_source);

    let mut direct_shell = build_full_queue_owner_shell();
    let direct_stage = call.apply_with_log_messages(
        &mut direct_shell,
        hold_preflight(),
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(6)),
        Ter::TES_SUCCESS,
        || queueing_preflight_result(SeqProxy::sequence(6)),
        |_| unreachable!("full queue path should not hit direct apply trace"),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |_preclaim_view| {
            PreclaimResult::new(
                100,
                "tx",
                None::<&str>,
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            )
        },
        || -> ApplyResult { unreachable!("full queue path should not hit try-clear") },
        || unreachable!("full queue path should not hit sandbox apply"),
    );

    let mut runtime = queueing_runtime();
    let mut runtime_shell = build_full_queue_owner_shell();
    let runtime_stage = QueueApplyRuntimeEnvelope::new(&mut runtime).apply_with_log_messages(
        &mut runtime_shell,
        &call,
        hold_preflight(),
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(6)),
        Ter::TES_SUCCESS,
    );

    assert_eq!(runtime_stage, direct_stage);
    assert_eq!(runtime_shell.owner(), direct_shell.owner());
    assert_eq!(runtime.preflight_calls, 1);
    assert_eq!(runtime.direct_apply_calls, 0);
    assert_eq!(runtime.prepare_multitxn_calls, 1);
    assert_eq!(runtime.preclaim_calls, 1);
    assert_eq!(runtime.try_clear_calls, 0);
    assert_eq!(runtime.apply_sandbox_calls, 0);
    assert!(runtime.trace_messages.is_empty());
}

#[test]
fn queue_apply_runtime_envelope_derived_preflight_facts_log_messages_match_call_envelope_on_full_queue_path()
 {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(6),
    };
    let view_source = queueing_view(&account);
    let call = QueueApplyCallEnvelope::new(&tx_source, &view_source);

    let mut direct_shell = build_full_queue_owner_shell();
    let direct_stage = call.apply_with_derived_preflight_facts_and_log_messages(
        &mut direct_shell,
        Ter::TES_SUCCESS,
        || queueing_preflight_result(SeqProxy::sequence(6)),
        |_| unreachable!("full queue path should not hit direct apply trace"),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |_preclaim_view| {
            PreclaimResult::new(
                100,
                "tx",
                None::<&str>,
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            )
        },
        || -> ApplyResult { unreachable!("full queue path should not hit try-clear") },
        || unreachable!("full queue path should not hit sandbox apply"),
    );

    let mut runtime = queueing_runtime();
    let mut runtime_shell = build_full_queue_owner_shell();
    let runtime_stage = QueueApplyRuntimeEnvelope::new(&mut runtime)
        .apply_with_derived_preflight_facts_and_log_messages(
            &mut runtime_shell,
            &call,
            Ter::TES_SUCCESS,
        );

    assert_eq!(runtime_stage, direct_stage);
    assert_eq!(runtime_shell.owner(), direct_shell.owner());
    assert_eq!(runtime.preflight_calls, 1);
    assert_eq!(runtime.direct_apply_calls, 0);
    assert_eq!(runtime.prepare_multitxn_calls, 0);
    assert_eq!(runtime.preclaim_calls, 0);
    assert_eq!(runtime.try_clear_calls, 0);
    assert_eq!(runtime.apply_sandbox_calls, 0);
    assert!(runtime.trace_messages.is_empty());
}

#[test]
fn queue_apply_runtime_envelope_derived_preflight_and_hold_admission_log_messages_match_call_envelope_on_full_queue_path()
 {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(6),
    };
    let view_source = queueing_view(&account);
    let call = QueueApplyCallEnvelope::new(&tx_source, &view_source);

    let mut direct_shell = build_full_queue_owner_shell();
    let direct_stage = call.apply_with_derived_preflight_facts_and_hold_admission_and_log_messages(
        &mut direct_shell,
        || queueing_preflight_result(SeqProxy::sequence(6)),
        |_| unreachable!("full queue path should not hit direct apply trace"),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |_preclaim_view| {
            PreclaimResult::new(
                100,
                "tx",
                None::<&str>,
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            )
        },
        || -> ApplyResult { unreachable!("full queue path should not hit try-clear") },
        || unreachable!("full queue path should not hit sandbox apply"),
    );

    let mut runtime = queueing_runtime();
    let mut runtime_shell = build_full_queue_owner_shell();
    let runtime_stage = QueueApplyRuntimeEnvelope::new(&mut runtime)
        .apply_with_derived_preflight_facts_and_hold_admission_and_log_messages(
            &mut runtime_shell,
            &call,
        );

    assert_eq!(runtime_stage, direct_stage);
    assert_eq!(runtime_shell.owner(), direct_shell.owner());
    assert_eq!(runtime.preflight_calls, 1);
    assert_eq!(runtime.direct_apply_calls, 0);
    assert_eq!(runtime.prepare_multitxn_calls, 0);
    assert_eq!(runtime.preclaim_calls, 0);
    assert_eq!(runtime.try_clear_calls, 0);
    assert_eq!(runtime.apply_sandbox_calls, 0);
    assert!(runtime.trace_messages.is_empty());
}

#[test]
fn queue_apply_runtime_envelope_derived_hold_preflight_log_messages_match_call_envelope_on_direct_path()
 {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let view_source = direct_view(&account);
    let call = QueueApplyCallEnvelope::new(&tx_source, &view_source);

    let mut direct_shell = build_direct_owner_shell();
    let direct_stage = call.apply_with_derived_hold_preflight_and_log_messages(
        &mut direct_shell,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(5)),
        Ter::TES_SUCCESS,
        || queueing_preflight_result(SeqProxy::sequence(5)),
        |_| {},
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |_preclaim_view| {
            PreclaimResult::new(
                100,
                "tx",
                None::<&str>,
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            )
        },
        || -> ApplyResult { unreachable!("derived hold preflight path should not hit try-clear") },
        || unreachable!("derived hold preflight path should not hit sandbox apply"),
    );

    let mut runtime = direct_runtime();
    let mut runtime_shell = build_direct_owner_shell();
    let runtime_stage = QueueApplyRuntimeEnvelope::new(&mut runtime)
        .apply_with_derived_hold_preflight_and_log_messages(
            &mut runtime_shell,
            &call,
            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
            TxConsequences::new(1, SeqProxy::sequence(5)),
            Ter::TES_SUCCESS,
        );

    assert_eq!(runtime_stage, direct_stage);
    assert_eq!(runtime_shell.owner(), direct_shell.owner());
    assert_eq!(runtime.preflight_calls, 1);
    assert_eq!(runtime.direct_apply_calls, 1);
    assert_eq!(runtime.prepare_multitxn_calls, 0);
    assert_eq!(runtime.preclaim_calls, 0);
    assert_eq!(runtime.try_clear_calls, 0);
    assert_eq!(runtime.apply_sandbox_calls, 0);
    assert_eq!(
        runtime.trace_messages,
        vec![
            format_direct_apply_start_log_message("ABC123"),
            format_direct_apply_finish_log_message(&tx::DirectApplyExecution::<String, _> {
                transaction_id: "ABC123",
                attempt: tx::DirectApplyAttemptResult {
                    apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, true),
                    removed_replacement: None,
                },
            }),
        ]
    );
}

#[test]
fn queue_apply_runtime_envelope_derived_hold_admission_log_messages_match_call_envelope_on_direct_path()
 {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let view_source = direct_view(&account);
    let call = QueueApplyCallEnvelope::new(&tx_source, &view_source);
    let hold_preflight = QueueHoldPreflight::new(
        false,
        false,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        Some(250),
    );

    let mut direct_shell = build_direct_owner_shell();
    let direct_stage = call.apply_with_derived_hold_admission_and_log_messages(
        &mut direct_shell,
        hold_preflight,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(5)),
        || queueing_preflight_result(SeqProxy::sequence(5)),
        |_| {},
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |_preclaim_view| {
            PreclaimResult::new(
                100,
                "tx",
                None::<&str>,
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            )
        },
        || -> ApplyResult { unreachable!("derived hold admission path should not hit try-clear") },
        || unreachable!("derived hold admission path should not hit sandbox apply"),
    );

    let mut runtime = direct_runtime();
    let mut runtime_shell = build_direct_owner_shell();
    let runtime_stage = QueueApplyRuntimeEnvelope::new(&mut runtime)
        .apply_with_derived_hold_admission_and_log_messages(
            &mut runtime_shell,
            &call,
            hold_preflight,
            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
            TxConsequences::new(1, SeqProxy::sequence(5)),
        );

    assert_eq!(runtime_stage, direct_stage);
    assert_eq!(runtime_shell.owner(), direct_shell.owner());
    assert_eq!(runtime.preflight_calls, 1);
    assert_eq!(runtime.direct_apply_calls, 1);
    assert_eq!(runtime.prepare_multitxn_calls, 0);
    assert_eq!(runtime.preclaim_calls, 0);
    assert_eq!(runtime.try_clear_calls, 0);
    assert_eq!(runtime.apply_sandbox_calls, 0);
    assert_eq!(
        runtime.trace_messages,
        vec![
            format_direct_apply_start_log_message("ABC123"),
            format_direct_apply_finish_log_message(&tx::DirectApplyExecution::<String, _> {
                transaction_id: "ABC123",
                attempt: tx::DirectApplyAttemptResult {
                    apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, true),
                    removed_replacement: None,
                },
            }),
        ]
    );
}

#[test]
fn queue_apply_runtime_envelope_owned_metrics_log_messages_match_call_envelope_on_owner_snapshot_path()
 {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(6),
    };
    let mut view_source = direct_view(&account);
    view_source.metrics_snapshot = QueueFeeMetricsSnapshot {
        txns_expected: 256,
        escalation_multiplier: tx::TXQ_BASE_LEVEL,
    };
    view_source.open_ledger_tx_count = 40;
    let call = QueueApplyCallEnvelope::new(&tx_source, &view_source);

    let mut direct_shell = build_direct_owner_shell();
    let direct_stage = call.apply_with_owned_metrics_and_log_messages(
        &mut direct_shell,
        hold_preflight(),
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(6)),
        Ter::TES_SUCCESS,
        || queueing_preflight_result(SeqProxy::sequence(6)),
        |_| unreachable!("owner metrics path should not direct apply"),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |_preclaim_view| {
            PreclaimResult::new(
                100,
                "tx",
                None::<&str>,
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            )
        },
        || -> ApplyResult { unreachable!("owner metrics path should reject before try-clear") },
        || unreachable!("owner metrics path should reject before sandbox apply"),
    );

    let mut runtime = direct_runtime();
    let mut runtime_shell = build_direct_owner_shell();
    let runtime_stage = QueueApplyRuntimeEnvelope::new(&mut runtime)
        .apply_with_owned_metrics_and_log_messages(
            &mut runtime_shell,
            &call,
            hold_preflight(),
            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
            TxConsequences::new(1, SeqProxy::sequence(6)),
            Ter::TES_SUCCESS,
        );

    assert_eq!(runtime_stage, direct_stage);
    assert_eq!(runtime_shell.owner(), direct_shell.owner());
    assert_eq!(runtime.preflight_calls, 1);
    assert_eq!(runtime.direct_apply_calls, 0);
    assert_eq!(runtime.prepare_multitxn_calls, 0);
    assert_eq!(runtime.preclaim_calls, 0);
    assert_eq!(runtime.try_clear_calls, 0);
    assert_eq!(runtime.apply_sandbox_calls, 0);
    assert!(runtime.trace_messages.is_empty());
}

#[test]
fn queue_apply_runtime_envelope_owned_metrics_derived_preflight_facts_log_messages_match_call_envelope_on_full_queue_path()
 {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(6),
    };
    let mut view_source = queueing_view(&account);
    view_source.metrics_snapshot = QueueFeeMetricsSnapshot {
        txns_expected: 256,
        escalation_multiplier: tx::TXQ_BASE_LEVEL,
    };
    view_source.open_ledger_tx_count = 40;
    let call = QueueApplyCallEnvelope::new(&tx_source, &view_source);

    let mut direct_shell = build_full_queue_owner_shell();
    let direct_stage = call.apply_with_owned_metrics_and_derived_preflight_facts_and_log_messages(
        &mut direct_shell,
        || preflight_result(SeqProxy::sequence(6)),
        |_| unreachable!("full queue path should not hit direct apply trace"),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |_preclaim_view| {
            PreclaimResult::new(
                100,
                "tx",
                None::<&str>,
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            )
        },
        || -> ApplyResult { unreachable!("full queue path should not hit try-clear") },
        || unreachable!("full queue path should not hit sandbox apply"),
    );

    let mut runtime = TestApplyRuntime::default();
    let mut runtime_shell = build_full_queue_owner_shell();
    let runtime_stage = QueueApplyRuntimeEnvelope::new(&mut runtime)
        .apply_with_owned_metrics_and_derived_preflight_facts_and_log_messages(
            &mut runtime_shell,
            &call,
        );

    assert_eq!(runtime_stage, direct_stage);
    assert_eq!(runtime_shell.owner(), direct_shell.owner());
    assert_eq!(runtime.preflight_calls, 1);
    assert_eq!(runtime.direct_apply_calls, 0);
    assert_eq!(runtime.prepare_multitxn_calls, 0);
    assert_eq!(runtime.preclaim_calls, 0);
    assert_eq!(runtime.try_clear_calls, 0);
    assert_eq!(runtime.apply_sandbox_calls, 0);
    assert!(runtime.trace_messages.is_empty());
}

#[test]
fn queue_apply_runtime_envelope_owned_metrics_derived_preflight_and_hold_admission_log_messages_match_call_envelope_on_full_queue_path()
 {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(6),
    };
    let mut view_source = queueing_view(&account);
    view_source.metrics_snapshot = QueueFeeMetricsSnapshot {
        txns_expected: 256,
        escalation_multiplier: tx::TXQ_BASE_LEVEL,
    };
    view_source.open_ledger_tx_count = 40;
    let call = QueueApplyCallEnvelope::new(&tx_source, &view_source);

    let mut direct_shell = build_full_queue_owner_shell();
    let direct_stage = call
        .apply_with_owned_metrics_and_derived_preflight_facts_and_hold_admission_and_log_messages(
            &mut direct_shell,
            || preflight_result(SeqProxy::sequence(6)),
            |_| unreachable!("full queue path should not hit direct apply trace"),
            || ApplyResult::new(Ter::TES_SUCCESS, true, true),
            |_| true,
            |_preclaim_view| {
                PreclaimResult::new(
                    100,
                    "tx",
                    None::<&str>,
                    ApplyFlags::NONE,
                    "journal",
                    Ter::TES_SUCCESS,
                )
            },
            || -> ApplyResult { unreachable!("full queue path should not hit try-clear") },
            || unreachable!("full queue path should not hit sandbox apply"),
        );

    let mut runtime = TestApplyRuntime::default();
    let mut runtime_shell = build_full_queue_owner_shell();
    let runtime_stage = QueueApplyRuntimeEnvelope::new(&mut runtime)
        .apply_with_owned_metrics_and_derived_preflight_facts_and_hold_admission_and_log_messages(
            &mut runtime_shell,
            &call,
        );

    assert_eq!(runtime_stage, direct_stage);
    assert_eq!(runtime_shell.owner(), direct_shell.owner());
    assert_eq!(runtime.preflight_calls, 1);
    assert_eq!(runtime.direct_apply_calls, 0);
    assert_eq!(runtime.prepare_multitxn_calls, 0);
    assert_eq!(runtime.preclaim_calls, 0);
    assert_eq!(runtime.try_clear_calls, 0);
    assert_eq!(runtime.apply_sandbox_calls, 0);
    assert!(runtime.trace_messages.is_empty());
}

#[test]
fn queue_apply_runtime_envelope_after_preflight_log_sinks_match_call_envelope_on_direct_path() {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let view_source = direct_view(&account);
    let call = QueueApplyCallEnvelope::new(&tx_source, &view_source);
    let preflight = queueing_preflight_result(SeqProxy::sequence(5));

    let mut direct_shell = build_direct_owner_shell();
    let direct_emitted = RefCell::new(Vec::new());
    let direct_stage = call.apply_after_preflight_with_log_sinks(
        &mut direct_shell,
        &preflight,
        hold_preflight(),
        Ter::TES_SUCCESS,
        |message| direct_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| direct_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| direct_emitted.borrow_mut().push(format!("info:{message}")),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |_preclaim_view| {
            PreclaimResult::new(
                100,
                "tx",
                None::<&str>,
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            )
        },
        || -> ApplyResult { unreachable!("direct apply path should not hit try-clear") },
        || unreachable!("direct apply path should not hit sandbox apply"),
    );

    let mut runtime = TestApplyRuntime::default();
    let mut runtime_shell = build_direct_owner_shell();
    let runtime_emitted = RefCell::new(Vec::new());
    let runtime_stage = QueueApplyRuntimeEnvelope::new(&mut runtime)
        .apply_after_preflight_with_log_sinks(
            &mut runtime_shell,
            &call,
            &preflight,
            hold_preflight(),
            Ter::TES_SUCCESS,
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
        );

    let expected_emitted = vec![
        format!("trace:{}", format_direct_apply_start_log_message("ABC123")),
        format!(
            "trace:{}",
            format_direct_apply_finish_log_message(&tx::DirectApplyExecution::<String, _> {
                transaction_id: "ABC123",
                attempt: tx::DirectApplyAttemptResult {
                    apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, true),
                    removed_replacement: None,
                },
            })
        ),
    ];
    let direct_emitted = direct_emitted.into_inner();
    let runtime_emitted = runtime_emitted.into_inner();

    assert_eq!(runtime_stage, direct_stage);
    assert_eq!(runtime_shell.owner(), direct_shell.owner());
    assert_eq!(runtime_emitted, direct_emitted);
    assert_eq!(runtime_emitted, expected_emitted);
    assert_eq!(runtime.preflight_calls, 0);
    assert_eq!(runtime.direct_apply_calls, 1);
    assert_eq!(runtime.prepare_multitxn_calls, 0);
    assert_eq!(runtime.preclaim_calls, 0);
    assert_eq!(runtime.try_clear_calls, 0);
    assert_eq!(runtime.apply_sandbox_calls, 0);
    assert!(runtime.trace_messages.is_empty());
}

#[test]
fn queue_apply_runtime_envelope_after_preflight_returned_log_messages_match_call_envelope_on_direct_path()
 {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let view_source = direct_view(&account);
    let call = QueueApplyCallEnvelope::new(&tx_source, &view_source);
    let preflight = queueing_preflight_result(SeqProxy::sequence(5));

    let mut direct_shell = build_direct_owner_shell();
    let direct_stage = call.apply_after_preflight_with_log_messages(
        &mut direct_shell,
        &preflight,
        hold_preflight(),
        Ter::TES_SUCCESS,
        |_| {},
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |_preclaim_view| {
            PreclaimResult::new(
                100,
                "tx",
                None::<&str>,
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            )
        },
        || -> ApplyResult { unreachable!("direct apply path should not hit try-clear") },
        || unreachable!("direct apply path should not hit sandbox apply"),
    );

    let mut runtime = TestApplyRuntime::default();
    let mut runtime_shell = build_direct_owner_shell();
    let runtime_stage = QueueApplyRuntimeEnvelope::new(&mut runtime)
        .apply_after_preflight_with_log_messages(
            &mut runtime_shell,
            &call,
            &preflight,
            hold_preflight(),
            Ter::TES_SUCCESS,
        );

    assert_eq!(runtime_stage, direct_stage);
    assert_eq!(runtime_shell.owner(), direct_shell.owner());
    assert_eq!(runtime.preflight_calls, 0);
    assert_eq!(runtime.direct_apply_calls, 1);
    assert_eq!(runtime.prepare_multitxn_calls, 0);
    assert_eq!(runtime.preclaim_calls, 0);
    assert_eq!(runtime.try_clear_calls, 0);
    assert_eq!(runtime.apply_sandbox_calls, 0);
    assert_eq!(
        runtime.trace_messages,
        vec![
            format_direct_apply_start_log_message("ABC123"),
            format_direct_apply_finish_log_message(&tx::DirectApplyExecution::<String, _> {
                transaction_id: "ABC123",
                attempt: tx::DirectApplyAttemptResult {
                    apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, true),
                    removed_replacement: None,
                },
            }),
        ]
    );
}

#[test]
fn queue_apply_runtime_envelope_owned_metrics_derived_hold_preflight_log_messages_match_call_envelope_on_owner_snapshot_path()
 {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let mut view_source = direct_view(&account);
    view_source.metrics_snapshot = QueueFeeMetricsSnapshot {
        txns_expected: 256,
        escalation_multiplier: tx::TXQ_BASE_LEVEL,
    };
    view_source.open_ledger_tx_count = 40;
    let call = QueueApplyCallEnvelope::new(&tx_source, &view_source);

    let mut direct_shell = build_direct_owner_shell();
    let direct_stage = call.apply_with_owned_metrics_and_derived_hold_preflight_and_log_messages(
        &mut direct_shell,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(5)),
        Ter::TES_SUCCESS,
        || queueing_preflight_result(SeqProxy::sequence(5)),
        |_| unreachable!("owner metrics path should not direct apply"),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |_preclaim_view| {
            PreclaimResult::new(
                100,
                "tx",
                None::<&str>,
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            )
        },
        || -> ApplyResult { unreachable!("owner metrics path should reject before try-clear") },
        || unreachable!("owner metrics path should reject before sandbox apply"),
    );

    let mut runtime = direct_runtime();
    let mut runtime_shell = build_direct_owner_shell();
    let runtime_stage = QueueApplyRuntimeEnvelope::new(&mut runtime)
        .apply_with_owned_metrics_and_derived_hold_preflight_and_log_messages(
            &mut runtime_shell,
            &call,
            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
            TxConsequences::new(1, SeqProxy::sequence(5)),
            Ter::TES_SUCCESS,
        );

    assert_eq!(runtime_stage, direct_stage);
    assert_eq!(runtime_shell.owner(), direct_shell.owner());
    assert_eq!(runtime.preflight_calls, 1);
    assert_eq!(runtime.direct_apply_calls, 0);
    assert_eq!(runtime.prepare_multitxn_calls, 0);
    assert_eq!(runtime.preclaim_calls, 1);
    assert_eq!(runtime.try_clear_calls, 0);
    assert_eq!(runtime.apply_sandbox_calls, 0);
    assert!(runtime.trace_messages.is_empty());
}

#[test]
fn queue_apply_runtime_envelope_after_preflight_owned_metrics_derived_hold_preflight_log_messages_match_call_envelope_on_owner_snapshot_path()
 {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let mut view_source = direct_view(&account);
    view_source.metrics_snapshot = QueueFeeMetricsSnapshot {
        txns_expected: 256,
        escalation_multiplier: tx::TXQ_BASE_LEVEL,
    };
    view_source.open_ledger_tx_count = 40;
    let call = QueueApplyCallEnvelope::new(&tx_source, &view_source);
    let preflight = queueing_preflight_result(SeqProxy::sequence(5));

    let mut direct_shell = build_direct_owner_shell();
    let direct_stage = call
        .apply_after_preflight_with_owned_metrics_and_derived_hold_preflight_and_log_messages(
            &mut direct_shell,
            &preflight,
            Ter::TES_SUCCESS,
            |_| unreachable!("owner metrics path should not direct apply"),
            || ApplyResult::new(Ter::TES_SUCCESS, true, true),
            |_| true,
            |_preclaim_view| {
                PreclaimResult::new(
                    100,
                    "tx",
                    None::<&str>,
                    ApplyFlags::NONE,
                    "journal",
                    Ter::TES_SUCCESS,
                )
            },
            || -> ApplyResult { unreachable!("owner metrics path should reject before try-clear") },
            || unreachable!("owner metrics path should reject before sandbox apply"),
        );

    let mut runtime = direct_runtime();
    let mut runtime_shell = build_direct_owner_shell();
    let runtime_stage = QueueApplyRuntimeEnvelope::new(&mut runtime)
        .apply_after_preflight_with_owned_metrics_and_derived_hold_preflight_and_log_messages(
            &mut runtime_shell,
            &call,
            &preflight,
            Ter::TES_SUCCESS,
        );

    assert_eq!(runtime_stage, direct_stage);
    assert_eq!(runtime_shell.owner(), direct_shell.owner());
    assert_eq!(runtime.preflight_calls, 0);
    assert_eq!(runtime.direct_apply_calls, 0);
    assert_eq!(runtime.prepare_multitxn_calls, 0);
    assert_eq!(runtime.preclaim_calls, 1);
    assert_eq!(runtime.try_clear_calls, 0);
    assert_eq!(runtime.apply_sandbox_calls, 0);
    assert!(runtime.trace_messages.is_empty());
}

#[test]
fn queue_apply_runtime_envelope_owned_metrics_log_sinks_match_call_envelope_on_owner_snapshot_path()
{
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let mut view_source = direct_view(&account);
    view_source.metrics_snapshot = QueueFeeMetricsSnapshot {
        txns_expected: 256,
        escalation_multiplier: tx::TXQ_BASE_LEVEL,
    };
    view_source.open_ledger_tx_count = 40;
    let call = QueueApplyCallEnvelope::new(&tx_source, &view_source);

    let mut direct_shell = build_direct_owner_shell();
    let direct_emitted = RefCell::new(Vec::new());
    let direct_stage = call.apply_with_owned_metrics_and_log_sinks(
        &mut direct_shell,
        hold_preflight(),
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(5)),
        Ter::TES_SUCCESS,
        || queueing_preflight_result(SeqProxy::sequence(5)),
        |_| unreachable!("owner metrics path should not direct apply"),
        |message| direct_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| direct_emitted.borrow_mut().push(format!("info:{message}")),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |_preclaim_view| {
            PreclaimResult::new(
                100,
                "tx",
                None::<&str>,
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            )
        },
        || -> ApplyResult { unreachable!("owner metrics path should reject before try-clear") },
        || unreachable!("owner metrics path should reject before sandbox apply"),
    );

    let mut runtime = direct_runtime();
    let mut runtime_shell = build_direct_owner_shell();
    let runtime_emitted = RefCell::new(Vec::new());
    let runtime_stage = QueueApplyRuntimeEnvelope::new(&mut runtime)
        .apply_with_owned_metrics_and_log_sinks(
            &mut runtime_shell,
            &call,
            hold_preflight(),
            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
            TxConsequences::new(1, SeqProxy::sequence(5)),
            Ter::TES_SUCCESS,
            |_| unreachable!("owner metrics path should not direct apply"),
            |message| {
                runtime_emitted
                    .borrow_mut()
                    .push(format!("debug:{message}"))
            },
            |message| runtime_emitted.borrow_mut().push(format!("info:{message}")),
        );

    assert_eq!(runtime_stage, direct_stage);
    assert_eq!(runtime_shell.owner(), direct_shell.owner());
    assert_eq!(runtime_emitted.into_inner(), direct_emitted.into_inner());
    assert_eq!(runtime.preflight_calls, 1);
    assert_eq!(runtime.direct_apply_calls, 0);
    assert_eq!(runtime.prepare_multitxn_calls, 0);
    assert_eq!(runtime.preclaim_calls, 1);
    assert_eq!(runtime.try_clear_calls, 0);
    assert_eq!(runtime.apply_sandbox_calls, 0);
    assert!(runtime.trace_messages.is_empty());
}

#[test]
fn queue_apply_runtime_envelope_after_preflight_owned_metrics_log_sinks_match_call_envelope_on_owner_snapshot_path()
 {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let mut view_source = direct_view(&account);
    view_source.metrics_snapshot = QueueFeeMetricsSnapshot {
        txns_expected: 256,
        escalation_multiplier: tx::TXQ_BASE_LEVEL,
    };
    view_source.open_ledger_tx_count = 40;
    let call = QueueApplyCallEnvelope::new(&tx_source, &view_source);
    let preflight = queueing_preflight_result(SeqProxy::sequence(5));

    let mut direct_shell = build_direct_owner_shell();
    let direct_emitted = RefCell::new(Vec::new());
    let direct_stage = call.apply_after_preflight_with_owned_metrics_and_log_sinks(
        &mut direct_shell,
        &preflight,
        hold_preflight(),
        Ter::TES_SUCCESS,
        |_| unreachable!("owner metrics path should not direct apply"),
        |message| direct_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| direct_emitted.borrow_mut().push(format!("info:{message}")),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |_preclaim_view| {
            PreclaimResult::new(
                100,
                "tx",
                None::<&str>,
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            )
        },
        || -> ApplyResult { unreachable!("owner metrics path should reject before try-clear") },
        || unreachable!("owner metrics path should reject before sandbox apply"),
    );

    let mut runtime = direct_runtime();
    let mut runtime_shell = build_direct_owner_shell();
    let runtime_emitted = RefCell::new(Vec::new());
    let runtime_stage = QueueApplyRuntimeEnvelope::new(&mut runtime)
        .apply_after_preflight_with_owned_metrics_and_log_sinks(
            &mut runtime_shell,
            &call,
            &preflight,
            hold_preflight(),
            Ter::TES_SUCCESS,
            |_| unreachable!("owner metrics path should not direct apply"),
            |message| {
                runtime_emitted
                    .borrow_mut()
                    .push(format!("debug:{message}"))
            },
            |message| runtime_emitted.borrow_mut().push(format!("info:{message}")),
        );

    assert_eq!(runtime_stage, direct_stage);
    assert_eq!(runtime_shell.owner(), direct_shell.owner());
    assert_eq!(runtime_emitted.into_inner(), direct_emitted.into_inner());
    assert_eq!(runtime.preflight_calls, 0);
    assert_eq!(runtime.direct_apply_calls, 0);
    assert_eq!(runtime.prepare_multitxn_calls, 0);
    assert_eq!(runtime.preclaim_calls, 1);
    assert_eq!(runtime.try_clear_calls, 0);
    assert_eq!(runtime.apply_sandbox_calls, 0);
    assert!(runtime.trace_messages.is_empty());
}

#[test]
fn queue_apply_runtime_envelope_owned_metrics_caller_preclaim_match_call_envelope_on_owner_snapshot_path()
 {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let mut view_source = direct_view(&account);
    view_source.metrics_snapshot = QueueFeeMetricsSnapshot {
        txns_expected: 256,
        escalation_multiplier: tx::TXQ_BASE_LEVEL,
    };
    view_source.open_ledger_tx_count = 40;
    let call = QueueApplyCallEnvelope::new(&tx_source, &view_source);
    let expected_prepared = RefCell::new(None::<tx::QueueApplyPreparedPreclaimInputs<String>>);

    let mut direct_shell = build_direct_owner_shell();
    let direct_stage = call.apply_with_owned_metrics_and_caller_preclaim(
        &mut direct_shell,
        hold_preflight(),
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(5)),
        Ter::TES_SUCCESS,
        || queueing_preflight_result(SeqProxy::sequence(5)),
        |_| unreachable!("owner metrics path should not direct apply"),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |prepared| {
            expected_prepared.replace(Some(prepared.clone()));
            Ok(tx::QueueApplyPreclaimStage {
                view_source: prepared.view_source,
                trace_message: "trace".to_string(),
                preclaim_result: queueing_preflight_result(SeqProxy::sequence(5))
                    .to_preclaim(100, Ter::TES_SUCCESS),
            })
        },
        || -> ApplyResult { unreachable!("owner metrics path should reject before try-clear") },
        || unreachable!("owner metrics path should reject before sandbox apply"),
    );

    let mut runtime = direct_runtime();
    let mut runtime_shell = build_direct_owner_shell();
    let runtime_stage = QueueApplyRuntimeEnvelope::new(&mut runtime)
        .apply_with_owned_metrics_and_caller_preclaim(
            &mut runtime_shell,
            &call,
            hold_preflight(),
            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
            TxConsequences::new(1, SeqProxy::sequence(5)),
            Ter::TES_SUCCESS,
            |prepared| {
                assert_eq!(Some(prepared.clone()), *expected_prepared.borrow());
                Ok(tx::QueueApplyPreclaimStage {
                    view_source: prepared.view_source,
                    trace_message: "trace".to_string(),
                    preclaim_result: queueing_preflight_result(SeqProxy::sequence(5))
                        .to_preclaim(100, Ter::TES_SUCCESS),
                })
            },
        );

    assert_eq!(runtime_stage, direct_stage);
    assert_eq!(runtime_shell.owner(), direct_shell.owner());
    assert_eq!(runtime.preflight_calls, 1);
    assert_eq!(runtime.direct_apply_calls, 0);
    assert_eq!(runtime.prepare_multitxn_calls, 0);
    assert_eq!(runtime.preclaim_calls, 0);
    assert_eq!(runtime.try_clear_calls, 0);
    assert_eq!(runtime.apply_sandbox_calls, 0);
    assert!(runtime.trace_messages.is_empty());
}

#[test]
fn queue_apply_runtime_envelope_after_preflight_owned_metrics_caller_preclaim_match_call_envelope_on_owner_snapshot_path()
 {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let mut view_source = direct_view(&account);
    view_source.metrics_snapshot = QueueFeeMetricsSnapshot {
        txns_expected: 256,
        escalation_multiplier: tx::TXQ_BASE_LEVEL,
    };
    view_source.open_ledger_tx_count = 40;
    let call = QueueApplyCallEnvelope::new(&tx_source, &view_source);
    let preflight = queueing_preflight_result(SeqProxy::sequence(5));
    let expected_prepared = RefCell::new(None::<tx::QueueApplyPreparedPreclaimInputs<String>>);

    let mut direct_shell = build_direct_owner_shell();
    let direct_stage = call.apply_after_preflight_with_owned_metrics_and_caller_preclaim(
        &mut direct_shell,
        &preflight,
        hold_preflight(),
        Ter::TES_SUCCESS,
        |_| unreachable!("owner metrics path should not direct apply"),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |prepared| {
            expected_prepared.replace(Some(prepared.clone()));
            Ok(tx::QueueApplyPreclaimStage {
                view_source: prepared.view_source,
                trace_message: "trace".to_string(),
                preclaim_result: queueing_preflight_result(SeqProxy::sequence(5))
                    .to_preclaim(100, Ter::TES_SUCCESS),
            })
        },
        || -> ApplyResult { unreachable!("owner metrics path should reject before try-clear") },
        || unreachable!("owner metrics path should reject before sandbox apply"),
    );

    let mut runtime = direct_runtime();
    let mut runtime_shell = build_direct_owner_shell();
    let runtime_stage = QueueApplyRuntimeEnvelope::new(&mut runtime)
        .apply_after_preflight_with_owned_metrics_and_caller_preclaim(
            &mut runtime_shell,
            &call,
            &preflight,
            hold_preflight(),
            Ter::TES_SUCCESS,
            |prepared| {
                assert_eq!(Some(prepared.clone()), *expected_prepared.borrow());
                Ok(tx::QueueApplyPreclaimStage {
                    view_source: prepared.view_source,
                    trace_message: "trace".to_string(),
                    preclaim_result: queueing_preflight_result(SeqProxy::sequence(5))
                        .to_preclaim(100, Ter::TES_SUCCESS),
                })
            },
        );

    assert_eq!(runtime_stage, direct_stage);
    assert_eq!(runtime_shell.owner(), direct_shell.owner());
    assert_eq!(runtime.preflight_calls, 0);
    assert_eq!(runtime.direct_apply_calls, 0);
    assert_eq!(runtime.prepare_multitxn_calls, 0);
    assert_eq!(runtime.preclaim_calls, 0);
    assert_eq!(runtime.try_clear_calls, 0);
    assert_eq!(runtime.apply_sandbox_calls, 0);
    assert!(runtime.trace_messages.is_empty());
}

#[test]
fn queue_apply_runtime_envelope_owned_metrics_caller_preclaim_log_messages_match_call_envelope_on_owner_snapshot_path()
 {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let mut view_source = direct_view(&account);
    view_source.metrics_snapshot = QueueFeeMetricsSnapshot {
        txns_expected: 256,
        escalation_multiplier: tx::TXQ_BASE_LEVEL,
    };
    view_source.open_ledger_tx_count = 40;
    let call = QueueApplyCallEnvelope::new(&tx_source, &view_source);
    let expected_prepared = RefCell::new(None::<tx::QueueApplyPreparedPreclaimInputs<String>>);

    let mut direct_shell = build_direct_owner_shell();
    let direct_stage = call.apply_with_owned_metrics_and_caller_preclaim_and_log_messages(
        &mut direct_shell,
        hold_preflight(),
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(5)),
        Ter::TES_SUCCESS,
        || queueing_preflight_result(SeqProxy::sequence(5)),
        |_| unreachable!("owner metrics path should not direct apply"),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |prepared| {
            expected_prepared.replace(Some(prepared.clone()));
            Ok(tx::QueueApplyPreclaimStage {
                view_source: prepared.view_source,
                trace_message: "trace".to_string(),
                preclaim_result: queueing_preflight_result(SeqProxy::sequence(5))
                    .to_preclaim(100, Ter::TES_SUCCESS),
            })
        },
        || -> ApplyResult { unreachable!("owner metrics path should reject before try-clear") },
        || unreachable!("owner metrics path should reject before sandbox apply"),
    );

    let mut runtime = direct_runtime();
    let mut runtime_shell = build_direct_owner_shell();
    let runtime_stage = QueueApplyRuntimeEnvelope::new(&mut runtime)
        .apply_with_owned_metrics_and_caller_preclaim_and_log_messages(
            &mut runtime_shell,
            &call,
            hold_preflight(),
            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
            TxConsequences::new(1, SeqProxy::sequence(5)),
            Ter::TES_SUCCESS,
            |prepared| {
                assert_eq!(Some(prepared.clone()), *expected_prepared.borrow());
                Ok(tx::QueueApplyPreclaimStage {
                    view_source: prepared.view_source,
                    trace_message: "trace".to_string(),
                    preclaim_result: queueing_preflight_result(SeqProxy::sequence(5))
                        .to_preclaim(100, Ter::TES_SUCCESS),
                })
            },
        );

    assert_eq!(runtime_stage, direct_stage);
    assert_eq!(runtime_shell.owner(), direct_shell.owner());
    assert_eq!(runtime.preflight_calls, 1);
    assert_eq!(runtime.direct_apply_calls, 0);
    assert_eq!(runtime.prepare_multitxn_calls, 0);
    assert_eq!(runtime.preclaim_calls, 0);
    assert_eq!(runtime.try_clear_calls, 0);
    assert_eq!(runtime.apply_sandbox_calls, 0);
    assert!(runtime.trace_messages.is_empty());
}

#[test]
fn queue_apply_runtime_envelope_after_preflight_owned_metrics_log_messages_match_call_envelope_on_owner_snapshot_path()
 {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let mut view_source = direct_view(&account);
    view_source.metrics_snapshot = QueueFeeMetricsSnapshot {
        txns_expected: 256,
        escalation_multiplier: tx::TXQ_BASE_LEVEL,
    };
    view_source.open_ledger_tx_count = 40;
    let call = QueueApplyCallEnvelope::new(&tx_source, &view_source);
    let preflight = queueing_preflight_result(SeqProxy::sequence(5));

    let mut direct_shell = build_direct_owner_shell();
    let direct_stage = call.apply_after_preflight_with_owned_metrics_and_log_messages(
        &mut direct_shell,
        &preflight,
        hold_preflight(),
        Ter::TES_SUCCESS,
        |_| unreachable!("owner metrics path should not direct apply"),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |_preclaim_view| {
            PreclaimResult::new(
                100,
                "tx",
                None::<&str>,
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            )
        },
        || -> ApplyResult { unreachable!("owner metrics path should reject before try-clear") },
        || unreachable!("owner metrics path should reject before sandbox apply"),
    );

    let mut runtime = direct_runtime();
    let mut runtime_shell = build_direct_owner_shell();
    let runtime_stage = QueueApplyRuntimeEnvelope::new(&mut runtime)
        .apply_after_preflight_with_owned_metrics_and_log_messages(
            &mut runtime_shell,
            &call,
            &preflight,
            hold_preflight(),
            Ter::TES_SUCCESS,
        );

    assert_eq!(runtime_stage, direct_stage);
    assert_eq!(runtime_shell.owner(), direct_shell.owner());
    assert_eq!(runtime.preflight_calls, 0);
    assert_eq!(runtime.direct_apply_calls, 0);
    assert_eq!(runtime.prepare_multitxn_calls, 0);
    assert_eq!(runtime.preclaim_calls, 1);
    assert_eq!(runtime.try_clear_calls, 0);
    assert_eq!(runtime.apply_sandbox_calls, 0);
    assert!(runtime.trace_messages.is_empty());
}

#[test]
fn queue_apply_runtime_envelope_after_preflight_derived_hold_preflight_log_messages_match_call_envelope_on_direct_path()
 {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let view_source = direct_view(&account);
    let call = QueueApplyCallEnvelope::new(&tx_source, &view_source);
    let preflight = queueing_preflight_result(SeqProxy::sequence(5));

    let mut direct_shell = build_direct_owner_shell();
    let direct_stage = call.apply_after_preflight_with_derived_hold_preflight_and_log_messages(
        &mut direct_shell,
        &preflight,
        Ter::TES_SUCCESS,
        |_| {},
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |_preclaim_view| {
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
            unreachable!("after-preflight derived hold preflight path should not hit try-clear")
        },
        || unreachable!("after-preflight derived hold preflight path should not hit sandbox apply"),
    );

    let mut runtime = direct_runtime();
    let mut runtime_shell = build_direct_owner_shell();
    let runtime_stage = QueueApplyRuntimeEnvelope::new(&mut runtime)
        .apply_after_preflight_with_derived_hold_preflight_and_log_messages(
            &mut runtime_shell,
            &call,
            &preflight,
            Ter::TES_SUCCESS,
        );

    assert_eq!(runtime_stage, direct_stage);
    assert_eq!(runtime_shell.owner(), direct_shell.owner());
    assert_eq!(runtime.preflight_calls, 0);
    assert_eq!(runtime.direct_apply_calls, 1);
    assert_eq!(runtime.prepare_multitxn_calls, 0);
    assert_eq!(runtime.preclaim_calls, 0);
    assert_eq!(runtime.try_clear_calls, 0);
    assert_eq!(runtime.apply_sandbox_calls, 0);
    assert_eq!(
        runtime.trace_messages,
        vec![
            format_direct_apply_start_log_message("ABC123"),
            format_direct_apply_finish_log_message(&tx::DirectApplyExecution::<String, _> {
                transaction_id: "ABC123",
                attempt: tx::DirectApplyAttemptResult {
                    apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, true),
                    removed_replacement: None,
                },
            }),
        ]
    );
}

#[test]
fn queue_apply_runtime_envelope_owned_metrics_derived_hold_admission_log_messages_match_call_envelope_on_owner_snapshot_path()
 {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(6),
    };
    let mut view_source = direct_view(&account);
    view_source.metrics_snapshot = QueueFeeMetricsSnapshot {
        txns_expected: 256,
        escalation_multiplier: tx::TXQ_BASE_LEVEL,
    };
    view_source.open_ledger_tx_count = 40;
    let call = QueueApplyCallEnvelope::new(&tx_source, &view_source);
    let hold_preflight = QueueHoldPreflight::new(
        false,
        false,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        Some(250),
    );

    let mut direct_shell = build_direct_owner_shell();
    let direct_stage = call.apply_with_owned_metrics_and_derived_hold_admission_and_log_messages(
        &mut direct_shell,
        hold_preflight,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(6)),
        || queueing_preflight_result(SeqProxy::sequence(6)),
        |_| unreachable!("owner metrics derived hold admission path should not direct apply"),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |_preclaim_view| {
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
            unreachable!("owner metrics derived hold admission path should reject before try-clear")
        },
        || {
            unreachable!(
                "owner metrics derived hold admission path should reject before sandbox apply"
            )
        },
    );

    let mut runtime = direct_runtime();
    let mut runtime_shell = build_direct_owner_shell();
    let runtime_stage = QueueApplyRuntimeEnvelope::new(&mut runtime)
        .apply_with_owned_metrics_and_derived_hold_admission_and_log_messages(
            &mut runtime_shell,
            &call,
            hold_preflight,
            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
            TxConsequences::new(1, SeqProxy::sequence(6)),
        );

    assert_eq!(runtime_stage, direct_stage);
    assert_eq!(runtime_shell.owner(), direct_shell.owner());
    assert_eq!(runtime.preflight_calls, 1);
    assert_eq!(runtime.direct_apply_calls, 0);
    assert_eq!(runtime.prepare_multitxn_calls, 0);
    assert_eq!(runtime.preclaim_calls, 0);
    assert_eq!(runtime.try_clear_calls, 0);
    assert_eq!(runtime.apply_sandbox_calls, 0);
    assert!(runtime.trace_messages.is_empty());
}

#[test]
fn queue_apply_runtime_envelope_after_preflight_derived_hold_admission_log_messages_match_call_envelope_on_direct_path()
 {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let view_source = direct_view(&account);
    let call = QueueApplyCallEnvelope::new(&tx_source, &view_source);
    let preflight = queueing_preflight_result(SeqProxy::sequence(5));
    let hold_preflight = QueueHoldPreflight::new(
        false,
        false,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        Some(250),
    );

    let mut direct_shell = build_direct_owner_shell();
    let direct_stage = call.apply_after_preflight_with_derived_hold_admission_and_log_messages(
        &mut direct_shell,
        &preflight,
        hold_preflight,
        |_| {},
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |_preclaim_view| {
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
            unreachable!("after-preflight derived hold admission path should not hit try-clear")
        },
        || unreachable!("after-preflight derived hold admission path should not hit sandbox apply"),
    );

    let mut runtime = direct_runtime();
    let mut runtime_shell = build_direct_owner_shell();
    let runtime_stage = QueueApplyRuntimeEnvelope::new(&mut runtime)
        .apply_after_preflight_with_derived_hold_admission_and_log_messages(
            &mut runtime_shell,
            &call,
            &preflight,
            hold_preflight,
        );

    assert_eq!(runtime_stage, direct_stage);
    assert_eq!(runtime_shell.owner(), direct_shell.owner());
    assert_eq!(runtime.preflight_calls, 0);
    assert_eq!(runtime.direct_apply_calls, 1);
    assert_eq!(runtime.prepare_multitxn_calls, 0);
    assert_eq!(runtime.preclaim_calls, 0);
    assert_eq!(runtime.try_clear_calls, 0);
    assert_eq!(runtime.apply_sandbox_calls, 0);
    assert_eq!(
        runtime.trace_messages,
        vec![
            format_direct_apply_start_log_message("ABC123"),
            format_direct_apply_finish_log_message(&tx::DirectApplyExecution::<String, _> {
                transaction_id: "ABC123",
                attempt: tx::DirectApplyAttemptResult {
                    apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, true),
                    removed_replacement: None,
                },
            }),
        ]
    );
}

#[test]
fn queue_apply_runtime_envelope_after_preflight_owned_metrics_caller_preclaim_log_messages_match_call_envelope_on_owner_snapshot_path()
 {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let mut view_source = direct_view(&account);
    view_source.metrics_snapshot = QueueFeeMetricsSnapshot {
        txns_expected: 256,
        escalation_multiplier: tx::TXQ_BASE_LEVEL,
    };
    view_source.open_ledger_tx_count = 40;
    let call = QueueApplyCallEnvelope::new(&tx_source, &view_source);
    let preflight = queueing_preflight_result(SeqProxy::sequence(5));
    let expected_prepared = RefCell::new(None::<tx::QueueApplyPreparedPreclaimInputs<String>>);

    let mut direct_shell = build_direct_owner_shell();
    let direct_stage = call
        .apply_after_preflight_with_owned_metrics_and_caller_preclaim_and_log_messages(
            &mut direct_shell,
            &preflight,
            hold_preflight(),
            Ter::TES_SUCCESS,
            |_| unreachable!("owner metrics path should not direct apply"),
            || ApplyResult::new(Ter::TES_SUCCESS, true, true),
            |_| true,
            |prepared| {
                expected_prepared.replace(Some(prepared.clone()));
                Ok(tx::QueueApplyPreclaimStage {
                    view_source: prepared.view_source,
                    trace_message: "trace".to_string(),
                    preclaim_result: queueing_preflight_result(SeqProxy::sequence(5))
                        .to_preclaim(100, Ter::TES_SUCCESS),
                })
            },
            || -> ApplyResult { unreachable!("owner metrics path should reject before try-clear") },
            || unreachable!("owner metrics path should reject before sandbox apply"),
        );

    let mut runtime = direct_runtime();
    let mut runtime_shell = build_direct_owner_shell();
    let runtime_stage = QueueApplyRuntimeEnvelope::new(&mut runtime)
        .apply_after_preflight_with_owned_metrics_and_caller_preclaim_and_log_messages(
            &mut runtime_shell,
            &call,
            &preflight,
            hold_preflight(),
            Ter::TES_SUCCESS,
            |prepared| {
                assert_eq!(Some(prepared.clone()), *expected_prepared.borrow());
                Ok(tx::QueueApplyPreclaimStage {
                    view_source: prepared.view_source,
                    trace_message: "trace".to_string(),
                    preclaim_result: queueing_preflight_result(SeqProxy::sequence(5))
                        .to_preclaim(100, Ter::TES_SUCCESS),
                })
            },
        );

    assert_eq!(runtime_stage, direct_stage);
    assert_eq!(runtime_shell.owner(), direct_shell.owner());
    assert_eq!(runtime.preflight_calls, 0);
    assert_eq!(runtime.direct_apply_calls, 0);
    assert_eq!(runtime.prepare_multitxn_calls, 0);
    assert_eq!(runtime.preclaim_calls, 0);
    assert_eq!(runtime.try_clear_calls, 0);
    assert_eq!(runtime.apply_sandbox_calls, 0);
    assert!(runtime.trace_messages.is_empty());
}

#[test]
fn queue_apply_runtime_envelope_after_preflight_owned_metrics_derived_hold_admission_log_messages_match_call_envelope_on_owner_snapshot_path()
 {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let mut view_source = direct_view(&account);
    view_source.metrics_snapshot = QueueFeeMetricsSnapshot {
        txns_expected: 256,
        escalation_multiplier: tx::TXQ_BASE_LEVEL,
    };
    view_source.open_ledger_tx_count = 40;
    let call = QueueApplyCallEnvelope::new(&tx_source, &view_source);
    let preflight = queueing_preflight_result(SeqProxy::sequence(5));
    let hold_preflight = QueueHoldPreflight::new(
        false,
        false,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        Some(250),
    );

    let mut direct_shell = build_direct_owner_shell();
    let direct_stage = call
        .apply_after_preflight_with_owned_metrics_and_derived_hold_admission_and_log_messages(
            &mut direct_shell,
            &preflight,
            hold_preflight,
            |_| unreachable!("owner metrics derived hold admission path should not direct apply"),
            || ApplyResult::new(Ter::TES_SUCCESS, true, true),
            |_| true,
            |_preclaim_view| {
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
                unreachable!(
                    "owner metrics derived hold admission path should reject before try-clear"
                )
            },
            || {
                unreachable!(
                    "owner metrics derived hold admission path should reject before sandbox apply"
                )
            },
        );

    let mut runtime = direct_runtime();
    let mut runtime_shell = build_direct_owner_shell();
    let runtime_stage = QueueApplyRuntimeEnvelope::new(&mut runtime)
        .apply_after_preflight_with_owned_metrics_and_derived_hold_admission_and_log_messages(
            &mut runtime_shell,
            &call,
            &preflight,
            hold_preflight,
        );

    assert_eq!(runtime_stage, direct_stage);
    assert_eq!(runtime_shell.owner(), direct_shell.owner());
    assert_eq!(runtime.preflight_calls, 0);
    assert_eq!(runtime.direct_apply_calls, 0);
    assert_eq!(runtime.prepare_multitxn_calls, 0);
    assert_eq!(runtime.preclaim_calls, 1);
    assert_eq!(runtime.try_clear_calls, 0);
    assert_eq!(runtime.apply_sandbox_calls, 0);
    assert!(runtime.trace_messages.is_empty());
}

#[test]
fn queue_apply_runtime_envelope_caller_preclaim_uses_structured_current_preclaim_try_clear() {
    #[derive(Default)]
    struct CurrentPreclaimRuntime {
        preflight_calls: usize,
        direct_apply_calls: usize,
        prepare_multitxn_calls: usize,
        try_clear_calls: usize,
        current_preclaim_try_clear_calls: usize,
        apply_sandbox_calls: usize,
    }

    impl QueueApplyExecutionRuntime<&'static str, &'static str, &'static str>
        for CurrentPreclaimRuntime
    {
        fn run_preflight(
            &mut self,
        ) -> PreflightResult<&'static str, TxConsequences, &'static str, &'static str> {
            self.preflight_calls += 1;
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

        fn trace(&mut self, _message: &str) {}

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
            panic!("caller-preclaim runtime test should bypass runtime preclaim");
        }

        fn run_try_clear(&mut self) -> ApplyResult {
            self.try_clear_calls += 1;
            panic!("caller-preclaim runtime test should use structured current-preclaim try-clear");
        }

        fn apply_sandbox(&mut self) {
            self.apply_sandbox_calls += 1;
        }
    }

    impl QueueApplyCurrentPreclaimClearRuntime<&'static str, &'static str, &'static str>
        for CurrentPreclaimRuntime
    {
        fn run_try_clear_with_current_preclaim(&mut self) -> tx::TryClearAccountResult {
            self.current_preclaim_try_clear_calls += 1;
            structured_try_clear_success()
        }
    }

    let account = String::from("acct");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(6),
    };
    let view_source = TestObservedViewSource {
        rules: rules(),
        account_lookup: QueueApplyObservedAccountLookup::Present {
            sequence: 5,
            balance_drops: 1_000,
        },
        ticket_lookup: QueueApplyObservedTicketLookup::Present,
        calculated_base_fee_drops: 10,
        fee_paid_drops: 6_000,
        default_base_fee_drops: 10,
        metrics_snapshot: QueueFeeMetricsSnapshot {
            txns_expected: 32,
            escalation_multiplier: tx::TXQ_BASE_LEVEL * 500,
        },
        open_ledger_tx_count: 33,
        open_ledger_seq: 100,
        reserve_drops: 200,
        base_fee_drops: 10,
    };
    let call = QueueApplyCallEnvelope::new(&tx_source, &view_source);

    let mut queued_account = TxQAccount::new(account.clone());
    queued_account.add(
        SeqProxy::sequence(5),
        MaybeTxCore::new(
            MaybeTx::new(
                Uint256::from_u64(5),
                20,
                account.clone(),
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
                account.clone(),
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

    let mut shell = QueueApplyOwnerShell::new(QueueApplyLiveOwner::new(
        2,
        10,
        25,
        Some(10),
        OrderCandidates::new(Uint256::from_u64(0)),
        QueueViews::new(BTreeMap::from([(account.clone(), queued_account)]), vec![]),
    ));
    let mut runtime = CurrentPreclaimRuntime::default();

    let stage = QueueApplyRuntimeEnvelope::new(&mut runtime).apply_with_caller_preclaim(
        &mut shell,
        &call,
        hold_preflight(),
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::with_potential_spend(50, SeqProxy::sequence(6), 500),
        Ter::TES_SUCCESS,
        |prepared| {
            assert_eq!(
                prepared.view_source,
                tx::QueueApplyPreclaimViewSource::MultiTxnOpenView
            );
            Ok(tx::QueueApplyPreclaimStage {
                view_source: prepared.view_source,
                trace_message: "trace".to_string(),
                preclaim_result: PreflightResult::new(
                    "tx",
                    None::<&str>,
                    rules(),
                    TxConsequences::with_potential_spend(50, SeqProxy::sequence(6), 500),
                    ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
                    "journal",
                    Ter::TES_SUCCESS,
                )
                .to_preclaim(100, Ter::TES_SUCCESS),
            })
        },
    );

    assert_eq!(
        stage.apply_result(),
        ApplyResult::new(Ter::TES_SUCCESS, true, true)
    );
    assert_eq!(runtime.preflight_calls, 1);
    assert_eq!(runtime.direct_apply_calls, 0);
    assert_eq!(runtime.prepare_multitxn_calls, 1);
    assert_eq!(runtime.try_clear_calls, 0);
    assert_eq!(runtime.current_preclaim_try_clear_calls, 1);
    assert_eq!(runtime.apply_sandbox_calls, 1);
}

#[test]
fn queue_apply_runtime_envelope_after_preflight_caller_preclaim_uses_structured_current_preclaim_try_clear()
 {
    #[derive(Default)]
    struct CurrentPreclaimRuntime {
        preflight_calls: usize,
        direct_apply_calls: usize,
        prepare_multitxn_calls: usize,
        try_clear_calls: usize,
        current_preclaim_try_clear_calls: usize,
        apply_sandbox_calls: usize,
    }

    impl QueueApplyExecutionRuntime<&'static str, &'static str, &'static str>
        for CurrentPreclaimRuntime
    {
        fn run_preflight(
            &mut self,
        ) -> PreflightResult<&'static str, TxConsequences, &'static str, &'static str> {
            self.preflight_calls += 1;
            panic!("after-preflight caller-preclaim test should not rerun preflight");
        }

        fn trace(&mut self, _message: &str) {}

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
            panic!("after-preflight caller-preclaim test should bypass runtime preclaim");
        }

        fn run_try_clear(&mut self) -> ApplyResult {
            self.try_clear_calls += 1;
            panic!(
                "after-preflight caller-preclaim test should use structured current-preclaim try-clear"
            );
        }

        fn apply_sandbox(&mut self) {
            self.apply_sandbox_calls += 1;
        }
    }

    impl QueueApplyCurrentPreclaimClearRuntime<&'static str, &'static str, &'static str>
        for CurrentPreclaimRuntime
    {
        fn run_try_clear_with_current_preclaim(&mut self) -> tx::TryClearAccountResult {
            self.current_preclaim_try_clear_calls += 1;
            structured_try_clear_success()
        }
    }

    let account = String::from("acct");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(6),
    };
    let view_source = TestObservedViewSource {
        rules: rules(),
        account_lookup: QueueApplyObservedAccountLookup::Present {
            sequence: 5,
            balance_drops: 1_000,
        },
        ticket_lookup: QueueApplyObservedTicketLookup::Present,
        calculated_base_fee_drops: 10,
        fee_paid_drops: 6_000,
        default_base_fee_drops: 10,
        metrics_snapshot: QueueFeeMetricsSnapshot {
            txns_expected: 32,
            escalation_multiplier: tx::TXQ_BASE_LEVEL * 500,
        },
        open_ledger_tx_count: 33,
        open_ledger_seq: 100,
        reserve_drops: 200,
        base_fee_drops: 10,
    };
    let call = QueueApplyCallEnvelope::new(&tx_source, &view_source);
    let preflight_result = PreflightResult::new(
        "tx",
        None::<&str>,
        rules(),
        TxConsequences::with_potential_spend(50, SeqProxy::sequence(6), 500),
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        "journal",
        Ter::TES_SUCCESS,
    );

    let mut queued_account = TxQAccount::new(account.clone());
    queued_account.add(
        SeqProxy::sequence(5),
        MaybeTxCore::new(
            MaybeTx::new(
                Uint256::from_u64(5),
                20,
                account.clone(),
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
                account.clone(),
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

    let mut shell = QueueApplyOwnerShell::new(QueueApplyLiveOwner::new(
        2,
        10,
        25,
        Some(10),
        OrderCandidates::new(Uint256::from_u64(0)),
        QueueViews::new(BTreeMap::from([(account.clone(), queued_account)]), vec![]),
    ));
    let mut runtime = CurrentPreclaimRuntime::default();

    let stage = QueueApplyRuntimeEnvelope::new(&mut runtime)
        .apply_after_preflight_with_caller_preclaim(
            &mut shell,
            &call,
            &preflight_result,
            hold_preflight(),
            Ter::TES_SUCCESS,
            |prepared| {
                assert_eq!(
                    prepared.view_source,
                    tx::QueueApplyPreclaimViewSource::MultiTxnOpenView
                );
                Ok(tx::QueueApplyPreclaimStage {
                    view_source: prepared.view_source,
                    trace_message: "trace".to_string(),
                    preclaim_result: PreflightResult::new(
                        "tx",
                        None::<&str>,
                        rules(),
                        TxConsequences::with_potential_spend(50, SeqProxy::sequence(6), 500),
                        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
                        "journal",
                        Ter::TES_SUCCESS,
                    )
                    .to_preclaim(100, Ter::TES_SUCCESS),
                })
            },
        );

    assert_eq!(
        stage.apply_result(),
        ApplyResult::new(Ter::TES_SUCCESS, true, true)
    );
    assert_eq!(runtime.preflight_calls, 0);
    assert_eq!(runtime.direct_apply_calls, 0);
    assert_eq!(runtime.prepare_multitxn_calls, 1);
    assert_eq!(runtime.try_clear_calls, 0);
    assert_eq!(runtime.current_preclaim_try_clear_calls, 1);
    assert_eq!(runtime.apply_sandbox_calls, 1);
}
