use std::{cell::RefCell, collections::BTreeMap};

use basics::base_uint::Uint256;
use protocol::{Rules, SeqProxy, Ter};
use tx::{
    ApplyFlags, ApplyResult, BlockerQueueAdmission, FeeQueueEntry, FeeQueueKey, MaybeTx,
    MaybeTxCore, OrderCandidates, PreclaimResult, PreflightResult, QueueAdvanceCandidate,
    QueueApplyAccountStage, QueueApplyCurrentPreclaimClearRuntime, QueueApplyEntryStage,
    QueueApplyExecutionRuntime, QueueApplyHoldPreflightTxSource, QueueApplyLiveOwner,
    QueueApplyObservedAccountLookup, QueueApplyObservedTicketLookup, QueueApplyObservedTxSource,
    QueueApplyObservedViewSource, QueueApplyOwnerShell, QueueApplyPreclaimStage,
    QueueApplyPreflightStage, QueueApplyPreparedPreclaimInputs, QueueApplyQueuedStage,
    QueueApplyViewAdjustment, QueueFeeMetricsSnapshot, QueueHoldPreflight, QueueViews,
    TxConsequences, TxConsequencesCategory, TxQAccount, TxQSetup,
    format_direct_apply_finish_log_message, format_direct_apply_start_log_message,
    format_queue_apply_enqueue_debug_message, format_queue_apply_full_queue_evict_info_message,
    run_queue_apply_after_preflight_with_app_view,
    run_queue_apply_after_preflight_with_app_view_and_caller_preclaim,
    run_queue_apply_after_preflight_with_app_view_and_caller_preclaim_and_log_messages,
    run_queue_apply_after_preflight_with_app_view_and_caller_preclaim_and_log_sinks,
    run_queue_apply_after_preflight_with_app_view_and_log_messages,
    run_queue_apply_after_preflight_with_app_view_and_log_sinks,
    run_queue_apply_after_preflight_with_app_view_and_log_sinks_and_derived_hold_admission,
    run_queue_apply_after_preflight_with_app_view_and_log_sinks_and_derived_hold_preflight,
    run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot,
    run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot_and_caller_preclaim,
    run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot_and_caller_preclaim_and_log_messages,
    run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot_and_caller_preclaim_and_log_sinks,
    run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot_and_log_messages,
    run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot_and_log_sinks,
    run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot_and_log_sinks_and_derived_hold_admission,
    run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot_and_log_sinks_and_derived_hold_preflight,
    run_queue_apply_after_preflight_with_live_owner_from_sources,
    run_queue_apply_after_preflight_with_live_owner_from_sources_and_caller_preclaim,
    run_queue_apply_after_preflight_with_live_owner_from_sources_and_log_messages,
    run_queue_apply_after_preflight_with_live_owner_from_sources_and_log_sinks,
    run_queue_apply_with_app_view, run_queue_apply_with_app_view_and_caller_preclaim,
    run_queue_apply_with_app_view_and_caller_preclaim_and_log_messages,
    run_queue_apply_with_app_view_and_caller_preclaim_and_log_sinks,
    run_queue_apply_with_app_view_and_derived_hold_admission_and_log_messages,
    run_queue_apply_with_app_view_and_derived_hold_preflight_and_log_messages,
    run_queue_apply_with_app_view_and_derived_preflight_facts_and_hold_admission_and_log_messages,
    run_queue_apply_with_app_view_and_derived_preflight_facts_and_log_messages,
    run_queue_apply_with_app_view_and_log_messages, run_queue_apply_with_app_view_and_log_sinks,
    run_queue_apply_with_app_view_and_log_sinks_and_derived_hold_admission,
    run_queue_apply_with_app_view_and_log_sinks_and_derived_hold_preflight,
    run_queue_apply_with_app_view_and_log_sinks_and_derived_preflight_facts,
    run_queue_apply_with_app_view_and_log_sinks_and_derived_preflight_facts_and_hold_admission,
    run_queue_apply_with_app_view_and_metrics_snapshot,
    run_queue_apply_with_app_view_and_metrics_snapshot_and_caller_preclaim,
    run_queue_apply_with_app_view_and_metrics_snapshot_and_caller_preclaim_and_log_messages,
    run_queue_apply_with_app_view_and_metrics_snapshot_and_caller_preclaim_and_log_sinks,
    run_queue_apply_with_app_view_and_metrics_snapshot_and_log_messages,
    run_queue_apply_with_app_view_and_metrics_snapshot_and_log_sinks,
    run_queue_apply_with_app_view_and_metrics_snapshot_and_log_sinks_and_derived_hold_admission,
    run_queue_apply_with_app_view_and_metrics_snapshot_and_log_sinks_and_derived_hold_preflight,
    run_queue_apply_with_app_view_and_metrics_snapshot_and_log_sinks_and_derived_preflight_facts,
    run_queue_apply_with_app_view_and_metrics_snapshot_and_log_sinks_and_derived_preflight_facts_and_hold_admission,
    run_queue_apply_with_live_owner_from_sources,
    run_queue_apply_with_live_owner_from_sources_and_caller_preclaim,
    run_queue_apply_with_live_owner_from_sources_and_log_messages,
    run_queue_apply_with_live_owner_from_sources_and_log_sinks,
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

impl QueueApplyExecutionRuntime<&'static str, &'static str, &'static str> for TestApplyApp {
    fn run_preflight(
        &mut self,
    ) -> PreflightResult<&'static str, TxConsequences, &'static str, &'static str> {
        self.preflight_calls += 1;
        preflight_result(SeqProxy::sequence(6))
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

impl QueueApplyCurrentPreclaimClearRuntime<&'static str, &'static str, &'static str>
    for TestLogSinkApp
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

fn setup() -> TxQSetup {
    TxQSetup::default()
}

fn build_owner(
    current_max_size: Option<usize>,
) -> QueueApplyLiveOwner<String, &'static str, &'static str, &'static str> {
    QueueApplyLiveOwner::new_from_setup(
        &setup(),
        current_max_size,
        OrderCandidates::new(Uint256::from_u64(0)),
        build_views(),
    )
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

fn build_flow_owner(
    current_max_size: Option<usize>,
) -> QueueApplyLiveOwner<String, &'static str, &'static str, &'static str> {
    QueueApplyLiveOwner::new_from_setup(
        &setup(),
        current_max_size,
        OrderCandidates::new(Uint256::from_u64(0)),
        build_flow_views(),
    )
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

    QueueApplyOwnerShell::new(QueueApplyLiveOwner::new_from_setup(
        &setup(),
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
    QueueApplyOwnerShell::new(QueueApplyLiveOwner::new_from_setup(
        &setup(),
        Some(10),
        OrderCandidates::new(Uint256::from_u64(0)),
        QueueViews::new(BTreeMap::new(), Vec::new()),
    ))
}

fn queueing_app() -> TestLogSinkApp {
    TestLogSinkApp {
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

fn direct_app() -> TestLogSinkApp {
    TestLogSinkApp {
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
fn queue_apply_owner_shell_matches_live_owner_wrapper() {
    let account = String::from("acct");
    let tx_source = tx_source(&account);
    let view_source = view_source();

    let mut live_owner = build_owner(Some(10));
    let live_owner_stage = run_queue_apply_with_live_owner_from_sources(
        &mut live_owner,
        &tx_source,
        &view_source,
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

    let mut shell = QueueApplyOwnerShell::new(build_owner(Some(10)));
    let shell_stage = shell.apply(
        &tx_source,
        &view_source,
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

    assert_eq!(shell_stage, live_owner_stage);
    assert_eq!(shell.owner(), &live_owner);
    assert_eq!(shell_stage, expected_stage());
}

#[test]
fn queue_apply_owner_shell_after_preflight_matches_live_owner_wrapper() {
    let account = String::from("acct");
    let tx_source = tx_source(&account);
    let view_source = view_source();
    let preflight = preflight_result(SeqProxy::sequence(6));

    let mut live_owner = build_owner(Some(10));
    let live_owner_stage = run_queue_apply_after_preflight_with_live_owner_from_sources(
        &mut live_owner,
        &tx_source,
        &view_source,
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

    let mut shell = QueueApplyOwnerShell::new(build_owner(Some(10)));
    let shell_stage = shell.apply_after_preflight(
        &tx_source,
        &view_source,
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

    assert_eq!(shell_stage, live_owner_stage);
    assert_eq!(shell.owner(), &live_owner);
    assert_eq!(shell_stage, expected_stage());
}

#[test]
fn queue_apply_owner_shell_log_messages_match_live_owner_wrapper() {
    let account = String::from("acct");
    let tx_source = flow_tx_source(&account);
    let view_source = flow_view_source();
    let preflight = flow_preflight_result();

    let mut live_owner = build_flow_owner(Some(10));
    let live_owner_stage = run_queue_apply_with_live_owner_from_sources_and_log_messages(
        &mut live_owner,
        &tx_source,
        &view_source,
        hold_preflight(),
        ApplyFlags::NONE,
        preflight.clone().consequences,
        Ter::TES_SUCCESS,
        || preflight.clone(),
        |_| unreachable!("direct apply should fall through"),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |_| {
            PreclaimResult::new(
                100,
                "tx",
                None::<&str>,
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            )
        },
        || ApplyResult::new(Ter::TES_SUCCESS, true, false),
        || {},
    );

    let mut shell = QueueApplyOwnerShell::new(build_flow_owner(Some(10)));
    let shell_stage = shell.apply_with_log_messages(
        &tx_source,
        &view_source,
        hold_preflight(),
        ApplyFlags::NONE,
        preflight.clone().consequences,
        Ter::TES_SUCCESS,
        || preflight.clone(),
        |_| unreachable!("direct apply should fall through"),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |_| {
            PreclaimResult::new(
                100,
                "tx",
                None::<&str>,
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            )
        },
        || ApplyResult::new(Ter::TES_SUCCESS, true, false),
        || {},
    );

    assert_eq!(shell_stage, live_owner_stage);
    assert_eq!(shell.owner(), &live_owner);
}

#[test]
fn queue_apply_owner_shell_after_preflight_log_messages_match_live_owner_wrapper() {
    let account = String::from("acct");
    let tx_source = flow_tx_source(&account);
    let view_source = flow_view_source();
    let preflight = flow_preflight_result();

    let mut live_owner = build_flow_owner(Some(10));
    let live_owner_stage =
        run_queue_apply_after_preflight_with_live_owner_from_sources_and_log_messages(
            &mut live_owner,
            &tx_source,
            &view_source,
            &preflight,
            hold_preflight(),
            Ter::TES_SUCCESS,
            |_| unreachable!("direct apply should fall through"),
            || ApplyResult::new(Ter::TES_SUCCESS, true, true),
            |_| true,
            |_| {
                PreclaimResult::new(
                    100,
                    "tx",
                    None::<&str>,
                    ApplyFlags::NONE,
                    "journal",
                    Ter::TES_SUCCESS,
                )
            },
            || ApplyResult::new(Ter::TES_SUCCESS, true, false),
            || {},
        );

    let mut shell = QueueApplyOwnerShell::new(build_flow_owner(Some(10)));
    let shell_stage = shell.apply_after_preflight_with_log_messages(
        &tx_source,
        &view_source,
        &preflight,
        hold_preflight(),
        Ter::TES_SUCCESS,
        |_| unreachable!("direct apply should fall through"),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |_| {
            PreclaimResult::new(
                100,
                "tx",
                None::<&str>,
                ApplyFlags::NONE,
                "journal",
                Ter::TES_SUCCESS,
            )
        },
        || ApplyResult::new(Ter::TES_SUCCESS, true, false),
        || {},
    );

    assert_eq!(shell_stage, live_owner_stage);
    assert_eq!(shell.owner(), &live_owner);
}

#[test]
fn queue_apply_owner_shell_with_caller_preclaim_matches_live_owner_wrapper() {
    let account = String::from("acct");
    let tx_source = flow_tx_source(&account);
    let view_source = flow_view_source();
    let preflight = flow_preflight_result();
    let expected_prepared = RefCell::new(None::<QueueApplyPreparedPreclaimInputs<String>>);

    let mut live_owner = build_flow_owner(Some(10));
    let live_owner_stage = run_queue_apply_with_live_owner_from_sources_and_caller_preclaim(
        &mut live_owner,
        &tx_source,
        &view_source,
        hold_preflight(),
        ApplyFlags::NONE,
        preflight.clone().consequences,
        Ter::TES_SUCCESS,
        || preflight.clone(),
        |_| unreachable!("direct apply should fall through"),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |prepared| {
            expected_prepared.replace(Some(prepared.clone()));
            Ok(QueueApplyPreclaimStage {
                view_source: prepared.view_source,
                trace_message: "trace".to_string(),
                preclaim_result: flow_preflight_result().to_preclaim(9, Ter::TES_SUCCESS),
            })
        },
        || ApplyResult::new(Ter::TES_SUCCESS, true, false),
        || {},
    );

    let mut shell = QueueApplyOwnerShell::new(build_flow_owner(Some(10)));
    let shell_stage = shell.apply_with_caller_preclaim(
        &tx_source,
        &view_source,
        hold_preflight(),
        ApplyFlags::NONE,
        preflight.clone().consequences,
        Ter::TES_SUCCESS,
        || preflight.clone(),
        |_| unreachable!("direct apply should fall through"),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |prepared| {
            assert_eq!(Some(prepared.clone()), *expected_prepared.borrow());
            Ok(QueueApplyPreclaimStage {
                view_source: prepared.view_source,
                trace_message: "trace".to_string(),
                preclaim_result: flow_preflight_result().to_preclaim(9, Ter::TES_SUCCESS),
            })
        },
        || ApplyResult::new(Ter::TES_SUCCESS, true, false),
        || {},
    );

    assert_eq!(shell_stage, live_owner_stage);
    assert_eq!(shell.owner(), &live_owner);
}

#[test]
fn queue_apply_owner_shell_after_preflight_with_caller_preclaim_matches_live_owner_wrapper() {
    let account = String::from("acct");
    let tx_source = flow_tx_source(&account);
    let view_source = flow_view_source();
    let preflight = flow_preflight_result();
    let expected_prepared = RefCell::new(None::<QueueApplyPreparedPreclaimInputs<String>>);

    let mut live_owner = build_flow_owner(Some(10));
    let live_owner_stage =
        run_queue_apply_after_preflight_with_live_owner_from_sources_and_caller_preclaim(
            &mut live_owner,
            &tx_source,
            &view_source,
            &preflight,
            hold_preflight(),
            Ter::TES_SUCCESS,
            |_| unreachable!("direct apply should fall through"),
            || ApplyResult::new(Ter::TES_SUCCESS, true, true),
            |_| true,
            |prepared| {
                expected_prepared.replace(Some(prepared.clone()));
                Ok(QueueApplyPreclaimStage {
                    view_source: prepared.view_source,
                    trace_message: "trace".to_string(),
                    preclaim_result: flow_preflight_result().to_preclaim(9, Ter::TES_SUCCESS),
                })
            },
            || ApplyResult::new(Ter::TES_SUCCESS, true, false),
            || {},
        );

    let mut shell = QueueApplyOwnerShell::new(build_flow_owner(Some(10)));
    let shell_stage = shell.apply_after_preflight_with_caller_preclaim(
        &tx_source,
        &view_source,
        &preflight,
        hold_preflight(),
        Ter::TES_SUCCESS,
        |_| unreachable!("direct apply should fall through"),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |prepared| {
            assert_eq!(Some(prepared.clone()), *expected_prepared.borrow());
            Ok(QueueApplyPreclaimStage {
                view_source: prepared.view_source,
                trace_message: "trace".to_string(),
                preclaim_result: flow_preflight_result().to_preclaim(9, Ter::TES_SUCCESS),
            })
        },
        || ApplyResult::new(Ter::TES_SUCCESS, true, false),
        || {},
    );

    assert_eq!(shell_stage, live_owner_stage);
    assert_eq!(shell.owner(), &live_owner);
}

#[test]
fn queue_apply_owner_shell_exposes_live_owner_accessors() {
    let mut shell = QueueApplyOwnerShell::new(build_owner(Some(1)));
    assert!(!shell.owner().observed_queue(Ter::TES_SUCCESS).queue_is_full);

    let removed = shell.owner_mut().views_mut().accounts.remove("acct");
    assert!(removed.is_some());
    shell
        .owner_mut()
        .views_mut()
        .fee_order
        .push(tx::FeeQueueEntry::new(
            tx::FeeQueueKey::new(String::from("acct"), SeqProxy::sequence(5)),
            tx::QueueAdvanceCandidate {
                fee_level: 90,
                tx_id: Uint256::from_u64(5),
                seq_proxy: SeqProxy::sequence(5),
            },
        ));

    let observed = shell.owner().observed_queue(Ter::TES_SUCCESS);
    assert!(observed.queue_is_full);
    assert_eq!(observed.maximum_txn_per_account, 10);
}

#[test]
fn queue_apply_owner_shell_app_view_method_matches_free_wrapper() {
    let account = String::from("acct");
    let tx_source = tx_source(&account);
    let view_source = view_source();

    let mut free_app = TestApplyApp::default();
    let mut free_shell = QueueApplyOwnerShell::new(build_owner(Some(10)));
    let free_stage = run_queue_apply_with_app_view(
        &mut free_shell,
        &mut free_app,
        &view_source,
        &tx_source,
        hold_preflight(),
        ApplyFlags::NONE,
        blocker_consequences(SeqProxy::sequence(6)),
        Ter::TES_SUCCESS,
    );

    let mut method_app = TestApplyApp::default();
    let mut method_shell = QueueApplyOwnerShell::new(build_owner(Some(10)));
    let method_stage = method_shell.apply_with_app_view(
        &mut method_app,
        &view_source,
        &tx_source,
        hold_preflight(),
        ApplyFlags::NONE,
        blocker_consequences(SeqProxy::sequence(6)),
        Ter::TES_SUCCESS,
    );

    assert_eq!(method_stage, free_stage);
    assert_eq!(method_shell.owner(), free_shell.owner());
    assert_eq!(method_stage, expected_stage());
    assert_eq!(method_app.preflight_calls, 1);
    assert_eq!(method_app.direct_apply_calls, 0);
    assert_eq!(method_app.prepare_multitxn_calls, 0);
    assert_eq!(method_app.preclaim_calls, 0);
    assert_eq!(method_app.try_clear_calls, 0);
    assert_eq!(method_app.apply_sandbox_calls, 0);
    assert!(method_app.trace_messages.is_empty());
}

#[test]
fn queue_apply_owner_shell_app_view_owned_metrics_match_explicit_snapshot_wrapper() {
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

    let hold_preflight = QueueHoldPreflight::new(
        false,
        false,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        Some(250),
    );
    let consequences = TxConsequences::new(1, SeqProxy::sequence(5));

    let mut free_app = direct_app();
    let mut free_shell = build_direct_owner_shell();
    let free_snapshot = free_shell.owner().metrics().snapshot();
    let free_stage = run_queue_apply_with_app_view_and_metrics_snapshot(
        &mut free_shell,
        &mut free_app,
        &view_source,
        &tx_source,
        free_snapshot,
        hold_preflight,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        consequences,
        Ter::TES_SUCCESS,
    );

    let mut plain_app = direct_app();
    let mut plain_shell = build_direct_owner_shell();
    let plain_stage = run_queue_apply_with_app_view(
        &mut plain_shell,
        &mut plain_app,
        &view_source,
        &tx_source,
        hold_preflight,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        consequences,
        Ter::TES_SUCCESS,
    );

    let mut method_app = direct_app();
    let mut method_shell = build_direct_owner_shell();
    let method_stage = method_shell.apply_with_app_view_and_owned_metrics(
        &mut method_app,
        &view_source,
        &tx_source,
        hold_preflight,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        consequences,
        Ter::TES_SUCCESS,
    );

    assert_eq!(method_stage, free_stage);
    assert_eq!(method_shell.owner(), free_shell.owner());
    assert_ne!(plain_stage, method_stage);
    assert_eq!(plain_app.direct_apply_calls, 1);
    assert_eq!(method_app.direct_apply_calls, 0);
}

#[test]
fn queue_apply_owner_shell_app_view_owned_metrics_log_messages_match_explicit_snapshot_wrapper() {
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

    let hold_preflight = QueueHoldPreflight::new(
        false,
        false,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        Some(250),
    );
    let consequences = TxConsequences::new(1, SeqProxy::sequence(5));

    let mut free_app = direct_app();
    let mut free_shell = build_direct_owner_shell();
    let free_snapshot = free_shell.owner().metrics().snapshot();
    let free_stage = run_queue_apply_with_app_view_and_metrics_snapshot_and_log_messages(
        &mut free_shell,
        &mut free_app,
        &view_source,
        &tx_source,
        free_snapshot,
        hold_preflight,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        consequences,
        Ter::TES_SUCCESS,
    );

    let mut plain_app = direct_app();
    let mut plain_shell = build_direct_owner_shell();
    let plain_stage = run_queue_apply_with_app_view_and_log_messages(
        &mut plain_shell,
        &mut plain_app,
        &view_source,
        &tx_source,
        hold_preflight,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        consequences,
        Ter::TES_SUCCESS,
    );

    let mut method_app = direct_app();
    let mut method_shell = build_direct_owner_shell();
    let method_stage = method_shell.apply_with_app_view_and_owned_metrics_and_log_messages(
        &mut method_app,
        &view_source,
        &tx_source,
        hold_preflight,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        consequences,
        Ter::TES_SUCCESS,
    );

    assert_eq!(method_stage, free_stage);
    assert_eq!(method_shell.owner(), free_shell.owner());
    assert_ne!(plain_stage, method_stage);
    assert_eq!(
        method_stage.queue_log_messages,
        free_stage.queue_log_messages
    );
    assert_eq!(plain_app.direct_apply_calls, 1);
    assert_eq!(method_app.direct_apply_calls, 0);
}

#[test]
fn queue_apply_owner_shell_app_view_caller_preclaim_matches_free_wrapper() {
    let account = String::from("acct");
    let tx_source = flow_tx_source(&account);
    let view_source = flow_view_source();
    let expected_prepared = RefCell::new(None::<QueueApplyPreparedPreclaimInputs<String>>);

    let mut free_app = FlowApplyApp::default();
    let mut free_shell = QueueApplyOwnerShell::new(build_flow_owner(Some(10)));
    let free_stage = run_queue_apply_with_app_view_and_caller_preclaim(
        &mut free_shell,
        &mut free_app,
        &view_source,
        &tx_source,
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

    let mut method_app = FlowApplyApp::default();
    let mut method_shell = QueueApplyOwnerShell::new(build_flow_owner(Some(10)));
    let method_stage = method_shell.apply_with_app_view_and_caller_preclaim(
        &mut method_app,
        &view_source,
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

    assert_eq!(method_stage, free_stage);
    assert_eq!(method_shell.owner(), free_shell.owner());
    assert_eq!(method_app.preflight_calls, 1);
    assert_eq!(method_app.preclaim_calls, 0);
    assert_eq!(method_app.direct_apply_calls, 0);
    assert_eq!(method_app.prepare_multitxn_calls, 1);
}

#[test]
fn queue_apply_owner_shell_app_view_caller_preclaim_log_messages_match_free_wrapper_on_full_queue_path()
 {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(6),
    };
    let view_source = queueing_view(&account);
    let expected_prepared = RefCell::new(None::<QueueApplyPreparedPreclaimInputs<String>>);

    let mut free_app = queueing_app();
    let mut free_shell = build_full_queue_owner_shell();
    let free_stage = run_queue_apply_with_app_view_and_caller_preclaim_and_log_messages(
        &mut free_shell,
        &mut free_app,
        &view_source,
        &tx_source,
        hold_preflight(),
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(6)),
        Ter::TES_SUCCESS,
        |prepared| {
            expected_prepared.replace(Some(prepared.clone()));
            Ok(QueueApplyPreclaimStage {
                view_source: prepared.view_source,
                trace_message: "trace".to_string(),
                preclaim_result: queueing_preflight_result(SeqProxy::sequence(6))
                    .to_preclaim(100, Ter::TES_SUCCESS),
            })
        },
    );

    let mut method_app = queueing_app();
    let mut method_shell = build_full_queue_owner_shell();
    let method_stage = method_shell.apply_with_app_view_and_caller_preclaim_and_log_messages(
        &mut method_app,
        &view_source,
        &tx_source,
        hold_preflight(),
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(6)),
        Ter::TES_SUCCESS,
        |prepared| {
            assert_eq!(Some(prepared.clone()), *expected_prepared.borrow());
            Ok(QueueApplyPreclaimStage {
                view_source: prepared.view_source,
                trace_message: "trace".to_string(),
                preclaim_result: queueing_preflight_result(SeqProxy::sequence(6))
                    .to_preclaim(100, Ter::TES_SUCCESS),
            })
        },
    );

    assert_eq!(method_stage, free_stage);
    assert_eq!(method_shell.owner(), free_shell.owner());
    assert_eq!(
        method_stage.queue_log_messages,
        free_stage.queue_log_messages
    );
    assert_eq!(method_app.preflight_calls, 1);
    assert_eq!(method_app.preclaim_calls, 0);
    assert_eq!(method_app.direct_apply_calls, 0);
    assert_eq!(method_app.prepare_multitxn_calls, 1);
}

#[test]
fn queue_apply_owner_shell_app_view_caller_preclaim_owned_metrics_match_explicit_snapshot_wrapper()
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
    let hold_preflight = QueueHoldPreflight::new(
        false,
        false,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        Some(250),
    );
    let expected_prepared = RefCell::new(None::<QueueApplyPreparedPreclaimInputs<String>>);

    let mut free_app = direct_app();
    let mut free_shell = build_direct_owner_shell();
    let free_snapshot = free_shell.owner().metrics().snapshot();
    let free_stage = run_queue_apply_with_app_view_and_metrics_snapshot_and_caller_preclaim(
        &mut free_shell,
        &mut free_app,
        &view_source,
        &tx_source,
        free_snapshot,
        hold_preflight,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(5)),
        Ter::TES_SUCCESS,
        |prepared| {
            expected_prepared.replace(Some(prepared.clone()));
            Ok(QueueApplyPreclaimStage {
                view_source: prepared.view_source,
                trace_message: "trace".to_string(),
                preclaim_result: queueing_preflight_result(SeqProxy::sequence(5))
                    .to_preclaim(9, Ter::TES_SUCCESS),
            })
        },
    );

    let plain_prepared = RefCell::new(0usize);
    let mut plain_app = direct_app();
    let mut plain_shell = build_direct_owner_shell();
    let plain_stage = run_queue_apply_with_app_view_and_caller_preclaim(
        &mut plain_shell,
        &mut plain_app,
        &view_source,
        &tx_source,
        hold_preflight,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(5)),
        Ter::TES_SUCCESS,
        |prepared| {
            *plain_prepared.borrow_mut() += 1;
            Ok(QueueApplyPreclaimStage {
                view_source: prepared.view_source,
                trace_message: "trace".to_string(),
                preclaim_result: queueing_preflight_result(SeqProxy::sequence(5))
                    .to_preclaim(9, Ter::TES_SUCCESS),
            })
        },
    );

    let mut method_app = direct_app();
    let mut method_shell = build_direct_owner_shell();
    let method_stage = method_shell.apply_with_app_view_and_owned_metrics_and_caller_preclaim(
        &mut method_app,
        &view_source,
        &tx_source,
        hold_preflight,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(5)),
        Ter::TES_SUCCESS,
        |prepared| {
            assert_eq!(Some(prepared.clone()), *expected_prepared.borrow());
            Ok(QueueApplyPreclaimStage {
                view_source: prepared.view_source,
                trace_message: "trace".to_string(),
                preclaim_result: queueing_preflight_result(SeqProxy::sequence(5))
                    .to_preclaim(9, Ter::TES_SUCCESS),
            })
        },
    );

    assert_eq!(method_stage, free_stage);
    assert_eq!(method_shell.owner(), free_shell.owner());
    assert_ne!(plain_stage, method_stage);
    assert_eq!(*plain_prepared.borrow(), 0);
    assert_eq!(plain_app.direct_apply_calls, 1);
    assert_eq!(method_app.direct_apply_calls, 0);
}

#[test]
fn queue_apply_owner_shell_app_view_caller_preclaim_owned_metrics_log_messages_match_explicit_snapshot_wrapper()
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
    let hold_preflight = QueueHoldPreflight::new(
        false,
        false,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        Some(250),
    );
    let expected_prepared = RefCell::new(None::<QueueApplyPreparedPreclaimInputs<String>>);

    let mut free_app = direct_app();
    let mut free_shell = build_direct_owner_shell();
    let free_snapshot = free_shell.owner().metrics().snapshot();
    let free_stage =
        run_queue_apply_with_app_view_and_metrics_snapshot_and_caller_preclaim_and_log_messages(
            &mut free_shell,
            &mut free_app,
            &view_source,
            &tx_source,
            free_snapshot,
            hold_preflight,
            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
            TxConsequences::new(1, SeqProxy::sequence(5)),
            Ter::TES_SUCCESS,
            |prepared| {
                expected_prepared.replace(Some(prepared.clone()));
                Ok(QueueApplyPreclaimStage {
                    view_source: prepared.view_source,
                    trace_message: "trace".to_string(),
                    preclaim_result: queueing_preflight_result(SeqProxy::sequence(5))
                        .to_preclaim(9, Ter::TES_SUCCESS),
                })
            },
        );

    let plain_prepared = RefCell::new(0usize);
    let mut plain_app = direct_app();
    let mut plain_shell = build_direct_owner_shell();
    let plain_stage = run_queue_apply_with_app_view_and_caller_preclaim_and_log_messages(
        &mut plain_shell,
        &mut plain_app,
        &view_source,
        &tx_source,
        hold_preflight,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(5)),
        Ter::TES_SUCCESS,
        |prepared| {
            *plain_prepared.borrow_mut() += 1;
            Ok(QueueApplyPreclaimStage {
                view_source: prepared.view_source,
                trace_message: "trace".to_string(),
                preclaim_result: queueing_preflight_result(SeqProxy::sequence(5))
                    .to_preclaim(9, Ter::TES_SUCCESS),
            })
        },
    );

    let mut method_app = direct_app();
    let mut method_shell = build_direct_owner_shell();
    let method_stage = method_shell
        .apply_with_app_view_and_owned_metrics_and_caller_preclaim_and_log_messages(
            &mut method_app,
            &view_source,
            &tx_source,
            hold_preflight,
            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
            TxConsequences::new(1, SeqProxy::sequence(5)),
            Ter::TES_SUCCESS,
            |prepared| {
                assert_eq!(Some(prepared.clone()), *expected_prepared.borrow());
                Ok(QueueApplyPreclaimStage {
                    view_source: prepared.view_source,
                    trace_message: "trace".to_string(),
                    preclaim_result: queueing_preflight_result(SeqProxy::sequence(5))
                        .to_preclaim(9, Ter::TES_SUCCESS),
                })
            },
        );

    assert_eq!(method_stage, free_stage);
    assert_eq!(method_shell.owner(), free_shell.owner());
    assert_ne!(plain_stage, method_stage);
    assert_eq!(
        method_stage.queue_log_messages,
        free_stage.queue_log_messages
    );
    assert_eq!(*plain_prepared.borrow(), 0);
    assert_eq!(plain_app.direct_apply_calls, 1);
    assert_eq!(method_app.direct_apply_calls, 0);
}

#[test]
fn queue_apply_owner_shell_app_view_after_preflight_method_matches_free_wrapper() {
    let account = String::from("acct");
    let tx_source = tx_source(&account);
    let view_source = view_source();
    let preflight = preflight_result(SeqProxy::sequence(6));

    let mut free_app = TestApplyApp::default();
    let mut free_shell = QueueApplyOwnerShell::new(build_owner(Some(10)));
    let free_stage = run_queue_apply_after_preflight_with_app_view(
        &mut free_shell,
        &mut free_app,
        &view_source,
        &tx_source,
        &preflight,
        hold_preflight(),
        Ter::TES_SUCCESS,
    );

    let mut method_app = TestApplyApp::default();
    let mut method_shell = QueueApplyOwnerShell::new(build_owner(Some(10)));
    let method_stage = method_shell.apply_after_preflight_with_app_view(
        &mut method_app,
        &view_source,
        &tx_source,
        &preflight,
        hold_preflight(),
        Ter::TES_SUCCESS,
    );

    assert_eq!(method_stage, free_stage);
    assert_eq!(method_shell.owner(), free_shell.owner());
    assert_eq!(method_stage, expected_stage());
    assert_eq!(method_app.preflight_calls, 0);
    assert_eq!(method_app.direct_apply_calls, 0);
    assert_eq!(method_app.prepare_multitxn_calls, 0);
    assert_eq!(method_app.preclaim_calls, 0);
    assert_eq!(method_app.try_clear_calls, 0);
    assert_eq!(method_app.apply_sandbox_calls, 0);
    assert!(method_app.trace_messages.is_empty());
}

#[test]
fn queue_apply_owner_shell_app_view_after_preflight_caller_preclaim_owned_metrics_match_explicit_snapshot_wrapper()
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
    let preflight = queueing_preflight_result(SeqProxy::sequence(5));
    let hold_preflight = QueueHoldPreflight::new(
        false,
        false,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        Some(250),
    );
    let expected_prepared = RefCell::new(None::<QueueApplyPreparedPreclaimInputs<String>>);

    let mut free_app = direct_app();
    let mut free_shell = build_direct_owner_shell();
    let free_snapshot = free_shell.owner().metrics().snapshot();
    let free_stage =
        run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot_and_caller_preclaim(
            &mut free_shell,
            &mut free_app,
            &view_source,
            &tx_source,
            free_snapshot,
            &preflight,
            hold_preflight,
            Ter::TES_SUCCESS,
            |prepared| {
                expected_prepared.replace(Some(prepared.clone()));
                Ok(QueueApplyPreclaimStage {
                    view_source: prepared.view_source,
                    trace_message: "trace".to_string(),
                    preclaim_result: queueing_preflight_result(SeqProxy::sequence(5))
                        .to_preclaim(9, Ter::TES_SUCCESS),
                })
            },
        );

    let plain_prepared = RefCell::new(0usize);
    let mut plain_app = direct_app();
    let mut plain_shell = build_direct_owner_shell();
    let plain_stage = run_queue_apply_after_preflight_with_app_view_and_caller_preclaim(
        &mut plain_shell,
        &mut plain_app,
        &view_source,
        &tx_source,
        &preflight,
        hold_preflight,
        Ter::TES_SUCCESS,
        |prepared| {
            *plain_prepared.borrow_mut() += 1;
            Ok(QueueApplyPreclaimStage {
                view_source: prepared.view_source,
                trace_message: "trace".to_string(),
                preclaim_result: queueing_preflight_result(SeqProxy::sequence(5))
                    .to_preclaim(9, Ter::TES_SUCCESS),
            })
        },
    );

    let mut method_app = direct_app();
    let mut method_shell = build_direct_owner_shell();
    let method_stage = method_shell
        .apply_after_preflight_with_app_view_and_owned_metrics_and_caller_preclaim(
            &mut method_app,
            &view_source,
            &tx_source,
            &preflight,
            hold_preflight,
            Ter::TES_SUCCESS,
            |prepared| {
                assert_eq!(Some(prepared.clone()), *expected_prepared.borrow());
                Ok(QueueApplyPreclaimStage {
                    view_source: prepared.view_source,
                    trace_message: "trace".to_string(),
                    preclaim_result: queueing_preflight_result(SeqProxy::sequence(5))
                        .to_preclaim(9, Ter::TES_SUCCESS),
                })
            },
        );

    assert_eq!(method_stage, free_stage);
    assert_eq!(method_shell.owner(), free_shell.owner());
    assert_ne!(plain_stage, method_stage);
    assert_eq!(*plain_prepared.borrow(), 0);
    assert_eq!(plain_app.direct_apply_calls, 1);
    assert_eq!(method_app.direct_apply_calls, 0);
}

#[test]
fn queue_apply_owner_shell_app_view_after_preflight_caller_preclaim_owned_metrics_log_messages_match_explicit_snapshot_wrapper()
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
    let preflight = queueing_preflight_result(SeqProxy::sequence(5));
    let hold_preflight = QueueHoldPreflight::new(
        false,
        false,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        Some(250),
    );
    let expected_prepared = RefCell::new(None::<QueueApplyPreparedPreclaimInputs<String>>);

    let mut free_app = direct_app();
    let mut free_shell = build_direct_owner_shell();
    let free_snapshot = free_shell.owner().metrics().snapshot();
    let free_stage = run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot_and_caller_preclaim_and_log_messages(
        &mut free_shell,
        &mut free_app,
        &view_source,
        &tx_source,
        free_snapshot,
        &preflight,
        hold_preflight,
        Ter::TES_SUCCESS,
        |prepared| {
            expected_prepared.replace(Some(prepared.clone()));
            Ok(QueueApplyPreclaimStage {
                view_source: prepared.view_source,
                trace_message: "trace".to_string(),
                preclaim_result: queueing_preflight_result(SeqProxy::sequence(5))
                    .to_preclaim(9, Ter::TES_SUCCESS),
            })
        },
    );

    let plain_prepared = RefCell::new(0usize);
    let mut plain_app = direct_app();
    let mut plain_shell = build_direct_owner_shell();
    let plain_stage =
        run_queue_apply_after_preflight_with_app_view_and_caller_preclaim_and_log_messages(
            &mut plain_shell,
            &mut plain_app,
            &view_source,
            &tx_source,
            &preflight,
            hold_preflight,
            Ter::TES_SUCCESS,
            |prepared| {
                *plain_prepared.borrow_mut() += 1;
                Ok(QueueApplyPreclaimStage {
                    view_source: prepared.view_source,
                    trace_message: "trace".to_string(),
                    preclaim_result: queueing_preflight_result(SeqProxy::sequence(5))
                        .to_preclaim(9, Ter::TES_SUCCESS),
                })
            },
        );

    let mut method_app = direct_app();
    let mut method_shell = build_direct_owner_shell();
    let method_stage = method_shell
        .apply_after_preflight_with_app_view_and_owned_metrics_and_caller_preclaim_and_log_messages(
            &mut method_app,
            &view_source,
            &tx_source,
            &preflight,
            hold_preflight,
            Ter::TES_SUCCESS,
            |prepared| {
                assert_eq!(Some(prepared.clone()), *expected_prepared.borrow());
                Ok(QueueApplyPreclaimStage {
                    view_source: prepared.view_source,
                    trace_message: "trace".to_string(),
                    preclaim_result: queueing_preflight_result(SeqProxy::sequence(5))
                        .to_preclaim(9, Ter::TES_SUCCESS),
                })
            },
        );

    assert_eq!(method_stage, free_stage);
    assert_eq!(method_shell.owner(), free_shell.owner());
    assert_ne!(plain_stage, method_stage);
    assert_eq!(
        method_stage.queue_log_messages,
        free_stage.queue_log_messages
    );
    assert_eq!(*plain_prepared.borrow(), 0);
    assert_eq!(plain_app.direct_apply_calls, 1);
    assert_eq!(method_app.direct_apply_calls, 0);
}

#[test]
fn queue_apply_owner_shell_app_view_after_preflight_owned_metrics_match_explicit_snapshot_wrapper()
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
    let preflight = queueing_preflight_result(SeqProxy::sequence(5));
    let hold_preflight = QueueHoldPreflight::new(
        false,
        false,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        Some(250),
    );

    let mut free_app = direct_app();
    let mut free_shell = build_direct_owner_shell();
    let free_snapshot = free_shell.owner().metrics().snapshot();
    let free_stage = run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot(
        &mut free_shell,
        &mut free_app,
        &view_source,
        &tx_source,
        free_snapshot,
        &preflight,
        hold_preflight,
        Ter::TES_SUCCESS,
    );

    let mut plain_app = direct_app();
    let mut plain_shell = build_direct_owner_shell();
    let plain_stage = run_queue_apply_after_preflight_with_app_view(
        &mut plain_shell,
        &mut plain_app,
        &view_source,
        &tx_source,
        &preflight,
        hold_preflight,
        Ter::TES_SUCCESS,
    );

    let mut method_app = direct_app();
    let mut method_shell = build_direct_owner_shell();
    let method_stage = method_shell.apply_after_preflight_with_app_view_and_owned_metrics(
        &mut method_app,
        &view_source,
        &tx_source,
        &preflight,
        hold_preflight,
        Ter::TES_SUCCESS,
    );

    assert_eq!(method_stage, free_stage);
    assert_eq!(method_shell.owner(), free_shell.owner());
    assert_ne!(plain_stage, method_stage);
    assert_eq!(plain_app.direct_apply_calls, 1);
    assert_eq!(method_app.direct_apply_calls, 0);
}

#[test]
fn queue_apply_owner_shell_app_view_after_preflight_owned_metrics_log_messages_match_explicit_snapshot_wrapper()
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
    let preflight = queueing_preflight_result(SeqProxy::sequence(5));
    let hold_preflight = QueueHoldPreflight::new(
        false,
        false,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        Some(250),
    );

    let mut free_app = direct_app();
    let mut free_shell = build_direct_owner_shell();
    let free_snapshot = free_shell.owner().metrics().snapshot();
    let free_stage =
        run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot_and_log_messages(
            &mut free_shell,
            &mut free_app,
            &view_source,
            &tx_source,
            free_snapshot,
            &preflight,
            hold_preflight,
            Ter::TES_SUCCESS,
        );

    let mut plain_app = direct_app();
    let mut plain_shell = build_direct_owner_shell();
    let plain_stage = run_queue_apply_after_preflight_with_app_view_and_log_messages(
        &mut plain_shell,
        &mut plain_app,
        &view_source,
        &tx_source,
        &preflight,
        hold_preflight,
        Ter::TES_SUCCESS,
    );

    let mut method_app = direct_app();
    let mut method_shell = build_direct_owner_shell();
    let method_stage = method_shell
        .apply_after_preflight_with_app_view_and_owned_metrics_and_log_messages(
            &mut method_app,
            &view_source,
            &tx_source,
            &preflight,
            hold_preflight,
            Ter::TES_SUCCESS,
        );

    assert_eq!(method_stage, free_stage);
    assert_eq!(method_shell.owner(), free_shell.owner());
    assert_ne!(plain_stage, method_stage);
    assert_eq!(
        method_stage.queue_log_messages,
        free_stage.queue_log_messages
    );
    assert_eq!(plain_app.direct_apply_calls, 1);
    assert_eq!(method_app.direct_apply_calls, 0);
}

#[test]
fn queue_apply_owner_shell_app_view_after_preflight_caller_preclaim_matches_free_wrapper() {
    let account = String::from("acct");
    let tx_source = flow_tx_source(&account);
    let view_source = flow_view_source();
    let preflight = flow_preflight_result();
    let expected_prepared = RefCell::new(None::<QueueApplyPreparedPreclaimInputs<String>>);

    let mut free_app = FlowApplyApp::default();
    let mut free_shell = QueueApplyOwnerShell::new(build_flow_owner(Some(10)));
    let free_stage = run_queue_apply_after_preflight_with_app_view_and_caller_preclaim(
        &mut free_shell,
        &mut free_app,
        &view_source,
        &tx_source,
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

    let mut method_app = FlowApplyApp::default();
    let mut method_shell = QueueApplyOwnerShell::new(build_flow_owner(Some(10)));
    let method_stage = method_shell.apply_after_preflight_with_app_view_and_caller_preclaim(
        &mut method_app,
        &view_source,
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

    assert_eq!(method_stage, free_stage);
    assert_eq!(method_shell.owner(), free_shell.owner());
    assert_eq!(method_app.preflight_calls, 0);
    assert_eq!(method_app.preclaim_calls, 0);
    assert_eq!(method_app.direct_apply_calls, 0);
    assert_eq!(method_app.prepare_multitxn_calls, 1);
}

#[test]
fn queue_apply_owner_shell_app_view_after_preflight_caller_preclaim_log_messages_match_free_wrapper_on_full_queue_path()
 {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(6),
    };
    let view_source = queueing_view(&account);
    let preflight = queueing_preflight_result(SeqProxy::sequence(6));
    let expected_prepared = RefCell::new(None::<QueueApplyPreparedPreclaimInputs<String>>);

    let mut free_app = queueing_app();
    let mut free_shell = build_full_queue_owner_shell();
    let free_stage =
        run_queue_apply_after_preflight_with_app_view_and_caller_preclaim_and_log_messages(
            &mut free_shell,
            &mut free_app,
            &view_source,
            &tx_source,
            &preflight,
            hold_preflight(),
            Ter::TES_SUCCESS,
            |prepared| {
                expected_prepared.replace(Some(prepared.clone()));
                Ok(QueueApplyPreclaimStage {
                    view_source: prepared.view_source,
                    trace_message: "trace".to_string(),
                    preclaim_result: queueing_preflight_result(SeqProxy::sequence(6))
                        .to_preclaim(100, Ter::TES_SUCCESS),
                })
            },
        );

    let mut method_app = queueing_app();
    let mut method_shell = build_full_queue_owner_shell();
    let method_stage = method_shell
        .apply_after_preflight_with_app_view_and_caller_preclaim_and_log_messages(
            &mut method_app,
            &view_source,
            &tx_source,
            &preflight,
            hold_preflight(),
            Ter::TES_SUCCESS,
            |prepared| {
                assert_eq!(Some(prepared.clone()), *expected_prepared.borrow());
                Ok(QueueApplyPreclaimStage {
                    view_source: prepared.view_source,
                    trace_message: "trace".to_string(),
                    preclaim_result: queueing_preflight_result(SeqProxy::sequence(6))
                        .to_preclaim(100, Ter::TES_SUCCESS),
                })
            },
        );

    assert_eq!(method_stage, free_stage);
    assert_eq!(method_shell.owner(), free_shell.owner());
    assert_eq!(
        method_stage.queue_log_messages,
        free_stage.queue_log_messages
    );
    assert_eq!(method_app.preflight_calls, 0);
    assert_eq!(method_app.preclaim_calls, 0);
    assert_eq!(method_app.direct_apply_calls, 0);
    assert_eq!(method_app.prepare_multitxn_calls, 1);
}

#[test]
fn queue_apply_owner_shell_app_view_caller_preclaim_log_sinks_match_free_wrapper_on_full_queue_path()
 {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(6),
    };
    let view_source = queueing_view(&account);
    let expected_prepared = RefCell::new(None::<QueueApplyPreparedPreclaimInputs<String>>);

    let mut free_app = queueing_app();
    let mut free_shell = build_full_queue_owner_shell();
    let free_emitted = RefCell::new(Vec::new());
    let free_stage = run_queue_apply_with_app_view_and_caller_preclaim_and_log_sinks(
        &mut free_shell,
        &mut free_app,
        &view_source,
        &tx_source,
        hold_preflight(),
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(6)),
        Ter::TES_SUCCESS,
        |prepared| {
            expected_prepared.replace(Some(prepared.clone()));
            Ok(QueueApplyPreclaimStage {
                view_source: prepared.view_source,
                trace_message: "trace".to_string(),
                preclaim_result: queueing_preflight_result(SeqProxy::sequence(6))
                    .to_preclaim(100, Ter::TES_SUCCESS),
            })
        },
        |message| free_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| free_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| free_emitted.borrow_mut().push(format!("info:{message}")),
    );

    let mut method_app = queueing_app();
    let mut method_shell = build_full_queue_owner_shell();
    let method_emitted = RefCell::new(Vec::new());
    let method_stage = method_shell.apply_with_app_view_and_caller_preclaim_and_log_sinks(
        &mut method_app,
        &view_source,
        &tx_source,
        hold_preflight(),
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(6)),
        Ter::TES_SUCCESS,
        |prepared| {
            assert_eq!(Some(prepared.clone()), *expected_prepared.borrow());
            Ok(QueueApplyPreclaimStage {
                view_source: prepared.view_source,
                trace_message: "trace".to_string(),
                preclaim_result: queueing_preflight_result(SeqProxy::sequence(6))
                    .to_preclaim(100, Ter::TES_SUCCESS),
            })
        },
        |message| method_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| method_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| method_emitted.borrow_mut().push(format!("info:{message}")),
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

    let free_emitted = free_emitted.into_inner();
    let method_emitted = method_emitted.into_inner();

    assert_eq!(method_stage, free_stage);
    assert_eq!(method_shell.owner(), free_shell.owner());
    assert_eq!(method_emitted, free_emitted);
    assert_eq!(method_emitted, expected_emitted);
    assert_eq!(method_app.preflight_calls, 1);
    assert_eq!(method_app.preclaim_calls, 0);
    assert_eq!(method_app.direct_apply_calls, 0);
    assert_eq!(method_app.prepare_multitxn_calls, 1);
    assert!(free_app.trace_messages.is_empty());
    assert!(method_app.trace_messages.is_empty());
}

#[test]
fn queue_apply_owner_shell_app_view_owned_metrics_log_sinks_match_explicit_snapshot_wrapper() {
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
    let hold_preflight = QueueHoldPreflight::new(
        false,
        false,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        Some(250),
    );

    let mut free_app = direct_app();
    let mut free_shell = build_direct_owner_shell();
    let free_snapshot = free_shell.owner().metrics().snapshot();
    let free_emitted = RefCell::new(Vec::new());
    let free_stage = run_queue_apply_with_app_view_and_metrics_snapshot_and_log_sinks(
        &mut free_shell,
        &mut free_app,
        &view_source,
        &tx_source,
        free_snapshot,
        hold_preflight,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(5)),
        Ter::TES_SUCCESS,
        |message| free_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| free_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| free_emitted.borrow_mut().push(format!("info:{message}")),
    );

    let mut plain_app = direct_app();
    let mut plain_shell = build_direct_owner_shell();
    let plain_emitted = RefCell::new(Vec::new());
    let plain_stage = run_queue_apply_with_app_view_and_log_sinks(
        &mut plain_shell,
        &mut plain_app,
        &view_source,
        &tx_source,
        hold_preflight,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(5)),
        Ter::TES_SUCCESS,
        |message| plain_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| plain_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| plain_emitted.borrow_mut().push(format!("info:{message}")),
    );

    let mut method_app = direct_app();
    let mut method_shell = build_direct_owner_shell();
    let method_emitted = RefCell::new(Vec::new());
    let method_stage = method_shell.apply_with_app_view_and_owned_metrics_and_log_sinks(
        &mut method_app,
        &view_source,
        &tx_source,
        hold_preflight,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(5)),
        Ter::TES_SUCCESS,
        |message| method_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| method_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| method_emitted.borrow_mut().push(format!("info:{message}")),
    );

    assert_eq!(method_stage, free_stage);
    assert_eq!(method_shell.owner(), free_shell.owner());
    assert_eq!(method_emitted.into_inner(), free_emitted.into_inner());
    assert_ne!(plain_stage, method_stage);
    assert_eq!(plain_app.direct_apply_calls, 1);
    assert_eq!(method_app.direct_apply_calls, 0);
    let _ = plain_emitted.into_inner();
}

#[test]
fn queue_apply_owner_shell_app_view_caller_preclaim_owned_metrics_log_sinks_match_explicit_snapshot_wrapper()
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
    let hold_preflight = QueueHoldPreflight::new(
        false,
        false,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        Some(250),
    );
    let expected_prepared = RefCell::new(None::<QueueApplyPreparedPreclaimInputs<String>>);

    let mut free_app = direct_app();
    let mut free_shell = build_direct_owner_shell();
    let free_snapshot = free_shell.owner().metrics().snapshot();
    let free_emitted = RefCell::new(Vec::new());
    let free_stage =
        run_queue_apply_with_app_view_and_metrics_snapshot_and_caller_preclaim_and_log_sinks(
            &mut free_shell,
            &mut free_app,
            &view_source,
            &tx_source,
            free_snapshot,
            hold_preflight,
            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
            TxConsequences::new(1, SeqProxy::sequence(5)),
            Ter::TES_SUCCESS,
            |prepared| {
                expected_prepared.replace(Some(prepared.clone()));
                Ok(QueueApplyPreclaimStage {
                    view_source: prepared.view_source,
                    trace_message: "trace".to_string(),
                    preclaim_result: queueing_preflight_result(SeqProxy::sequence(5))
                        .to_preclaim(9, Ter::TES_SUCCESS),
                })
            },
            |message| free_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| free_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| free_emitted.borrow_mut().push(format!("info:{message}")),
        );

    let plain_prepared = RefCell::new(0usize);
    let mut plain_app = direct_app();
    let mut plain_shell = build_direct_owner_shell();
    let plain_emitted = RefCell::new(Vec::new());
    let plain_stage = run_queue_apply_with_app_view_and_caller_preclaim_and_log_sinks(
        &mut plain_shell,
        &mut plain_app,
        &view_source,
        &tx_source,
        hold_preflight,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(5)),
        Ter::TES_SUCCESS,
        |prepared| {
            *plain_prepared.borrow_mut() += 1;
            Ok(QueueApplyPreclaimStage {
                view_source: prepared.view_source,
                trace_message: "trace".to_string(),
                preclaim_result: queueing_preflight_result(SeqProxy::sequence(5))
                    .to_preclaim(9, Ter::TES_SUCCESS),
            })
        },
        |message| plain_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| plain_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| plain_emitted.borrow_mut().push(format!("info:{message}")),
    );

    let mut method_app = direct_app();
    let mut method_shell = build_direct_owner_shell();
    let method_emitted = RefCell::new(Vec::new());
    let method_stage = method_shell
        .apply_with_app_view_and_owned_metrics_and_caller_preclaim_and_log_sinks(
            &mut method_app,
            &view_source,
            &tx_source,
            hold_preflight,
            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
            TxConsequences::new(1, SeqProxy::sequence(5)),
            Ter::TES_SUCCESS,
            |prepared| {
                assert_eq!(Some(prepared.clone()), *expected_prepared.borrow());
                Ok(QueueApplyPreclaimStage {
                    view_source: prepared.view_source,
                    trace_message: "trace".to_string(),
                    preclaim_result: queueing_preflight_result(SeqProxy::sequence(5))
                        .to_preclaim(9, Ter::TES_SUCCESS),
                })
            },
            |message| method_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| method_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| method_emitted.borrow_mut().push(format!("info:{message}")),
        );

    assert_eq!(method_stage, free_stage);
    assert_eq!(method_shell.owner(), free_shell.owner());
    assert_eq!(method_emitted.into_inner(), free_emitted.into_inner());
    assert_ne!(plain_stage, method_stage);
    assert_eq!(*plain_prepared.borrow(), 0);
    assert_eq!(plain_app.direct_apply_calls, 1);
    assert_eq!(method_app.direct_apply_calls, 0);
    let _ = plain_emitted.into_inner();
}

#[test]
fn queue_apply_owner_shell_app_view_after_preflight_caller_preclaim_log_sinks_match_free_wrapper_on_full_queue_path()
 {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(6),
    };
    let view_source = queueing_view(&account);
    let preflight = queueing_preflight_result(SeqProxy::sequence(6));
    let expected_prepared = RefCell::new(None::<QueueApplyPreparedPreclaimInputs<String>>);

    let mut free_app = queueing_app();
    let mut free_shell = build_full_queue_owner_shell();
    let free_emitted = RefCell::new(Vec::new());
    let free_stage =
        run_queue_apply_after_preflight_with_app_view_and_caller_preclaim_and_log_sinks(
            &mut free_shell,
            &mut free_app,
            &view_source,
            &tx_source,
            &preflight,
            hold_preflight(),
            Ter::TES_SUCCESS,
            |prepared| {
                expected_prepared.replace(Some(prepared.clone()));
                Ok(QueueApplyPreclaimStage {
                    view_source: prepared.view_source,
                    trace_message: "trace".to_string(),
                    preclaim_result: queueing_preflight_result(SeqProxy::sequence(6))
                        .to_preclaim(100, Ter::TES_SUCCESS),
                })
            },
            |message| free_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| free_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| free_emitted.borrow_mut().push(format!("info:{message}")),
        );

    let mut method_app = queueing_app();
    let mut method_shell = build_full_queue_owner_shell();
    let method_emitted = RefCell::new(Vec::new());
    let method_stage = method_shell
        .apply_after_preflight_with_app_view_and_caller_preclaim_and_log_sinks(
            &mut method_app,
            &view_source,
            &tx_source,
            &preflight,
            hold_preflight(),
            Ter::TES_SUCCESS,
            |prepared| {
                assert_eq!(Some(prepared.clone()), *expected_prepared.borrow());
                Ok(QueueApplyPreclaimStage {
                    view_source: prepared.view_source,
                    trace_message: "trace".to_string(),
                    preclaim_result: queueing_preflight_result(SeqProxy::sequence(6))
                        .to_preclaim(100, Ter::TES_SUCCESS),
                })
            },
            |message| method_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| method_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| method_emitted.borrow_mut().push(format!("info:{message}")),
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

    let free_emitted = free_emitted.into_inner();
    let method_emitted = method_emitted.into_inner();

    assert_eq!(method_stage, free_stage);
    assert_eq!(method_shell.owner(), free_shell.owner());
    assert_eq!(method_emitted, free_emitted);
    assert_eq!(method_emitted, expected_emitted);
    assert_eq!(method_app.preflight_calls, 0);
    assert_eq!(method_app.preclaim_calls, 0);
    assert_eq!(method_app.direct_apply_calls, 0);
    assert_eq!(method_app.prepare_multitxn_calls, 1);
    assert!(free_app.trace_messages.is_empty());
    assert!(method_app.trace_messages.is_empty());
}

#[test]
fn queue_apply_owner_shell_app_view_after_preflight_owned_metrics_log_sinks_match_explicit_snapshot_wrapper()
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
    let preflight = queueing_preflight_result(SeqProxy::sequence(5));
    let hold_preflight = QueueHoldPreflight::new(
        false,
        false,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        Some(250),
    );

    let mut free_app = direct_app();
    let mut free_shell = build_direct_owner_shell();
    let free_snapshot = free_shell.owner().metrics().snapshot();
    let free_emitted = RefCell::new(Vec::new());
    let free_stage =
        run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot_and_log_sinks(
            &mut free_shell,
            &mut free_app,
            &view_source,
            &tx_source,
            free_snapshot,
            &preflight,
            hold_preflight,
            Ter::TES_SUCCESS,
            |message| free_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| free_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| free_emitted.borrow_mut().push(format!("info:{message}")),
        );

    let mut plain_app = direct_app();
    let mut plain_shell = build_direct_owner_shell();
    let plain_emitted = RefCell::new(Vec::new());
    let plain_stage = run_queue_apply_after_preflight_with_app_view_and_log_sinks(
        &mut plain_shell,
        &mut plain_app,
        &view_source,
        &tx_source,
        &preflight,
        hold_preflight,
        Ter::TES_SUCCESS,
        |message| plain_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| plain_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| plain_emitted.borrow_mut().push(format!("info:{message}")),
    );

    let mut method_app = direct_app();
    let mut method_shell = build_direct_owner_shell();
    let method_emitted = RefCell::new(Vec::new());
    let method_stage = method_shell
        .apply_after_preflight_with_app_view_and_owned_metrics_and_log_sinks(
            &mut method_app,
            &view_source,
            &tx_source,
            &preflight,
            hold_preflight,
            Ter::TES_SUCCESS,
            |message| method_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| method_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| method_emitted.borrow_mut().push(format!("info:{message}")),
        );

    assert_eq!(method_stage, free_stage);
    assert_eq!(method_shell.owner(), free_shell.owner());
    assert_eq!(method_emitted.into_inner(), free_emitted.into_inner());
    assert_ne!(plain_stage, method_stage);
    assert_eq!(plain_app.direct_apply_calls, 1);
    assert_eq!(method_app.direct_apply_calls, 0);
    let _ = plain_emitted.into_inner();
}

#[test]
fn queue_apply_owner_shell_app_view_after_preflight_caller_preclaim_owned_metrics_log_sinks_match_explicit_snapshot_wrapper()
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
    let preflight = queueing_preflight_result(SeqProxy::sequence(5));
    let hold_preflight = QueueHoldPreflight::new(
        false,
        false,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        Some(250),
    );
    let expected_prepared = RefCell::new(None::<QueueApplyPreparedPreclaimInputs<String>>);

    let mut free_app = direct_app();
    let mut free_shell = build_direct_owner_shell();
    let free_snapshot = free_shell.owner().metrics().snapshot();
    let free_emitted = RefCell::new(Vec::new());
    let free_stage =
        run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot_and_caller_preclaim_and_log_sinks(
            &mut free_shell,
            &mut free_app,
            &view_source,
            &tx_source,
            free_snapshot,
            &preflight,
            hold_preflight,
            Ter::TES_SUCCESS,
            |prepared| {
                expected_prepared.replace(Some(prepared.clone()));
                Ok(QueueApplyPreclaimStage {
                    view_source: prepared.view_source,
                    trace_message: "trace".to_string(),
                    preclaim_result: queueing_preflight_result(SeqProxy::sequence(5))
                        .to_preclaim(9, Ter::TES_SUCCESS),
                })
            },
            |message| free_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| free_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| free_emitted.borrow_mut().push(format!("info:{message}")),
        );

    let plain_prepared = RefCell::new(0usize);
    let mut plain_app = direct_app();
    let mut plain_shell = build_direct_owner_shell();
    let plain_emitted = RefCell::new(Vec::new());
    let plain_stage =
        run_queue_apply_after_preflight_with_app_view_and_caller_preclaim_and_log_sinks(
            &mut plain_shell,
            &mut plain_app,
            &view_source,
            &tx_source,
            &preflight,
            hold_preflight,
            Ter::TES_SUCCESS,
            |prepared| {
                *plain_prepared.borrow_mut() += 1;
                Ok(QueueApplyPreclaimStage {
                    view_source: prepared.view_source,
                    trace_message: "trace".to_string(),
                    preclaim_result: queueing_preflight_result(SeqProxy::sequence(5))
                        .to_preclaim(9, Ter::TES_SUCCESS),
                })
            },
            |message| plain_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| plain_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| plain_emitted.borrow_mut().push(format!("info:{message}")),
        );

    let mut method_app = direct_app();
    let mut method_shell = build_direct_owner_shell();
    let method_emitted = RefCell::new(Vec::new());
    let method_stage = method_shell
        .apply_after_preflight_with_app_view_and_owned_metrics_and_caller_preclaim_and_log_sinks(
            &mut method_app,
            &view_source,
            &tx_source,
            &preflight,
            hold_preflight,
            Ter::TES_SUCCESS,
            |prepared| {
                assert_eq!(Some(prepared.clone()), *expected_prepared.borrow());
                Ok(QueueApplyPreclaimStage {
                    view_source: prepared.view_source,
                    trace_message: "trace".to_string(),
                    preclaim_result: queueing_preflight_result(SeqProxy::sequence(5))
                        .to_preclaim(9, Ter::TES_SUCCESS),
                })
            },
            |message| method_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| method_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| method_emitted.borrow_mut().push(format!("info:{message}")),
        );

    assert_eq!(method_stage, free_stage);
    assert_eq!(method_shell.owner(), free_shell.owner());
    assert_eq!(method_emitted.into_inner(), free_emitted.into_inner());
    assert_ne!(plain_stage, method_stage);
    assert_eq!(*plain_prepared.borrow(), 0);
    assert_eq!(plain_app.direct_apply_calls, 1);
    assert_eq!(method_app.direct_apply_calls, 0);
    let _ = plain_emitted.into_inner();
}

#[test]
fn queue_apply_owner_shell_app_view_log_sinks_match_free_wrapper_on_full_queue_path() {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(6),
    };
    let view_source = queueing_view(&account);

    let mut free_app = queueing_app();
    let mut free_shell = build_full_queue_owner_shell();
    let free_emitted = RefCell::new(Vec::new());
    let free_stage = run_queue_apply_with_app_view_and_log_sinks(
        &mut free_shell,
        &mut free_app,
        &view_source,
        &tx_source,
        hold_preflight(),
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(6)),
        Ter::TES_SUCCESS,
        |message| free_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| free_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| free_emitted.borrow_mut().push(format!("info:{message}")),
    );

    let mut method_app = queueing_app();
    let mut method_shell = build_full_queue_owner_shell();
    let method_emitted = RefCell::new(Vec::new());
    let method_stage = method_shell.apply_with_app_view_and_log_sinks(
        &mut method_app,
        &view_source,
        &tx_source,
        hold_preflight(),
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(6)),
        Ter::TES_SUCCESS,
        |message| method_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| method_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| method_emitted.borrow_mut().push(format!("info:{message}")),
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

    let free_emitted = free_emitted.into_inner();
    let method_emitted = method_emitted.into_inner();

    assert_eq!(method_stage, free_stage);
    assert_eq!(method_shell.owner(), free_shell.owner());
    assert_eq!(method_emitted, free_emitted);
    assert_eq!(method_emitted, expected_emitted);
    assert!(free_app.trace_messages.is_empty());
    assert!(method_app.trace_messages.is_empty());
    assert_eq!(method_app.preflight_calls, 1);
    assert_eq!(method_app.direct_apply_calls, 0);
    assert_eq!(method_app.prepare_multitxn_calls, 1);
    assert_eq!(method_app.preclaim_calls, 1);
    assert_eq!(method_app.try_clear_calls, 0);
    assert_eq!(method_app.apply_sandbox_calls, 0);
}

#[test]
fn queue_apply_owner_shell_app_view_log_sinks_derive_hold_admission_like_free_wrapper() {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let view_source = queueing_view(&account);
    let hold_preflight = QueueHoldPreflight::new(
        false,
        false,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        Some(250),
    );

    let mut free_app = direct_app();
    let mut free_shell = build_direct_owner_shell();
    let free_emitted = RefCell::new(Vec::new());
    let free_stage = run_queue_apply_with_app_view_and_log_sinks_and_derived_hold_admission(
        &mut free_shell,
        &mut free_app,
        &view_source,
        &tx_source,
        hold_preflight,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(5)),
        |message| free_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| free_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| free_emitted.borrow_mut().push(format!("info:{message}")),
    );

    let mut method_app = direct_app();
    let mut method_shell = build_direct_owner_shell();
    let method_emitted = RefCell::new(Vec::new());
    let method_stage = method_shell.apply_with_app_view_and_log_sinks_and_derived_hold_admission(
        &mut method_app,
        &view_source,
        &tx_source,
        hold_preflight,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(5)),
        |message| method_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| method_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| method_emitted.borrow_mut().push(format!("info:{message}")),
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

    let free_emitted = free_emitted.into_inner();
    let method_emitted = method_emitted.into_inner();

    assert_eq!(method_stage, free_stage);
    assert_eq!(method_shell.owner(), free_shell.owner());
    assert_eq!(method_emitted, free_emitted);
    assert_eq!(method_emitted, expected_emitted);
    assert_eq!(method_app.preflight_calls, 1);
    assert_eq!(method_app.direct_apply_calls, 1);
    assert!(free_app.trace_messages.is_empty());
    assert!(method_app.trace_messages.is_empty());
}

#[test]
fn queue_apply_owner_shell_app_view_log_sinks_derive_hold_preflight_like_free_wrapper() {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(6),
    };
    let view_source = queueing_view(&account);

    let mut free_app = queueing_app();
    let mut free_shell = build_full_queue_owner_shell();
    let free_emitted = RefCell::new(Vec::new());
    let free_stage = run_queue_apply_with_app_view_and_log_sinks_and_derived_hold_preflight(
        &mut free_shell,
        &mut free_app,
        &view_source,
        &tx_source,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(6)),
        Ter::TES_SUCCESS,
        |message| free_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| free_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| free_emitted.borrow_mut().push(format!("info:{message}")),
    );

    let mut method_app = queueing_app();
    let mut method_shell = build_full_queue_owner_shell();
    let method_emitted = RefCell::new(Vec::new());
    let method_stage = method_shell.apply_with_app_view_and_log_sinks_and_derived_hold_preflight(
        &mut method_app,
        &view_source,
        &tx_source,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(6)),
        Ter::TES_SUCCESS,
        |message| method_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| method_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| method_emitted.borrow_mut().push(format!("info:{message}")),
    );

    let free_emitted = free_emitted.into_inner();
    let method_emitted = method_emitted.into_inner();

    assert_eq!(method_stage, free_stage);
    assert_eq!(method_shell.owner(), free_shell.owner());
    assert_eq!(method_emitted, free_emitted);
    assert_eq!(method_app.preflight_calls, 1);
    assert_eq!(method_app.direct_apply_calls, 0);
    assert!(free_app.trace_messages.is_empty());
    assert!(method_app.trace_messages.is_empty());
}

#[test]
fn queue_apply_owner_shell_app_view_derived_hold_admission_log_messages_match_free_wrapper() {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let view_source = queueing_view(&account);
    let hold_preflight = QueueHoldPreflight::new(
        false,
        false,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        Some(250),
    );

    let mut free_app = direct_app();
    let mut free_shell = build_direct_owner_shell();
    let free_stage = run_queue_apply_with_app_view_and_derived_hold_admission_and_log_messages(
        &mut free_shell,
        &mut free_app,
        &view_source,
        &tx_source,
        hold_preflight,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(5)),
    );

    let mut method_app = direct_app();
    let mut method_shell = build_direct_owner_shell();
    let method_stage = method_shell
        .apply_with_app_view_and_derived_hold_admission_and_log_messages(
            &mut method_app,
            &view_source,
            &tx_source,
            hold_preflight,
            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
            TxConsequences::new(1, SeqProxy::sequence(5)),
        );

    assert_eq!(method_stage, free_stage);
    assert_eq!(method_shell.owner(), free_shell.owner());
    assert!(method_stage.queue_log_messages.trace.is_empty());
    assert!(method_stage.queue_log_messages.debug.is_empty());
    assert!(method_stage.queue_log_messages.info.is_empty());
    assert_eq!(method_app.preflight_calls, 1);
    assert_eq!(method_app.direct_apply_calls, 1);
    assert!(free_app.trace_messages.is_empty());
    assert!(method_app.trace_messages.is_empty());
}

#[test]
fn queue_apply_owner_shell_app_view_derived_hold_preflight_log_messages_match_free_wrapper() {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(6),
    };
    let view_source = queueing_view(&account);

    let mut free_app = queueing_app();
    let mut free_shell = build_full_queue_owner_shell();
    let free_stage = run_queue_apply_with_app_view_and_derived_hold_preflight_and_log_messages(
        &mut free_shell,
        &mut free_app,
        &view_source,
        &tx_source,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(6)),
        Ter::TES_SUCCESS,
    );

    let mut method_app = queueing_app();
    let mut method_shell = build_full_queue_owner_shell();
    let method_stage = method_shell
        .apply_with_app_view_and_derived_hold_preflight_and_log_messages(
            &mut method_app,
            &view_source,
            &tx_source,
            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
            TxConsequences::new(1, SeqProxy::sequence(6)),
            Ter::TES_SUCCESS,
        );

    assert_eq!(method_stage, free_stage);
    assert_eq!(method_shell.owner(), free_shell.owner());
    assert_eq!(
        method_stage.queue_log_messages,
        free_stage.queue_log_messages
    );
    assert_eq!(method_app.preflight_calls, 1);
    assert_eq!(method_app.direct_apply_calls, 0);
    assert!(free_app.trace_messages.is_empty());
    assert!(method_app.trace_messages.is_empty());
}

#[test]
fn queue_apply_owner_shell_app_view_log_sinks_can_derive_preflight_facts_from_runtime() {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(6),
    };
    let view_source = queueing_view(&account);

    let mut free_app = queueing_app();
    let mut free_shell = build_full_queue_owner_shell();
    let free_emitted = RefCell::new(Vec::new());
    let free_stage = run_queue_apply_with_app_view_and_log_sinks_and_derived_preflight_facts(
        &mut free_shell,
        &mut free_app,
        &view_source,
        &tx_source,
        Ter::TES_SUCCESS,
        |message| free_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| free_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| free_emitted.borrow_mut().push(format!("info:{message}")),
    );

    let mut method_app = queueing_app();
    let mut method_shell = build_full_queue_owner_shell();
    let method_emitted = RefCell::new(Vec::new());
    let method_stage = method_shell.apply_with_app_view_and_log_sinks_and_derived_preflight_facts(
        &mut method_app,
        &view_source,
        &tx_source,
        Ter::TES_SUCCESS,
        |message| method_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| method_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| method_emitted.borrow_mut().push(format!("info:{message}")),
    );

    let free_emitted = free_emitted.into_inner();
    let method_emitted = method_emitted.into_inner();

    assert_eq!(method_stage, free_stage);
    assert_eq!(method_shell.owner(), free_shell.owner());
    assert_eq!(method_emitted, free_emitted);
    assert_eq!(method_app.preflight_calls, 1);
    assert_eq!(method_app.direct_apply_calls, 0);
    assert!(free_app.trace_messages.is_empty());
    assert!(method_app.trace_messages.is_empty());
}

#[test]
fn queue_apply_owner_shell_app_view_derived_preflight_facts_log_messages_match_free_wrapper() {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(6),
    };
    let view_source = queueing_view(&account);

    let mut free_app = queueing_app();
    let mut free_shell = build_full_queue_owner_shell();
    let free_stage = run_queue_apply_with_app_view_and_derived_preflight_facts_and_log_messages(
        &mut free_shell,
        &mut free_app,
        &view_source,
        &tx_source,
        Ter::TES_SUCCESS,
    );

    let mut method_app = queueing_app();
    let mut method_shell = build_full_queue_owner_shell();
    let method_stage = method_shell
        .apply_with_app_view_and_derived_preflight_facts_and_log_messages(
            &mut method_app,
            &view_source,
            &tx_source,
            Ter::TES_SUCCESS,
        );

    assert_eq!(method_stage, free_stage);
    assert_eq!(method_shell.owner(), free_shell.owner());
    assert_eq!(
        method_stage.queue_log_messages,
        free_stage.queue_log_messages
    );
    assert_eq!(method_app.preflight_calls, 1);
    assert_eq!(method_app.direct_apply_calls, 0);
    assert!(free_app.trace_messages.is_empty());
    assert!(method_app.trace_messages.is_empty());
}

#[test]
fn queue_apply_owner_shell_app_view_log_sinks_derive_preflight_and_hold_admission_like_free_wrapper()
 {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(6),
    };
    let view_source = queueing_view(&account);

    let mut free_app = queueing_app();
    let mut free_shell = build_full_queue_owner_shell();
    let free_emitted = RefCell::new(Vec::new());
    let free_stage =
        run_queue_apply_with_app_view_and_log_sinks_and_derived_preflight_facts_and_hold_admission(
            &mut free_shell,
            &mut free_app,
            &view_source,
            &tx_source,
            |message| free_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| free_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| free_emitted.borrow_mut().push(format!("info:{message}")),
        );

    let mut method_app = queueing_app();
    let mut method_shell = build_full_queue_owner_shell();
    let method_emitted = RefCell::new(Vec::new());
    let method_stage = method_shell
        .apply_with_app_view_and_log_sinks_and_derived_preflight_facts_and_hold_admission(
            &mut method_app,
            &view_source,
            &tx_source,
            |message| method_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| method_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| method_emitted.borrow_mut().push(format!("info:{message}")),
        );

    let free_emitted = free_emitted.into_inner();
    let method_emitted = method_emitted.into_inner();

    assert_eq!(method_stage, free_stage);
    assert_eq!(method_shell.owner(), free_shell.owner());
    assert_eq!(method_emitted, free_emitted);
    assert_eq!(method_app.preflight_calls, 1);
    assert_eq!(method_app.direct_apply_calls, 0);
    assert!(free_app.trace_messages.is_empty());
    assert!(method_app.trace_messages.is_empty());
}

#[test]
fn queue_apply_owner_shell_app_view_derived_preflight_and_hold_admission_log_messages_match_free_wrapper()
 {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(6),
    };
    let view_source = queueing_view(&account);

    let mut free_app = queueing_app();
    let mut free_shell = build_full_queue_owner_shell();
    let free_stage =
        run_queue_apply_with_app_view_and_derived_preflight_facts_and_hold_admission_and_log_messages(
            &mut free_shell,
            &mut free_app,
            &view_source,
            &tx_source,
        );

    let mut method_app = queueing_app();
    let mut method_shell = build_full_queue_owner_shell();
    let method_stage = method_shell
        .apply_with_app_view_and_derived_preflight_facts_and_hold_admission_and_log_messages(
            &mut method_app,
            &view_source,
            &tx_source,
        );

    assert_eq!(method_stage, free_stage);
    assert_eq!(method_shell.owner(), free_shell.owner());
    assert_eq!(
        method_stage.queue_log_messages,
        free_stage.queue_log_messages
    );
    assert_eq!(method_app.preflight_calls, 1);
    assert_eq!(method_app.direct_apply_calls, 0);
    assert!(free_app.trace_messages.is_empty());
    assert!(method_app.trace_messages.is_empty());
}

#[test]
fn queue_apply_owner_shell_app_view_after_preflight_log_sinks_derive_hold_preflight_like_free_wrapper()
 {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let view_source = direct_view(&account);
    let preflight = queueing_preflight_result(SeqProxy::sequence(5));

    let mut free_app = direct_app();
    let mut free_shell = build_direct_owner_shell();
    let free_emitted = RefCell::new(Vec::new());
    let free_stage =
        run_queue_apply_after_preflight_with_app_view_and_log_sinks_and_derived_hold_preflight(
            &mut free_shell,
            &mut free_app,
            &view_source,
            &tx_source,
            &preflight,
            Ter::TES_SUCCESS,
            |message| free_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| free_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| free_emitted.borrow_mut().push(format!("info:{message}")),
        );

    let mut method_app = direct_app();
    let mut method_shell = build_direct_owner_shell();
    let method_emitted = RefCell::new(Vec::new());
    let method_stage = method_shell
        .apply_after_preflight_with_app_view_and_log_sinks_and_derived_hold_preflight(
            &mut method_app,
            &view_source,
            &tx_source,
            &preflight,
            Ter::TES_SUCCESS,
            |message| method_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| method_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| method_emitted.borrow_mut().push(format!("info:{message}")),
        );

    let free_emitted = free_emitted.into_inner();
    let method_emitted = method_emitted.into_inner();

    assert_eq!(method_stage, free_stage);
    assert_eq!(method_shell.owner(), free_shell.owner());
    assert_eq!(method_emitted, free_emitted);
    assert_eq!(method_app.preflight_calls, 0);
    assert_eq!(method_app.direct_apply_calls, 1);
    assert!(free_app.trace_messages.is_empty());
    assert!(method_app.trace_messages.is_empty());
}

#[test]
fn queue_apply_owner_shell_app_view_after_preflight_log_sinks_match_free_wrapper_on_direct_path() {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let view_source = direct_view(&account);
    let preflight = queueing_preflight_result(SeqProxy::sequence(5));

    let mut free_app = direct_app();
    let mut free_shell = build_direct_owner_shell();
    let free_emitted = RefCell::new(Vec::new());
    let free_stage = run_queue_apply_after_preflight_with_app_view_and_log_sinks(
        &mut free_shell,
        &mut free_app,
        &view_source,
        &tx_source,
        &preflight,
        hold_preflight(),
        Ter::TES_SUCCESS,
        |message| free_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| free_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| free_emitted.borrow_mut().push(format!("info:{message}")),
    );

    let mut method_app = direct_app();
    let mut method_shell = build_direct_owner_shell();
    let method_emitted = RefCell::new(Vec::new());
    let method_stage = method_shell.apply_after_preflight_with_app_view_and_log_sinks(
        &mut method_app,
        &view_source,
        &tx_source,
        &preflight,
        hold_preflight(),
        Ter::TES_SUCCESS,
        |message| method_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| method_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| method_emitted.borrow_mut().push(format!("info:{message}")),
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
    let free_emitted = free_emitted.into_inner();
    let method_emitted = method_emitted.into_inner();

    assert_eq!(method_stage, free_stage);
    assert_eq!(method_shell.owner(), free_shell.owner());
    assert_eq!(method_emitted, free_emitted);
    assert_eq!(method_emitted, expected_emitted);
    assert!(free_app.trace_messages.is_empty());
    assert!(method_app.trace_messages.is_empty());
    assert_eq!(method_app.preflight_calls, 0);
    assert_eq!(method_app.direct_apply_calls, 1);
    assert_eq!(method_app.prepare_multitxn_calls, 0);
    assert_eq!(method_app.preclaim_calls, 0);
    assert_eq!(method_app.try_clear_calls, 0);
    assert_eq!(method_app.apply_sandbox_calls, 0);
}

#[test]
fn queue_apply_owner_shell_after_preflight_log_sinks_derive_hold_admission_like_free_wrapper() {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let view_source = direct_view(&account);
    let preflight = queueing_preflight_result(SeqProxy::sequence(5));
    let hold_preflight = QueueHoldPreflight::new(
        false,
        false,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        Some(250),
    );

    let mut free_app = direct_app();
    let mut free_shell = build_direct_owner_shell();
    let free_emitted = RefCell::new(Vec::new());
    let free_stage =
        run_queue_apply_after_preflight_with_app_view_and_log_sinks_and_derived_hold_admission(
            &mut free_shell,
            &mut free_app,
            &view_source,
            &tx_source,
            &preflight,
            hold_preflight,
            |message| free_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| free_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| free_emitted.borrow_mut().push(format!("info:{message}")),
        );

    let mut method_app = direct_app();
    let mut method_shell = build_direct_owner_shell();
    let method_emitted = RefCell::new(Vec::new());
    let method_stage = method_shell
        .apply_after_preflight_with_app_view_and_log_sinks_and_derived_hold_admission(
            &mut method_app,
            &view_source,
            &tx_source,
            &preflight,
            hold_preflight,
            |message| method_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| method_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| method_emitted.borrow_mut().push(format!("info:{message}")),
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

    let free_emitted = free_emitted.into_inner();
    let method_emitted = method_emitted.into_inner();

    assert_eq!(method_stage, free_stage);
    assert_eq!(method_shell.owner(), free_shell.owner());
    assert_eq!(method_emitted, free_emitted);
    assert_eq!(method_emitted, expected_emitted);
    assert_eq!(method_app.preflight_calls, 0);
    assert_eq!(method_app.direct_apply_calls, 1);
    assert!(free_app.trace_messages.is_empty());
    assert!(method_app.trace_messages.is_empty());
}

#[test]
fn queue_apply_owner_shell_app_view_owned_metrics_log_sinks_derive_hold_admission_match_explicit_snapshot_wrapper()
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
    let hold_preflight = QueueHoldPreflight::new(
        false,
        false,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        Some(250),
    );

    let mut free_app = direct_app();
    let mut free_shell = build_direct_owner_shell();
    let free_snapshot = free_shell.owner().metrics().snapshot();
    let free_emitted = RefCell::new(Vec::new());
    let free_stage =
        run_queue_apply_with_app_view_and_metrics_snapshot_and_log_sinks_and_derived_hold_admission(
            &mut free_shell,
            &mut free_app,
            &view_source,
            &tx_source,
            free_snapshot,
            hold_preflight,
            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
            TxConsequences::new(1, SeqProxy::sequence(5)),
            |message| free_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| free_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| free_emitted.borrow_mut().push(format!("info:{message}")),
        );

    let mut plain_app = direct_app();
    let mut plain_shell = build_direct_owner_shell();
    let plain_emitted = RefCell::new(Vec::new());
    let plain_stage = run_queue_apply_with_app_view_and_log_sinks_and_derived_hold_admission(
        &mut plain_shell,
        &mut plain_app,
        &view_source,
        &tx_source,
        hold_preflight,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(5)),
        |message| plain_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| plain_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| plain_emitted.borrow_mut().push(format!("info:{message}")),
    );

    let mut method_app = direct_app();
    let mut method_shell = build_direct_owner_shell();
    let method_emitted = RefCell::new(Vec::new());
    let method_stage = method_shell
        .apply_with_app_view_and_owned_metrics_and_log_sinks_and_derived_hold_admission(
            &mut method_app,
            &view_source,
            &tx_source,
            hold_preflight,
            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
            TxConsequences::new(1, SeqProxy::sequence(5)),
            |message| method_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| method_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| method_emitted.borrow_mut().push(format!("info:{message}")),
        );

    assert_eq!(method_stage, free_stage);
    assert_eq!(method_shell.owner(), free_shell.owner());
    assert_eq!(method_emitted.into_inner(), free_emitted.into_inner());
    assert_ne!(plain_stage, method_stage);
    assert_eq!(plain_app.direct_apply_calls, 1);
    assert_eq!(method_app.direct_apply_calls, 0);
    let _ = plain_emitted.into_inner();
}

#[test]
fn queue_apply_owner_shell_app_view_owned_metrics_log_sinks_derive_hold_preflight_match_explicit_snapshot_wrapper()
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

    let mut free_app = direct_app();
    let mut free_shell = build_direct_owner_shell();
    let free_snapshot = free_shell.owner().metrics().snapshot();
    let free_emitted = RefCell::new(Vec::new());
    let free_stage =
        run_queue_apply_with_app_view_and_metrics_snapshot_and_log_sinks_and_derived_hold_preflight(
            &mut free_shell,
            &mut free_app,
            &view_source,
            &tx_source,
            free_snapshot,
            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
            TxConsequences::new(1, SeqProxy::sequence(5)),
            Ter::TES_SUCCESS,
            |message| free_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| free_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| free_emitted.borrow_mut().push(format!("info:{message}")),
        );

    let mut plain_app = direct_app();
    let mut plain_shell = build_direct_owner_shell();
    let plain_emitted = RefCell::new(Vec::new());
    let plain_stage = run_queue_apply_with_app_view_and_log_sinks_and_derived_hold_preflight(
        &mut plain_shell,
        &mut plain_app,
        &view_source,
        &tx_source,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(5)),
        Ter::TES_SUCCESS,
        |message| plain_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| plain_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| plain_emitted.borrow_mut().push(format!("info:{message}")),
    );

    let mut method_app = direct_app();
    let mut method_shell = build_direct_owner_shell();
    let method_emitted = RefCell::new(Vec::new());
    let method_stage = method_shell
        .apply_with_app_view_and_owned_metrics_and_log_sinks_and_derived_hold_preflight(
            &mut method_app,
            &view_source,
            &tx_source,
            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
            TxConsequences::new(1, SeqProxy::sequence(5)),
            Ter::TES_SUCCESS,
            |message| method_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| method_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| method_emitted.borrow_mut().push(format!("info:{message}")),
        );

    assert_eq!(method_stage, free_stage);
    assert_eq!(method_shell.owner(), free_shell.owner());
    assert_eq!(method_emitted.into_inner(), free_emitted.into_inner());
    assert_ne!(plain_stage, method_stage);
    assert_eq!(plain_app.direct_apply_calls, 1);
    assert_eq!(method_app.direct_apply_calls, 0);
    let _ = plain_emitted.into_inner();
}

#[test]
fn queue_apply_owner_shell_app_view_owned_metrics_log_sinks_derive_preflight_facts_match_explicit_snapshot_wrapper()
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

    let mut free_app = direct_app();
    let mut free_shell = build_direct_owner_shell();
    let free_snapshot = free_shell.owner().metrics().snapshot();
    let free_emitted = RefCell::new(Vec::new());
    let free_stage =
        run_queue_apply_with_app_view_and_metrics_snapshot_and_log_sinks_and_derived_preflight_facts(
            &mut free_shell,
            &mut free_app,
            &view_source,
            &tx_source,
            free_snapshot,
            |message| free_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| free_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| free_emitted.borrow_mut().push(format!("info:{message}")),
        );

    let mut plain_app = direct_app();
    let mut plain_shell = build_direct_owner_shell();
    let plain_emitted = RefCell::new(Vec::new());
    let plain_stage = run_queue_apply_with_app_view_and_log_sinks_and_derived_preflight_facts(
        &mut plain_shell,
        &mut plain_app,
        &view_source,
        &tx_source,
        Ter::TES_SUCCESS,
        |message| plain_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| plain_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| plain_emitted.borrow_mut().push(format!("info:{message}")),
    );

    let mut method_app = direct_app();
    let mut method_shell = build_direct_owner_shell();
    let method_emitted = RefCell::new(Vec::new());
    let method_stage = method_shell
        .apply_with_app_view_and_owned_metrics_and_log_sinks_and_derived_preflight_facts(
            &mut method_app,
            &view_source,
            &tx_source,
            |message| method_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| method_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| method_emitted.borrow_mut().push(format!("info:{message}")),
        );

    assert_eq!(method_stage, free_stage);
    assert_eq!(method_shell.owner(), free_shell.owner());
    assert_eq!(method_emitted.into_inner(), free_emitted.into_inner());
    assert_ne!(plain_stage, method_stage);
    assert_eq!(plain_app.direct_apply_calls, 1);
    assert_eq!(method_app.direct_apply_calls, 0);
    let _ = plain_emitted.into_inner();
}

#[test]
fn queue_apply_owner_shell_app_view_owned_metrics_log_sinks_derive_preflight_and_hold_admission_match_explicit_snapshot_wrapper()
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

    let mut free_app = direct_app();
    let mut free_shell = build_direct_owner_shell();
    let free_snapshot = free_shell.owner().metrics().snapshot();
    let free_emitted = RefCell::new(Vec::new());
    let free_stage = run_queue_apply_with_app_view_and_metrics_snapshot_and_log_sinks_and_derived_preflight_facts_and_hold_admission(
        &mut free_shell,
        &mut free_app,
        &view_source,
        &tx_source,
        free_snapshot,
        |message| free_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| free_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| free_emitted.borrow_mut().push(format!("info:{message}")),
    );

    let mut plain_app = direct_app();
    let mut plain_shell = build_direct_owner_shell();
    let plain_emitted = RefCell::new(Vec::new());
    let plain_stage =
        run_queue_apply_with_app_view_and_log_sinks_and_derived_preflight_facts_and_hold_admission(
            &mut plain_shell,
            &mut plain_app,
            &view_source,
            &tx_source,
            |message| plain_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| plain_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| plain_emitted.borrow_mut().push(format!("info:{message}")),
        );

    let mut method_app = direct_app();
    let mut method_shell = build_direct_owner_shell();
    let method_emitted = RefCell::new(Vec::new());
    let method_stage = method_shell
        .apply_with_app_view_and_owned_metrics_and_log_sinks_and_derived_preflight_facts_and_hold_admission(
            &mut method_app,
            &view_source,
            &tx_source,
            |message| method_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| method_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| method_emitted.borrow_mut().push(format!("info:{message}")),
        );

    assert_eq!(method_stage, free_stage);
    assert_eq!(method_shell.owner(), free_shell.owner());
    assert_eq!(method_emitted.into_inner(), free_emitted.into_inner());
    assert_ne!(plain_stage, method_stage);
    assert_eq!(plain_app.direct_apply_calls, 1);
    assert_eq!(method_app.direct_apply_calls, 0);
    let _ = plain_emitted.into_inner();
}

#[test]
fn queue_apply_owner_shell_app_view_after_preflight_owned_metrics_log_sinks_derive_hold_preflight_match_explicit_snapshot_wrapper()
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
    let preflight = preflight_result(SeqProxy::sequence(5));

    let mut free_app = direct_app();
    let mut free_shell = build_direct_owner_shell();
    let free_snapshot = free_shell.owner().metrics().snapshot();
    let free_emitted = RefCell::new(Vec::new());
    let free_stage = run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot_and_log_sinks_and_derived_hold_preflight(
        &mut free_shell,
        &mut free_app,
        &view_source,
        &tx_source,
        free_snapshot,
        &preflight,
        Ter::TES_SUCCESS,
        |message| free_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| free_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| free_emitted.borrow_mut().push(format!("info:{message}")),
    );

    let mut plain_app = direct_app();
    let mut plain_shell = build_direct_owner_shell();
    let plain_emitted = RefCell::new(Vec::new());
    let plain_stage =
        run_queue_apply_after_preflight_with_app_view_and_log_sinks_and_derived_hold_preflight(
            &mut plain_shell,
            &mut plain_app,
            &view_source,
            &tx_source,
            &preflight,
            Ter::TES_SUCCESS,
            |message| plain_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| plain_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| plain_emitted.borrow_mut().push(format!("info:{message}")),
        );

    let mut method_app = direct_app();
    let mut method_shell = build_direct_owner_shell();
    let method_emitted = RefCell::new(Vec::new());
    let method_stage = method_shell
        .apply_after_preflight_with_app_view_and_owned_metrics_and_log_sinks_and_derived_hold_preflight(
            &mut method_app,
            &view_source,
            &tx_source,
            &preflight,
            Ter::TES_SUCCESS,
            |message| method_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| method_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| method_emitted.borrow_mut().push(format!("info:{message}")),
        );

    assert_eq!(method_stage, free_stage);
    assert_eq!(method_shell.owner(), free_shell.owner());
    assert_eq!(method_emitted.into_inner(), free_emitted.into_inner());
    assert_ne!(plain_stage, method_stage);
    assert_eq!(plain_app.direct_apply_calls, 1);
    assert_eq!(method_app.direct_apply_calls, 0);
    let _ = plain_emitted.into_inner();
}

#[test]
fn queue_apply_owner_shell_app_view_after_preflight_owned_metrics_log_sinks_derive_hold_admission_match_explicit_snapshot_wrapper()
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
    let preflight = preflight_result(SeqProxy::sequence(5));
    let hold_preflight = QueueHoldPreflight::new(
        false,
        false,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        Some(250),
    );

    let mut free_app = direct_app();
    let mut free_shell = build_direct_owner_shell();
    let free_snapshot = free_shell.owner().metrics().snapshot();
    let free_emitted = RefCell::new(Vec::new());
    let free_stage = run_queue_apply_after_preflight_with_app_view_and_metrics_snapshot_and_log_sinks_and_derived_hold_admission(
        &mut free_shell,
        &mut free_app,
        &view_source,
        &tx_source,
        free_snapshot,
        &preflight,
        hold_preflight,
        |message| free_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| free_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| free_emitted.borrow_mut().push(format!("info:{message}")),
    );

    let mut plain_app = direct_app();
    let mut plain_shell = build_direct_owner_shell();
    let plain_emitted = RefCell::new(Vec::new());
    let plain_stage =
        run_queue_apply_after_preflight_with_app_view_and_log_sinks_and_derived_hold_admission(
            &mut plain_shell,
            &mut plain_app,
            &view_source,
            &tx_source,
            &preflight,
            hold_preflight,
            |message| plain_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| plain_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| plain_emitted.borrow_mut().push(format!("info:{message}")),
        );

    let mut method_app = direct_app();
    let mut method_shell = build_direct_owner_shell();
    let method_emitted = RefCell::new(Vec::new());
    let method_stage = method_shell
        .apply_after_preflight_with_app_view_and_owned_metrics_and_log_sinks_and_derived_hold_admission(
            &mut method_app,
            &view_source,
            &tx_source,
            &preflight,
            hold_preflight,
            |message| method_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| method_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| method_emitted.borrow_mut().push(format!("info:{message}")),
        );

    assert_eq!(method_stage, free_stage);
    assert_eq!(method_shell.owner(), free_shell.owner());
    assert_eq!(method_emitted.into_inner(), free_emitted.into_inner());
    assert_ne!(plain_stage, method_stage);
    assert_eq!(plain_app.direct_apply_calls, 1);
    assert_eq!(method_app.direct_apply_calls, 0);
    let _ = plain_emitted.into_inner();
}

#[test]
fn queue_apply_owner_shell_log_sinks_match_live_owner_wrapper_on_full_queue_path() {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(6),
    };
    let view_source = queueing_view(&account);

    let mut live_owner = build_full_queue_owner_shell().owner().clone();
    let live_emitted = RefCell::new(Vec::new());
    let live_stage = run_queue_apply_with_live_owner_from_sources_and_log_sinks(
        &mut live_owner,
        &tx_source,
        &view_source,
        hold_preflight(),
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(6)),
        Ter::TES_SUCCESS,
        || queueing_preflight_result(SeqProxy::sequence(6)),
        |message| live_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| live_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| live_emitted.borrow_mut().push(format!("info:{message}")),
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

    let mut shell = build_full_queue_owner_shell();
    let shell_emitted = RefCell::new(Vec::new());
    let shell_stage = shell.apply_with_log_sinks(
        &tx_source,
        &view_source,
        hold_preflight(),
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(6)),
        Ter::TES_SUCCESS,
        || queueing_preflight_result(SeqProxy::sequence(6)),
        |message| shell_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| shell_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| shell_emitted.borrow_mut().push(format!("info:{message}")),
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
    let live_emitted = live_emitted.into_inner();
    let shell_emitted = shell_emitted.into_inner();

    assert_eq!(shell_stage, live_stage);
    assert_eq!(shell.owner(), &live_owner);
    assert_eq!(shell_emitted, live_emitted);
    assert_eq!(shell_emitted, expected_emitted);
}

#[test]
fn queue_apply_owner_shell_after_preflight_log_sinks_match_live_owner_wrapper_on_direct_path() {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let view_source = direct_view(&account);
    let preflight = queueing_preflight_result(SeqProxy::sequence(5));

    let mut live_owner = build_direct_owner_shell().owner().clone();
    let live_emitted = RefCell::new(Vec::new());
    let live_stage = run_queue_apply_after_preflight_with_live_owner_from_sources_and_log_sinks(
        &mut live_owner,
        &tx_source,
        &view_source,
        &preflight,
        hold_preflight(),
        Ter::TES_SUCCESS,
        |message| live_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| live_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| live_emitted.borrow_mut().push(format!("info:{message}")),
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

    let mut shell = build_direct_owner_shell();
    let shell_emitted = RefCell::new(Vec::new());
    let shell_stage = shell.apply_after_preflight_with_log_sinks(
        &tx_source,
        &view_source,
        &preflight,
        hold_preflight(),
        Ter::TES_SUCCESS,
        |message| shell_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| shell_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| shell_emitted.borrow_mut().push(format!("info:{message}")),
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
    let live_emitted = live_emitted.into_inner();
    let shell_emitted = shell_emitted.into_inner();

    assert_eq!(shell_stage, live_stage);
    assert_eq!(shell.owner(), &live_owner);
    assert_eq!(shell_emitted, live_emitted);
    assert_eq!(shell_emitted, expected_emitted);
}

#[test]
fn queue_apply_owner_shell_owned_metrics_match_live_owner_with_owner_snapshot_on_direct_path() {
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

    let hold_preflight = QueueHoldPreflight::new(
        false,
        false,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        Some(250),
    );
    let consequences = TxConsequences::new(1, SeqProxy::sequence(5));

    let mut live_owner = build_direct_owner_shell().owner().clone();
    let mut expected_view = view_source.clone();
    expected_view.metrics_snapshot = live_owner.metrics().snapshot();
    let live_stage = run_queue_apply_with_live_owner_from_sources(
        &mut live_owner,
        &tx_source,
        &expected_view,
        hold_preflight,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        consequences,
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

    let mut plain_shell = build_direct_owner_shell();
    let plain_stage = plain_shell.apply(
        &tx_source,
        &view_source,
        hold_preflight,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        consequences,
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
        || -> ApplyResult { unreachable!("plain direct path should not hit try-clear") },
        || unreachable!("plain direct path should not hit sandbox apply"),
    );

    let mut shell = build_direct_owner_shell();
    let shell_stage = shell.apply_with_owned_metrics(
        &tx_source,
        &view_source,
        hold_preflight,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        consequences,
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

    assert_eq!(shell_stage, live_stage);
    assert_eq!(shell.owner(), &live_owner);
    assert_ne!(plain_stage, shell_stage);
}

#[test]
fn queue_apply_owner_shell_after_preflight_owned_metrics_match_live_owner_with_owner_snapshot_on_direct_path()
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
    let preflight = queueing_preflight_result(SeqProxy::sequence(5));
    let hold_preflight = QueueHoldPreflight::new(
        false,
        false,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        Some(250),
    );

    let mut live_owner = build_direct_owner_shell().owner().clone();
    let mut expected_view = view_source.clone();
    expected_view.metrics_snapshot = live_owner.metrics().snapshot();
    let live_stage = run_queue_apply_after_preflight_with_live_owner_from_sources(
        &mut live_owner,
        &tx_source,
        &expected_view,
        &preflight,
        hold_preflight,
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

    let mut plain_shell = build_direct_owner_shell();
    let plain_stage = plain_shell.apply_after_preflight(
        &tx_source,
        &view_source,
        &preflight,
        hold_preflight,
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
        || -> ApplyResult { unreachable!("plain direct path should not hit try-clear") },
        || unreachable!("plain direct path should not hit sandbox apply"),
    );

    let mut shell = build_direct_owner_shell();
    let shell_stage = shell.apply_after_preflight_with_owned_metrics(
        &tx_source,
        &view_source,
        &preflight,
        hold_preflight,
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

    assert_eq!(shell_stage, live_stage);
    assert_eq!(shell.owner(), &live_owner);
    assert_ne!(plain_stage, shell_stage);
}

#[test]
fn queue_apply_owner_shell_owned_metrics_caller_preclaim_log_sinks_match_live_owner_with_owner_snapshot()
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
    let hold_preflight = QueueHoldPreflight::new(
        false,
        false,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        Some(250),
    );
    let consequences = TxConsequences::new(1, SeqProxy::sequence(5));
    let expected_prepared = RefCell::new(None::<QueueApplyPreparedPreclaimInputs<String>>);

    let mut live_owner = build_direct_owner_shell().owner().clone();
    let mut expected_view = view_source.clone();
    expected_view.metrics_snapshot = live_owner.metrics().snapshot();
    let live_stage = run_queue_apply_with_live_owner_from_sources_and_caller_preclaim(
        &mut live_owner,
        &tx_source,
        &expected_view,
        hold_preflight,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        consequences,
        Ter::TES_SUCCESS,
        || queueing_preflight_result(SeqProxy::sequence(5)),
        |_| unreachable!("owner metrics path should not direct apply"),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |prepared| {
            expected_prepared.replace(Some(prepared.clone()));
            Ok(QueueApplyPreclaimStage {
                view_source: prepared.view_source,
                trace_message: "trace".to_string(),
                preclaim_result: queueing_preflight_result(SeqProxy::sequence(5))
                    .to_preclaim(9, Ter::TES_SUCCESS),
            })
        },
        || -> ApplyResult { unreachable!("owner metrics path should reject before try-clear") },
        || unreachable!("owner metrics path should reject before sandbox apply"),
    );

    let plain_prepared = RefCell::new(0usize);
    let mut plain_shell = build_direct_owner_shell();
    let plain_stage = plain_shell.apply_with_caller_preclaim_and_log_sinks(
        &tx_source,
        &view_source,
        hold_preflight,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        consequences,
        Ter::TES_SUCCESS,
        || queueing_preflight_result(SeqProxy::sequence(5)),
        |_| {},
        |_| {},
        |_| {},
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |prepared| {
            *plain_prepared.borrow_mut() += 1;
            Ok(QueueApplyPreclaimStage {
                view_source: prepared.view_source,
                trace_message: "trace".to_string(),
                preclaim_result: queueing_preflight_result(SeqProxy::sequence(5))
                    .to_preclaim(9, Ter::TES_SUCCESS),
            })
        },
        || -> ApplyResult { unreachable!("plain direct path should not hit try-clear") },
        || unreachable!("plain direct path should not hit sandbox apply"),
    );

    let mut shell = build_direct_owner_shell();
    let shell_emitted = RefCell::new(Vec::new());
    let shell_stage = shell.apply_with_owned_metrics_and_caller_preclaim_and_log_sinks(
        &tx_source,
        &view_source,
        hold_preflight,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        consequences,
        Ter::TES_SUCCESS,
        || queueing_preflight_result(SeqProxy::sequence(5)),
        |_| unreachable!("owner metrics path should not direct apply"),
        |message| shell_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| shell_emitted.borrow_mut().push(format!("info:{message}")),
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |prepared| {
            assert_eq!(Some(prepared.clone()), *expected_prepared.borrow());
            Ok(QueueApplyPreclaimStage {
                view_source: prepared.view_source,
                trace_message: "trace".to_string(),
                preclaim_result: queueing_preflight_result(SeqProxy::sequence(5))
                    .to_preclaim(9, Ter::TES_SUCCESS),
            })
        },
        || -> ApplyResult { unreachable!("owner metrics path should reject before try-clear") },
        || unreachable!("owner metrics path should reject before sandbox apply"),
    );

    assert_eq!(shell_stage, live_stage);
    assert_eq!(shell.owner(), &live_owner);
    assert_ne!(plain_stage, shell_stage);
    assert_eq!(*plain_prepared.borrow(), 0);
    let _ = shell_emitted.into_inner();
}

#[test]
fn queue_apply_owner_shell_after_preflight_owned_metrics_caller_preclaim_log_sinks_match_live_owner_with_owner_snapshot()
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
    let preflight = queueing_preflight_result(SeqProxy::sequence(5));
    let hold_preflight = QueueHoldPreflight::new(
        false,
        false,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        Some(250),
    );
    let expected_prepared = RefCell::new(None::<QueueApplyPreparedPreclaimInputs<String>>);

    let mut live_owner = build_direct_owner_shell().owner().clone();
    let mut expected_view = view_source.clone();
    expected_view.metrics_snapshot = live_owner.metrics().snapshot();
    let live_stage =
        run_queue_apply_after_preflight_with_live_owner_from_sources_and_caller_preclaim(
            &mut live_owner,
            &tx_source,
            &expected_view,
            &preflight,
            hold_preflight,
            Ter::TES_SUCCESS,
            |_| unreachable!("owner metrics path should not direct apply"),
            || ApplyResult::new(Ter::TES_SUCCESS, true, true),
            |_| true,
            |prepared| {
                expected_prepared.replace(Some(prepared.clone()));
                Ok(QueueApplyPreclaimStage {
                    view_source: prepared.view_source,
                    trace_message: "trace".to_string(),
                    preclaim_result: queueing_preflight_result(SeqProxy::sequence(5))
                        .to_preclaim(9, Ter::TES_SUCCESS),
                })
            },
            || -> ApplyResult { unreachable!("owner metrics path should reject before try-clear") },
            || unreachable!("owner metrics path should reject before sandbox apply"),
        );

    let plain_prepared = RefCell::new(0usize);
    let mut plain_shell = build_direct_owner_shell();
    let plain_stage = plain_shell.apply_after_preflight_with_caller_preclaim_and_log_sinks(
        &tx_source,
        &view_source,
        &preflight,
        hold_preflight,
        Ter::TES_SUCCESS,
        |_| {},
        |_| {},
        |_| {},
        || ApplyResult::new(Ter::TES_SUCCESS, true, true),
        |_| true,
        |prepared| {
            *plain_prepared.borrow_mut() += 1;
            Ok(QueueApplyPreclaimStage {
                view_source: prepared.view_source,
                trace_message: "trace".to_string(),
                preclaim_result: queueing_preflight_result(SeqProxy::sequence(5))
                    .to_preclaim(9, Ter::TES_SUCCESS),
            })
        },
        || -> ApplyResult { unreachable!("plain direct path should not hit try-clear") },
        || unreachable!("plain direct path should not hit sandbox apply"),
    );

    let mut shell = build_direct_owner_shell();
    let shell_stage = shell
        .apply_after_preflight_with_owned_metrics_and_caller_preclaim_and_log_sinks(
            &tx_source,
            &view_source,
            &preflight,
            hold_preflight,
            Ter::TES_SUCCESS,
            |_| unreachable!("owner metrics path should not direct apply"),
            |_| {},
            |_| {},
            || ApplyResult::new(Ter::TES_SUCCESS, true, true),
            |_| true,
            |prepared| {
                assert_eq!(Some(prepared.clone()), *expected_prepared.borrow());
                Ok(QueueApplyPreclaimStage {
                    view_source: prepared.view_source,
                    trace_message: "trace".to_string(),
                    preclaim_result: queueing_preflight_result(SeqProxy::sequence(5))
                        .to_preclaim(9, Ter::TES_SUCCESS),
                })
            },
            || -> ApplyResult { unreachable!("owner metrics path should reject before try-clear") },
            || unreachable!("owner metrics path should reject before sandbox apply"),
        );

    assert_eq!(shell_stage, live_stage);
    assert_eq!(shell.owner(), &live_owner);
    assert_ne!(plain_stage, shell_stage);
    assert_eq!(*plain_prepared.borrow(), 0);
}
