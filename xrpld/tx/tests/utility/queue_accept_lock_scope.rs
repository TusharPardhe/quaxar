use std::collections::BTreeMap;

use basics::base_uint::Uint256;
use protocol::{Rules, SeqProxy, Ter};
use tx::{
    ApplyFlags, ApplyResult, FeeQueueEntry, FeeQueueKey, MaybeTx, MaybeTxCore, PreflightResult,
    QueueAcceptJournalOwner, QueueAcceptJournalSink, QueueAcceptLedgerViewSource,
    QueueAcceptLiveApplyRuntime, QueueAcceptLiveOwner, QueueAcceptLockScope,
    QueueAcceptLockScopeOwner, QueueAcceptOwnerShell, QueueAcceptOwnerState, QueueAdvanceCandidate,
    QueueViews, TxConsequences, TxQAccount, TxQSetup,
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

fn build_owner_shell(
    current_max_size: Option<usize>,
    parent_hash_comp: Uint256,
) -> QueueAcceptOwnerShell<&'static str, &'static str, &'static str, &'static str> {
    QueueAcceptOwnerShell::new(QueueAcceptLiveOwner::new_from_setup(
        &test_setup(),
        current_max_size,
        QueueAcceptOwnerState::new(parent_hash_comp),
        build_views(),
    ))
}

#[test]
fn queue_accept_lock_scope_owner_matches_journal_owner() {
    let mut journal_app = TestAcceptApplyOnlyApp {
        apply_result: ApplyResult::new(Ter::TER_RETRY, false, false),
        apply_calls: 0,
    };
    let mut locked_app = TestAcceptApplyOnlyApp {
        apply_result: ApplyResult::new(Ter::TER_RETRY, false, false),
        apply_calls: 0,
    };
    let view = TestLedgerView {
        open_ledger_tx_count: 32,
        parent_hash: Uint256::from_u64(9),
    };

    let mut journal_owner = QueueAcceptJournalOwner::new(
        build_owner_shell(Some(2), Uint256::from_u64(9)),
        TestJournal::default(),
    );
    let journal_result = journal_owner.accept(&mut journal_app, &view);

    let mut locked_owner = QueueAcceptLockScopeOwner::new(QueueAcceptJournalOwner::new(
        build_owner_shell(Some(2), Uint256::from_u64(9)),
        TestJournal::default(),
    ));
    let mut lock = TestLockScope;
    let locked_result = locked_owner.accept(&mut lock, &mut locked_app, &view);

    assert_eq!(locked_result, journal_result);
    assert_eq!(locked_owner.owner(), &journal_owner);
    assert_eq!(journal_app.apply_calls, 1);
    assert_eq!(locked_app.apply_calls, 1);
}

#[test]
fn queue_accept_lock_scope_owner_prepare_matches_journal_owner() {
    let view = TestLedgerView {
        open_ledger_tx_count: 32,
        parent_hash: Uint256::from_u64(9),
    };

    let mut journal_owner = QueueAcceptJournalOwner::new(
        build_owner_shell(Some(2), Uint256::from_u64(9)),
        TestJournal::default(),
    );
    let journal_prepared = journal_owner.prepare_accept(&view);

    let mut locked_owner = QueueAcceptLockScopeOwner::new(QueueAcceptJournalOwner::new(
        build_owner_shell(Some(2), Uint256::from_u64(9)),
        TestJournal::default(),
    ));
    let mut lock = TestLockScope;
    let locked_prepared = locked_owner.prepare_accept(&mut lock, &view);

    assert_eq!(locked_prepared, journal_prepared);
    assert_eq!(locked_owner.owner(), &journal_owner);
}

#[test]
fn queue_accept_lock_scope_owner_skips_warning_when_parent_hash_changes() {
    let mut journal_app = TestAcceptApplyOnlyApp {
        apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, false),
        apply_calls: 0,
    };
    let mut locked_app = TestAcceptApplyOnlyApp {
        apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, false),
        apply_calls: 0,
    };
    let view = TestLedgerView {
        open_ledger_tx_count: 0,
        parent_hash: Uint256::from_u64(6),
    };

    let mut journal_owner = QueueAcceptJournalOwner::new(
        build_owner_shell(Some(10), Uint256::from_u64(4)),
        TestJournal::default(),
    );
    let journal_result = journal_owner.accept(&mut journal_app, &view);

    let mut locked_owner = QueueAcceptLockScopeOwner::new(QueueAcceptJournalOwner::new(
        build_owner_shell(Some(10), Uint256::from_u64(4)),
        TestJournal::default(),
    ));
    let mut lock = TestLockScope;
    let locked_result = locked_owner.accept(&mut lock, &mut locked_app, &view);

    assert_eq!(locked_result, journal_result);
    assert_eq!(locked_owner.owner(), &journal_owner);
    assert!(
        locked_owner
            .owner()
            .journal()
            .emitted
            .iter()
            .all(|message| !message.starts_with("warn:"))
    );
    assert_eq!(journal_app.apply_calls, 1);
    assert_eq!(locked_app.apply_calls, 1);
}
