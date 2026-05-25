use std::{cell::RefCell, collections::BTreeMap};

use basics::base_uint::Uint256;
use protocol::{Rules, SeqProxy, Ter};
use tx::{
    ApplyFlags, ApplyResult, BlockerQueueAdmission, FeeQueueEntry, FeeQueueKey, MaybeTx,
    MaybeTxCore, OrderCandidates, PreclaimResult, PreflightResult, QueueAdvanceCandidate,
    QueueApplyAccountStage, QueueApplyCurrentPreclaimClearRuntime, QueueApplyEntryStage,
    QueueApplyExecutionRuntime, QueueApplyHoldPreflightTxSource, QueueApplyLiveOwner,
    QueueApplyLockScope, QueueApplyLockScopeOwner, QueueApplyObservedAccountLookup,
    QueueApplyObservedTicketLookup, QueueApplyObservedTxSource, QueueApplyObservedViewSource,
    QueueApplyOwnerShell, QueueApplyPreclaimStage, QueueApplyPreflightStage,
    QueueApplyPreparedPreclaimInputs, QueueApplyQueuedStage, QueueApplyTxQ,
    QueueApplyViewAdjustment, QueueFeeMetricsSnapshot, QueueHoldPreflight, QueueViews,
    TryClearAccountPlan, TryClearAccountResult, TxConsequences, TxConsequencesCategory, TxQAccount,
    format_direct_apply_finish_log_message, format_direct_apply_start_log_message,
    format_queue_apply_enqueue_debug_message, format_queue_apply_full_queue_evict_info_message,
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
struct TestLockScope;

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

impl QueueApplyLockScope for TestLockScope {}

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

    fn direct_apply(&mut self) -> tx::ApplyResult {
        self.direct_apply_calls += 1;
        tx::ApplyResult::new(Ter::TES_SUCCESS, true, true)
    }

    fn prepare_multitxn(&mut self, _adjustment: QueueApplyViewAdjustment) -> bool {
        self.prepare_multitxn_calls += 1;
        true
    }

    fn run_preclaim(
        &mut self,
        view_source: tx::QueueApplyPreclaimViewSource,
    ) -> tx::PreclaimResult<&'static str, &'static str, &'static str> {
        self.preclaim_calls += 1;
        assert!(!view_source.has_multi_txn());
        tx::PreclaimResult::new(
            100,
            "tx",
            None::<&str>,
            ApplyFlags::NONE,
            "journal",
            Ter::TES_SUCCESS,
        )
    }

    fn run_try_clear(&mut self) -> tx::ApplyResult {
        self.try_clear_calls += 1;
        tx::ApplyResult::new(Ter::TES_SUCCESS, true, true)
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

    fn direct_apply(&mut self) -> tx::ApplyResult {
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

    fn run_try_clear(&mut self) -> tx::ApplyResult {
        self.try_clear_calls += 1;
        tx::ApplyResult::new(Ter::TES_SUCCESS, true, true)
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

    fn direct_apply(&mut self) -> tx::ApplyResult {
        self.direct_apply_calls += 1;
        tx::ApplyResult::new(Ter::TES_SUCCESS, true, true)
    }

    fn prepare_multitxn(&mut self, _adjustment: QueueApplyViewAdjustment) -> bool {
        self.prepare_multitxn_calls += 1;
        true
    }

    fn run_preclaim(
        &mut self,
        _view_source: tx::QueueApplyPreclaimViewSource,
    ) -> tx::PreclaimResult<&'static str, &'static str, &'static str> {
        self.preclaim_calls += 1;
        panic!("caller-preclaim boundary should bypass runtime preclaim");
    }

    fn run_try_clear(&mut self) -> tx::ApplyResult {
        self.try_clear_calls += 1;
        tx::ApplyResult::new(Ter::TES_SUCCESS, true, true)
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

fn flow_tx_source<'a>(account: &'a String) -> TestObservedTxSource<'a> {
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

fn build_lock_scope_owner(
    current_max_size: Option<usize>,
) -> QueueApplyLockScopeOwner<String, &'static str, &'static str, &'static str> {
    QueueApplyLockScopeOwner::new(QueueApplyOwnerShell::new(QueueApplyLiveOwner::new(
        2,
        10,
        25,
        current_max_size,
        OrderCandidates::new(Uint256::from_u64(0)),
        build_views(),
    )))
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

fn divergent_owner_metrics_view(account: &str) -> TestObservedViewSource {
    let mut view = direct_view(account);
    view.metrics_snapshot = QueueFeeMetricsSnapshot {
        txns_expected: 256,
        escalation_multiplier: tx::TXQ_BASE_LEVEL,
    };
    view.open_ledger_tx_count = 40;
    view
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

fn build_full_queue_lock_scope_owner()
-> QueueApplyLockScopeOwner<String, &'static str, &'static str, &'static str> {
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

    QueueApplyLockScopeOwner::new(QueueApplyOwnerShell::new(QueueApplyLiveOwner::new(
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
    )))
}

fn build_direct_lock_scope_owner()
-> QueueApplyLockScopeOwner<String, &'static str, &'static str, &'static str> {
    QueueApplyLockScopeOwner::new(QueueApplyOwnerShell::new(QueueApplyLiveOwner::new(
        2,
        10,
        25,
        Some(10),
        OrderCandidates::new(Uint256::from_u64(0)),
        QueueViews::new(BTreeMap::new(), Vec::new()),
    )))
}

fn build_flow_lock_scope_owner()
-> QueueApplyLockScopeOwner<String, &'static str, &'static str, &'static str> {
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

    QueueApplyLockScopeOwner::new(QueueApplyOwnerShell::new(QueueApplyLiveOwner::new(
        2,
        10,
        25,
        Some(10),
        OrderCandidates::new(Uint256::from_u64(0)),
        QueueViews::new(
            BTreeMap::from([(queued_account_id, queued_account)]),
            vec![],
        ),
    )))
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
fn queue_apply_txq_after_preflight_accepts_structured_try_clear_result() {
    let account = String::from("acct");
    let tx_source = flow_tx_source(&account);
    let view_source = flow_view_source();
    let preflight = flow_preflight_result();

    let mut lock_scope_owner = build_flow_lock_scope_owner();
    let mut lock = TestLockScope;
    let lock_scope_stage = lock_scope_owner.apply_after_preflight(
        &mut lock,
        &tx_source,
        &view_source,
        &preflight,
        hold_preflight(),
        Ter::TES_SUCCESS,
        |_| unreachable!("direct apply should fall through without tracing"),
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
        || TryClearAccountResult::InsufficientFee {
            plan: TryClearAccountPlan {
                queued_seq_proxies: vec![SeqProxy::sequence(5)],
                queued_count: 1,
                target_was_already_queued: false,
                total_fee_level_paid: 50,
            },
            required_total_fee_level: 60,
        },
        || unreachable!("insufficient-fee clear-ahead must not apply sandbox"),
    );

    let mut txq = QueueApplyTxQ::new(build_flow_lock_scope_owner());
    let mut lock = TestLockScope;
    let txq_stage = txq.apply_after_preflight(
        &mut lock,
        &tx_source,
        &view_source,
        &preflight,
        hold_preflight(),
        Ter::TES_SUCCESS,
        |_| unreachable!("direct apply should fall through without tracing"),
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
        || TryClearAccountResult::InsufficientFee {
            plan: TryClearAccountPlan {
                queued_seq_proxies: vec![SeqProxy::sequence(5)],
                queued_count: 1,
                target_was_already_queued: false,
                total_fee_level_paid: 50,
            },
            required_total_fee_level: 60,
        },
        || unreachable!("insufficient-fee clear-ahead must not apply sandbox"),
    );

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(
        txq_stage.apply_result(),
        ApplyResult::new(Ter::TER_QUEUED, false, false)
    );
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
}

#[test]
fn queue_apply_txq_app_view_method_matches_lock_scope_owner() {
    let account = String::from("acct");
    let tx_source = tx_source(&account);
    let view_source = view_source();

    let mut lock_scope_owner = build_lock_scope_owner(Some(10));
    let mut lock = TestLockScope;
    let mut lock_scope_app = TestApplyApp::default();
    let lock_scope_stage = lock_scope_owner.apply_with_app_view(
        &mut lock,
        &mut lock_scope_app,
        &view_source,
        &tx_source,
        hold_preflight(),
        ApplyFlags::NONE,
        blocker_consequences(SeqProxy::sequence(6)),
        Ter::TES_SUCCESS,
    );

    let mut txq = QueueApplyTxQ::new(build_lock_scope_owner(Some(10)));
    let mut lock = TestLockScope;
    let mut txq_app = TestApplyApp::default();
    let txq_stage = txq.apply_with_app_view(
        &mut lock,
        &mut txq_app,
        &view_source,
        &tx_source,
        hold_preflight(),
        ApplyFlags::NONE,
        blocker_consequences(SeqProxy::sequence(6)),
        Ter::TES_SUCCESS,
    );

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_eq!(txq_stage, expected_stage());
    assert_eq!(txq_app.preflight_calls, 1);
    assert_eq!(txq_app.direct_apply_calls, 0);
    assert_eq!(txq_app.prepare_multitxn_calls, 0);
    assert_eq!(txq_app.preclaim_calls, 0);
    assert_eq!(txq_app.try_clear_calls, 0);
    assert_eq!(txq_app.apply_sandbox_calls, 0);
    assert!(txq_app.trace_messages.is_empty());
}

#[test]
fn queue_apply_txq_app_view_owned_metrics_matches_lock_scope_owner() {
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

    let mut lock_scope_owner = build_direct_lock_scope_owner();
    let mut lock = TestLockScope;
    let mut lock_scope_app = direct_app();
    let lock_scope_stage = lock_scope_owner.apply_with_app_view_and_owned_metrics(
        &mut lock,
        &mut lock_scope_app,
        &view_source,
        &tx_source,
        hold_preflight,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        consequences,
        Ter::TES_SUCCESS,
    );

    let mut txq = QueueApplyTxQ::new(build_direct_lock_scope_owner());
    let mut lock = TestLockScope;
    let mut txq_app = direct_app();
    let txq_stage = txq.apply_with_app_view_and_owned_metrics(
        &mut lock,
        &mut txq_app,
        &view_source,
        &tx_source,
        hold_preflight,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        consequences,
        Ter::TES_SUCCESS,
    );

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_eq!(txq_app.direct_apply_calls, 0);
}

#[test]
fn queue_apply_txq_app_view_owned_metrics_log_messages_match_lock_scope_owner() {
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

    let mut lock_scope_owner = build_direct_lock_scope_owner();
    let mut lock = TestLockScope;
    let mut lock_scope_app = direct_app();
    let lock_scope_stage = lock_scope_owner.apply_with_app_view_and_owned_metrics_and_log_messages(
        &mut lock,
        &mut lock_scope_app,
        &view_source,
        &tx_source,
        hold_preflight,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        consequences,
        Ter::TES_SUCCESS,
    );

    let mut plain_txq = QueueApplyTxQ::new(build_direct_lock_scope_owner());
    let mut plain_lock = TestLockScope;
    let mut plain_app = direct_app();
    let plain_stage = plain_txq.apply_with_log_messages(
        &mut plain_lock,
        &mut plain_app,
        &view_source,
        &tx_source,
        hold_preflight,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        consequences,
        Ter::TES_SUCCESS,
    );

    let mut txq = QueueApplyTxQ::new(build_direct_lock_scope_owner());
    let mut lock = TestLockScope;
    let mut txq_app = direct_app();
    let txq_stage = txq.apply_with_app_view_and_owned_metrics_and_log_messages(
        &mut lock,
        &mut txq_app,
        &view_source,
        &tx_source,
        hold_preflight,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        consequences,
        Ter::TES_SUCCESS,
    );

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_eq!(
        txq_stage.queue_log_messages,
        lock_scope_stage.queue_log_messages
    );
    assert_ne!(plain_stage, txq_stage);
    assert_eq!(plain_app.direct_apply_calls, 1);
    assert_eq!(txq_app.direct_apply_calls, 0);
}

#[test]
fn queue_apply_txq_app_view_caller_preclaim_owned_metrics_matches_lock_scope_owner() {
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

    let mut lock_scope_owner = build_direct_lock_scope_owner();
    let mut lock = TestLockScope;
    let mut lock_scope_app = direct_app();
    let lock_scope_stage = lock_scope_owner
        .apply_with_app_view_and_owned_metrics_and_caller_preclaim(
            &mut lock,
            &mut lock_scope_app,
            &view_source,
            &tx_source,
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

    let mut txq = QueueApplyTxQ::new(build_direct_lock_scope_owner());
    let mut lock = TestLockScope;
    let mut txq_app = direct_app();
    let txq_stage = txq.apply_with_app_view_and_owned_metrics_and_caller_preclaim(
        &mut lock,
        &mut txq_app,
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

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_eq!(txq_app.direct_apply_calls, 0);
}

#[test]
fn queue_apply_txq_app_view_caller_preclaim_owned_metrics_log_messages_match_lock_scope_owner() {
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

    let mut lock_scope_owner = build_direct_lock_scope_owner();
    let mut lock = TestLockScope;
    let mut lock_scope_app = direct_app();
    let lock_scope_stage = lock_scope_owner
        .apply_with_app_view_and_owned_metrics_and_caller_preclaim_and_log_messages(
            &mut lock,
            &mut lock_scope_app,
            &view_source,
            &tx_source,
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

    let mut plain_txq = QueueApplyTxQ::new(build_direct_lock_scope_owner());
    let mut plain_lock = TestLockScope;
    let mut plain_txq_app = direct_app();
    let plain_stage = plain_txq.apply_with_app_view_and_caller_preclaim_and_log_messages(
        &mut plain_lock,
        &mut plain_txq_app,
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

    let mut txq = QueueApplyTxQ::new(build_direct_lock_scope_owner());
    let mut lock = TestLockScope;
    let mut txq_app = direct_app();
    let txq_stage = txq.apply_with_app_view_and_owned_metrics_and_caller_preclaim_and_log_messages(
        &mut lock,
        &mut txq_app,
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

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_ne!(plain_stage, txq_stage);
    assert_eq!(txq_app.direct_apply_calls, 0);
}

#[test]
fn queue_apply_txq_app_view_caller_preclaim_matches_lock_scope_owner() {
    let account = String::from("acct");
    let tx_source = flow_tx_source(&account);
    let view_source = flow_view_source();
    let expected_prepared = RefCell::new(None::<QueueApplyPreparedPreclaimInputs<String>>);

    let mut lock_scope_owner = build_flow_lock_scope_owner();
    let mut lock = TestLockScope;
    let mut lock_scope_app = FlowApplyApp::default();
    let lock_scope_stage = lock_scope_owner.apply_with_app_view_and_caller_preclaim(
        &mut lock,
        &mut lock_scope_app,
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

    let mut txq = QueueApplyTxQ::new(build_flow_lock_scope_owner());
    let mut lock = TestLockScope;
    let mut txq_app = FlowApplyApp::default();
    let txq_stage = txq.apply_with_app_view_and_caller_preclaim(
        &mut lock,
        &mut txq_app,
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

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_eq!(txq_app.preflight_calls, 1);
    assert_eq!(txq_app.preclaim_calls, 0);
    assert_eq!(txq_app.direct_apply_calls, 0);
    assert_eq!(txq_app.prepare_multitxn_calls, 1);
    assert_eq!(txq_app.try_clear_calls, 0);
    assert_eq!(txq_app.apply_sandbox_calls, 0);
    assert!(txq_app.trace_messages.is_empty());
}

#[test]
fn queue_apply_txq_app_view_caller_preclaim_log_messages_match_lock_scope_owner_on_full_queue_path()
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

    let mut lock_scope_owner = build_full_queue_lock_scope_owner();
    let mut lock = TestLockScope;
    let mut lock_scope_app = queueing_app();
    let lock_scope_stage = lock_scope_owner
        .apply_with_app_view_and_caller_preclaim_and_log_messages(
            &mut lock,
            &mut lock_scope_app,
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

    let mut txq = QueueApplyTxQ::new(build_full_queue_lock_scope_owner());
    let mut lock = TestLockScope;
    let mut txq_app = queueing_app();
    let txq_stage = txq.apply_with_app_view_and_caller_preclaim_and_log_messages(
        &mut lock,
        &mut txq_app,
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

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_eq!(
        txq_stage.queue_log_messages,
        lock_scope_stage.queue_log_messages
    );
    assert_eq!(txq_app.preflight_calls, 1);
    assert_eq!(txq_app.direct_apply_calls, 0);
    assert_eq!(txq_app.prepare_multitxn_calls, 1);
    assert_eq!(txq_app.preclaim_calls, 0);
    assert_eq!(txq_app.try_clear_calls, 0);
    assert_eq!(txq_app.apply_sandbox_calls, 0);
    assert!(txq_app.trace_messages.is_empty());
}

#[test]
fn queue_apply_txq_app_view_can_derive_hold_preflight_from_tx_source() {
    let account = String::from("acct");
    let tx_source = tx_source(&account);
    let view_source = view_source();

    let mut lock_scope_owner = build_lock_scope_owner(Some(10));
    let mut lock = TestLockScope;
    let mut lock_scope_app = TestApplyApp::default();
    let lock_scope_stage = lock_scope_owner.apply_with_app_view_and_derived_hold_preflight(
        &mut lock,
        &mut lock_scope_app,
        &view_source,
        &tx_source,
        ApplyFlags::NONE,
        blocker_consequences(SeqProxy::sequence(6)),
        Ter::TES_SUCCESS,
    );

    let mut txq = QueueApplyTxQ::new(build_lock_scope_owner(Some(10)));
    let mut lock = TestLockScope;
    let mut txq_app = TestApplyApp::default();
    let txq_stage = txq.apply_with_app_view_and_derived_hold_preflight(
        &mut lock,
        &mut txq_app,
        &view_source,
        &tx_source,
        ApplyFlags::NONE,
        blocker_consequences(SeqProxy::sequence(6)),
        Ter::TES_SUCCESS,
    );

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_eq!(txq_stage, expected_stage());
}

#[test]
fn queue_apply_txq_app_view_can_derive_hold_admission_from_live_facts() {
    let account = String::from("acct");
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

    let mut lock_scope_owner = build_direct_lock_scope_owner();
    let mut lock = TestLockScope;
    let mut lock_scope_app = direct_app();
    let lock_scope_stage = lock_scope_owner.apply_with_app_view_and_derived_hold_admission(
        &mut lock,
        &mut lock_scope_app,
        &view_source,
        &tx_source,
        hold_preflight,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(5)),
    );

    let mut txq = QueueApplyTxQ::new(build_direct_lock_scope_owner());
    let mut lock = TestLockScope;
    let mut txq_app = direct_app();
    let txq_stage = txq.apply_with_app_view_and_derived_hold_admission(
        &mut lock,
        &mut txq_app,
        &view_source,
        &tx_source,
        hold_preflight,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(5)),
    );

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_eq!(txq_app.preflight_calls, 1);
}

#[test]
fn queue_apply_txq_app_view_can_derive_preflight_facts_from_runtime() {
    let account = String::from("acct");
    let tx_source = tx_source(&account);
    let view_source = view_source();

    let mut lock_scope_owner = build_lock_scope_owner(Some(10));
    let mut lock = TestLockScope;
    let mut lock_scope_app = TestApplyApp::default();
    let lock_scope_stage = lock_scope_owner.apply_with_app_view_and_derived_preflight_facts(
        &mut lock,
        &mut lock_scope_app,
        &view_source,
        &tx_source,
        Ter::TES_SUCCESS,
    );

    let mut txq = QueueApplyTxQ::new(build_lock_scope_owner(Some(10)));
    let mut lock = TestLockScope;
    let mut txq_app = TestApplyApp::default();
    let txq_stage = txq.apply_with_app_view_and_derived_preflight_facts(
        &mut lock,
        &mut txq_app,
        &view_source,
        &tx_source,
        Ter::TES_SUCCESS,
    );

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_eq!(txq_stage, expected_stage());
    assert_eq!(txq_app.preflight_calls, 1);
}

#[test]
fn queue_apply_txq_app_view_can_derive_runtime_preflight_and_hold_admission() {
    let account = String::from("acct");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let view_source = queueing_view(&account);

    let mut lock_scope_owner = build_direct_lock_scope_owner();
    let mut lock = TestLockScope;
    let mut lock_scope_app = direct_app();
    let lock_scope_stage = lock_scope_owner
        .apply_with_app_view_and_derived_preflight_facts_and_hold_admission(
            &mut lock,
            &mut lock_scope_app,
            &view_source,
            &tx_source,
        );

    let mut txq = QueueApplyTxQ::new(build_direct_lock_scope_owner());
    let mut lock = TestLockScope;
    let mut txq_app = direct_app();
    let txq_stage = txq.apply_with_app_view_and_derived_preflight_facts_and_hold_admission(
        &mut lock,
        &mut txq_app,
        &view_source,
        &tx_source,
    );

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_eq!(txq_app.preflight_calls, 1);
}

#[test]
fn queue_apply_txq_app_view_after_preflight_matches_lock_scope_owner() {
    let account = String::from("acct");
    let tx_source = tx_source(&account);
    let view_source = view_source();
    let preflight = preflight_result(SeqProxy::sequence(6));

    let mut lock_scope_owner = build_lock_scope_owner(Some(10));
    let mut lock = TestLockScope;
    let mut lock_scope_app = TestApplyApp::default();
    let lock_scope_stage = lock_scope_owner.apply_after_preflight_with_app_view(
        &mut lock,
        &mut lock_scope_app,
        &view_source,
        &tx_source,
        &preflight,
        hold_preflight(),
        Ter::TES_SUCCESS,
    );

    let mut txq = QueueApplyTxQ::new(build_lock_scope_owner(Some(10)));
    let mut lock = TestLockScope;
    let mut txq_app = TestApplyApp::default();
    let txq_stage = txq.apply_after_preflight_with_app_view(
        &mut lock,
        &mut txq_app,
        &view_source,
        &tx_source,
        &preflight,
        hold_preflight(),
        Ter::TES_SUCCESS,
    );

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_eq!(txq_stage, expected_stage());
    assert_eq!(txq_app.preflight_calls, 0);
    assert_eq!(txq_app.direct_apply_calls, 0);
    assert_eq!(txq_app.prepare_multitxn_calls, 0);
    assert_eq!(txq_app.preclaim_calls, 0);
    assert_eq!(txq_app.try_clear_calls, 0);
    assert_eq!(txq_app.apply_sandbox_calls, 0);
    assert!(txq_app.trace_messages.is_empty());
}

#[test]
fn queue_apply_txq_app_view_after_preflight_owned_metrics_matches_lock_scope_owner() {
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

    let mut lock_scope_owner = build_direct_lock_scope_owner();
    let mut lock = TestLockScope;
    let mut lock_scope_app = direct_app();
    let lock_scope_stage = lock_scope_owner.apply_after_preflight_with_app_view_and_owned_metrics(
        &mut lock,
        &mut lock_scope_app,
        &view_source,
        &tx_source,
        &preflight,
        hold_preflight,
        Ter::TES_SUCCESS,
    );

    let mut txq = QueueApplyTxQ::new(build_direct_lock_scope_owner());
    let mut lock = TestLockScope;
    let mut txq_app = direct_app();
    let txq_stage = txq.apply_after_preflight_with_app_view_and_owned_metrics(
        &mut lock,
        &mut txq_app,
        &view_source,
        &tx_source,
        &preflight,
        hold_preflight,
        Ter::TES_SUCCESS,
    );

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_eq!(txq_app.direct_apply_calls, 0);
}

#[test]
fn queue_apply_txq_app_view_after_preflight_owned_metrics_log_messages_match_lock_scope_owner() {
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

    let mut lock_scope_owner = build_direct_lock_scope_owner();
    let mut lock = TestLockScope;
    let mut lock_scope_app = direct_app();
    let lock_scope_stage = lock_scope_owner
        .apply_after_preflight_with_app_view_and_owned_metrics_and_log_messages(
            &mut lock,
            &mut lock_scope_app,
            &view_source,
            &tx_source,
            &preflight,
            hold_preflight,
            Ter::TES_SUCCESS,
        );

    let mut plain_txq = QueueApplyTxQ::new(build_direct_lock_scope_owner());
    let mut plain_lock = TestLockScope;
    let mut plain_app = direct_app();
    let plain_stage = plain_txq.apply_after_preflight_with_log_messages(
        &mut plain_lock,
        &mut plain_app,
        &view_source,
        &tx_source,
        &preflight,
        hold_preflight,
        Ter::TES_SUCCESS,
    );

    let mut txq = QueueApplyTxQ::new(build_direct_lock_scope_owner());
    let mut lock = TestLockScope;
    let mut txq_app = direct_app();
    let txq_stage = txq.apply_after_preflight_with_app_view_and_owned_metrics_and_log_messages(
        &mut lock,
        &mut txq_app,
        &view_source,
        &tx_source,
        &preflight,
        hold_preflight,
        Ter::TES_SUCCESS,
    );

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_eq!(
        txq_stage.queue_log_messages,
        lock_scope_stage.queue_log_messages
    );
    assert_ne!(plain_stage, txq_stage);
    assert_eq!(plain_app.direct_apply_calls, 1);
    assert_eq!(txq_app.direct_apply_calls, 0);
}

#[test]
fn queue_apply_txq_app_view_after_preflight_caller_preclaim_owned_metrics_matches_lock_scope_owner()
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

    let mut lock_scope_owner = build_direct_lock_scope_owner();
    let mut lock = TestLockScope;
    let mut lock_scope_app = direct_app();
    let lock_scope_stage = lock_scope_owner
        .apply_after_preflight_with_app_view_and_owned_metrics_and_caller_preclaim(
            &mut lock,
            &mut lock_scope_app,
            &view_source,
            &tx_source,
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

    let mut txq = QueueApplyTxQ::new(build_direct_lock_scope_owner());
    let mut lock = TestLockScope;
    let mut txq_app = direct_app();
    let txq_stage = txq.apply_after_preflight_with_app_view_and_owned_metrics_and_caller_preclaim(
        &mut lock,
        &mut txq_app,
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

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_eq!(txq_app.direct_apply_calls, 0);
}

#[test]
fn queue_apply_txq_app_view_after_preflight_caller_preclaim_owned_metrics_log_messages_match_lock_scope_owner()
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

    let mut lock_scope_owner = build_direct_lock_scope_owner();
    let mut lock = TestLockScope;
    let mut lock_scope_app = direct_app();
    let lock_scope_stage = lock_scope_owner
        .apply_after_preflight_with_app_view_and_owned_metrics_and_caller_preclaim_and_log_messages(
            &mut lock,
            &mut lock_scope_app,
            &view_source,
            &tx_source,
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

    let mut plain_txq = QueueApplyTxQ::new(build_direct_lock_scope_owner());
    let mut plain_lock = TestLockScope;
    let mut plain_txq_app = direct_app();
    let plain_stage = plain_txq
        .apply_after_preflight_with_app_view_and_caller_preclaim_and_log_messages(
            &mut plain_lock,
            &mut plain_txq_app,
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

    let mut txq = QueueApplyTxQ::new(build_direct_lock_scope_owner());
    let mut lock = TestLockScope;
    let mut txq_app = direct_app();
    let txq_stage = txq
        .apply_after_preflight_with_app_view_and_owned_metrics_and_caller_preclaim_and_log_messages(
            &mut lock,
            &mut txq_app,
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

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_ne!(plain_stage, txq_stage);
    assert_eq!(txq_app.direct_apply_calls, 0);
}

#[test]
fn queue_apply_txq_app_view_after_preflight_caller_preclaim_matches_lock_scope_owner() {
    let account = String::from("acct");
    let tx_source = flow_tx_source(&account);
    let view_source = flow_view_source();
    let preflight = flow_preflight_result();
    let expected_prepared = RefCell::new(None::<QueueApplyPreparedPreclaimInputs<String>>);

    let mut lock_scope_owner = build_flow_lock_scope_owner();
    let mut lock = TestLockScope;
    let mut lock_scope_app = FlowApplyApp::default();
    let lock_scope_stage = lock_scope_owner
        .apply_after_preflight_with_app_view_and_caller_preclaim(
            &mut lock,
            &mut lock_scope_app,
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

    let mut txq = QueueApplyTxQ::new(build_flow_lock_scope_owner());
    let mut lock = TestLockScope;
    let mut txq_app = FlowApplyApp::default();
    let txq_stage = txq.apply_after_preflight_with_app_view_and_caller_preclaim(
        &mut lock,
        &mut txq_app,
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

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_eq!(txq_app.preflight_calls, 0);
    assert_eq!(txq_app.preclaim_calls, 0);
    assert_eq!(txq_app.direct_apply_calls, 0);
    assert_eq!(txq_app.prepare_multitxn_calls, 1);
    assert_eq!(txq_app.try_clear_calls, 0);
    assert_eq!(txq_app.apply_sandbox_calls, 0);
    assert!(txq_app.trace_messages.is_empty());
}

#[test]
fn queue_apply_txq_app_view_after_preflight_caller_preclaim_log_messages_match_lock_scope_owner_on_full_queue_path()
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

    let mut lock_scope_owner = build_full_queue_lock_scope_owner();
    let mut lock = TestLockScope;
    let mut lock_scope_app = queueing_app();
    let lock_scope_stage = lock_scope_owner
        .apply_after_preflight_with_app_view_and_caller_preclaim_and_log_messages(
            &mut lock,
            &mut lock_scope_app,
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

    let mut txq = QueueApplyTxQ::new(build_full_queue_lock_scope_owner());
    let mut lock = TestLockScope;
    let mut txq_app = queueing_app();
    let txq_stage = txq.apply_after_preflight_with_app_view_and_caller_preclaim_and_log_messages(
        &mut lock,
        &mut txq_app,
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

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_eq!(
        txq_stage.queue_log_messages,
        lock_scope_stage.queue_log_messages
    );
    assert_eq!(txq_app.preflight_calls, 0);
    assert_eq!(txq_app.direct_apply_calls, 0);
    assert_eq!(txq_app.prepare_multitxn_calls, 1);
    assert_eq!(txq_app.preclaim_calls, 0);
    assert_eq!(txq_app.try_clear_calls, 0);
    assert_eq!(txq_app.apply_sandbox_calls, 0);
    assert!(txq_app.trace_messages.is_empty());
}

#[test]
fn queue_apply_txq_after_preflight_can_derive_hold_preflight_from_tx_source() {
    let account = String::from("acct");
    let tx_source = tx_source(&account);
    let view_source = view_source();
    let preflight = preflight_result(SeqProxy::sequence(6));

    let mut lock_scope_owner = build_lock_scope_owner(Some(10));
    let mut lock = TestLockScope;
    let mut lock_scope_app = TestApplyApp::default();
    let lock_scope_stage = lock_scope_owner
        .apply_after_preflight_with_app_view_and_derived_hold_preflight(
            &mut lock,
            &mut lock_scope_app,
            &view_source,
            &tx_source,
            &preflight,
            Ter::TES_SUCCESS,
        );

    let mut txq = QueueApplyTxQ::new(build_lock_scope_owner(Some(10)));
    let mut lock = TestLockScope;
    let mut txq_app = TestApplyApp::default();
    let txq_stage = txq.apply_after_preflight_with_app_view_and_derived_hold_preflight(
        &mut lock,
        &mut txq_app,
        &view_source,
        &tx_source,
        &preflight,
        Ter::TES_SUCCESS,
    );

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_eq!(txq_stage, expected_stage());
}

#[test]
fn queue_apply_txq_after_preflight_can_derive_hold_admission_from_live_facts() {
    let account = String::from("acct");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let view_source = queueing_view(&account);
    let preflight = queueing_preflight_result(SeqProxy::sequence(5));
    let hold_preflight = QueueHoldPreflight::new(
        false,
        false,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        Some(250),
    );

    let mut lock_scope_owner = build_direct_lock_scope_owner();
    let mut lock = TestLockScope;
    let mut lock_scope_app = direct_app();
    let lock_scope_stage = lock_scope_owner
        .apply_after_preflight_with_app_view_and_derived_hold_admission(
            &mut lock,
            &mut lock_scope_app,
            &view_source,
            &tx_source,
            &preflight,
            hold_preflight,
        );

    let mut txq = QueueApplyTxQ::new(build_direct_lock_scope_owner());
    let mut lock = TestLockScope;
    let mut txq_app = direct_app();
    let txq_stage = txq.apply_after_preflight_with_app_view_and_derived_hold_admission(
        &mut lock,
        &mut txq_app,
        &view_source,
        &tx_source,
        &preflight,
        hold_preflight,
    );

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_eq!(txq_app.preflight_calls, 0);
}

#[test]
fn queue_apply_txq_exposes_lock_scope_owner_accessors() {
    let mut txq = QueueApplyTxQ::new(build_lock_scope_owner(Some(1)));
    assert!(
        !txq.lock_scope_owner()
            .owner()
            .owner()
            .observed_queue(Ter::TES_SUCCESS)
            .queue_is_full
    );

    let removed = txq
        .lock_scope_owner_mut()
        .owner_mut()
        .owner_mut()
        .views_mut()
        .accounts
        .remove("acct");
    assert!(removed.is_some());
    txq.lock_scope_owner_mut()
        .owner_mut()
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

    let observed = txq
        .lock_scope_owner()
        .owner()
        .owner()
        .observed_queue(Ter::TES_SUCCESS);
    assert!(observed.queue_is_full);
    assert_eq!(observed.maximum_txn_per_account, 10);
}

#[test]
fn queue_apply_txq_app_view_owned_metrics_log_sinks_match_lock_scope_owner() {
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

    let mut lock_scope_owner = build_direct_lock_scope_owner();
    let mut lock = TestLockScope;
    let mut lock_scope_app = direct_app();
    let lock_scope_emitted = RefCell::new(Vec::new());
    let lock_scope_stage = lock_scope_owner.apply_with_app_view_and_owned_metrics_and_log_sinks(
        &mut lock,
        &mut lock_scope_app,
        &view_source,
        &tx_source,
        hold_preflight,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(5)),
        Ter::TES_SUCCESS,
        |message| {
            lock_scope_emitted
                .borrow_mut()
                .push(format!("trace:{message}"))
        },
        |message| {
            lock_scope_emitted
                .borrow_mut()
                .push(format!("debug:{message}"))
        },
        |message| {
            lock_scope_emitted
                .borrow_mut()
                .push(format!("info:{message}"))
        },
    );

    let mut txq = QueueApplyTxQ::new(build_direct_lock_scope_owner());
    let mut lock = TestLockScope;
    let mut txq_app = direct_app();
    let txq_emitted = RefCell::new(Vec::new());
    let txq_stage = txq.apply_with_app_view_and_owned_metrics_and_log_sinks(
        &mut lock,
        &mut txq_app,
        &view_source,
        &tx_source,
        hold_preflight,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(5)),
        Ter::TES_SUCCESS,
        |message| txq_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| txq_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| txq_emitted.borrow_mut().push(format!("info:{message}")),
    );

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_eq!(txq_emitted.into_inner(), lock_scope_emitted.into_inner());
    assert_eq!(txq_app.direct_apply_calls, 0);
}

#[test]
fn queue_apply_txq_app_view_caller_preclaim_owned_metrics_log_sinks_match_lock_scope_owner() {
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

    let mut lock_scope_owner = build_direct_lock_scope_owner();
    let mut lock = TestLockScope;
    let mut lock_scope_app = direct_app();
    let lock_scope_emitted = RefCell::new(Vec::new());
    let lock_scope_stage = lock_scope_owner
        .apply_with_app_view_and_owned_metrics_and_caller_preclaim_and_log_sinks(
            &mut lock,
            &mut lock_scope_app,
            &view_source,
            &tx_source,
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
            |message| {
                lock_scope_emitted
                    .borrow_mut()
                    .push(format!("trace:{message}"))
            },
            |message| {
                lock_scope_emitted
                    .borrow_mut()
                    .push(format!("debug:{message}"))
            },
            |message| {
                lock_scope_emitted
                    .borrow_mut()
                    .push(format!("info:{message}"))
            },
        );

    let mut txq = QueueApplyTxQ::new(build_direct_lock_scope_owner());
    let mut lock = TestLockScope;
    let mut txq_app = direct_app();
    let txq_emitted = RefCell::new(Vec::new());
    let txq_stage = txq.apply_with_app_view_and_owned_metrics_and_caller_preclaim_and_log_sinks(
        &mut lock,
        &mut txq_app,
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
        |message| txq_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| txq_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| txq_emitted.borrow_mut().push(format!("info:{message}")),
    );

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_eq!(txq_emitted.into_inner(), lock_scope_emitted.into_inner());
    assert_eq!(txq_app.direct_apply_calls, 0);
}

#[test]
fn queue_apply_txq_caller_preclaim_log_sinks_match_lock_scope_owner_on_full_queue_path() {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(6),
    };
    let view_source = queueing_view(&account);
    let expected_prepared = RefCell::new(None::<QueueApplyPreparedPreclaimInputs<String>>);

    let mut lock_scope_owner = build_full_queue_lock_scope_owner();
    let mut lock = TestLockScope;
    let mut lock_scope_app = queueing_app();
    let lock_scope_emitted = RefCell::new(Vec::new());
    let lock_scope_stage = lock_scope_owner.apply_with_app_view_and_caller_preclaim_and_log_sinks(
        &mut lock,
        &mut lock_scope_app,
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
        |message| {
            lock_scope_emitted
                .borrow_mut()
                .push(format!("trace:{message}"))
        },
        |message| {
            lock_scope_emitted
                .borrow_mut()
                .push(format!("debug:{message}"))
        },
        |message| {
            lock_scope_emitted
                .borrow_mut()
                .push(format!("info:{message}"))
        },
    );

    let mut txq = QueueApplyTxQ::new(build_full_queue_lock_scope_owner());
    let mut lock = TestLockScope;
    let mut txq_app = queueing_app();
    let txq_emitted = RefCell::new(Vec::new());
    let txq_stage = txq.apply_with_app_view_and_caller_preclaim_and_log_sinks(
        &mut lock,
        &mut txq_app,
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
        |message| txq_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| txq_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| txq_emitted.borrow_mut().push(format!("info:{message}")),
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

    let lock_scope_emitted = lock_scope_emitted.into_inner();
    let txq_emitted = txq_emitted.into_inner();

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_eq!(txq_emitted, lock_scope_emitted);
    assert_eq!(txq_emitted, expected_emitted);
    assert_eq!(txq_app.preflight_calls, 1);
    assert_eq!(txq_app.preclaim_calls, 0);
    assert_eq!(txq_app.direct_apply_calls, 0);
    assert_eq!(txq_app.prepare_multitxn_calls, 1);
    assert!(lock_scope_app.trace_messages.is_empty());
    assert!(txq_app.trace_messages.is_empty());
}

#[test]
fn queue_apply_txq_app_view_after_preflight_owned_metrics_log_sinks_match_lock_scope_owner() {
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

    let mut lock_scope_owner = build_direct_lock_scope_owner();
    let mut lock = TestLockScope;
    let mut lock_scope_app = direct_app();
    let lock_scope_emitted = RefCell::new(Vec::new());
    let lock_scope_stage = lock_scope_owner
        .apply_after_preflight_with_app_view_and_owned_metrics_and_log_sinks(
            &mut lock,
            &mut lock_scope_app,
            &view_source,
            &tx_source,
            &preflight,
            hold_preflight,
            Ter::TES_SUCCESS,
            |message| {
                lock_scope_emitted
                    .borrow_mut()
                    .push(format!("trace:{message}"))
            },
            |message| {
                lock_scope_emitted
                    .borrow_mut()
                    .push(format!("debug:{message}"))
            },
            |message| {
                lock_scope_emitted
                    .borrow_mut()
                    .push(format!("info:{message}"))
            },
        );

    let mut txq = QueueApplyTxQ::new(build_direct_lock_scope_owner());
    let mut lock = TestLockScope;
    let mut txq_app = direct_app();
    let txq_emitted = RefCell::new(Vec::new());
    let txq_stage = txq.apply_after_preflight_with_app_view_and_owned_metrics_and_log_sinks(
        &mut lock,
        &mut txq_app,
        &view_source,
        &tx_source,
        &preflight,
        hold_preflight,
        Ter::TES_SUCCESS,
        |message| txq_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| txq_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| txq_emitted.borrow_mut().push(format!("info:{message}")),
    );

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_eq!(txq_emitted.into_inner(), lock_scope_emitted.into_inner());
    assert_eq!(txq_app.direct_apply_calls, 0);
}

#[test]
fn queue_apply_txq_app_view_after_preflight_caller_preclaim_owned_metrics_log_sinks_match_lock_scope_owner()
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

    let mut lock_scope_owner = build_direct_lock_scope_owner();
    let mut lock = TestLockScope;
    let mut lock_scope_app = direct_app();
    let lock_scope_emitted = RefCell::new(Vec::new());
    let lock_scope_stage = lock_scope_owner
        .apply_after_preflight_with_app_view_and_owned_metrics_and_caller_preclaim_and_log_sinks(
            &mut lock,
            &mut lock_scope_app,
            &view_source,
            &tx_source,
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
            |message| {
                lock_scope_emitted
                    .borrow_mut()
                    .push(format!("trace:{message}"))
            },
            |message| {
                lock_scope_emitted
                    .borrow_mut()
                    .push(format!("debug:{message}"))
            },
            |message| {
                lock_scope_emitted
                    .borrow_mut()
                    .push(format!("info:{message}"))
            },
        );

    let mut txq = QueueApplyTxQ::new(build_direct_lock_scope_owner());
    let mut lock = TestLockScope;
    let mut txq_app = direct_app();
    let txq_emitted = RefCell::new(Vec::new());
    let txq_stage = txq
        .apply_after_preflight_with_app_view_and_owned_metrics_and_caller_preclaim_and_log_sinks(
            &mut lock,
            &mut txq_app,
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
            |message| txq_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| txq_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| txq_emitted.borrow_mut().push(format!("info:{message}")),
        );

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_eq!(txq_emitted.into_inner(), lock_scope_emitted.into_inner());
    assert_eq!(txq_app.direct_apply_calls, 0);
}

#[test]
fn queue_apply_txq_after_preflight_caller_preclaim_log_sinks_match_lock_scope_owner_on_full_queue_path()
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

    let mut lock_scope_owner = build_full_queue_lock_scope_owner();
    let mut lock = TestLockScope;
    let mut lock_scope_app = queueing_app();
    let lock_scope_emitted = RefCell::new(Vec::new());
    let lock_scope_stage = lock_scope_owner
        .apply_after_preflight_with_app_view_and_caller_preclaim_and_log_sinks(
            &mut lock,
            &mut lock_scope_app,
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
            |message| {
                lock_scope_emitted
                    .borrow_mut()
                    .push(format!("trace:{message}"))
            },
            |message| {
                lock_scope_emitted
                    .borrow_mut()
                    .push(format!("debug:{message}"))
            },
            |message| {
                lock_scope_emitted
                    .borrow_mut()
                    .push(format!("info:{message}"))
            },
        );

    let mut txq = QueueApplyTxQ::new(build_full_queue_lock_scope_owner());
    let mut lock = TestLockScope;
    let mut txq_app = queueing_app();
    let txq_emitted = RefCell::new(Vec::new());
    let txq_stage = txq.apply_after_preflight_with_app_view_and_caller_preclaim_and_log_sinks(
        &mut lock,
        &mut txq_app,
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
        |message| txq_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| txq_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| txq_emitted.borrow_mut().push(format!("info:{message}")),
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

    let lock_scope_emitted = lock_scope_emitted.into_inner();
    let txq_emitted = txq_emitted.into_inner();

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_eq!(txq_emitted, lock_scope_emitted);
    assert_eq!(txq_emitted, expected_emitted);
    assert_eq!(txq_app.preflight_calls, 0);
    assert_eq!(txq_app.preclaim_calls, 0);
    assert_eq!(txq_app.direct_apply_calls, 0);
    assert_eq!(txq_app.prepare_multitxn_calls, 1);
    assert!(lock_scope_app.trace_messages.is_empty());
    assert!(txq_app.trace_messages.is_empty());
}

#[test]
fn queue_apply_txq_log_sinks_match_lock_scope_owner_on_full_queue_path() {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(6),
    };
    let view_source = queueing_view(&account);

    let mut lock_scope_owner = build_full_queue_lock_scope_owner();
    let mut lock = TestLockScope;
    let mut lock_scope_app = queueing_app();
    let lock_scope_emitted = RefCell::new(Vec::new());
    let lock_scope_stage = lock_scope_owner.apply_with_app_view_and_log_sinks(
        &mut lock,
        &mut lock_scope_app,
        &view_source,
        &tx_source,
        hold_preflight(),
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(6)),
        Ter::TES_SUCCESS,
        |message| {
            lock_scope_emitted
                .borrow_mut()
                .push(format!("trace:{message}"))
        },
        |message| {
            lock_scope_emitted
                .borrow_mut()
                .push(format!("debug:{message}"))
        },
        |message| {
            lock_scope_emitted
                .borrow_mut()
                .push(format!("info:{message}"))
        },
    );

    let mut txq = QueueApplyTxQ::new(build_full_queue_lock_scope_owner());
    let mut lock = TestLockScope;
    let mut txq_app = queueing_app();
    let txq_emitted = RefCell::new(Vec::new());
    let txq_stage = txq.apply_with_app_view_and_log_sinks(
        &mut lock,
        &mut txq_app,
        &view_source,
        &tx_source,
        hold_preflight(),
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(6)),
        Ter::TES_SUCCESS,
        |message| txq_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| txq_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| txq_emitted.borrow_mut().push(format!("info:{message}")),
    );

    let lock_scope_emitted = lock_scope_emitted.into_inner();
    let txq_emitted = txq_emitted.into_inner();

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_eq!(txq_emitted, lock_scope_emitted);
    assert_eq!(
        txq_emitted,
        vec![
            format!(
                "info:{}",
                format_queue_apply_full_queue_evict_info_message(
                    "b",
                    50,
                    Uint256::from_u64(9),
                    256
                )
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
        ]
    );
    assert_eq!(txq_app.preflight_calls, 1);
    assert_eq!(txq_app.direct_apply_calls, 0);
    assert_eq!(txq_app.prepare_multitxn_calls, 1);
    assert_eq!(txq_app.preclaim_calls, 1);
    assert!(lock_scope_app.trace_messages.is_empty());
    assert!(txq_app.trace_messages.is_empty());
}

#[test]
fn queue_apply_txq_log_messages_match_lock_scope_owner_on_full_queue_path() {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(6),
    };
    let view_source = queueing_view(&account);

    let mut lock_scope_owner = build_full_queue_lock_scope_owner();
    let mut lock = TestLockScope;
    let mut lock_scope_app = queueing_app();
    let lock_scope_stage = lock_scope_owner.apply_with_log_messages(
        &mut lock,
        &mut lock_scope_app,
        &view_source,
        &tx_source,
        hold_preflight(),
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(6)),
        Ter::TES_SUCCESS,
    );

    let mut txq = QueueApplyTxQ::new(build_full_queue_lock_scope_owner());
    let mut lock = TestLockScope;
    let mut txq_app = queueing_app();
    let txq_stage = txq.apply_with_log_messages(
        &mut lock,
        &mut txq_app,
        &view_source,
        &tx_source,
        hold_preflight(),
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(6)),
        Ter::TES_SUCCESS,
    );

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_eq!(txq_stage.queue_log_messages.info.len(), 1);
    assert_eq!(txq_stage.queue_log_messages.debug.len(), 1);
    assert!(txq_stage.queue_log_messages.trace.is_empty());
    assert_eq!(
        txq_stage.queue_log_messages.info,
        vec![format_queue_apply_full_queue_evict_info_message(
            "b",
            50,
            Uint256::from_u64(9),
            256
        )]
    );
    assert_eq!(
        txq_stage.queue_log_messages.debug,
        vec![format_queue_apply_enqueue_debug_message(
            Uint256::from_u64(9),
            Ter::TES_SUCCESS,
            true,
            "a",
            ApplyFlags::FAIL_HARD
        )]
    );
    assert_eq!(lock_scope_app.preflight_calls, 1);
    assert_eq!(txq_app.preflight_calls, 1);
    assert_eq!(lock_scope_app.direct_apply_calls, 0);
    assert_eq!(txq_app.direct_apply_calls, 0);
    assert_eq!(lock_scope_app.prepare_multitxn_calls, 1);
    assert_eq!(txq_app.prepare_multitxn_calls, 1);
    assert_eq!(lock_scope_app.preclaim_calls, 1);
    assert_eq!(txq_app.preclaim_calls, 1);
    assert!(lock_scope_app.trace_messages.is_empty());
    assert!(txq_app.trace_messages.is_empty());
}

#[test]
fn queue_apply_txq_log_sinks_derive_hold_admission_like_lock_scope_owner() {
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

    let mut lock_scope_owner = build_direct_lock_scope_owner();
    let mut lock = TestLockScope;
    let mut lock_scope_app = direct_app();
    let lock_scope_emitted = RefCell::new(Vec::new());
    let lock_scope_stage = lock_scope_owner
        .apply_with_app_view_and_log_sinks_and_derived_hold_admission(
            &mut lock,
            &mut lock_scope_app,
            &view_source,
            &tx_source,
            hold_preflight,
            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
            TxConsequences::new(1, SeqProxy::sequence(5)),
            |message| {
                lock_scope_emitted
                    .borrow_mut()
                    .push(format!("trace:{message}"))
            },
            |message| {
                lock_scope_emitted
                    .borrow_mut()
                    .push(format!("debug:{message}"))
            },
            |message| {
                lock_scope_emitted
                    .borrow_mut()
                    .push(format!("info:{message}"))
            },
        );

    let mut txq = QueueApplyTxQ::new(build_direct_lock_scope_owner());
    let mut lock = TestLockScope;
    let mut txq_app = direct_app();
    let txq_emitted = RefCell::new(Vec::new());
    let txq_stage = txq.apply_with_app_view_and_log_sinks_and_derived_hold_admission(
        &mut lock,
        &mut txq_app,
        &view_source,
        &tx_source,
        hold_preflight,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(5)),
        |message| txq_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| txq_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| txq_emitted.borrow_mut().push(format!("info:{message}")),
    );

    let lock_scope_emitted = lock_scope_emitted.into_inner();
    let txq_emitted = txq_emitted.into_inner();

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_eq!(txq_emitted, lock_scope_emitted);
    assert_eq!(txq_app.preflight_calls, 1);
    assert_eq!(txq_app.direct_apply_calls, 1);
    assert!(lock_scope_app.trace_messages.is_empty());
    assert!(txq_app.trace_messages.is_empty());
}

#[test]
fn queue_apply_txq_derived_hold_admission_log_messages_match_lock_scope_owner() {
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

    let mut lock_scope_owner = build_direct_lock_scope_owner();
    let mut lock = TestLockScope;
    let mut lock_scope_app = direct_app();
    let lock_scope_stage = lock_scope_owner
        .apply_with_app_view_and_derived_hold_admission_and_log_messages(
            &mut lock,
            &mut lock_scope_app,
            &view_source,
            &tx_source,
            hold_preflight,
            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
            TxConsequences::new(1, SeqProxy::sequence(5)),
        );

    let mut txq = QueueApplyTxQ::new(build_direct_lock_scope_owner());
    let mut lock = TestLockScope;
    let mut txq_app = direct_app();
    let txq_stage = txq.apply_with_app_view_and_derived_hold_admission_and_log_messages(
        &mut lock,
        &mut txq_app,
        &view_source,
        &tx_source,
        hold_preflight,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(5)),
    );

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert!(txq_stage.queue_log_messages.trace.is_empty());
    assert!(txq_stage.queue_log_messages.debug.is_empty());
    assert!(txq_stage.queue_log_messages.info.is_empty());
    assert_eq!(txq_app.preflight_calls, 1);
    assert_eq!(txq_app.direct_apply_calls, 1);
}

#[test]
fn queue_apply_txq_log_sinks_derive_hold_preflight_like_lock_scope_owner() {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(6),
    };
    let view_source = queueing_view(&account);

    let mut lock_scope_owner = build_full_queue_lock_scope_owner();
    let mut lock = TestLockScope;
    let mut lock_scope_app = queueing_app();
    let lock_scope_emitted = RefCell::new(Vec::new());
    let lock_scope_stage = lock_scope_owner
        .apply_with_app_view_and_log_sinks_and_derived_hold_preflight(
            &mut lock,
            &mut lock_scope_app,
            &view_source,
            &tx_source,
            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
            TxConsequences::new(1, SeqProxy::sequence(6)),
            Ter::TES_SUCCESS,
            |message| {
                lock_scope_emitted
                    .borrow_mut()
                    .push(format!("trace:{message}"))
            },
            |message| {
                lock_scope_emitted
                    .borrow_mut()
                    .push(format!("debug:{message}"))
            },
            |message| {
                lock_scope_emitted
                    .borrow_mut()
                    .push(format!("info:{message}"))
            },
        );

    let mut txq = QueueApplyTxQ::new(build_full_queue_lock_scope_owner());
    let mut lock = TestLockScope;
    let mut txq_app = queueing_app();
    let txq_emitted = RefCell::new(Vec::new());
    let txq_stage = txq.apply_with_app_view_and_log_sinks_and_derived_hold_preflight(
        &mut lock,
        &mut txq_app,
        &view_source,
        &tx_source,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(6)),
        Ter::TES_SUCCESS,
        |message| txq_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| txq_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| txq_emitted.borrow_mut().push(format!("info:{message}")),
    );

    let lock_scope_emitted = lock_scope_emitted.into_inner();
    let txq_emitted = txq_emitted.into_inner();

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_eq!(txq_emitted, lock_scope_emitted);
    assert_eq!(txq_app.preflight_calls, 1);
    assert_eq!(txq_app.direct_apply_calls, 0);
    assert!(lock_scope_app.trace_messages.is_empty());
    assert!(txq_app.trace_messages.is_empty());
}

#[test]
fn queue_apply_txq_derived_hold_preflight_log_messages_match_lock_scope_owner() {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(6),
    };
    let view_source = queueing_view(&account);

    let mut lock_scope_owner = build_full_queue_lock_scope_owner();
    let mut lock = TestLockScope;
    let mut lock_scope_app = queueing_app();
    let lock_scope_stage = lock_scope_owner
        .apply_with_app_view_and_derived_hold_preflight_and_log_messages(
            &mut lock,
            &mut lock_scope_app,
            &view_source,
            &tx_source,
            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
            TxConsequences::new(1, SeqProxy::sequence(6)),
            Ter::TES_SUCCESS,
        );

    let mut txq = QueueApplyTxQ::new(build_full_queue_lock_scope_owner());
    let mut lock = TestLockScope;
    let mut txq_app = queueing_app();
    let txq_stage = txq.apply_with_app_view_and_derived_hold_preflight_and_log_messages(
        &mut lock,
        &mut txq_app,
        &view_source,
        &tx_source,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(6)),
        Ter::TES_SUCCESS,
    );

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_eq!(
        txq_stage.queue_log_messages,
        lock_scope_stage.queue_log_messages
    );
    assert_eq!(txq_app.preflight_calls, 1);
    assert_eq!(txq_app.direct_apply_calls, 0);
}

#[test]
fn queue_apply_txq_log_sinks_can_derive_preflight_facts_from_runtime() {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(6),
    };
    let view_source = queueing_view(&account);

    let mut lock_scope_owner = build_full_queue_lock_scope_owner();
    let mut lock = TestLockScope;
    let mut lock_scope_app = queueing_app();
    let lock_scope_emitted = RefCell::new(Vec::new());
    let lock_scope_stage = lock_scope_owner
        .apply_with_app_view_and_log_sinks_and_derived_preflight_facts(
            &mut lock,
            &mut lock_scope_app,
            &view_source,
            &tx_source,
            Ter::TES_SUCCESS,
            |message| {
                lock_scope_emitted
                    .borrow_mut()
                    .push(format!("trace:{message}"))
            },
            |message| {
                lock_scope_emitted
                    .borrow_mut()
                    .push(format!("debug:{message}"))
            },
            |message| {
                lock_scope_emitted
                    .borrow_mut()
                    .push(format!("info:{message}"))
            },
        );

    let mut txq = QueueApplyTxQ::new(build_full_queue_lock_scope_owner());
    let mut lock = TestLockScope;
    let mut txq_app = queueing_app();
    let txq_emitted = RefCell::new(Vec::new());
    let txq_stage = txq.apply_with_app_view_and_log_sinks_and_derived_preflight_facts(
        &mut lock,
        &mut txq_app,
        &view_source,
        &tx_source,
        Ter::TES_SUCCESS,
        |message| txq_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| txq_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| txq_emitted.borrow_mut().push(format!("info:{message}")),
    );

    let lock_scope_emitted = lock_scope_emitted.into_inner();
    let txq_emitted = txq_emitted.into_inner();

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_eq!(txq_emitted, lock_scope_emitted);
    assert_eq!(txq_app.preflight_calls, 1);
}

#[test]
fn queue_apply_txq_derived_preflight_facts_log_messages_match_lock_scope_owner() {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(6),
    };
    let view_source = queueing_view(&account);

    let mut lock_scope_owner = build_full_queue_lock_scope_owner();
    let mut lock = TestLockScope;
    let mut lock_scope_app = queueing_app();
    let lock_scope_stage = lock_scope_owner
        .apply_with_app_view_and_derived_preflight_facts_and_log_messages(
            &mut lock,
            &mut lock_scope_app,
            &view_source,
            &tx_source,
            Ter::TES_SUCCESS,
        );

    let mut txq = QueueApplyTxQ::new(build_full_queue_lock_scope_owner());
    let mut lock = TestLockScope;
    let mut txq_app = queueing_app();
    let txq_stage = txq.apply_with_app_view_and_derived_preflight_facts_and_log_messages(
        &mut lock,
        &mut txq_app,
        &view_source,
        &tx_source,
        Ter::TES_SUCCESS,
    );

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_eq!(
        txq_stage.queue_log_messages,
        lock_scope_stage.queue_log_messages
    );
    assert_eq!(txq_app.preflight_calls, 1);
    assert_eq!(txq_app.direct_apply_calls, 0);
}

#[test]
fn queue_apply_txq_log_sinks_derive_preflight_and_hold_admission_like_lock_scope_owner() {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(6),
    };
    let view_source = queueing_view(&account);

    let mut lock_scope_owner = build_full_queue_lock_scope_owner();
    let mut lock = TestLockScope;
    let mut lock_scope_app = queueing_app();
    let lock_scope_emitted = RefCell::new(Vec::new());
    let lock_scope_stage = lock_scope_owner
        .apply_with_app_view_and_log_sinks_and_derived_preflight_facts_and_hold_admission(
            &mut lock,
            &mut lock_scope_app,
            &view_source,
            &tx_source,
            |message| {
                lock_scope_emitted
                    .borrow_mut()
                    .push(format!("trace:{message}"))
            },
            |message| {
                lock_scope_emitted
                    .borrow_mut()
                    .push(format!("debug:{message}"))
            },
            |message| {
                lock_scope_emitted
                    .borrow_mut()
                    .push(format!("info:{message}"))
            },
        );

    let mut txq = QueueApplyTxQ::new(build_full_queue_lock_scope_owner());
    let mut lock = TestLockScope;
    let mut txq_app = queueing_app();
    let txq_emitted = RefCell::new(Vec::new());
    let txq_stage = txq
        .apply_with_app_view_and_log_sinks_and_derived_preflight_facts_and_hold_admission(
            &mut lock,
            &mut txq_app,
            &view_source,
            &tx_source,
            |message| txq_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| txq_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| txq_emitted.borrow_mut().push(format!("info:{message}")),
        );

    let lock_scope_emitted = lock_scope_emitted.into_inner();
    let txq_emitted = txq_emitted.into_inner();

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_eq!(txq_emitted, lock_scope_emitted);
    assert_eq!(txq_app.preflight_calls, 1);
    assert!(lock_scope_app.trace_messages.is_empty());
    assert!(txq_app.trace_messages.is_empty());
}

#[test]
fn queue_apply_txq_derived_preflight_and_hold_admission_log_messages_match_lock_scope_owner() {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(6),
    };
    let view_source = queueing_view(&account);

    let mut lock_scope_owner = build_full_queue_lock_scope_owner();
    let mut lock = TestLockScope;
    let mut lock_scope_app = queueing_app();
    let lock_scope_stage = lock_scope_owner
        .apply_with_app_view_and_derived_preflight_facts_and_hold_admission_and_log_messages(
            &mut lock,
            &mut lock_scope_app,
            &view_source,
            &tx_source,
        );

    let mut txq = QueueApplyTxQ::new(build_full_queue_lock_scope_owner());
    let mut lock = TestLockScope;
    let mut txq_app = queueing_app();
    let txq_stage = txq
        .apply_with_app_view_and_derived_preflight_facts_and_hold_admission_and_log_messages(
            &mut lock,
            &mut txq_app,
            &view_source,
            &tx_source,
        );

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_eq!(
        txq_stage.queue_log_messages,
        lock_scope_stage.queue_log_messages
    );
    assert_eq!(txq_app.preflight_calls, 1);
    assert_eq!(txq_app.direct_apply_calls, 0);
    assert!(lock_scope_app.trace_messages.is_empty());
    assert!(txq_app.trace_messages.is_empty());
}

#[test]
fn queue_apply_txq_after_preflight_log_sinks_derive_hold_preflight_like_lock_scope_owner() {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let view_source = direct_view(&account);
    let preflight = queueing_preflight_result(SeqProxy::sequence(5));

    let mut lock_scope_owner = build_direct_lock_scope_owner();
    let mut lock = TestLockScope;
    let mut lock_scope_app = direct_app();
    let lock_scope_emitted = RefCell::new(Vec::new());
    let lock_scope_stage = lock_scope_owner
        .apply_after_preflight_with_app_view_and_log_sinks_and_derived_hold_preflight(
            &mut lock,
            &mut lock_scope_app,
            &view_source,
            &tx_source,
            &preflight,
            Ter::TES_SUCCESS,
            |message| {
                lock_scope_emitted
                    .borrow_mut()
                    .push(format!("trace:{message}"))
            },
            |message| {
                lock_scope_emitted
                    .borrow_mut()
                    .push(format!("debug:{message}"))
            },
            |message| {
                lock_scope_emitted
                    .borrow_mut()
                    .push(format!("info:{message}"))
            },
        );

    let mut txq = QueueApplyTxQ::new(build_direct_lock_scope_owner());
    let mut lock = TestLockScope;
    let mut txq_app = direct_app();
    let txq_emitted = RefCell::new(Vec::new());
    let txq_stage = txq
        .apply_after_preflight_with_app_view_and_log_sinks_and_derived_hold_preflight(
            &mut lock,
            &mut txq_app,
            &view_source,
            &tx_source,
            &preflight,
            Ter::TES_SUCCESS,
            |message| txq_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| txq_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| txq_emitted.borrow_mut().push(format!("info:{message}")),
        );

    let lock_scope_emitted = lock_scope_emitted.into_inner();
    let txq_emitted = txq_emitted.into_inner();

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_eq!(txq_emitted, lock_scope_emitted);
    assert_eq!(txq_app.preflight_calls, 0);
    assert_eq!(txq_app.direct_apply_calls, 1);
    assert!(lock_scope_app.trace_messages.is_empty());
    assert!(txq_app.trace_messages.is_empty());
}

#[test]
fn queue_apply_txq_after_preflight_derived_hold_preflight_log_messages_match_lock_scope_owner() {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let view_source = direct_view(&account);
    let preflight = queueing_preflight_result(SeqProxy::sequence(5));

    let mut lock_scope_owner = build_direct_lock_scope_owner();
    let mut lock = TestLockScope;
    let mut lock_scope_app = direct_app();
    let lock_scope_stage = lock_scope_owner
        .apply_after_preflight_with_app_view_and_derived_hold_preflight_and_log_messages(
            &mut lock,
            &mut lock_scope_app,
            &view_source,
            &tx_source,
            &preflight,
            Ter::TES_SUCCESS,
        );

    let mut txq = QueueApplyTxQ::new(build_direct_lock_scope_owner());
    let mut lock = TestLockScope;
    let mut txq_app = direct_app();
    let txq_stage = txq
        .apply_after_preflight_with_app_view_and_derived_hold_preflight_and_log_messages(
            &mut lock,
            &mut txq_app,
            &view_source,
            &tx_source,
            &preflight,
            Ter::TES_SUCCESS,
        );

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert!(txq_stage.queue_log_messages.trace.is_empty());
    assert!(txq_stage.queue_log_messages.debug.is_empty());
    assert!(txq_stage.queue_log_messages.info.is_empty());
    assert_eq!(txq_app.preflight_calls, 0);
    assert_eq!(txq_app.direct_apply_calls, 1);
}

#[test]
fn queue_apply_txq_after_preflight_log_sinks_match_lock_scope_owner_on_direct_path() {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let view_source = direct_view(&account);
    let preflight = queueing_preflight_result(SeqProxy::sequence(5));

    let mut lock_scope_owner = build_direct_lock_scope_owner();
    let mut lock = TestLockScope;
    let mut lock_scope_app = direct_app();
    let lock_scope_emitted = RefCell::new(Vec::new());
    let lock_scope_stage = lock_scope_owner.apply_after_preflight_with_app_view_and_log_sinks(
        &mut lock,
        &mut lock_scope_app,
        &view_source,
        &tx_source,
        &preflight,
        hold_preflight(),
        Ter::TES_SUCCESS,
        |message| {
            lock_scope_emitted
                .borrow_mut()
                .push(format!("trace:{message}"))
        },
        |message| {
            lock_scope_emitted
                .borrow_mut()
                .push(format!("debug:{message}"))
        },
        |message| {
            lock_scope_emitted
                .borrow_mut()
                .push(format!("info:{message}"))
        },
    );

    let mut txq = QueueApplyTxQ::new(build_direct_lock_scope_owner());
    let mut lock = TestLockScope;
    let mut txq_app = direct_app();
    let txq_emitted = RefCell::new(Vec::new());
    let txq_stage = txq.apply_after_preflight_with_app_view_and_log_sinks(
        &mut lock,
        &mut txq_app,
        &view_source,
        &tx_source,
        &preflight,
        hold_preflight(),
        Ter::TES_SUCCESS,
        |message| txq_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| txq_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| txq_emitted.borrow_mut().push(format!("info:{message}")),
    );

    let lock_scope_emitted = lock_scope_emitted.into_inner();
    let txq_emitted = txq_emitted.into_inner();

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_eq!(txq_emitted, lock_scope_emitted);
    assert_eq!(
        txq_emitted,
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
    assert_eq!(txq_app.preflight_calls, 0);
    assert_eq!(txq_app.direct_apply_calls, 1);
    assert!(lock_scope_app.trace_messages.is_empty());
    assert!(txq_app.trace_messages.is_empty());
}

#[test]
fn queue_apply_txq_after_preflight_log_messages_match_lock_scope_owner_on_direct_path() {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let view_source = direct_view(&account);
    let preflight = queueing_preflight_result(SeqProxy::sequence(5));

    let mut lock_scope_owner = build_direct_lock_scope_owner();
    let mut lock = TestLockScope;
    let mut lock_scope_app = direct_app();
    let lock_scope_stage = lock_scope_owner.apply_after_preflight_with_log_messages(
        &mut lock,
        &mut lock_scope_app,
        &view_source,
        &tx_source,
        &preflight,
        hold_preflight(),
        Ter::TES_SUCCESS,
    );

    let mut txq = QueueApplyTxQ::new(build_direct_lock_scope_owner());
    let mut lock = TestLockScope;
    let mut txq_app = direct_app();
    let txq_stage = txq.apply_after_preflight_with_log_messages(
        &mut lock,
        &mut txq_app,
        &view_source,
        &tx_source,
        &preflight,
        hold_preflight(),
        Ter::TES_SUCCESS,
    );

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert!(txq_stage.queue_log_messages.trace.is_empty());
    assert!(txq_stage.queue_log_messages.info.is_empty());
    assert!(txq_stage.queue_log_messages.debug.is_empty());
    assert_eq!(lock_scope_app.preflight_calls, 0);
    assert_eq!(txq_app.preflight_calls, 0);
    assert_eq!(lock_scope_app.direct_apply_calls, 1);
    assert_eq!(txq_app.direct_apply_calls, 1);
    assert_eq!(txq_app.trace_messages, lock_scope_app.trace_messages);
}

#[test]
fn queue_apply_txq_after_preflight_log_sinks_derive_hold_admission_like_lock_scope_owner() {
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

    let mut lock_scope_owner = build_direct_lock_scope_owner();
    let mut lock = TestLockScope;
    let mut lock_scope_app = direct_app();
    let lock_scope_emitted = RefCell::new(Vec::new());
    let lock_scope_stage = lock_scope_owner
        .apply_after_preflight_with_app_view_and_log_sinks_and_derived_hold_admission(
            &mut lock,
            &mut lock_scope_app,
            &view_source,
            &tx_source,
            &preflight,
            hold_preflight,
            |message| {
                lock_scope_emitted
                    .borrow_mut()
                    .push(format!("trace:{message}"))
            },
            |message| {
                lock_scope_emitted
                    .borrow_mut()
                    .push(format!("debug:{message}"))
            },
            |message| {
                lock_scope_emitted
                    .borrow_mut()
                    .push(format!("info:{message}"))
            },
        );

    let mut txq = QueueApplyTxQ::new(build_direct_lock_scope_owner());
    let mut lock = TestLockScope;
    let mut txq_app = direct_app();
    let txq_emitted = RefCell::new(Vec::new());
    let txq_stage = txq
        .apply_after_preflight_with_app_view_and_log_sinks_and_derived_hold_admission(
            &mut lock,
            &mut txq_app,
            &view_source,
            &tx_source,
            &preflight,
            hold_preflight,
            |message| txq_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| txq_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| txq_emitted.borrow_mut().push(format!("info:{message}")),
        );

    let lock_scope_emitted = lock_scope_emitted.into_inner();
    let txq_emitted = txq_emitted.into_inner();

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_eq!(txq_emitted, lock_scope_emitted);
    assert_eq!(txq_app.preflight_calls, 0);
    assert_eq!(txq_app.direct_apply_calls, 1);
    assert!(lock_scope_app.trace_messages.is_empty());
    assert!(txq_app.trace_messages.is_empty());
}

#[test]
fn queue_apply_txq_after_preflight_derived_hold_admission_log_messages_match_lock_scope_owner() {
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

    let mut lock_scope_owner = build_direct_lock_scope_owner();
    let mut lock = TestLockScope;
    let mut lock_scope_app = direct_app();
    let lock_scope_stage = lock_scope_owner
        .apply_after_preflight_with_app_view_and_derived_hold_admission_and_log_messages(
            &mut lock,
            &mut lock_scope_app,
            &view_source,
            &tx_source,
            &preflight,
            hold_preflight,
        );

    let mut txq = QueueApplyTxQ::new(build_direct_lock_scope_owner());
    let mut lock = TestLockScope;
    let mut txq_app = direct_app();
    let txq_stage = txq
        .apply_after_preflight_with_app_view_and_derived_hold_admission_and_log_messages(
            &mut lock,
            &mut txq_app,
            &view_source,
            &tx_source,
            &preflight,
            hold_preflight,
        );

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert!(txq_stage.queue_log_messages.trace.is_empty());
    assert!(txq_stage.queue_log_messages.debug.is_empty());
    assert!(txq_stage.queue_log_messages.info.is_empty());
    assert_eq!(txq_app.preflight_calls, 0);
    assert_eq!(txq_app.direct_apply_calls, 1);
}

#[test]
fn queue_apply_txq_owned_metrics_log_sinks_derive_hold_admission_match_lock_scope_owner() {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let view_source = divergent_owner_metrics_view(&account);
    let hold_preflight = QueueHoldPreflight::new(
        false,
        false,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        Some(250),
    );

    let mut lock_scope_owner = build_direct_lock_scope_owner();
    let mut lock = TestLockScope;
    let mut lock_scope_app = direct_app();
    let lock_scope_emitted = RefCell::new(Vec::new());
    let lock_scope_stage = lock_scope_owner
        .apply_with_app_view_and_owned_metrics_and_log_sinks_and_derived_hold_admission(
            &mut lock,
            &mut lock_scope_app,
            &view_source,
            &tx_source,
            hold_preflight,
            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
            TxConsequences::new(1, SeqProxy::sequence(5)),
            |message| {
                lock_scope_emitted
                    .borrow_mut()
                    .push(format!("trace:{message}"))
            },
            |message| {
                lock_scope_emitted
                    .borrow_mut()
                    .push(format!("debug:{message}"))
            },
            |message| {
                lock_scope_emitted
                    .borrow_mut()
                    .push(format!("info:{message}"))
            },
        );

    let mut plain_txq = QueueApplyTxQ::new(build_direct_lock_scope_owner());
    let mut plain_lock = TestLockScope;
    let mut plain_app = direct_app();
    let plain_stage = plain_txq.apply_with_app_view_and_log_sinks_and_derived_hold_admission(
        &mut plain_lock,
        &mut plain_app,
        &view_source,
        &tx_source,
        hold_preflight,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(5)),
        |_message| {},
        |_message| {},
        |_message| {},
    );

    let mut txq = QueueApplyTxQ::new(build_direct_lock_scope_owner());
    let mut lock = TestLockScope;
    let mut txq_app = direct_app();
    let txq_emitted = RefCell::new(Vec::new());
    let txq_stage = txq
        .apply_with_app_view_and_owned_metrics_and_log_sinks_and_derived_hold_admission(
            &mut lock,
            &mut txq_app,
            &view_source,
            &tx_source,
            hold_preflight,
            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
            TxConsequences::new(1, SeqProxy::sequence(5)),
            |message| txq_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| txq_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| txq_emitted.borrow_mut().push(format!("info:{message}")),
        );

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_eq!(txq_emitted.into_inner(), lock_scope_emitted.into_inner());
    assert_ne!(plain_stage, txq_stage);
    assert_eq!(plain_app.direct_apply_calls, 1);
    assert_eq!(txq_app.direct_apply_calls, 0);
}

#[test]
fn queue_apply_txq_owned_metrics_derived_hold_admission_log_messages_match_lock_scope_owner() {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let view_source = divergent_owner_metrics_view(&account);
    let hold_preflight = QueueHoldPreflight::new(
        false,
        false,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        Some(250),
    );

    let mut lock_scope_owner = build_direct_lock_scope_owner();
    let mut lock = TestLockScope;
    let mut lock_scope_app = direct_app();
    let lock_scope_stage = lock_scope_owner
        .apply_with_app_view_and_owned_metrics_and_derived_hold_admission_and_log_messages(
            &mut lock,
            &mut lock_scope_app,
            &view_source,
            &tx_source,
            hold_preflight,
            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
            TxConsequences::new(1, SeqProxy::sequence(5)),
        );

    let mut plain_txq = QueueApplyTxQ::new(build_direct_lock_scope_owner());
    let mut plain_lock = TestLockScope;
    let mut plain_app = direct_app();
    let plain_stage = plain_txq.apply_with_app_view_and_derived_hold_admission_and_log_messages(
        &mut plain_lock,
        &mut plain_app,
        &view_source,
        &tx_source,
        hold_preflight,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(5)),
    );

    let mut txq = QueueApplyTxQ::new(build_direct_lock_scope_owner());
    let mut lock = TestLockScope;
    let mut txq_app = direct_app();
    let txq_stage = txq
        .apply_with_app_view_and_owned_metrics_and_derived_hold_admission_and_log_messages(
            &mut lock,
            &mut txq_app,
            &view_source,
            &tx_source,
            hold_preflight,
            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
            TxConsequences::new(1, SeqProxy::sequence(5)),
        );

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_ne!(plain_stage, txq_stage);
    assert_eq!(
        txq_stage.queue_log_messages,
        lock_scope_stage.queue_log_messages
    );
    assert_eq!(plain_app.direct_apply_calls, 1);
    assert_eq!(txq_app.direct_apply_calls, 0);
}

#[test]
fn queue_apply_txq_owned_metrics_log_sinks_derive_hold_preflight_match_lock_scope_owner() {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let view_source = divergent_owner_metrics_view(&account);

    let mut lock_scope_owner = build_direct_lock_scope_owner();
    let mut lock = TestLockScope;
    let mut lock_scope_app = direct_app();
    let lock_scope_emitted = RefCell::new(Vec::new());
    let lock_scope_stage = lock_scope_owner
        .apply_with_app_view_and_owned_metrics_and_log_sinks_and_derived_hold_preflight(
            &mut lock,
            &mut lock_scope_app,
            &view_source,
            &tx_source,
            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
            TxConsequences::new(1, SeqProxy::sequence(5)),
            Ter::TES_SUCCESS,
            |message| {
                lock_scope_emitted
                    .borrow_mut()
                    .push(format!("trace:{message}"))
            },
            |message| {
                lock_scope_emitted
                    .borrow_mut()
                    .push(format!("debug:{message}"))
            },
            |message| {
                lock_scope_emitted
                    .borrow_mut()
                    .push(format!("info:{message}"))
            },
        );

    let mut plain_txq = QueueApplyTxQ::new(build_direct_lock_scope_owner());
    let mut plain_lock = TestLockScope;
    let mut plain_app = direct_app();
    let plain_stage = plain_txq.apply_with_app_view_and_log_sinks_and_derived_hold_preflight(
        &mut plain_lock,
        &mut plain_app,
        &view_source,
        &tx_source,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(5)),
        Ter::TES_SUCCESS,
        |_message| {},
        |_message| {},
        |_message| {},
    );

    let mut txq = QueueApplyTxQ::new(build_direct_lock_scope_owner());
    let mut lock = TestLockScope;
    let mut txq_app = direct_app();
    let txq_emitted = RefCell::new(Vec::new());
    let txq_stage = txq
        .apply_with_app_view_and_owned_metrics_and_log_sinks_and_derived_hold_preflight(
            &mut lock,
            &mut txq_app,
            &view_source,
            &tx_source,
            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
            TxConsequences::new(1, SeqProxy::sequence(5)),
            Ter::TES_SUCCESS,
            |message| txq_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| txq_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| txq_emitted.borrow_mut().push(format!("info:{message}")),
        );

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_eq!(txq_emitted.into_inner(), lock_scope_emitted.into_inner());
    assert_ne!(plain_stage, txq_stage);
    assert_eq!(plain_app.direct_apply_calls, 1);
    assert_eq!(txq_app.direct_apply_calls, 0);
}

#[test]
fn queue_apply_txq_owned_metrics_derived_hold_preflight_log_messages_match_lock_scope_owner() {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let view_source = divergent_owner_metrics_view(&account);

    let mut lock_scope_owner = build_direct_lock_scope_owner();
    let mut lock = TestLockScope;
    let mut lock_scope_app = direct_app();
    let lock_scope_stage = lock_scope_owner
        .apply_with_app_view_and_owned_metrics_and_derived_hold_preflight_and_log_messages(
            &mut lock,
            &mut lock_scope_app,
            &view_source,
            &tx_source,
            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
            TxConsequences::new(1, SeqProxy::sequence(5)),
            Ter::TES_SUCCESS,
        );

    let mut plain_txq = QueueApplyTxQ::new(build_direct_lock_scope_owner());
    let mut plain_lock = TestLockScope;
    let mut plain_app = direct_app();
    let plain_stage = plain_txq.apply_with_app_view_and_derived_hold_preflight_and_log_messages(
        &mut plain_lock,
        &mut plain_app,
        &view_source,
        &tx_source,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        TxConsequences::new(1, SeqProxy::sequence(5)),
        Ter::TES_SUCCESS,
    );

    let mut txq = QueueApplyTxQ::new(build_direct_lock_scope_owner());
    let mut lock = TestLockScope;
    let mut txq_app = direct_app();
    let txq_stage = txq
        .apply_with_app_view_and_owned_metrics_and_derived_hold_preflight_and_log_messages(
            &mut lock,
            &mut txq_app,
            &view_source,
            &tx_source,
            ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
            TxConsequences::new(1, SeqProxy::sequence(5)),
            Ter::TES_SUCCESS,
        );

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_ne!(plain_stage, txq_stage);
    assert_eq!(
        txq_stage.queue_log_messages,
        lock_scope_stage.queue_log_messages
    );
    assert_eq!(plain_app.direct_apply_calls, 1);
    assert_eq!(txq_app.direct_apply_calls, 0);
}

#[test]
fn queue_apply_txq_owned_metrics_log_sinks_derive_preflight_facts_match_lock_scope_owner() {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let view_source = divergent_owner_metrics_view(&account);

    let mut lock_scope_owner = build_direct_lock_scope_owner();
    let mut lock = TestLockScope;
    let mut lock_scope_app = direct_app();
    let lock_scope_emitted = RefCell::new(Vec::new());
    let lock_scope_stage = lock_scope_owner
        .apply_with_app_view_and_owned_metrics_and_log_sinks_and_derived_preflight_facts(
            &mut lock,
            &mut lock_scope_app,
            &view_source,
            &tx_source,
            |message| {
                lock_scope_emitted
                    .borrow_mut()
                    .push(format!("trace:{message}"))
            },
            |message| {
                lock_scope_emitted
                    .borrow_mut()
                    .push(format!("debug:{message}"))
            },
            |message| {
                lock_scope_emitted
                    .borrow_mut()
                    .push(format!("info:{message}"))
            },
        );

    let mut plain_txq = QueueApplyTxQ::new(build_direct_lock_scope_owner());
    let mut plain_lock = TestLockScope;
    let mut plain_app = direct_app();
    let plain_stage = plain_txq.apply_with_app_view_and_log_sinks_and_derived_preflight_facts(
        &mut plain_lock,
        &mut plain_app,
        &view_source,
        &tx_source,
        Ter::TES_SUCCESS,
        |_message| {},
        |_message| {},
        |_message| {},
    );

    let mut txq = QueueApplyTxQ::new(build_direct_lock_scope_owner());
    let mut lock = TestLockScope;
    let mut txq_app = direct_app();
    let txq_emitted = RefCell::new(Vec::new());
    let txq_stage = txq
        .apply_with_app_view_and_owned_metrics_and_log_sinks_and_derived_preflight_facts(
            &mut lock,
            &mut txq_app,
            &view_source,
            &tx_source,
            |message| txq_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| txq_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| txq_emitted.borrow_mut().push(format!("info:{message}")),
        );

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_eq!(txq_emitted.into_inner(), lock_scope_emitted.into_inner());
    assert_ne!(plain_stage, txq_stage);
    assert_eq!(plain_app.direct_apply_calls, 1);
    assert_eq!(txq_app.direct_apply_calls, 0);
}

#[test]
fn queue_apply_txq_owned_metrics_derived_preflight_facts_log_messages_match_lock_scope_owner() {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let view_source = divergent_owner_metrics_view(&account);

    let mut lock_scope_owner = build_direct_lock_scope_owner();
    let mut lock = TestLockScope;
    let mut lock_scope_app = direct_app();
    let lock_scope_stage = lock_scope_owner
        .apply_with_app_view_and_owned_metrics_and_derived_preflight_facts_and_log_messages(
            &mut lock,
            &mut lock_scope_app,
            &view_source,
            &tx_source,
        );

    let mut plain_txq = QueueApplyTxQ::new(build_direct_lock_scope_owner());
    let mut plain_lock = TestLockScope;
    let mut plain_app = direct_app();
    let plain_stage = plain_txq.apply_with_app_view_and_derived_preflight_facts_and_log_messages(
        &mut plain_lock,
        &mut plain_app,
        &view_source,
        &tx_source,
        Ter::TES_SUCCESS,
    );

    let mut txq = QueueApplyTxQ::new(build_direct_lock_scope_owner());
    let mut lock = TestLockScope;
    let mut txq_app = direct_app();
    let txq_stage = txq
        .apply_with_app_view_and_owned_metrics_and_derived_preflight_facts_and_log_messages(
            &mut lock,
            &mut txq_app,
            &view_source,
            &tx_source,
        );

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_ne!(plain_stage, txq_stage);
    assert_eq!(
        txq_stage.queue_log_messages,
        lock_scope_stage.queue_log_messages
    );
    assert_eq!(plain_app.direct_apply_calls, 1);
    assert_eq!(txq_app.direct_apply_calls, 0);
}

#[test]
fn queue_apply_txq_owned_metrics_log_sinks_derive_preflight_and_hold_admission_match_lock_scope_owner()
 {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let view_source = divergent_owner_metrics_view(&account);

    let mut lock_scope_owner = build_direct_lock_scope_owner();
    let mut lock = TestLockScope;
    let mut lock_scope_app = direct_app();
    let lock_scope_emitted = RefCell::new(Vec::new());
    let lock_scope_stage = lock_scope_owner
        .apply_with_app_view_and_owned_metrics_and_log_sinks_and_derived_preflight_facts_and_hold_admission(
            &mut lock,
            &mut lock_scope_app,
            &view_source,
            &tx_source,
            |message| lock_scope_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| lock_scope_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| lock_scope_emitted.borrow_mut().push(format!("info:{message}")),
        );

    let mut plain_txq = QueueApplyTxQ::new(build_direct_lock_scope_owner());
    let mut plain_lock = TestLockScope;
    let mut plain_app = direct_app();
    let plain_stage = plain_txq
        .apply_with_app_view_and_log_sinks_and_derived_preflight_facts_and_hold_admission(
            &mut plain_lock,
            &mut plain_app,
            &view_source,
            &tx_source,
            |_message| {},
            |_message| {},
            |_message| {},
        );

    let mut txq = QueueApplyTxQ::new(build_direct_lock_scope_owner());
    let mut lock = TestLockScope;
    let mut txq_app = direct_app();
    let txq_emitted = RefCell::new(Vec::new());
    let txq_stage = txq
        .apply_with_app_view_and_owned_metrics_and_log_sinks_and_derived_preflight_facts_and_hold_admission(
            &mut lock,
            &mut txq_app,
            &view_source,
            &tx_source,
            |message| txq_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| txq_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| txq_emitted.borrow_mut().push(format!("info:{message}")),
        );

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_eq!(txq_emitted.into_inner(), lock_scope_emitted.into_inner());
    assert_ne!(plain_stage, txq_stage);
    assert_eq!(plain_app.direct_apply_calls, 1);
    assert_eq!(txq_app.direct_apply_calls, 0);
}

#[test]
fn queue_apply_txq_owned_metrics_derived_preflight_and_hold_admission_log_messages_match_lock_scope_owner()
 {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let view_source = divergent_owner_metrics_view(&account);

    let mut lock_scope_owner = build_direct_lock_scope_owner();
    let mut lock = TestLockScope;
    let mut lock_scope_app = direct_app();
    let lock_scope_stage = lock_scope_owner
        .apply_with_app_view_and_owned_metrics_and_derived_preflight_facts_and_hold_admission_and_log_messages(
            &mut lock,
            &mut lock_scope_app,
            &view_source,
            &tx_source,
        );

    let mut plain_txq = QueueApplyTxQ::new(build_direct_lock_scope_owner());
    let mut plain_lock = TestLockScope;
    let mut plain_app = direct_app();
    let plain_stage = plain_txq
        .apply_with_app_view_and_derived_preflight_facts_and_hold_admission_and_log_messages(
            &mut plain_lock,
            &mut plain_app,
            &view_source,
            &tx_source,
        );

    let mut txq = QueueApplyTxQ::new(build_direct_lock_scope_owner());
    let mut lock = TestLockScope;
    let mut txq_app = direct_app();
    let txq_stage = txq
        .apply_with_app_view_and_owned_metrics_and_derived_preflight_facts_and_hold_admission_and_log_messages(
            &mut lock,
            &mut txq_app,
            &view_source,
            &tx_source,
        );

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_ne!(plain_stage, txq_stage);
    assert_eq!(
        txq_stage.queue_log_messages,
        lock_scope_stage.queue_log_messages
    );
    assert_eq!(plain_app.direct_apply_calls, 1);
    assert_eq!(txq_app.direct_apply_calls, 0);
}

#[test]
fn queue_apply_txq_after_preflight_owned_metrics_log_sinks_derive_hold_preflight_match_lock_scope_owner()
 {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let view_source = divergent_owner_metrics_view(&account);
    let preflight = preflight_result(SeqProxy::sequence(5));

    let mut lock_scope_owner = build_direct_lock_scope_owner();
    let mut lock = TestLockScope;
    let mut lock_scope_app = direct_app();
    let lock_scope_emitted = RefCell::new(Vec::new());
    let lock_scope_stage = lock_scope_owner
        .apply_after_preflight_with_app_view_and_owned_metrics_and_log_sinks_and_derived_hold_preflight(
            &mut lock,
            &mut lock_scope_app,
            &view_source,
            &tx_source,
            &preflight,
            Ter::TES_SUCCESS,
            |message| lock_scope_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| lock_scope_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| lock_scope_emitted.borrow_mut().push(format!("info:{message}")),
        );

    let mut plain_txq = QueueApplyTxQ::new(build_direct_lock_scope_owner());
    let mut plain_lock = TestLockScope;
    let mut plain_app = direct_app();
    let plain_stage = plain_txq
        .apply_after_preflight_with_app_view_and_log_sinks_and_derived_hold_preflight(
            &mut plain_lock,
            &mut plain_app,
            &view_source,
            &tx_source,
            &preflight,
            Ter::TES_SUCCESS,
            |_message| {},
            |_message| {},
            |_message| {},
        );

    let mut txq = QueueApplyTxQ::new(build_direct_lock_scope_owner());
    let mut lock = TestLockScope;
    let mut txq_app = direct_app();
    let txq_emitted = RefCell::new(Vec::new());
    let txq_stage = txq
        .apply_after_preflight_with_app_view_and_owned_metrics_and_log_sinks_and_derived_hold_preflight(
            &mut lock,
            &mut txq_app,
            &view_source,
            &tx_source,
            &preflight,
            Ter::TES_SUCCESS,
            |message| txq_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| txq_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| txq_emitted.borrow_mut().push(format!("info:{message}")),
        );

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_eq!(txq_emitted.into_inner(), lock_scope_emitted.into_inner());
    assert_ne!(plain_stage, txq_stage);
    assert_eq!(plain_app.direct_apply_calls, 1);
    assert_eq!(txq_app.direct_apply_calls, 0);
}

#[test]
fn queue_apply_txq_after_preflight_owned_metrics_log_sinks_derive_hold_admission_match_lock_scope_owner()
 {
    let account = String::from("a");
    let tx_source = TestObservedTxSource {
        account: &account,
        transaction_id: "ABC123",
        tx_id: Uint256::from_u64(9),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let view_source = divergent_owner_metrics_view(&account);
    let preflight = preflight_result(SeqProxy::sequence(5));
    let hold_preflight = QueueHoldPreflight::new(
        false,
        false,
        ApplyFlags::FAIL_HARD | ApplyFlags::RETRY,
        Some(250),
    );

    let mut lock_scope_owner = build_direct_lock_scope_owner();
    let mut lock = TestLockScope;
    let mut lock_scope_app = direct_app();
    let lock_scope_emitted = RefCell::new(Vec::new());
    let lock_scope_stage = lock_scope_owner
        .apply_after_preflight_with_app_view_and_owned_metrics_and_log_sinks_and_derived_hold_admission(
            &mut lock,
            &mut lock_scope_app,
            &view_source,
            &tx_source,
            &preflight,
            hold_preflight,
            |message| lock_scope_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| lock_scope_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| lock_scope_emitted.borrow_mut().push(format!("info:{message}")),
        );

    let mut plain_txq = QueueApplyTxQ::new(build_direct_lock_scope_owner());
    let mut plain_lock = TestLockScope;
    let mut plain_app = direct_app();
    let plain_stage = plain_txq
        .apply_after_preflight_with_app_view_and_log_sinks_and_derived_hold_admission(
            &mut plain_lock,
            &mut plain_app,
            &view_source,
            &tx_source,
            &preflight,
            hold_preflight,
            |_message| {},
            |_message| {},
            |_message| {},
        );

    let mut txq = QueueApplyTxQ::new(build_direct_lock_scope_owner());
    let mut lock = TestLockScope;
    let mut txq_app = direct_app();
    let txq_emitted = RefCell::new(Vec::new());
    let txq_stage = txq
        .apply_after_preflight_with_app_view_and_owned_metrics_and_log_sinks_and_derived_hold_admission(
            &mut lock,
            &mut txq_app,
            &view_source,
            &tx_source,
            &preflight,
            hold_preflight,
            |message| txq_emitted.borrow_mut().push(format!("trace:{message}")),
            |message| txq_emitted.borrow_mut().push(format!("debug:{message}")),
            |message| txq_emitted.borrow_mut().push(format!("info:{message}")),
        );

    assert_eq!(txq_stage, lock_scope_stage);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_eq!(txq_emitted.into_inner(), lock_scope_emitted.into_inner());
    assert_ne!(plain_stage, txq_stage);
    assert_eq!(plain_app.direct_apply_calls, 1);
    assert_eq!(txq_app.direct_apply_calls, 0);
}
