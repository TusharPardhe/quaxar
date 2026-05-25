use std::collections::BTreeMap;

use basics::base_uint::Uint256;
use protocol::{Rules, SeqProxy, Ter};
use tx::{
    ApplyFlags, ApplyResult, FeeQueueEntry, FeeQueueKey, MaybeTx, MaybeTxCore, PreflightResult,
    QueueAcceptJournalOwner, QueueAcceptJournalSink, QueueAcceptLedgerViewSource,
    QueueAcceptLiveApplyRuntime, QueueAcceptLiveOwner, QueueAcceptLockScope,
    QueueAcceptLockScopeOwner, QueueAcceptOwnerShell, QueueAcceptOwnerState, QueueAcceptTxQ,
    QueueAdvanceCandidate, QueueFeeMetricsSnapshot, QueueTxQAccountState,
    QueueTxQClosedLedgerAppSource, QueueTxQClosedLedgerView, QueueTxQMetrics,
    QueueTxQRequiredFeeAndSeq, QueueTxQRequiredFeeTxSource, QueueTxQRequiredFeeViewSource,
    QueueTxQRpcAppSource, QueueTxQRpcDrops, QueueTxQRpcLevels, QueueTxQRpcReport, QueueTxQRpcView,
    QueueViews, TXQ_BASE_LEVEL, TxConsequences, TxDetails, TxQAccount, TxQSetup,
};

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

#[derive(Debug, Clone, Copy)]
struct TestRequiredFeeView {
    open_ledger_tx_count: usize,
    base_fee_drops: u64,
    account_sequence: Option<u32>,
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

#[derive(Debug, Clone)]
struct TestClosedLedgerApp {
    validated_fee_levels: Vec<u64>,
}

#[derive(Debug, Clone, Copy)]
struct TestClosedLedgerView {
    ledger_seq: u32,
}

#[derive(Debug, Clone, Copy)]
struct TestRequiredFeeTx {
    account: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct TestJournal {
    emitted: Vec<String>,
}

#[derive(Debug, Default)]
struct TestLockScope;

impl QueueAcceptLockScope for TestLockScope {}

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

impl QueueTxQClosedLedgerView for TestClosedLedgerView {
    fn ledger_seq(&self) -> u32 {
        self.ledger_seq
    }
}

impl QueueTxQClosedLedgerAppSource<TestClosedLedgerView> for TestClosedLedgerApp {
    fn validated_fee_levels(&self, _view: &TestClosedLedgerView) -> Vec<u64> {
        self.validated_fee_levels.clone()
    }
}

impl QueueAcceptJournalSink for TestJournal {
    fn trace(&mut self, message: &str) {
        self.emitted.push(format!("trace:{message}"));
    }

    fn debug(&mut self, message: &str) {
        self.emitted.push(format!("debug:{message}"));
    }

    fn info(&mut self, message: &str) {
        self.emitted.push(format!("info:{message}"));
    }

    fn warn(&mut self, message: &str) {
        self.emitted.push(format!("warn:{message}"));
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

fn build_lock_scope_owner(
    current_max_size: Option<usize>,
    parent_hash_comp: Uint256,
) -> QueueAcceptLockScopeOwner<&'static str, &'static str, &'static str, &'static str, TestJournal>
{
    QueueAcceptLockScopeOwner::new(QueueAcceptJournalOwner::new(
        QueueAcceptOwnerShell::new(QueueAcceptLiveOwner::new_from_setup(
            &test_setup(),
            current_max_size,
            QueueAcceptOwnerState::new(parent_hash_comp),
            build_views(),
        )),
        TestJournal::default(),
    ))
}

fn build_gap_lock_scope_owner()
-> QueueAcceptLockScopeOwner<&'static str, &'static str, &'static str, &'static str, TestJournal> {
    let mut account = TxQAccount::new("acct");
    account.add(
        SeqProxy::sequence(5),
        MaybeTxCore::new(
            queued("acct", SeqProxy::sequence(5), 5, 300),
            TxConsequences::with_sequences_consumed(1, SeqProxy::sequence(5), 1),
        ),
    );
    account.add(
        SeqProxy::sequence(6),
        MaybeTxCore::new(
            queued("acct", SeqProxy::sequence(6), 6, 250),
            TxConsequences::with_sequences_consumed(1, SeqProxy::sequence(6), 1),
        ),
    );
    account.add(
        SeqProxy::sequence(8),
        MaybeTxCore::new(
            queued("acct", SeqProxy::sequence(8), 8, 200),
            TxConsequences::with_sequences_consumed(1, SeqProxy::sequence(8), 1),
        ),
    );

    QueueAcceptLockScopeOwner::new(QueueAcceptJournalOwner::new(
        QueueAcceptOwnerShell::new(QueueAcceptLiveOwner::new_from_setup(
            &test_setup(),
            Some(10),
            QueueAcceptOwnerState::new(Uint256::from_u64(4)),
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
                        FeeQueueKey::new("acct", SeqProxy::sequence(6)),
                        QueueAdvanceCandidate {
                            fee_level: 250,
                            tx_id: Uint256::from_u64(6),
                            seq_proxy: SeqProxy::sequence(6),
                        },
                    ),
                    FeeQueueEntry::new(
                        FeeQueueKey::new("acct", SeqProxy::sequence(8)),
                        QueueAdvanceCandidate {
                            fee_level: 200,
                            tx_id: Uint256::from_u64(8),
                            seq_proxy: SeqProxy::sequence(8),
                        },
                    ),
                ],
            ),
        )),
        TestJournal::default(),
    ))
}

fn build_process_closed_ledger_txq()
-> QueueAcceptTxQ<&'static str, &'static str, &'static str, &'static str, TestJournal> {
    let mut keep = TxQAccount::new("keep");
    keep.add(
        SeqProxy::sequence(5),
        MaybeTxCore::new(
            queued("keep", SeqProxy::sequence(5), 5, 300),
            TxConsequences::new(1, SeqProxy::sequence(5)),
        ),
    );
    keep.add(
        SeqProxy::sequence(6),
        MaybeTxCore::new(
            queued("keep", SeqProxy::sequence(6), 6, 250),
            TxConsequences::new(1, SeqProxy::sequence(6)),
        ),
    );

    let mut drop = TxQAccount::new("drop");
    drop.add(
        SeqProxy::sequence(9),
        MaybeTxCore::new(
            queued("drop", SeqProxy::sequence(9), 9, 150),
            TxConsequences::new(1, SeqProxy::sequence(9)),
        ),
    );

    keep.transactions
        .get_mut(&SeqProxy::sequence(5))
        .expect("keep expired transaction should exist")
        .payload
        .last_valid = Some(20);
    keep.transactions
        .get_mut(&SeqProxy::sequence(6))
        .expect("keep live transaction should exist")
        .payload
        .last_valid = Some(21);
    drop.transactions
        .get_mut(&SeqProxy::sequence(9))
        .expect("drop expired transaction should exist")
        .payload
        .last_valid = Some(20);

    let views = QueueViews::new(
        BTreeMap::from([("drop", drop), ("keep", keep)]),
        vec![
            FeeQueueEntry::new(
                FeeQueueKey::new("keep", SeqProxy::sequence(5)),
                QueueAdvanceCandidate {
                    fee_level: 300,
                    tx_id: Uint256::from_u64(5),
                    seq_proxy: SeqProxy::sequence(5),
                },
            ),
            FeeQueueEntry::new(
                FeeQueueKey::new("keep", SeqProxy::sequence(6)),
                QueueAdvanceCandidate {
                    fee_level: 250,
                    tx_id: Uint256::from_u64(6),
                    seq_proxy: SeqProxy::sequence(6),
                },
            ),
            FeeQueueEntry::new(
                FeeQueueKey::new("drop", SeqProxy::sequence(9)),
                QueueAdvanceCandidate {
                    fee_level: 150,
                    tx_id: Uint256::from_u64(9),
                    seq_proxy: SeqProxy::sequence(9),
                },
            ),
        ],
    );

    QueueAcceptTxQ::new(QueueAcceptLockScopeOwner::new(
        QueueAcceptJournalOwner::new(
            QueueAcceptOwnerShell::new(QueueAcceptLiveOwner::new_from_setup(
                &test_setup(),
                Some(50),
                QueueAcceptOwnerState::new(Uint256::from_u64(9)),
                views,
            )),
            TestJournal::default(),
        ),
    ))
}

fn build_multi_account_lock_scope_owner()
-> QueueAcceptLockScopeOwner<&'static str, &'static str, &'static str, &'static str, TestJournal> {
    let mut acct = TxQAccount::new("acct");
    acct.add(
        SeqProxy::sequence(5),
        MaybeTxCore::new(
            queued("acct", SeqProxy::sequence(5), 5, 300),
            TxConsequences::new(1, SeqProxy::sequence(5)),
        ),
    );
    acct.add(
        SeqProxy::sequence(9),
        MaybeTxCore::new(
            queued("acct", SeqProxy::sequence(9), 9, 60),
            TxConsequences::new(1, SeqProxy::sequence(9)),
        ),
    );

    let mut other = TxQAccount::new("other");
    other.add(
        SeqProxy::sequence(7),
        MaybeTxCore::new(
            queued("other", SeqProxy::sequence(7), 7, 500),
            TxConsequences::new(2, SeqProxy::sequence(7)),
        ),
    );

    QueueAcceptLockScopeOwner::new(QueueAcceptJournalOwner::new(
        QueueAcceptOwnerShell::new(QueueAcceptLiveOwner::new_from_setup(
            &test_setup(),
            Some(10),
            QueueAcceptOwnerState::new(Uint256::from_u64(4)),
            QueueViews::new(
                BTreeMap::from([("acct", acct), ("other", other)]),
                vec![
                    FeeQueueEntry::new(
                        FeeQueueKey::new("other", SeqProxy::sequence(7)),
                        QueueAdvanceCandidate {
                            fee_level: 500,
                            tx_id: Uint256::from_u64(7),
                            seq_proxy: SeqProxy::sequence(7),
                        },
                    ),
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
            ),
        )),
        TestJournal::default(),
    ))
}

fn build_max_fee_lock_scope_owner()
-> QueueAcceptLockScopeOwner<&'static str, &'static str, &'static str, &'static str, TestJournal> {
    let mut account = TxQAccount::new("max");
    account.add(
        SeqProxy::sequence(1),
        MaybeTxCore::new(
            queued("max", SeqProxy::sequence(1), 1, u64::MAX),
            TxConsequences::new(1, SeqProxy::sequence(1)),
        ),
    );

    QueueAcceptLockScopeOwner::new(QueueAcceptJournalOwner::new(
        QueueAcceptOwnerShell::new(QueueAcceptLiveOwner::new_from_setup(
            &test_setup(),
            Some(1),
            QueueAcceptOwnerState::new(Uint256::from_u64(1)),
            QueueViews::new(
                BTreeMap::from([("max", account)]),
                vec![FeeQueueEntry::new(
                    FeeQueueKey::new("max", SeqProxy::sequence(1)),
                    QueueAdvanceCandidate {
                        fee_level: u64::MAX,
                        tx_id: Uint256::from_u64(1),
                        seq_proxy: SeqProxy::sequence(1),
                    },
                )],
            ),
        )),
        TestJournal::default(),
    ))
}

#[test]
fn queue_accept_txq_matches_lock_scope_owner() {
    let mut locked_app = TestAcceptApplyOnlyApp {
        apply_result: ApplyResult::new(Ter::TER_RETRY, false, false),
        apply_calls: 0,
    };
    let mut txq_app = TestAcceptApplyOnlyApp {
        apply_result: ApplyResult::new(Ter::TER_RETRY, false, false),
        apply_calls: 0,
    };
    let view = TestLedgerView {
        open_ledger_tx_count: 32,
        parent_hash: Uint256::from_u64(9),
    };

    let mut lock_scope_owner = build_lock_scope_owner(Some(2), Uint256::from_u64(9));
    let mut lock = TestLockScope;
    let lock_scope_result = lock_scope_owner.accept(&mut lock, &mut locked_app, &view);

    let mut txq = QueueAcceptTxQ::new(build_lock_scope_owner(Some(2), Uint256::from_u64(9)));
    let mut lock = TestLockScope;
    let txq_result = txq.accept(&mut lock, &mut txq_app, &view);

    assert_eq!(txq_result, lock_scope_result);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert_eq!(locked_app.apply_calls, 1);
    assert_eq!(txq_app.apply_calls, 1);
}

#[test]
fn queue_accept_txq_prepare_matches_lock_scope_owner() {
    let view = TestLedgerView {
        open_ledger_tx_count: 32,
        parent_hash: Uint256::from_u64(9),
    };

    let mut lock_scope_owner = build_lock_scope_owner(Some(2), Uint256::from_u64(9));
    let mut lock = TestLockScope;
    let lock_scope_prepared = lock_scope_owner.prepare_accept(&mut lock, &view);

    let mut txq = QueueAcceptTxQ::new(build_lock_scope_owner(Some(2), Uint256::from_u64(9)));
    let mut lock = TestLockScope;
    let txq_prepared = txq.prepare_accept(&mut lock, &view);

    assert_eq!(txq_prepared, lock_scope_prepared);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
}

#[test]
fn queue_accept_txq_skips_warning_when_parent_hash_changes() {
    let mut locked_app = TestAcceptApplyOnlyApp {
        apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, false),
        apply_calls: 0,
    };
    let mut txq_app = TestAcceptApplyOnlyApp {
        apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, false),
        apply_calls: 0,
    };
    let view = TestLedgerView {
        open_ledger_tx_count: 0,
        parent_hash: Uint256::from_u64(6),
    };

    let mut lock_scope_owner = build_lock_scope_owner(Some(10), Uint256::from_u64(4));
    let mut lock = TestLockScope;
    let lock_scope_result = lock_scope_owner.accept(&mut lock, &mut locked_app, &view);

    let mut txq = QueueAcceptTxQ::new(build_lock_scope_owner(Some(10), Uint256::from_u64(4)));
    let mut lock = TestLockScope;
    let txq_result = txq.accept(&mut lock, &mut txq_app, &view);

    assert_eq!(txq_result, lock_scope_result);
    assert_eq!(txq.lock_scope_owner(), &lock_scope_owner);
    assert!(
        txq.lock_scope_owner()
            .owner()
            .journal()
            .emitted
            .iter()
            .all(|message| !message.starts_with("warn:"))
    );
    assert_eq!(locked_app.apply_calls, 1);
    assert_eq!(txq_app.apply_calls, 1);
}

#[test]
fn queue_accept_txq_next_queuable_seq_returns_zero_for_missing_account() {
    let txq = QueueAcceptTxQ::new(build_lock_scope_owner(Some(2), Uint256::from_u64(9)));
    let mut lock = TestLockScope;

    assert_eq!(
        txq.next_queuable_seq(&mut lock, QueueTxQAccountState::<&'static str>::Missing),
        SeqProxy::sequence(0)
    );
}

#[test]
fn queue_accept_txq_next_queuable_seq_returns_account_sequence_when_not_queued() {
    let txq = QueueAcceptTxQ::new(build_lock_scope_owner(Some(2), Uint256::from_u64(9)));
    let mut lock = TestLockScope;

    assert_eq!(
        txq.next_queuable_seq(
            &mut lock,
            QueueTxQAccountState::Present {
                account: &"other",
                seq_proxy: SeqProxy::sequence(11),
            },
        ),
        SeqProxy::sequence(11)
    );
}

#[test]
fn queue_accept_txq_next_queuable_seq_finds_first_gap_for_queued_account() {
    let txq = QueueAcceptTxQ::new(build_gap_lock_scope_owner());
    let mut lock = TestLockScope;

    assert_eq!(
        txq.next_queuable_seq(
            &mut lock,
            QueueTxQAccountState::Present {
                account: &"acct",
                seq_proxy: SeqProxy::sequence(5),
            },
        ),
        SeqProxy::sequence(7)
    );
}

#[test]
fn queue_accept_txq_get_metrics_matches_current_cpp_boundary_when_not_full() {
    let txq = QueueAcceptTxQ::new(build_lock_scope_owner(Some(10), Uint256::from_u64(9)));
    let view = TestLedgerView {
        open_ledger_tx_count: 33,
        parent_hash: Uint256::from_u64(9),
    };
    let mut lock = TestLockScope;

    assert_eq!(
        txq.get_metrics(&mut lock, &view),
        QueueTxQMetrics {
            tx_count: 2,
            tx_q_max_size: Some(10),
            tx_in_ledger: 33,
            tx_per_ledger: 32,
            reference_fee_level: TXQ_BASE_LEVEL,
            min_processing_fee_level: TXQ_BASE_LEVEL,
            med_fee_level: TXQ_BASE_LEVEL * 500,
            open_ledger_fee_level: 136_125,
        }
    );
}

#[test]
fn queue_accept_txq_get_metrics_raises_min_processing_fee_when_full() {
    let txq = QueueAcceptTxQ::new(build_lock_scope_owner(Some(2), Uint256::from_u64(9)));
    let view = TestLedgerView {
        open_ledger_tx_count: 32,
        parent_hash: Uint256::from_u64(9),
    };
    let mut lock = TestLockScope;

    assert_eq!(
        txq.get_metrics(&mut lock, &view),
        QueueTxQMetrics {
            tx_count: 2,
            tx_q_max_size: Some(2),
            tx_in_ledger: 32,
            tx_per_ledger: 32,
            reference_fee_level: TXQ_BASE_LEVEL,
            min_processing_fee_level: 61,
            med_fee_level: TXQ_BASE_LEVEL * 500,
            open_ledger_fee_level: TXQ_BASE_LEVEL,
        }
    );
}

#[test]
fn queue_accept_txq_get_metrics_wraps_full_queue_fee_level_at_u64_max() {
    let txq = QueueAcceptTxQ::new(build_max_fee_lock_scope_owner());
    let view = TestLedgerView {
        open_ledger_tx_count: 0,
        parent_hash: Uint256::from_u64(1),
    };
    let mut lock = TestLockScope;

    assert_eq!(
        txq.get_metrics(&mut lock, &view).min_processing_fee_level,
        0
    );
}

#[test]
fn queue_accept_txq_get_rpc_fee_report_matches_current_cpp_boundary() {
    let txq = QueueAcceptTxQ::new(build_lock_scope_owner(Some(10), Uint256::from_u64(9)));
    let view = TestRpcView {
        ledger_current_index: 777,
        open_ledger_tx_count: 33,
        base_fee_drops: 10,
    };
    let mut lock = TestLockScope;

    assert_eq!(
        txq.get_rpc_fee_report(&mut lock, &view),
        QueueTxQRpcReport {
            ledger_current_index: 777,
            expected_ledger_size: "32".to_string(),
            current_ledger_size: "33".to_string(),
            current_queue_size: "2".to_string(),
            max_queue_size: Some("10".to_string()),
            levels: QueueTxQRpcLevels {
                reference_level: "256".to_string(),
                minimum_level: "256".to_string(),
                median_level: "128000".to_string(),
                open_ledger_level: "136125".to_string(),
            },
            drops: QueueTxQRpcDrops {
                base_fee: "10".to_string(),
                median_fee: "5000".to_string(),
                minimum_fee: "10".to_string(),
                open_ledger_fee: "5318".to_string(),
            },
        }
    );
}

#[test]
fn queue_accept_txq_get_rpc_fee_report_uses_effective_one_drop_base_when_zero_base_fee_escalates() {
    let txq = QueueAcceptTxQ::new(build_lock_scope_owner(Some(10), Uint256::from_u64(9)));
    let view = TestRpcView {
        ledger_current_index: 777,
        open_ledger_tx_count: 33,
        base_fee_drops: 0,
    };
    let mut lock = TestLockScope;

    assert_eq!(
        txq.get_rpc_fee_report(&mut lock, &view),
        QueueTxQRpcReport {
            ledger_current_index: 777,
            expected_ledger_size: "32".to_string(),
            current_ledger_size: "33".to_string(),
            current_queue_size: "2".to_string(),
            max_queue_size: Some("10".to_string()),
            levels: QueueTxQRpcLevels {
                reference_level: "256".to_string(),
                minimum_level: "256".to_string(),
                median_level: "128000".to_string(),
                open_ledger_level: "136125".to_string(),
            },
            drops: QueueTxQRpcDrops {
                base_fee: "0".to_string(),
                median_fee: "0".to_string(),
                minimum_fee: "0".to_string(),
                open_ledger_fee: "532".to_string(),
            },
        }
    );
}

#[test]
fn queue_accept_txq_get_rpc_fee_report_uses_full_queue_minimum_fee_rule() {
    let txq = QueueAcceptTxQ::new(build_lock_scope_owner(Some(2), Uint256::from_u64(9)));
    let view = TestRpcView {
        ledger_current_index: 777,
        open_ledger_tx_count: 32,
        base_fee_drops: 10,
    };
    let mut lock = TestLockScope;

    assert_eq!(
        txq.get_rpc_fee_report(&mut lock, &view).drops.minimum_fee,
        "2"
    );
}

#[test]
fn queue_accept_txq_get_rpc_fee_report_uses_wrapped_minimum_fee_at_u64_max() {
    let txq = QueueAcceptTxQ::new(build_max_fee_lock_scope_owner());
    let view = TestRpcView {
        ledger_current_index: 777,
        open_ledger_tx_count: 0,
        base_fee_drops: 10,
    };
    let mut lock = TestLockScope;

    assert_eq!(
        txq.get_rpc_fee_report(&mut lock, &view).drops.minimum_fee,
        "0"
    );
}

#[test]
fn queue_accept_txq_get_rpc_fee_report_from_app_matches_direct_view_boundary() {
    let txq = QueueAcceptTxQ::new(build_lock_scope_owner(Some(10), Uint256::from_u64(9)));
    let app = TestRpcApp {
        current_view: Some(TestRpcView {
            ledger_current_index: 777,
            open_ledger_tx_count: 33,
            base_fee_drops: 10,
        }),
    };
    let mut lock = TestLockScope;

    assert_eq!(
        txq.get_rpc_fee_report_from_app(&mut lock, &app),
        Some(QueueTxQRpcReport {
            ledger_current_index: 777,
            expected_ledger_size: "32".to_string(),
            current_ledger_size: "33".to_string(),
            current_queue_size: "2".to_string(),
            max_queue_size: Some("10".to_string()),
            levels: QueueTxQRpcLevels {
                reference_level: "256".to_string(),
                minimum_level: "256".to_string(),
                median_level: "128000".to_string(),
                open_ledger_level: "136125".to_string(),
            },
            drops: QueueTxQRpcDrops {
                base_fee: "10".to_string(),
                median_fee: "5000".to_string(),
                minimum_fee: "10".to_string(),
                open_ledger_fee: "5318".to_string(),
            },
        })
    );
}

#[test]
fn queue_accept_txq_get_rpc_fee_report_from_app_returns_none_without_current_view() {
    let txq = QueueAcceptTxQ::new(build_lock_scope_owner(Some(10), Uint256::from_u64(9)));
    let app = TestRpcApp { current_view: None };
    let mut lock = TestLockScope;

    assert_eq!(txq.get_rpc_fee_report_from_app(&mut lock, &app), None);
}

#[test]
fn queue_accept_txq_do_rpc_matches_get_rpc_fee_report_from_app() {
    let txq = QueueAcceptTxQ::new(build_lock_scope_owner(Some(10), Uint256::from_u64(9)));
    let app = TestRpcApp {
        current_view: Some(TestRpcView {
            ledger_current_index: 777,
            open_ledger_tx_count: 33,
            base_fee_drops: 10,
        }),
    };
    let mut lock = TestLockScope;

    assert_eq!(
        txq.do_rpc(&mut lock, &app),
        txq.get_rpc_fee_report_from_app(&mut lock, &app)
    );
}

#[test]
fn queue_accept_txq_process_closed_ledger_updates_metrics_and_owner_views() {
    let mut txq = build_process_closed_ledger_txq();
    let app = TestClosedLedgerApp {
        validated_fee_levels: vec![TXQ_BASE_LEVEL * 600; 300],
    };
    let view = TestClosedLedgerView { ledger_seq: 20 };
    let mut lock = TestLockScope;

    let result = txq.process_closed_ledger(&mut lock, &app, &view, false);

    assert_eq!(result.validated_tx_count, 300);
    assert_eq!(
        result.metrics_snapshot,
        QueueFeeMetricsSnapshot {
            txns_expected: 360,
            escalation_multiplier: TXQ_BASE_LEVEL * 600,
        }
    );
    assert_eq!(result.maintenance.next_max_size, 1080);
    assert_eq!(
        result
            .maintenance
            .expired_candidates
            .iter()
            .map(|candidate| (candidate.account, candidate.seq_proxy, candidate.last_valid))
            .collect::<Vec<_>>(),
        vec![
            ("keep", SeqProxy::sequence(5), Some(20)),
            ("drop", SeqProxy::sequence(9), Some(20)),
        ]
    );
    assert_eq!(result.maintenance.emptied_accounts, vec!["drop"]);

    let live_owner = txq.lock_scope_owner().owner().owner().owner();
    assert_eq!(live_owner.current_max_size(), Some(1080));
    assert_eq!(live_owner.views().fee_order.len(), 1);
    assert_eq!(
        live_owner.views().fee_order[0].key,
        FeeQueueKey::new("keep", SeqProxy::sequence(6))
    );
    assert!(!live_owner.views().accounts.contains_key("drop"));
    let keep = live_owner
        .views()
        .accounts
        .get("keep")
        .expect("keep account should remain");
    assert!(keep.drop_penalty);
    assert_eq!(keep.get_txn_count(), 1);
    assert!(keep.transactions.contains_key(&SeqProxy::sequence(6)));
}

#[test]
fn queue_accept_txq_process_closed_ledger_keeps_max_size_when_time_leap() {
    let mut txq = build_process_closed_ledger_txq();
    let app = TestClosedLedgerApp {
        validated_fee_levels: vec![TXQ_BASE_LEVEL * 600; 300],
    };
    let view = TestClosedLedgerView { ledger_seq: 20 };
    let mut lock = TestLockScope;

    let result = txq.process_closed_ledger(&mut lock, &app, &view, true);

    assert_eq!(result.maintenance.next_max_size, 50);
    assert_eq!(
        txq.lock_scope_owner()
            .owner()
            .owner()
            .owner()
            .current_max_size(),
        Some(50)
    );
}

#[test]
fn queue_accept_txq_get_account_txs_returns_empty_for_missing_account() {
    let txq = QueueAcceptTxQ::new(build_lock_scope_owner(Some(2), Uint256::from_u64(9)));
    let mut lock = TestLockScope;

    assert_eq!(
        txq.get_account_txs(&mut lock, &"missing"),
        Vec::<TxDetails<&'static str, &'static str>>::new()
    );
}

#[test]
fn queue_accept_txq_get_account_txs_uses_account_sequence_order() {
    let txq = QueueAcceptTxQ::new(build_gap_lock_scope_owner());
    let mut lock = TestLockScope;

    assert_eq!(
        txq.get_account_txs(&mut lock, &"acct"),
        vec![
            queued("acct", SeqProxy::sequence(5), 5, 300).get_tx_details(),
            queued("acct", SeqProxy::sequence(6), 6, 250).get_tx_details(),
            queued("acct", SeqProxy::sequence(8), 8, 200).get_tx_details(),
        ]
    );
}

#[test]
fn queue_accept_txq_get_txs_uses_fee_order_across_accounts() {
    let txq = QueueAcceptTxQ::new(build_multi_account_lock_scope_owner());
    let mut lock = TestLockScope;

    assert_eq!(
        txq.get_txs(&mut lock),
        vec![
            queued("other", SeqProxy::sequence(7), 7, 500).get_tx_details(),
            queued("acct", SeqProxy::sequence(5), 5, 300).get_tx_details(),
            queued("acct", SeqProxy::sequence(9), 9, 60).get_tx_details(),
        ]
    );
}

#[test]
fn queue_accept_txq_get_tx_required_fee_and_seq_returns_zero_sequences_for_missing_account() {
    let txq = QueueAcceptTxQ::new(build_multi_account_lock_scope_owner());
    let view = TestRequiredFeeView {
        open_ledger_tx_count: 32,
        base_fee_drops: 10,
        account_sequence: None,
    };
    let tx = TestRequiredFeeTx { account: "other" };
    let mut lock = TestLockScope;

    assert_eq!(
        txq.get_tx_required_fee_and_seq(&mut lock, &view, &tx),
        QueueTxQRequiredFeeAndSeq {
            required_fee_drops: 10,
            account_seq: 0,
            available_seq: 0,
        }
    );
}

#[test]
fn queue_accept_txq_get_tx_required_fee_and_seq_uses_account_gap_rule_when_present() {
    let txq = QueueAcceptTxQ::new(build_gap_lock_scope_owner());
    let view = TestRequiredFeeView {
        open_ledger_tx_count: 33,
        base_fee_drops: 10,
        account_sequence: Some(5),
    };
    let tx = TestRequiredFeeTx { account: "acct" };
    let mut lock = TestLockScope;

    assert_eq!(
        txq.get_tx_required_fee_and_seq(&mut lock, &view, &tx),
        QueueTxQRequiredFeeAndSeq {
            required_fee_drops: 5_317,
            account_seq: 5,
            available_seq: 7,
        }
    );
}

#[test]
fn queue_accept_txq_get_tx_required_fee_and_seq_clamps_overflow() {
    let txq = QueueAcceptTxQ::new(build_lock_scope_owner(Some(10), Uint256::from_u64(9)));
    let view = TestRequiredFeeView {
        open_ledger_tx_count: usize::MAX,
        base_fee_drops: u64::MAX,
        account_sequence: Some(11),
    };
    let tx = TestRequiredFeeTx { account: "acct" };
    let mut lock = TestLockScope;

    assert_eq!(
        txq.get_tx_required_fee_and_seq(&mut lock, &view, &tx),
        QueueTxQRequiredFeeAndSeq {
            required_fee_drops: i64::MAX,
            account_seq: 11,
            available_seq: 11,
        }
    );
}
