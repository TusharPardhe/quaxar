use std::{cell::RefCell, collections::BTreeMap};

use basics::base_uint::Uint256;
use protocol::{Rules, SeqProxy, Ter};
use tx::{
    ApplyFlags, ApplyResult, FeeQueueEntry, FeeQueueKey, MaybeTx, MaybeTxCore, OrderCandidates,
    PreclaimResult, PreflightResult, QueueAdvanceCandidate, QueueApplyHoldPreflightTxSource,
    QueueApplyJournalEnvelope, QueueApplyJournalSink, QueueApplyLiveOwner,
    QueueApplyObservedAccountLookup, QueueApplyObservedTicketLookup, QueueApplyObservedTxSource,
    QueueApplyObservedViewSource, QueueApplyOwnerShell, QueueApplyPreclaimStage,
    QueueApplyPreparedPreclaimInputs, QueueApplyViewAdjustment, QueueFeeMetricsSnapshot,
    QueueHoldPreflight, QueueViews, TxConsequences, TxQAccount,
    format_direct_apply_finish_log_message, format_direct_apply_start_log_message,
    format_queue_apply_enqueue_debug_message,
    run_queue_apply_after_preflight_with_app_view_and_log_messages,
    run_queue_apply_with_app_view_and_log_messages,
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
    account: String,
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

#[derive(Debug, Clone)]
struct TestApplyApp {
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

#[derive(Debug, Default)]
struct TestJournal {
    emitted: Vec<String>,
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
        if account == &self.account {
            self.account_lookup
        } else {
            QueueApplyObservedAccountLookup::Missing
        }
    }

    fn ticket_lookup(
        &self,
        account: &String,
        _tx_seq_proxy: SeqProxy,
    ) -> QueueApplyObservedTicketLookup {
        if account == &self.account {
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

impl tx::QueueApplyExecutionRuntime<&'static str, &'static str, &'static str> for TestApplyApp {
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

impl tx::QueueApplyCurrentPreclaimClearRuntime<&'static str, &'static str, &'static str>
    for TestApplyApp
{
    fn run_try_clear_with_current_preclaim(&mut self) -> tx::TryClearAccountResult {
        structured_try_clear_success()
    }
}

impl QueueApplyJournalSink for TestJournal {
    fn trace(&mut self, message: &str) {
        self.emitted.push(format!("trace:{message}"));
    }

    fn debug(&mut self, message: &str) {
        self.emitted.push(format!("debug:{message}"));
    }

    fn info(&mut self, message: &str) {
        self.emitted.push(format!("info:{message}"));
    }
}

fn rules() -> Rules {
    Rules::new(std::iter::empty())
}

fn preflight_result(
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

fn hold_preflight() -> QueueHoldPreflight {
    QueueHoldPreflight::new(false, false, ApplyFlags::NONE, Some(250))
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
        preflight_result(seq_proxy),
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

fn queued_view(account: &str) -> TestLedgerView {
    TestLedgerView {
        account: account.to_owned(),
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

fn direct_view(account: &str) -> TestLedgerView {
    TestLedgerView {
        account: account.to_owned(),
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

fn queued_app() -> TestApplyApp {
    TestApplyApp {
        preflight_result: preflight_result(SeqProxy::sequence(6)),
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

fn direct_app() -> TestApplyApp {
    TestApplyApp {
        preflight_result: preflight_result(SeqProxy::sequence(5)),
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

fn caller_preclaim_stage(
    prepared: QueueApplyPreparedPreclaimInputs<String>,
) -> QueueApplyPreclaimStage<&'static str, &'static str, &'static str> {
    QueueApplyPreclaimStage {
        view_source: prepared.view_source,
        trace_message: "trace".to_string(),
        preclaim_result: preflight_result(SeqProxy::sequence(6)).to_preclaim(100, Ter::TES_SUCCESS),
    }
}

#[test]
fn queue_apply_journal_envelope_matches_owner_shell_app_view_log_sink_wrapper_on_full_queue_path() {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(6),
    };
    let view = queued_view(&account);

    let mut direct_app = queued_app();
    let mut direct_owner = build_full_queue_owner_shell();
    let direct_emitted = RefCell::new(Vec::new());
    let direct_stage = direct_owner.apply_with_app_view_and_log_sinks(
        &mut direct_app,
        &view,
        &tx_source,
        hold_preflight(),
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(6)),
        Ter::TES_SUCCESS,
        |message| direct_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| direct_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| direct_emitted.borrow_mut().push(format!("info:{message}")),
    );

    let mut journal_app = queued_app();
    let mut journal_owner = build_full_queue_owner_shell();
    let mut journal = TestJournal::default();
    let mut envelope = QueueApplyJournalEnvelope::new(&mut journal_app, &view, &mut journal);
    let journal_stage = envelope.apply(
        &mut journal_owner,
        &tx_source,
        hold_preflight(),
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(6)),
        Ter::TES_SUCCESS,
    );

    assert_eq!(journal_stage, direct_stage);
    assert_eq!(journal_owner.owner(), direct_owner.owner());
    assert_eq!(journal.emitted, direct_emitted.into_inner());
    assert_eq!(journal.emitted.len(), 2);
    assert!(journal.emitted[0].starts_with("info:Removing last item of account b from queue"));
    assert_eq!(
        journal.emitted[1],
        format!(
            "debug:{}",
            format_queue_apply_enqueue_debug_message(
                Uint256::from_u64(9),
                Ter::TES_SUCCESS,
                true,
                "a",
                ApplyFlags::FAIL_HARD
            )
        )
    );
    assert_eq!(direct_app.preflight_calls, 1);
    assert_eq!(journal_app.preflight_calls, 1);
    assert_eq!(direct_app.direct_apply_calls, 0);
    assert_eq!(journal_app.direct_apply_calls, 0);
    assert_eq!(direct_app.prepare_multitxn_calls, 1);
    assert_eq!(journal_app.prepare_multitxn_calls, 1);
    assert_eq!(direct_app.preclaim_calls, 1);
    assert_eq!(journal_app.preclaim_calls, 1);
    assert!(direct_app.trace_messages.is_empty());
    assert!(journal_app.trace_messages.is_empty());
}

#[test]
fn queue_apply_journal_envelope_matches_owner_shell_app_view_log_sink_wrapper_after_preflight_on_direct_path()
 {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let view = direct_view(&account);
    let preflight = preflight_result(SeqProxy::sequence(5));

    let mut direct_runtime = direct_app();
    let mut direct_owner = build_direct_owner_shell();
    let direct_emitted = RefCell::new(Vec::new());
    let direct_stage = direct_owner.apply_after_preflight_with_app_view_and_log_sinks(
        &mut direct_runtime,
        &view,
        &tx_source,
        &preflight,
        hold_preflight(),
        Ter::TES_SUCCESS,
        |message| direct_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| direct_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| direct_emitted.borrow_mut().push(format!("info:{message}")),
    );

    let mut journal_app = direct_app();
    let mut journal_owner = build_direct_owner_shell();
    let mut journal = TestJournal::default();
    let mut envelope = QueueApplyJournalEnvelope::new(&mut journal_app, &view, &mut journal);
    let journal_stage = envelope.apply_after_preflight(
        &mut journal_owner,
        &tx_source,
        &preflight,
        hold_preflight(),
        Ter::TES_SUCCESS,
    );

    assert_eq!(journal_stage, direct_stage);
    assert_eq!(journal_owner.owner(), direct_owner.owner());
    assert_eq!(journal.emitted, direct_emitted.into_inner());
    assert_eq!(
        journal.emitted,
        vec![
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
        ]
    );
    assert_eq!(direct_runtime.preflight_calls, 0);
    assert_eq!(journal_app.preflight_calls, 0);
    assert_eq!(direct_runtime.direct_apply_calls, 1);
    assert_eq!(journal_app.direct_apply_calls, 1);
    assert!(direct_runtime.trace_messages.is_empty());
    assert!(journal_app.trace_messages.is_empty());
}

#[test]
fn queue_apply_journal_envelope_log_messages_match_app_view_boundary_on_full_queue_path() {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(6),
    };
    let view = queued_view(&account);

    let mut direct_app = queued_app();
    let mut direct_owner = build_full_queue_owner_shell();
    let direct_stage = run_queue_apply_with_app_view_and_log_messages(
        &mut direct_owner,
        &mut direct_app,
        &view,
        &tx_source,
        hold_preflight(),
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(6)),
        Ter::TES_SUCCESS,
    );

    let mut journal_app = queued_app();
    let mut journal_owner = build_full_queue_owner_shell();
    let mut journal = TestJournal::default();
    let mut envelope = QueueApplyJournalEnvelope::new(&mut journal_app, &view, &mut journal);
    let journal_stage = envelope.apply_with_log_messages(
        &mut journal_owner,
        &tx_source,
        hold_preflight(),
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(6)),
        Ter::TES_SUCCESS,
    );

    let expected_emitted = direct_stage
        .queue_log_messages
        .info
        .iter()
        .map(|message| format!("info:{message}"))
        .chain(
            direct_stage
                .queue_log_messages
                .debug
                .iter()
                .map(|message| format!("debug:{message}")),
        )
        .collect::<Vec<_>>();

    assert_eq!(journal_stage, direct_stage);
    assert_eq!(journal_owner.owner(), direct_owner.owner());
    assert_eq!(journal.emitted, expected_emitted);
    assert_eq!(journal_stage.queue_log_messages.info.len(), 1);
    assert_eq!(journal_stage.queue_log_messages.debug.len(), 1);
    assert!(journal_stage.queue_log_messages.trace.is_empty());
    assert_eq!(direct_app.preflight_calls, 1);
    assert_eq!(journal_app.preflight_calls, 1);
    assert_eq!(direct_app.direct_apply_calls, 0);
    assert_eq!(journal_app.direct_apply_calls, 0);
    assert_eq!(direct_app.prepare_multitxn_calls, 1);
    assert_eq!(journal_app.prepare_multitxn_calls, 1);
    assert_eq!(direct_app.preclaim_calls, 1);
    assert_eq!(journal_app.preclaim_calls, 1);
    assert!(direct_app.trace_messages.is_empty());
    assert!(journal_app.trace_messages.is_empty());
}

#[test]
fn queue_apply_journal_envelope_after_preflight_log_messages_match_app_view_boundary_on_direct_path()
 {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let view = direct_view(&account);
    let preflight = preflight_result(SeqProxy::sequence(5));

    let mut direct_runtime = direct_app();
    let mut direct_owner = build_direct_owner_shell();
    let direct_stage = run_queue_apply_after_preflight_with_app_view_and_log_messages(
        &mut direct_owner,
        &mut direct_runtime,
        &view,
        &tx_source,
        &preflight,
        hold_preflight(),
        Ter::TES_SUCCESS,
    );

    let mut journal_app = direct_app();
    let mut journal_owner = build_direct_owner_shell();
    let mut journal = TestJournal::default();
    let mut envelope = QueueApplyJournalEnvelope::new(&mut journal_app, &view, &mut journal);
    let journal_stage = envelope.apply_after_preflight_with_log_messages(
        &mut journal_owner,
        &tx_source,
        &preflight,
        hold_preflight(),
        Ter::TES_SUCCESS,
    );

    assert_eq!(journal_stage, direct_stage);
    assert_eq!(journal_owner.owner(), direct_owner.owner());
    assert!(journal.emitted.is_empty());
    assert!(journal_stage.queue_log_messages.trace.is_empty());
    assert!(journal_stage.queue_log_messages.info.is_empty());
    assert!(journal_stage.queue_log_messages.debug.is_empty());
    assert_eq!(direct_runtime.preflight_calls, 0);
    assert_eq!(journal_app.preflight_calls, 0);
    assert_eq!(direct_runtime.direct_apply_calls, 1);
    assert_eq!(journal_app.direct_apply_calls, 1);
    assert_eq!(journal_app.trace_messages, direct_runtime.trace_messages);
    assert_eq!(
        journal_app.trace_messages,
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
fn queue_apply_journal_envelope_matches_owner_shell_app_view_caller_preclaim_log_sink_wrapper_on_full_queue_path()
 {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(6),
    };
    let view = queued_view(&account);
    let expected_prepared = RefCell::new(None::<QueueApplyPreparedPreclaimInputs<String>>);

    let mut direct_app = queued_app();
    let mut direct_owner = build_full_queue_owner_shell();
    let direct_emitted = RefCell::new(Vec::new());
    let direct_stage = direct_owner.apply_with_app_view_and_caller_preclaim_and_log_sinks(
        &mut direct_app,
        &view,
        &tx_source,
        hold_preflight(),
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(6)),
        Ter::TES_SUCCESS,
        |prepared| {
            expected_prepared.replace(Some(prepared.clone()));
            Ok(caller_preclaim_stage(prepared))
        },
        |message| direct_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| direct_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| direct_emitted.borrow_mut().push(format!("info:{message}")),
    );

    let mut journal_app = queued_app();
    let mut journal_owner = build_full_queue_owner_shell();
    let mut journal = TestJournal::default();
    let mut envelope = QueueApplyJournalEnvelope::new(&mut journal_app, &view, &mut journal);
    let journal_stage = envelope.apply_with_caller_preclaim(
        &mut journal_owner,
        &tx_source,
        hold_preflight(),
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(6)),
        Ter::TES_SUCCESS,
        |prepared| {
            assert_eq!(Some(prepared.clone()), *expected_prepared.borrow());
            Ok(caller_preclaim_stage(prepared))
        },
    );

    assert_eq!(journal_stage, direct_stage);
    assert_eq!(journal_owner.owner(), direct_owner.owner());
    assert_eq!(journal.emitted, direct_emitted.into_inner());
    assert_eq!(journal.emitted.len(), 2);
    assert_eq!(direct_app.preflight_calls, 1);
    assert_eq!(journal_app.preflight_calls, 1);
    assert_eq!(direct_app.preclaim_calls, 0);
    assert_eq!(journal_app.preclaim_calls, 0);
    assert_eq!(direct_app.prepare_multitxn_calls, 1);
    assert_eq!(journal_app.prepare_multitxn_calls, 1);
}

#[test]
fn queue_apply_journal_envelope_derives_hold_preflight_like_owner_shell_app_view_wrapper() {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(6),
    };
    let view = queued_view(&account);

    let mut direct_app = queued_app();
    let mut direct_owner = build_full_queue_owner_shell();
    let direct_emitted = RefCell::new(Vec::new());
    let direct_stage = direct_owner.apply_with_app_view_and_log_sinks_and_derived_hold_preflight(
        &mut direct_app,
        &view,
        &tx_source,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(6)),
        Ter::TES_SUCCESS,
        |message| direct_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| direct_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| direct_emitted.borrow_mut().push(format!("info:{message}")),
    );

    let mut journal_app = queued_app();
    let mut journal_owner = build_full_queue_owner_shell();
    let mut journal = TestJournal::default();
    let mut envelope = QueueApplyJournalEnvelope::new(&mut journal_app, &view, &mut journal);
    let journal_stage = envelope.apply_with_derived_hold_preflight(
        &mut journal_owner,
        &tx_source,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(6)),
        Ter::TES_SUCCESS,
    );

    assert_eq!(journal_stage, direct_stage);
    assert_eq!(journal_owner.owner(), direct_owner.owner());
    assert_eq!(journal.emitted, direct_emitted.into_inner());
    assert_eq!(direct_app.preflight_calls, 1);
    assert_eq!(journal_app.preflight_calls, 1);
    assert_eq!(direct_app.preclaim_calls, 0);
    assert_eq!(journal_app.preclaim_calls, 0);
}

#[test]
fn queue_apply_journal_envelope_derives_preflight_facts_like_owner_shell_app_view_wrapper() {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(6),
    };
    let view = queued_view(&account);

    let mut direct_app = queued_app();
    let mut direct_owner = build_full_queue_owner_shell();
    let direct_emitted = RefCell::new(Vec::new());
    let direct_stage = direct_owner.apply_with_app_view_and_log_sinks_and_derived_preflight_facts(
        &mut direct_app,
        &view,
        &tx_source,
        Ter::TES_SUCCESS,
        |message| direct_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| direct_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| direct_emitted.borrow_mut().push(format!("info:{message}")),
    );

    let mut journal_app = queued_app();
    let mut journal_owner = build_full_queue_owner_shell();
    let mut journal = TestJournal::default();
    let mut envelope = QueueApplyJournalEnvelope::new(&mut journal_app, &view, &mut journal);
    let journal_stage = envelope.apply_with_derived_preflight_facts(
        &mut journal_owner,
        &tx_source,
        Ter::TES_SUCCESS,
    );

    assert_eq!(journal_stage, direct_stage);
    assert_eq!(journal_owner.owner(), direct_owner.owner());
    assert_eq!(journal.emitted, direct_emitted.into_inner());
    assert_eq!(direct_app.preflight_calls, 1);
    assert_eq!(journal_app.preflight_calls, 1);
    assert_eq!(direct_app.preclaim_calls, 0);
    assert_eq!(journal_app.preclaim_calls, 0);
}

#[test]
fn queue_apply_journal_envelope_derives_preflight_and_hold_admission_like_owner_shell_app_view_wrapper()
 {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(6),
    };
    let view = queued_view(&account);

    let mut direct_app = queued_app();
    let mut direct_owner = build_full_queue_owner_shell();
    let direct_emitted = RefCell::new(Vec::new());
    let direct_stage = direct_owner
        .apply_with_app_view_and_log_sinks_and_derived_preflight_facts_and_hold_admission(
            &mut direct_app,
            &view,
            &tx_source,
            |message| direct_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| direct_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| direct_emitted.borrow_mut().push(format!("info:{message}")),
        );

    let mut journal_app = queued_app();
    let mut journal_owner = build_full_queue_owner_shell();
    let mut journal = TestJournal::default();
    let mut envelope = QueueApplyJournalEnvelope::new(&mut journal_app, &view, &mut journal);
    let journal_stage = envelope
        .apply_with_derived_preflight_facts_and_hold_admission(&mut journal_owner, &tx_source);

    assert_eq!(journal_stage, direct_stage);
    assert_eq!(journal_owner.owner(), direct_owner.owner());
    assert_eq!(journal.emitted, direct_emitted.into_inner());
    assert_eq!(direct_app.preflight_calls, 1);
    assert_eq!(journal_app.preflight_calls, 1);
    assert_eq!(direct_app.preclaim_calls, 0);
    assert_eq!(journal_app.preclaim_calls, 0);
}

#[test]
fn queue_apply_journal_envelope_after_preflight_derives_hold_preflight_like_owner_shell_app_view_wrapper()
 {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let view = direct_view(&account);
    let preflight = preflight_result(SeqProxy::sequence(5));

    let mut direct_runtime = direct_app();
    let mut direct_owner = build_direct_owner_shell();
    let direct_emitted = RefCell::new(Vec::new());
    let direct_stage = direct_owner
        .apply_after_preflight_with_app_view_and_log_sinks_and_derived_hold_preflight(
            &mut direct_runtime,
            &view,
            &tx_source,
            &preflight,
            Ter::TES_SUCCESS,
            |message| direct_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| direct_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| direct_emitted.borrow_mut().push(format!("info:{message}")),
        );

    let mut journal_app = direct_app();
    let mut journal_owner = build_direct_owner_shell();
    let mut journal = TestJournal::default();
    let mut envelope = QueueApplyJournalEnvelope::new(&mut journal_app, &view, &mut journal);
    let journal_stage = envelope.apply_after_preflight_with_derived_hold_preflight(
        &mut journal_owner,
        &tx_source,
        &preflight,
        Ter::TES_SUCCESS,
    );

    assert_eq!(journal_stage, direct_stage);
    assert_eq!(journal_owner.owner(), direct_owner.owner());
    assert_eq!(journal.emitted, direct_emitted.into_inner());
    assert_eq!(direct_runtime.preflight_calls, 0);
    assert_eq!(journal_app.preflight_calls, 0);
    assert_eq!(direct_runtime.direct_apply_calls, 1);
    assert_eq!(journal_app.direct_apply_calls, 1);
}

#[test]
fn queue_apply_journal_envelope_after_preflight_derives_hold_admission_like_owner_shell_app_view_wrapper()
 {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let view = direct_view(&account);
    let preflight = preflight_result(SeqProxy::sequence(5));

    let mut direct_runtime = direct_app();
    let mut direct_owner = build_direct_owner_shell();
    let direct_emitted = RefCell::new(Vec::new());
    let direct_stage = direct_owner
        .apply_after_preflight_with_app_view_and_log_sinks_and_derived_hold_admission(
            &mut direct_runtime,
            &view,
            &tx_source,
            &preflight,
            hold_preflight(),
            |message| direct_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| direct_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| direct_emitted.borrow_mut().push(format!("info:{message}")),
        );

    let mut journal_app = direct_app();
    let mut journal_owner = build_direct_owner_shell();
    let mut journal = TestJournal::default();
    let mut envelope = QueueApplyJournalEnvelope::new(&mut journal_app, &view, &mut journal);
    let journal_stage = envelope.apply_after_preflight_with_derived_hold_admission(
        &mut journal_owner,
        &tx_source,
        &preflight,
        hold_preflight(),
    );

    assert_eq!(journal_stage, direct_stage);
    assert_eq!(journal_owner.owner(), direct_owner.owner());
    assert_eq!(journal.emitted, direct_emitted.into_inner());
    assert_eq!(direct_runtime.preflight_calls, 0);
    assert_eq!(journal_app.preflight_calls, 0);
    assert_eq!(direct_runtime.direct_apply_calls, 1);
    assert_eq!(journal_app.direct_apply_calls, 1);
}

#[test]
fn queue_apply_journal_envelope_owned_metrics_derived_hold_preflight_match_owner_shell_app_view_on_owner_snapshot_path()
 {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let mut view = direct_view(&account);
    view.metrics_snapshot = QueueFeeMetricsSnapshot {
        txns_expected: 256,
        escalation_multiplier: tx::TXQ_BASE_LEVEL,
    };
    view.open_ledger_tx_count = 40;

    let mut direct_runtime = direct_app();
    let mut direct_owner = build_direct_owner_shell();
    let direct_emitted = RefCell::new(Vec::new());
    let direct_stage = direct_owner
        .apply_with_app_view_and_owned_metrics_and_log_sinks_and_derived_hold_preflight(
            &mut direct_runtime,
            &view,
            &tx_source,
            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
            TxConsequences::new(1, SeqProxy::sequence(5)),
            Ter::TES_SUCCESS,
            |message| direct_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| direct_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| direct_emitted.borrow_mut().push(format!("info:{message}")),
        );

    let mut journal_app = direct_app();
    let mut journal_owner = build_direct_owner_shell();
    let mut journal = TestJournal::default();
    let mut envelope = QueueApplyJournalEnvelope::new(&mut journal_app, &view, &mut journal);
    let journal_stage = envelope.apply_with_owned_metrics_and_derived_hold_preflight(
        &mut journal_owner,
        &tx_source,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(5)),
        Ter::TES_SUCCESS,
    );

    assert_eq!(journal_stage, direct_stage);
    assert_eq!(journal_owner.owner(), direct_owner.owner());
    assert_eq!(journal.emitted, direct_emitted.into_inner());
    assert_eq!(
        journal_app.direct_apply_calls,
        direct_runtime.direct_apply_calls
    );
}

#[test]
fn queue_apply_journal_envelope_owned_metrics_derived_preflight_facts_match_owner_shell_app_view_on_owner_snapshot_path()
 {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let mut view = direct_view(&account);
    view.metrics_snapshot = QueueFeeMetricsSnapshot {
        txns_expected: 256,
        escalation_multiplier: tx::TXQ_BASE_LEVEL,
    };
    view.open_ledger_tx_count = 40;

    let mut direct_runtime = direct_app();
    let mut direct_owner = build_direct_owner_shell();
    let direct_emitted = RefCell::new(Vec::new());
    let direct_stage = direct_owner
        .apply_with_app_view_and_owned_metrics_and_log_sinks_and_derived_preflight_facts(
            &mut direct_runtime,
            &view,
            &tx_source,
            |message| direct_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| direct_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| direct_emitted.borrow_mut().push(format!("info:{message}")),
        );

    let mut journal_app = direct_app();
    let mut journal_owner = build_direct_owner_shell();
    let mut journal = TestJournal::default();
    let mut envelope = QueueApplyJournalEnvelope::new(&mut journal_app, &view, &mut journal);
    let journal_stage = envelope
        .apply_with_owned_metrics_and_derived_preflight_facts(&mut journal_owner, &tx_source);

    assert_eq!(journal_stage, direct_stage);
    assert_eq!(journal_owner.owner(), direct_owner.owner());
    assert_eq!(journal.emitted, direct_emitted.into_inner());
    assert_eq!(
        journal_app.direct_apply_calls,
        direct_runtime.direct_apply_calls
    );
}

#[test]
fn queue_apply_journal_envelope_owned_metrics_derived_preflight_and_hold_admission_match_owner_shell_app_view_on_owner_snapshot_path()
 {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let mut view = direct_view(&account);
    view.metrics_snapshot = QueueFeeMetricsSnapshot {
        txns_expected: 256,
        escalation_multiplier: tx::TXQ_BASE_LEVEL,
    };
    view.open_ledger_tx_count = 40;

    let mut direct_runtime = direct_app();
    let mut direct_owner = build_direct_owner_shell();
    let direct_emitted = RefCell::new(Vec::new());
    let direct_stage = direct_owner
        .apply_with_app_view_and_owned_metrics_and_log_sinks_and_derived_preflight_facts_and_hold_admission(
            &mut direct_runtime,
            &view,
            &tx_source,
            |message| direct_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| direct_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| direct_emitted.borrow_mut().push(format!("info:{message}")),
        );

    let mut journal_app = direct_app();
    let mut journal_owner = build_direct_owner_shell();
    let mut journal = TestJournal::default();
    let mut envelope = QueueApplyJournalEnvelope::new(&mut journal_app, &view, &mut journal);
    let journal_stage = envelope
        .apply_with_owned_metrics_and_derived_preflight_facts_and_hold_admission(
            &mut journal_owner,
            &tx_source,
        );

    assert_eq!(journal_stage, direct_stage);
    assert_eq!(journal_owner.owner(), direct_owner.owner());
    assert_eq!(journal.emitted, direct_emitted.into_inner());
    assert_eq!(
        journal_app.direct_apply_calls,
        direct_runtime.direct_apply_calls
    );
}

#[test]
fn queue_apply_journal_envelope_after_preflight_owned_metrics_derived_hold_preflight_match_owner_shell_app_view_on_owner_snapshot_path()
 {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let mut view = direct_view(&account);
    view.metrics_snapshot = QueueFeeMetricsSnapshot {
        txns_expected: 256,
        escalation_multiplier: tx::TXQ_BASE_LEVEL,
    };
    view.open_ledger_tx_count = 40;
    let preflight = preflight_result(SeqProxy::sequence(5));

    let mut direct_runtime = direct_app();
    let mut direct_owner = build_direct_owner_shell();
    let direct_emitted = RefCell::new(Vec::new());
    let direct_stage = direct_owner
        .apply_after_preflight_with_app_view_and_owned_metrics_and_log_sinks_and_derived_hold_preflight(
            &mut direct_runtime,
            &view,
            &tx_source,
            &preflight,
            Ter::TES_SUCCESS,
            |message| direct_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| direct_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| direct_emitted.borrow_mut().push(format!("info:{message}")),
        );

    let mut journal_app = direct_app();
    let mut journal_owner = build_direct_owner_shell();
    let mut journal = TestJournal::default();
    let mut envelope = QueueApplyJournalEnvelope::new(&mut journal_app, &view, &mut journal);
    let journal_stage = envelope
        .apply_after_preflight_with_owned_metrics_and_derived_hold_preflight(
            &mut journal_owner,
            &tx_source,
            &preflight,
            Ter::TES_SUCCESS,
        );

    assert_eq!(journal_stage, direct_stage);
    assert_eq!(journal_owner.owner(), direct_owner.owner());
    assert_eq!(journal.emitted, direct_emitted.into_inner());
    assert_eq!(
        journal_app.direct_apply_calls,
        direct_runtime.direct_apply_calls
    );
}

#[test]
fn queue_apply_journal_envelope_after_preflight_owned_metrics_derived_hold_admission_match_owner_shell_app_view_on_owner_snapshot_path()
 {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let mut view = direct_view(&account);
    view.metrics_snapshot = QueueFeeMetricsSnapshot {
        txns_expected: 256,
        escalation_multiplier: tx::TXQ_BASE_LEVEL,
    };
    view.open_ledger_tx_count = 40;
    let preflight = preflight_result(SeqProxy::sequence(5));
    let hold_preflight = QueueHoldPreflight::new(
        false,
        false,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        Some(250),
    );

    let mut direct_runtime = direct_app();
    let mut direct_owner = build_direct_owner_shell();
    let direct_emitted = RefCell::new(Vec::new());
    let direct_stage = direct_owner
        .apply_after_preflight_with_app_view_and_owned_metrics_and_log_sinks_and_derived_hold_admission(
            &mut direct_runtime,
            &view,
            &tx_source,
            &preflight,
            hold_preflight,
            |message| direct_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| direct_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| direct_emitted.borrow_mut().push(format!("info:{message}")),
        );

    let mut journal_app = direct_app();
    let mut journal_owner = build_direct_owner_shell();
    let mut journal = TestJournal::default();
    let mut envelope = QueueApplyJournalEnvelope::new(&mut journal_app, &view, &mut journal);
    let journal_stage = envelope
        .apply_after_preflight_with_owned_metrics_and_derived_hold_admission(
            &mut journal_owner,
            &tx_source,
            &preflight,
            hold_preflight,
        );

    assert_eq!(journal_stage, direct_stage);
    assert_eq!(journal_owner.owner(), direct_owner.owner());
    assert_eq!(journal.emitted, direct_emitted.into_inner());
    assert_eq!(
        journal_app.direct_apply_calls,
        direct_runtime.direct_apply_calls
    );
}

#[test]
fn queue_apply_journal_envelope_after_preflight_matches_owner_shell_app_view_caller_preclaim_log_sink_wrapper_on_full_queue_path()
 {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(6),
    };
    let view = queued_view(&account);
    let preflight = preflight_result(SeqProxy::sequence(6));
    let expected_prepared = RefCell::new(None::<QueueApplyPreparedPreclaimInputs<String>>);

    let mut direct_app = queued_app();
    let mut direct_owner = build_full_queue_owner_shell();
    let direct_emitted = RefCell::new(Vec::new());
    let direct_stage = direct_owner
        .apply_after_preflight_with_app_view_and_caller_preclaim_and_log_sinks(
            &mut direct_app,
            &view,
            &tx_source,
            &preflight,
            hold_preflight(),
            Ter::TES_SUCCESS,
            |prepared| {
                expected_prepared.replace(Some(prepared.clone()));
                Ok(caller_preclaim_stage(prepared))
            },
            |message| direct_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| direct_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| direct_emitted.borrow_mut().push(format!("info:{message}")),
        );

    let mut journal_app = queued_app();
    let mut journal_owner = build_full_queue_owner_shell();
    let mut journal = TestJournal::default();
    let mut envelope = QueueApplyJournalEnvelope::new(&mut journal_app, &view, &mut journal);
    let journal_stage = envelope.apply_after_preflight_with_caller_preclaim(
        &mut journal_owner,
        &tx_source,
        &preflight,
        hold_preflight(),
        Ter::TES_SUCCESS,
        |prepared| {
            assert_eq!(Some(prepared.clone()), *expected_prepared.borrow());
            Ok(caller_preclaim_stage(prepared))
        },
    );

    assert_eq!(journal_stage, direct_stage);
    assert_eq!(journal_owner.owner(), direct_owner.owner());
    assert_eq!(journal.emitted, direct_emitted.into_inner());
    assert_eq!(journal.emitted.len(), 2);
    assert_eq!(direct_app.preflight_calls, 0);
    assert_eq!(journal_app.preflight_calls, 0);
    assert_eq!(direct_app.preclaim_calls, 0);
    assert_eq!(journal_app.preclaim_calls, 0);
    assert_eq!(direct_app.prepare_multitxn_calls, 1);
    assert_eq!(journal_app.prepare_multitxn_calls, 1);
}

#[test]
fn queue_apply_journal_envelope_owned_metrics_match_owner_shell_app_view_on_owner_snapshot_path() {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let mut view = direct_view(&account);
    view.metrics_snapshot = QueueFeeMetricsSnapshot {
        txns_expected: 256,
        escalation_multiplier: tx::TXQ_BASE_LEVEL,
    };
    view.open_ledger_tx_count = 40;
    let hold_preflight = QueueHoldPreflight::new(
        false,
        false,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        Some(250),
    );

    let mut direct_runtime = direct_app();
    let mut direct_owner = build_direct_owner_shell();
    let direct_emitted = RefCell::new(Vec::new());
    let direct_stage = direct_owner.apply_with_app_view_and_owned_metrics_and_log_sinks(
        &mut direct_runtime,
        &view,
        &tx_source,
        hold_preflight,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(5)),
        Ter::TES_SUCCESS,
        |message| direct_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| direct_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| direct_emitted.borrow_mut().push(format!("info:{message}")),
    );

    let mut journal_app = direct_app();
    let mut journal_owner = build_direct_owner_shell();
    let mut journal = TestJournal::default();
    let mut envelope = QueueApplyJournalEnvelope::new(&mut journal_app, &view, &mut journal);
    let journal_stage = envelope.apply_with_owned_metrics(
        &mut journal_owner,
        &tx_source,
        hold_preflight,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(5)),
        Ter::TES_SUCCESS,
    );

    assert_eq!(journal_stage, direct_stage);
    assert_eq!(journal_owner.owner(), direct_owner.owner());
    assert_eq!(journal.emitted, direct_emitted.into_inner());
    assert_eq!(journal_app.preflight_calls, direct_runtime.preflight_calls);
    assert_eq!(
        journal_app.direct_apply_calls,
        direct_runtime.direct_apply_calls
    );
    assert_eq!(
        journal_app.prepare_multitxn_calls,
        direct_runtime.prepare_multitxn_calls
    );
    assert_eq!(journal_app.preclaim_calls, direct_runtime.preclaim_calls);
    assert_eq!(journal_app.try_clear_calls, direct_runtime.try_clear_calls);
    assert_eq!(
        journal_app.apply_sandbox_calls,
        direct_runtime.apply_sandbox_calls
    );
}

#[test]
fn queue_apply_journal_envelope_owned_metrics_caller_preclaim_matches_owner_shell_app_view_on_owner_snapshot_path()
 {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let mut view = direct_view(&account);
    view.metrics_snapshot = QueueFeeMetricsSnapshot {
        txns_expected: 256,
        escalation_multiplier: tx::TXQ_BASE_LEVEL,
    };
    view.open_ledger_tx_count = 40;
    let hold_preflight = QueueHoldPreflight::new(
        false,
        false,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        Some(250),
    );
    let expected_prepared = RefCell::new(None::<QueueApplyPreparedPreclaimInputs<String>>);

    let mut direct_runtime = direct_app();
    let mut direct_owner = build_direct_owner_shell();
    let direct_emitted = RefCell::new(Vec::new());
    let direct_stage = direct_owner
        .apply_with_app_view_and_owned_metrics_and_caller_preclaim_and_log_sinks(
            &mut direct_runtime,
            &view,
            &tx_source,
            hold_preflight,
            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
            TxConsequences::new(1, SeqProxy::sequence(5)),
            Ter::TES_SUCCESS,
            |prepared| {
                expected_prepared.replace(Some(prepared.clone()));
                Ok(caller_preclaim_stage(prepared))
            },
            |message| direct_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| direct_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| direct_emitted.borrow_mut().push(format!("info:{message}")),
        );

    let mut journal_app = direct_app();
    let mut journal_owner = build_direct_owner_shell();
    let mut journal = TestJournal::default();
    let mut envelope = QueueApplyJournalEnvelope::new(&mut journal_app, &view, &mut journal);
    let journal_stage = envelope.apply_with_owned_metrics_and_caller_preclaim(
        &mut journal_owner,
        &tx_source,
        hold_preflight,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(5)),
        Ter::TES_SUCCESS,
        |prepared| {
            assert_eq!(Some(prepared.clone()), *expected_prepared.borrow());
            Ok(caller_preclaim_stage(prepared))
        },
    );

    assert_eq!(journal_stage, direct_stage);
    assert_eq!(journal_owner.owner(), direct_owner.owner());
    assert_eq!(journal.emitted, direct_emitted.into_inner());
    assert_eq!(direct_runtime.preclaim_calls, 0);
    assert_eq!(journal_app.preclaim_calls, 0);
}

#[test]
fn queue_apply_journal_envelope_after_preflight_owned_metrics_caller_preclaim_matches_owner_shell_app_view_on_owner_snapshot_path()
 {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let mut view = direct_view(&account);
    view.metrics_snapshot = QueueFeeMetricsSnapshot {
        txns_expected: 256,
        escalation_multiplier: tx::TXQ_BASE_LEVEL,
    };
    view.open_ledger_tx_count = 40;
    let preflight = preflight_result(SeqProxy::sequence(5));
    let hold_preflight = QueueHoldPreflight::new(
        false,
        false,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        Some(250),
    );
    let expected_prepared = RefCell::new(None::<QueueApplyPreparedPreclaimInputs<String>>);

    let mut direct_runtime = direct_app();
    let mut direct_owner = build_direct_owner_shell();
    let direct_emitted = RefCell::new(Vec::new());
    let direct_stage = direct_owner
        .apply_after_preflight_with_app_view_and_owned_metrics_and_caller_preclaim_and_log_sinks(
            &mut direct_runtime,
            &view,
            &tx_source,
            &preflight,
            hold_preflight,
            Ter::TES_SUCCESS,
            |prepared| {
                expected_prepared.replace(Some(prepared.clone()));
                Ok(QueueApplyPreclaimStage {
                    view_source: prepared.view_source,
                    trace_message: "trace".to_string(),
                    preclaim_result: preflight_result(SeqProxy::sequence(5))
                        .to_preclaim(9, Ter::TES_SUCCESS),
                })
            },
            |message| direct_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| direct_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| direct_emitted.borrow_mut().push(format!("info:{message}")),
        );

    let mut journal_app = direct_app();
    let mut journal_owner = build_direct_owner_shell();
    let mut journal = TestJournal::default();
    let mut envelope = QueueApplyJournalEnvelope::new(&mut journal_app, &view, &mut journal);
    let journal_stage = envelope.apply_after_preflight_with_owned_metrics_and_caller_preclaim(
        &mut journal_owner,
        &tx_source,
        &preflight,
        hold_preflight,
        Ter::TES_SUCCESS,
        |prepared| {
            assert_eq!(Some(prepared.clone()), *expected_prepared.borrow());
            Ok(QueueApplyPreclaimStage {
                view_source: prepared.view_source,
                trace_message: "trace".to_string(),
                preclaim_result: preflight_result(SeqProxy::sequence(5))
                    .to_preclaim(9, Ter::TES_SUCCESS),
            })
        },
    );

    assert_eq!(journal_stage, direct_stage);
    assert_eq!(journal_owner.owner(), direct_owner.owner());
    assert_eq!(journal.emitted, direct_emitted.into_inner());
    assert_eq!(journal_app.preflight_calls, direct_runtime.preflight_calls);
    assert_eq!(
        journal_app.direct_apply_calls,
        direct_runtime.direct_apply_calls
    );
    assert_eq!(
        journal_app.prepare_multitxn_calls,
        direct_runtime.prepare_multitxn_calls
    );
    assert_eq!(journal_app.preclaim_calls, direct_runtime.preclaim_calls);
    assert_eq!(journal_app.try_clear_calls, direct_runtime.try_clear_calls);
    assert_eq!(
        journal_app.apply_sandbox_calls,
        direct_runtime.apply_sandbox_calls
    );
}
