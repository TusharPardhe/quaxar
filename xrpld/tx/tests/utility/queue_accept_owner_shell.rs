use std::cell::RefCell;
use std::collections::BTreeMap;

use basics::base_uint::Uint256;
use protocol::{Rules, SeqProxy, Ter};
use tx::{
    ApplyFlags, ApplyResult, FeeQueueEntry, FeeQueueKey, MaybeTx, MaybeTxCore, PreflightResult,
    QueueAcceptLedgerViewSource, QueueAcceptLiveApplyRuntime, QueueAcceptLiveOwner,
    QueueAcceptOwnerShell, QueueAcceptOwnerState, QueueAdvanceCandidate, QueueViews,
    TxConsequences, TxQAccount, TxQSetup, prepare_queue_accept_with_live_owner,
    run_queue_accept_with_live_owner, run_queue_accept_with_live_owner_and_log_sinks,
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

fn build_live_owner(
    current_max_size: Option<usize>,
    parent_hash_comp: Uint256,
) -> QueueAcceptLiveOwner<&'static str, &'static str, &'static str, &'static str> {
    QueueAcceptLiveOwner::new_from_setup(
        &test_setup(),
        current_max_size,
        QueueAcceptOwnerState::new(parent_hash_comp),
        build_views(),
    )
}

#[test]
fn queue_accept_owner_shell_matches_live_owner_wrapper() {
    let mut live_owner_app = TestAcceptApplyOnlyApp {
        apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, false),
        apply_calls: 0,
    };
    let mut shell_app = TestAcceptApplyOnlyApp {
        apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, false),
        apply_calls: 0,
    };
    let view = TestLedgerView {
        open_ledger_tx_count: 0,
        parent_hash: Uint256::from_u64(6),
    };

    let mut live_owner = build_live_owner(Some(10), Uint256::from_u64(4));
    let live_owner_result =
        run_queue_accept_with_live_owner(&mut live_owner, &mut live_owner_app, &view);

    let mut shell = QueueAcceptOwnerShell::new(build_live_owner(Some(10), Uint256::from_u64(4)));
    let shell_result = shell.accept(&mut shell_app, &view);

    assert_eq!(shell_result, live_owner_result);
    assert_eq!(shell.owner(), &live_owner);
    assert_eq!(live_owner_app.apply_calls, 1);
    assert_eq!(shell_app.apply_calls, 1);
}

#[test]
fn queue_accept_owner_shell_prepare_matches_live_owner_prepare_boundary() {
    let view = TestLedgerView {
        open_ledger_tx_count: 32,
        parent_hash: Uint256::from_u64(9),
    };

    let mut live_owner = build_live_owner(Some(2), Uint256::from_u64(9));
    let live_owner_prepared = prepare_queue_accept_with_live_owner(&mut live_owner, &view);

    let mut shell = QueueAcceptOwnerShell::new(build_live_owner(Some(2), Uint256::from_u64(9)));
    let shell_prepared = shell.prepare_accept(&view);

    assert_eq!(shell_prepared, live_owner_prepared);
    assert_eq!(shell.owner(), &live_owner);
}

#[test]
fn queue_accept_owner_shell_sink_wrapper_matches_live_owner_sink_wrapper() {
    let mut live_owner_app = TestAcceptApplyOnlyApp {
        apply_result: ApplyResult::new(Ter::TER_RETRY, false, false),
        apply_calls: 0,
    };
    let mut shell_app = TestAcceptApplyOnlyApp {
        apply_result: ApplyResult::new(Ter::TER_RETRY, false, false),
        apply_calls: 0,
    };
    let view = TestLedgerView {
        open_ledger_tx_count: 32,
        parent_hash: Uint256::from_u64(9),
    };

    let mut live_owner = build_live_owner(Some(2), Uint256::from_u64(9));
    let live_owner_emitted = RefCell::new(Vec::new());
    let live_owner_result = run_queue_accept_with_live_owner_and_log_sinks(
        &mut live_owner,
        &mut live_owner_app,
        &view,
        |message| {
            live_owner_emitted
                .borrow_mut()
                .push(format!("trace:{message}"))
        },
        |message| {
            live_owner_emitted
                .borrow_mut()
                .push(format!("debug:{message}"))
        },
        |message| {
            live_owner_emitted
                .borrow_mut()
                .push(format!("info:{message}"))
        },
        |message| {
            live_owner_emitted
                .borrow_mut()
                .push(format!("warn:{message}"))
        },
    );

    let mut shell = QueueAcceptOwnerShell::new(build_live_owner(Some(2), Uint256::from_u64(9)));
    let shell_emitted = RefCell::new(Vec::new());
    let shell_result = shell.accept_with_log_sinks(
        &mut shell_app,
        &view,
        |message| shell_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| shell_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| shell_emitted.borrow_mut().push(format!("info:{message}")),
        |message| shell_emitted.borrow_mut().push(format!("warn:{message}")),
    );

    assert_eq!(shell_result, live_owner_result);
    assert_eq!(shell.owner(), &live_owner);
    assert_eq!(shell_emitted.into_inner(), live_owner_emitted.into_inner());
    assert_eq!(live_owner_app.apply_calls, 1);
    assert_eq!(shell_app.apply_calls, 1);
}
