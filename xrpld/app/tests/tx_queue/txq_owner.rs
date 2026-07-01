use std::collections::BTreeMap;

use app::TxQ;
use basics::base_uint::Uint256;
use protocol::{Rules, SeqProxy, Ter};
use tx::{
    ApplyFlags, ApplyResult, FeeQueueEntry, FeeQueueKey, MaybeTx, MaybeTxCore, OrderCandidates,
    PreclaimResult, PreflightResult, QueueAcceptJournalOwner, QueueAcceptJournalSink,
    QueueAcceptLedgerViewSource, QueueAcceptLiveApplyRuntime, QueueAcceptLiveOwner,
    QueueAcceptLockScope, QueueAcceptLockScopeOwner, QueueAcceptOwnerShell, QueueAcceptOwnerState,
    QueueAcceptTxQ, QueueAdvanceCandidate, QueueApplyCurrentPreclaimClearRuntime,
    QueueApplyExecutionRuntime, QueueApplyLockScope, QueueApplyObservedAccountLookup,
    QueueApplyObservedTicketLookup, QueueApplyObservedTxSource, QueueApplyObservedViewSource,
    QueueApplyTxQ, QueueFeeMetricsSnapshot, QueueHoldPreflight, QueueTxQAccountState,
    QueueTxQClosedLedgerAppSource, QueueTxQClosedLedgerView, QueueTxQRequiredFeeAndSeq,
    QueueTxQRequiredFeeTxSource, QueueTxQRequiredFeeViewSource, QueueTxQRpcAppSource,
    QueueTxQRpcReport, QueueTxQRpcView, QueueViews, TryClearAccountResult, TxConsequences,
    TxQAccount, TxQSetup,
};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct TestJournal;

impl QueueAcceptJournalSink for TestJournal {
    fn trace(&mut self, _message: &str) {}

    fn debug(&mut self, _message: &str) {}

    fn info(&mut self, _message: &str) {}

    fn warn(&mut self, _message: &str) {}
}

#[derive(Debug, Default)]
struct TestAcceptLock;

impl QueueAcceptLockScope for TestAcceptLock {}

#[derive(Debug, Default)]
struct TestApplyLock;

impl QueueApplyLockScope for TestApplyLock {}

#[derive(Debug, Clone)]
struct TestAcceptApp {
    apply_result: ApplyResult,
    apply_calls: usize,
}

impl QueueAcceptLiveApplyRuntime<&'static str, &'static str, &'static str, &'static str>
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

#[derive(Debug, Clone, Copy)]
struct TestAcceptView {
    open_ledger_tx_count: usize,
    parent_hash: Uint256,
}

impl QueueAcceptLedgerViewSource for TestAcceptView {
    fn open_ledger_tx_count(&self) -> usize {
        self.open_ledger_tx_count
    }

    fn parent_hash(&self) -> Uint256 {
        self.parent_hash
    }
}

#[derive(Debug, Clone, Copy)]
struct TestRequiredFeeView {
    open_ledger_tx_count: usize,
    base_fee_drops: u64,
    account_sequence: Option<u32>,
}

#[derive(Debug, Clone, Copy)]
struct TestRequiredFeeTx {
    account: &'static str,
}

#[derive(Debug, Clone, Copy)]
struct TestRpcView {
    ledger_current_index: u32,
    open_ledger_tx_count: usize,
    base_fee_drops: u64,
}

#[derive(Debug, Clone, Copy)]
struct TestRpcApp {
    current_view: Option<TestRpcView>,
}

#[derive(Debug, Clone, Copy)]
struct TestClosedLedgerView {
    ledger_seq: u32,
}

impl QueueTxQClosedLedgerView for TestClosedLedgerView {
    fn ledger_seq(&self) -> u32 {
        self.ledger_seq
    }
}

impl QueueTxQRequiredFeeViewSource<&'static str, TestRequiredFeeTx> for TestRequiredFeeView {
    fn open_ledger_tx_count(&self) -> usize {
        self.open_ledger_tx_count
    }

    fn calculate_base_fee_drops(&self, _tx: &TestRequiredFeeTx) -> u64 {
        self.base_fee_drops
    }

    fn account_sequence(&self, _account: &&'static str) -> Option<u32> {
        self.account_sequence
    }
}

impl QueueTxQRequiredFeeTxSource<&'static str> for TestRequiredFeeTx {
    fn account(&self) -> &&'static str {
        &self.account
    }
}

impl QueueTxQRpcView for TestRpcView {
    fn ledger_current_index(&self) -> u32 {
        self.ledger_current_index
    }

    fn open_ledger_tx_count(&self) -> usize {
        self.open_ledger_tx_count
    }

    fn base_fee_drops(&self) -> u64 {
        self.base_fee_drops
    }
}

impl QueueTxQRpcAppSource for TestRpcApp {
    type View = TestRpcView;

    fn current_rpc_view(&self) -> Option<Self::View> {
        self.current_view
    }
}

#[derive(Debug, Clone)]
struct TestClosedLedgerApp {
    validated_fee_levels: Vec<u64>,
}

impl QueueTxQClosedLedgerAppSource<TestClosedLedgerView> for TestClosedLedgerApp {
    fn validated_fee_levels(&self, _view: &TestClosedLedgerView) -> Vec<u64> {
        self.validated_fee_levels.clone()
    }
}

#[derive(Debug, Clone)]
struct TestApplyApp {
    preflight_result: PreflightResult<&'static str, TxConsequences, &'static str, &'static str>,
    apply_result: ApplyResult,
    preflight_calls: usize,
    direct_apply_calls: usize,
    prepare_multitxn_calls: usize,
    preclaim_calls: usize,
    try_clear_calls: usize,
    apply_sandbox_calls: usize,
    trace_messages: Vec<String>,
}

impl QueueApplyExecutionRuntime<&'static str, &'static str, &'static str> for TestApplyApp {
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

    fn prepare_multitxn(&mut self, _adjustment: tx::QueueApplyViewAdjustment) -> bool {
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
    fn run_try_clear_with_current_preclaim(&mut self) -> TryClearAccountResult {
        TryClearAccountResult::ClearQueue {
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
}

#[derive(Debug, Clone)]
struct TestApplyTxSource {
    account: &'static str,
    transaction_id: &'static str,
    tx_id: Uint256,
    tx_seq_proxy: SeqProxy,
}

impl QueueApplyObservedTxSource for TestApplyTxSource {
    type Account = &'static str;
    type TransactionId = &'static str;

    fn account(&self) -> &Self::Account {
        &self.account
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

impl tx::QueueApplyHoldPreflightTxSource for TestApplyTxSource {
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

#[derive(Debug, Clone)]
struct TestApplyView {
    account: &'static str,
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

impl QueueApplyObservedViewSource<&'static str> for TestApplyView {
    fn rules(&self) -> &Rules {
        &self.rules
    }

    fn account_lookup(&self, account: &&'static str) -> QueueApplyObservedAccountLookup {
        if account == &self.account {
            self.account_lookup
        } else {
            QueueApplyObservedAccountLookup::Missing
        }
    }

    fn ticket_lookup(
        &self,
        account: &&'static str,
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

fn rules() -> Rules {
    Rules::new(std::iter::empty())
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
            rules(),
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

fn build_accept_txq()
-> QueueAcceptTxQ<&'static str, &'static str, &'static str, &'static str, TestJournal> {
    QueueAcceptTxQ::new(QueueAcceptLockScopeOwner::new(
        QueueAcceptJournalOwner::new(
            QueueAcceptOwnerShell::new(QueueAcceptLiveOwner::new_from_setup(
                &test_setup(),
                Some(2),
                QueueAcceptOwnerState::new(Uint256::from_u64(9)),
                build_views(),
            )),
            TestJournal,
        ),
    ))
}

fn build_app_txq() -> TxQ<&'static str, &'static str, &'static str, &'static str> {
    TxQ::new_from_setup(
        test_setup(),
        Some(2),
        QueueAcceptOwnerState::new(Uint256::from_u64(9)),
        build_views(),
    )
}

fn build_apply_txq() -> QueueApplyTxQ<&'static str, &'static str, &'static str, &'static str> {
    QueueApplyTxQ::new(tx::QueueApplyLockScopeOwner::new(
        tx::QueueApplyOwnerShell::new(tx::QueueApplyLiveOwner::new_from_setup(
            &test_setup(),
            Some(10),
            OrderCandidates::new(Uint256::from_u64(9)),
            build_views(),
        )),
    ))
}

fn build_apply_view() -> TestApplyView {
    TestApplyView {
        account: "acct",
        rules: rules(),
        account_lookup: QueueApplyObservedAccountLookup::Present {
            balance_drops: 2_000_000,
            sequence: 5,
        },
        ticket_lookup: QueueApplyObservedTicketLookup::Missing,
        calculated_base_fee_drops: 10,
        fee_paid_drops: 100,
        default_base_fee_drops: 10,
        metrics_snapshot: QueueFeeMetricsSnapshot {
            txns_expected: 32,
            escalation_multiplier: tx::TXQ_BASE_LEVEL * 500,
        },
        open_ledger_tx_count: 5,
        open_ledger_seq: 200,
        reserve_drops: 10,
        base_fee_drops: 10,
    }
}

fn build_apply_runtime() -> TestApplyApp {
    TestApplyApp {
        preflight_result: PreflightResult::new(
            "tx",
            None::<&str>,
            rules(),
            TxConsequences::new(1, SeqProxy::sequence(5)),
            ApplyFlags::FAIL_HARD,
            "journal",
            Ter::TES_SUCCESS,
        ),
        apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, true),
        preflight_calls: 0,
        direct_apply_calls: 0,
        prepare_multitxn_calls: 0,
        preclaim_calls: 0,
        try_clear_calls: 0,
        apply_sandbox_calls: 0,
        trace_messages: Vec::new(),
    }
}

#[test]
fn txq_owner_accept_matches_landed_txq_facade() {
    let view = TestAcceptView {
        open_ledger_tx_count: 32,
        parent_hash: Uint256::from_u64(9),
    };
    let mut app_runtime = TestAcceptApp {
        apply_result: ApplyResult::new(Ter::TER_RETRY, false, false),
        apply_calls: 0,
    };
    let mut tx_runtime = app_runtime.clone();

    let mut app_txq = build_app_txq();
    let mut accept_txq = build_accept_txq();
    let mut app_lock = TestAcceptLock;
    let mut tx_lock = TestAcceptLock;

    let app_result = app_txq.accept(&mut app_lock, &mut app_runtime, &view);
    let tx_result = accept_txq.accept(&mut tx_lock, &mut tx_runtime, &view);

    assert_eq!(app_result, tx_result);
    assert_eq!(app_runtime.apply_calls, 1);
    assert_eq!(tx_runtime.apply_calls, 1);
}

#[test]
fn txq_owner_process_closed_ledger_matches_landed_accept_owner() {
    let app = TestClosedLedgerApp {
        validated_fee_levels: vec![200, 300, 500],
    };
    let view = TestClosedLedgerView { ledger_seq: 20 };
    let mut app_txq = build_app_txq();
    let mut accept_txq = build_accept_txq();
    let mut app_lock = TestAcceptLock;
    let mut tx_lock = TestAcceptLock;

    let app_result = app_txq.process_closed_ledger(&mut app_lock, &app, &view, false);
    let tx_result = accept_txq.process_closed_ledger(&mut tx_lock, &app, &view, false);

    assert_eq!(app_result, tx_result);
    assert_eq!(
        app_txq.views(),
        accept_txq
            .lock_scope_owner()
            .owner()
            .owner()
            .owner()
            .views()
    );
}

#[test]
fn txq_owner_next_queuable_seq_matches_landed_accept_owner() {
    let app_txq = build_app_txq();
    let accept_txq = build_accept_txq();
    let account_state = QueueTxQAccountState::Present {
        account: &"acct",
        seq_proxy: SeqProxy::sequence(5),
    };
    let mut app_lock = TestAcceptLock;
    let mut tx_lock = TestAcceptLock;

    let app_result = app_txq.next_queuable_seq(&mut app_lock, account_state);
    let tx_result = accept_txq.next_queuable_seq(&mut tx_lock, account_state);

    assert_eq!(app_result, tx_result);
}

#[test]
fn txq_owner_get_tx_required_fee_and_seq_matches_landed_accept_owner() {
    let app_txq = build_app_txq();
    let accept_txq = build_accept_txq();
    let view = TestRequiredFeeView {
        open_ledger_tx_count: 33,
        base_fee_drops: 10,
        account_sequence: Some(5),
    };
    let tx = TestRequiredFeeTx { account: "acct" };
    let mut app_lock = TestAcceptLock;
    let mut tx_lock = TestAcceptLock;

    let app_result: QueueTxQRequiredFeeAndSeq =
        app_txq.get_tx_required_fee_and_seq(&mut app_lock, &view, &tx);
    let tx_result: QueueTxQRequiredFeeAndSeq =
        accept_txq.get_tx_required_fee_and_seq(&mut tx_lock, &view, &tx);

    assert_eq!(app_result, tx_result);
}

#[test]
fn txq_owner_do_rpc_matches_landed_accept_owner() {
    let app_txq = build_app_txq();
    let accept_txq = build_accept_txq();
    let app = TestRpcApp {
        current_view: Some(TestRpcView {
            ledger_current_index: 99,
            open_ledger_tx_count: 33,
            base_fee_drops: 10,
        }),
    };
    let mut app_lock = TestAcceptLock;
    let mut tx_lock = TestAcceptLock;

    let app_result: Option<QueueTxQRpcReport> = app_txq.do_rpc(&mut app_lock, &app);
    let tx_result: Option<QueueTxQRpcReport> = accept_txq.do_rpc(&mut tx_lock, &app);

    assert_eq!(app_result, tx_result);
}

#[test]
fn txq_owner_apply_matches_landed_apply_facade() {
    let tx_source = TestApplyTxSource {
        account: "acct",
        transaction_id: "tx-1",
        tx_id: Uint256::from_u64(77),
        tx_seq_proxy: SeqProxy::sequence(5),
    };
    let view = build_apply_view();
    let hold_preflight = QueueHoldPreflight {
        has_previous_txn_id: false,
        has_account_txn_id: false,
        last_valid_ledger: Some(250),
        flags: ApplyFlags::FAIL_HARD,
        has_delegate: false,
        has_fee_sponsor: false,
    };
    let consequences = TxConsequences::new(1, SeqProxy::sequence(5));
    let mut app_runtime = build_apply_runtime();
    let mut tx_runtime = build_apply_runtime();
    let mut app_txq = build_app_txq();
    let mut apply_txq = build_apply_txq();
    let mut app_lock = TestApplyLock;
    let mut tx_lock = TestApplyLock;

    let app_result = app_txq.apply_with_app_view(
        &mut app_lock,
        &mut app_runtime,
        &view,
        &tx_source,
        hold_preflight,
        ApplyFlags::FAIL_HARD,
        consequences,
        Ter::TES_SUCCESS,
    );
    let tx_result = apply_txq.apply_with_app_view(
        &mut tx_lock,
        &mut tx_runtime,
        &view,
        &tx_source,
        hold_preflight,
        ApplyFlags::FAIL_HARD,
        consequences,
        Ter::TES_SUCCESS,
    );

    assert_eq!(app_result, tx_result);
    assert_eq!(app_runtime.preflight_calls, 1);
    assert_eq!(tx_runtime.preflight_calls, 1);
}
