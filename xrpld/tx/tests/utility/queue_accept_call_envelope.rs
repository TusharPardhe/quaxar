use std::cell::RefCell;
use std::collections::BTreeMap;

use basics::base_uint::Uint256;
use protocol::{Rules, SeqProxy, Ter};
use tx::{
    ApplyFlags, ApplyResult, FeeQueueEntry, FeeQueueKey, MaybeTx, MaybeTxCore, PreflightResult,
    QueueAcceptCallEnvelope, QueueAcceptLedgerViewSource, QueueAcceptLiveApplyRuntime,
    QueueAcceptLiveOwner, QueueAcceptOwnerShell, QueueAcceptOwnerState, QueueAdvanceCandidate,
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
fn queue_accept_call_envelope_matches_owner_shell_accept() {
    let mut direct_app = TestAcceptApplyOnlyApp {
        apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, false),
        apply_calls: 0,
    };
    let mut envelope_app = TestAcceptApplyOnlyApp {
        apply_result: ApplyResult::new(Ter::TES_SUCCESS, true, false),
        apply_calls: 0,
    };
    let view = TestLedgerView {
        open_ledger_tx_count: 0,
        parent_hash: Uint256::from_u64(6),
    };

    let mut direct_shell = build_owner_shell(Some(10), Uint256::from_u64(4));
    let direct_result = direct_shell.accept(&mut direct_app, &view);

    let mut envelope_shell = build_owner_shell(Some(10), Uint256::from_u64(4));
    let mut envelope = QueueAcceptCallEnvelope::new(&mut envelope_app, &view);
    let envelope_result = envelope.accept(&mut envelope_shell);

    assert_eq!(envelope_result, direct_result);
    assert_eq!(envelope_shell.owner(), direct_shell.owner());
    assert_eq!(direct_app.apply_calls, 1);
    assert_eq!(envelope_app.apply_calls, 1);
}

#[test]
fn queue_accept_call_envelope_matches_owner_shell_prepare() {
    let mut app = TestAcceptApplyOnlyApp {
        apply_result: ApplyResult::new(Ter::TER_RETRY, false, false),
        apply_calls: 0,
    };
    let view = TestLedgerView {
        open_ledger_tx_count: 32,
        parent_hash: Uint256::from_u64(9),
    };

    let mut direct_shell = build_owner_shell(Some(2), Uint256::from_u64(9));
    let direct_prepared = direct_shell.prepare_accept(&view);

    let mut envelope_shell = build_owner_shell(Some(2), Uint256::from_u64(9));
    let mut envelope = QueueAcceptCallEnvelope::new(&mut app, &view);
    let envelope_prepared = envelope.prepare_accept(&mut envelope_shell);

    assert_eq!(envelope_prepared, direct_prepared);
    assert_eq!(envelope_shell.owner(), direct_shell.owner());
}

#[test]
fn queue_accept_call_envelope_matches_owner_shell_sink_wrapper() {
    let mut direct_app = TestAcceptApplyOnlyApp {
        apply_result: ApplyResult::new(Ter::TER_RETRY, false, false),
        apply_calls: 0,
    };
    let mut envelope_app = TestAcceptApplyOnlyApp {
        apply_result: ApplyResult::new(Ter::TER_RETRY, false, false),
        apply_calls: 0,
    };
    let view = TestLedgerView {
        open_ledger_tx_count: 32,
        parent_hash: Uint256::from_u64(9),
    };

    let mut direct_shell = build_owner_shell(Some(2), Uint256::from_u64(9));
    let direct_emitted = RefCell::new(Vec::new());
    let direct_result = direct_shell.accept_with_log_sinks(
        &mut direct_app,
        &view,
        |message| direct_emitted.borrow_mut().push(format!("trace:{message}")),
        |message| direct_emitted.borrow_mut().push(format!("debug:{message}")),
        |message| direct_emitted.borrow_mut().push(format!("info:{message}")),
        |message| direct_emitted.borrow_mut().push(format!("warn:{message}")),
    );

    let mut envelope_shell = build_owner_shell(Some(2), Uint256::from_u64(9));
    let envelope_emitted = RefCell::new(Vec::new());
    let mut envelope = QueueAcceptCallEnvelope::new(&mut envelope_app, &view);
    let envelope_result = envelope.accept_with_log_sinks(
        &mut envelope_shell,
        |message| {
            envelope_emitted
                .borrow_mut()
                .push(format!("trace:{message}"))
        },
        |message| {
            envelope_emitted
                .borrow_mut()
                .push(format!("debug:{message}"))
        },
        |message| {
            envelope_emitted
                .borrow_mut()
                .push(format!("info:{message}"))
        },
        |message| {
            envelope_emitted
                .borrow_mut()
                .push(format!("warn:{message}"))
        },
    );

    assert_eq!(envelope_result, direct_result);
    assert_eq!(envelope_shell.owner(), direct_shell.owner());
    assert_eq!(envelope_emitted.into_inner(), direct_emitted.into_inner());
    assert_eq!(direct_app.apply_calls, 1);
    assert_eq!(envelope_app.apply_calls, 1);
}
