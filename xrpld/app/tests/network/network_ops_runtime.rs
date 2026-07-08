use app::{
    AppNetworkOpsAppliedTransactionReport, AppNetworkOpsApplyHeldOutcome, AppNetworkOpsApplyReport,
    AppNetworkOpsPreprocessReport, AppNetworkOpsProcessReport, AppNetworkOpsRuntime,
    AppNetworkOpsSubmitReport, ApplicationRoot, CurrentLedgerState, NetworkOpsApplyBatchStart,
    NetworkOpsApplyBatchTail, NetworkOpsApplyResultPreamble, NetworkOpsApplyStatusBranch,
    NetworkOpsAsyncDispatch, NetworkOpsCurrentLedgerState, NetworkOpsDispatchState,
    NetworkOpsOperatingMode, NetworkOpsPreprocessDecision, NetworkOpsProcessDispatch,
    NetworkOpsProcessSetOwnerSync, NetworkOpsRelayBranch, NetworkOpsRetryHoldBranch,
    NetworkOpsSubmitFlowOutcome, NetworkOpsTransactionSetOutcome, SharedLedgerMasterState,
    SharedNetworkOpsState, SubmitResult, TimeKeeper, TransStatus, Transaction,
};
use basics::base_uint::Uint256;
use basics::sha_map_hash::SHAMapHash;
use protocol::{AccountID, STAmount, STTx, Ter, TxType, XRPAmount, get_field_by_symbol};
use std::sync::{Arc, Mutex};
use tx::{ApplyFlags, ApplyResult};
use xrpl_core::{HashRouter, HashRouterFlags, HashRouterSetup};

fn account(hex: &str) -> AccountID {
    AccountID::from_hex(hex).expect("account hex should parse")
}

fn payment_tx(
    source: AccountID,
    destination: AccountID,
    sequence: u32,
    ticket_sequence: Option<u32>,
    fee_drops: u64,
) -> Arc<STTx> {
    Arc::new(STTx::new(TxType::PAYMENT, |tx| {
        tx.set_account_id(get_field_by_symbol("sfAccount"), source);
        tx.set_account_id(get_field_by_symbol("sfDestination"), destination);
        tx.set_field_amount(
            get_field_by_symbol("sfAmount"),
            STAmount::new_native(1_000_000, false),
        );
        tx.set_field_amount(
            get_field_by_symbol("sfFee"),
            STAmount::new_native(fee_drops, false),
        );
        tx.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
        if let Some(ticket_sequence) = ticket_sequence {
            tx.set_field_u32(get_field_by_symbol("sfTicketSequence"), ticket_sequence);
        }
    }))
}

fn runtime(
    mode: NetworkOpsOperatingMode,
    ledger_master_runtime: Arc<app::AppLedgerMasterRuntime>,
) -> AppNetworkOpsRuntime {
    AppNetworkOpsRuntime::new(
        Arc::new(SharedNetworkOpsState::new(mode)),
        ledger_master_runtime,
        Arc::new(HashRouter::new(HashRouterSetup::default())),
        Arc::new(app::TransactionMaster::new()),
        Arc::new(SharedLedgerMasterState::new(Arc::new(TimeKeeper::new()))),
        Arc::new(TimeKeeper::new()),
    )
}

#[test]
fn app_network_ops_runtime_preprocess_canonicalizes_cached_valid_transaction() {
    let ledger_master_runtime = Arc::new(app::AppLedgerMasterRuntime::default());
    let runtime = runtime(
        NetworkOpsOperatingMode::Tracking,
        Arc::clone(&ledger_master_runtime),
    );
    let tx = payment_tx(
        account("1010101010101010101010101010101010101010"),
        account("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
        1,
        None,
        10,
    );
    let txid = tx.get_transaction_id();
    assert!(
        runtime
            .hash_router()
            .set_flags(txid, HashRouterFlags::PRIVATE2 | HashRouterFlags::PRIVATE4)
    );

    let mut cached = Arc::new(Mutex::new(Transaction::new(Arc::clone(&tx))));
    runtime.transaction_master().canonicalize(&mut cached);

    let mut incoming = Arc::new(Mutex::new(Transaction::new(Arc::clone(&tx))));
    let report = runtime.preprocess_transaction(&mut incoming);

    assert_eq!(
        report,
        AppNetworkOpsPreprocessReport {
            accepted: true,
            decision: NetworkOpsPreprocessDecision::Continue,
            transaction_id: txid,
            status: TransStatus::NEW,
            result: Ter::TEM_UNCERTAIN,
            canonicalized: true,
        }
    );
    assert!(Arc::ptr_eq(&incoming, &cached));
}

#[test]
fn app_network_ops_runtime_process_async_stages_valid_transaction() {
    let ledger_master_runtime = Arc::new(app::AppLedgerMasterRuntime::default());
    let runtime = runtime(
        NetworkOpsOperatingMode::Tracking,
        Arc::clone(&ledger_master_runtime),
    );
    let tx = payment_tx(
        account("2020202020202020202020202020202020202020"),
        account("BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB"),
        1,
        None,
        10,
    );
    let txid = tx.get_transaction_id();
    assert!(
        runtime
            .hash_router()
            .set_flags(txid, HashRouterFlags::PRIVATE2 | HashRouterFlags::PRIVATE4)
    );

    let mut shared = Arc::new(Mutex::new(Transaction::new(tx)));
    let report = runtime.process_transaction(&mut shared, false, false, false, || false, || {});

    assert_eq!(
        report,
        AppNetworkOpsProcessReport {
            preprocess: AppNetworkOpsPreprocessReport {
                accepted: true,
                decision: NetworkOpsPreprocessDecision::Continue,
                transaction_id: txid,
                status: TransStatus::NEW,
                result: Ter::TEM_UNCERTAIN,
                canonicalized: true,
            },
            dispatch: NetworkOpsProcessDispatch::Async,
            async_dispatch: Some(NetworkOpsAsyncDispatch::Enqueued),
            sync_dispatch: None,
            pending_transactions: 1,
            dispatch_state: NetworkOpsDispatchState::None,
        }
    );
    assert!(
        shared
            .lock()
            .expect("transaction mutex must not be poisoned")
            .get_applying()
    );
    assert_eq!(runtime.pending_transaction_count(), 1);
}

#[test]
fn application_root_submit_transaction_queues_process_job_and_stages_pending_transaction() {
    let mut root = ApplicationRoot::new(0).expect("root shell should build");
    let runtime = root.attach_default_network_ops_runtime();
    let tx = payment_tx(
        account("3030303030303030303030303030303030303030"),
        account("CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC"),
        1,
        None,
        10,
    );
    let txid = tx.get_transaction_id();
    assert!(
        runtime
            .hash_router()
            .set_flags(txid, HashRouterFlags::PRIVATE2 | HashRouterFlags::PRIVATE4)
    );

    let report = root
        .submit_transaction_to_network_ops(Arc::clone(&tx))
        .expect("network ops runtime should be attached");

    assert_eq!(
        report,
        AppNetworkOpsSubmitReport {
            outcome: NetworkOpsSubmitFlowOutcome::Queued,
            transaction_id: txid,
            process_job_added: true,
        }
    );
    assert_eq!(root.network_ops_pending_transaction_count(), Some(0));

    root.job_queue().run_until_idle();

    assert_eq!(root.network_ops_pending_transaction_count(), Some(1));
    let cached = root
        .fetch_cached_transaction(&txid)
        .expect("submitted transaction should be canonicalized into the root cache");
    assert!(
        cached
            .lock()
            .expect("transaction mutex must not be poisoned")
            .get_applying()
    );
}

#[test]
fn app_network_ops_runtime_applies_held_transactions_into_pending_queue() {
    let ledger_master_runtime = Arc::new(app::AppLedgerMasterRuntime::default());
    let runtime = runtime(
        NetworkOpsOperatingMode::Full,
        Arc::clone(&ledger_master_runtime),
    );
    let source = account("8888888888888888888888888888888888888888");
    let first = payment_tx(
        source,
        account("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
        1,
        None,
        10,
    );
    let second = payment_tx(
        source,
        account("BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB"),
        2,
        None,
        11,
    );

    ledger_master_runtime.add_held_sttx(Arc::clone(&second));
    ledger_master_runtime.add_held_sttx(Arc::clone(&first));

    let syncs = Mutex::new(Vec::new());
    let outcome =
        runtime.apply_held_transactions_to_queue(SHAMapHash::new(Uint256::from_u64(111)), |sync| {
            syncs
                .lock()
                .expect("sync mutex must not be poisoned")
                .push(sync);
        });

    assert_eq!(
        outcome,
        AppNetworkOpsApplyHeldOutcome {
            drained_count: 2,
            process_outcome: Some(NetworkOpsTransactionSetOutcome::SyncBatch { added_count: 2 }),
        }
    );
    assert_eq!(runtime.pending_transaction_count(), 2);
    assert_eq!(runtime.submit_held_count(), 0);
    assert_eq!(runtime.dispatch_state(), NetworkOpsDispatchState::None);
    assert_eq!(
        syncs.into_inner().expect("sync mutex must not be poisoned"),
        vec![NetworkOpsProcessSetOwnerSync {
            added_count: 2,
            had_pending_before: false,
            has_applying_after_merge: true,
        }]
    );
}

#[test]
fn app_network_ops_runtime_promotes_included_transaction_into_submit_held() {
    let ledger_master_runtime = Arc::new(app::AppLedgerMasterRuntime::default());
    let runtime = runtime(
        NetworkOpsOperatingMode::Tracking,
        Arc::clone(&ledger_master_runtime),
    );
    let source = account("9999999999999999999999999999999999999999");
    let current = payment_tx(
        source,
        account("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
        5,
        None,
        10,
    );
    let next = payment_tx(
        source,
        account("BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB"),
        6,
        None,
        11,
    );
    let ticket = payment_tx(
        source,
        account("CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC"),
        0,
        Some(2),
        12,
    );

    ledger_master_runtime.add_held_sttx(Arc::clone(&next));
    ledger_master_runtime.add_held_sttx(Arc::clone(&ticket));

    let current = Arc::new(Mutex::new(Transaction::new(Arc::clone(&current))));
    assert_eq!(runtime.promote_included_transaction(&current), 2);
    assert_eq!(runtime.pending_transaction_count(), 0);
    assert_eq!(runtime.submit_held_count(), 2);
}

#[test]
fn app_network_ops_runtime_apply_pending_keeps_cpp_owner_side_effect_order() {
    let ledger_master_runtime = Arc::new(app::AppLedgerMasterRuntime::default());
    let runtime = runtime(
        NetworkOpsOperatingMode::Tracking,
        Arc::clone(&ledger_master_runtime),
    );
    let source = account("1212121212121212121212121212121212121212");
    let current = payment_tx(
        source,
        account("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
        5,
        None,
        10,
    );
    let next = payment_tx(
        source,
        account("BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB"),
        6,
        None,
        11,
    );
    let ticket = payment_tx(
        source,
        account("CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC"),
        0,
        Some(2),
        12,
    );

    ledger_master_runtime.add_held_sttx(Arc::clone(&next));
    ledger_master_runtime.add_held_sttx(Arc::clone(&ticket));

    let current = Arc::new(Mutex::new(Transaction::new(Arc::clone(&current))));
    assert!(runtime.stage_transaction(Arc::clone(&current), true, true, false));

    let relayed = Mutex::new(Vec::new());
    let report = runtime
        .apply_pending_with(
            700,
            Some(701),
            |_tx, flags| {
                assert_eq!(flags, ApplyFlags::UNLIMITED);
                ApplyResult::new(Ter::TES_SUCCESS, true, false)
            },
            || {},
            |_tx, _result| {},
            |_tx| {},
            |_tx| false,
            |_tx| Some(9u32),
            |tx, deferred, skip| {
                relayed
                    .lock()
                    .expect("relay mutex must not be poisoned")
                    .push((
                        tx.lock()
                            .expect("transaction mutex must not be poisoned")
                            .get_id(),
                        deferred,
                        skip,
                    ));
            },
            |_tx| NetworkOpsCurrentLedgerState {
                fee: XRPAmount::from_drops(25),
                account_seq: 11,
                available_seq: 13,
            },
        )
        .expect("pending batch should apply");

    let current_id = current
        .lock()
        .expect("transaction mutex must not be poisoned")
        .get_id();
    assert_eq!(
        report,
        AppNetworkOpsApplyReport {
            start: NetworkOpsApplyBatchStart {
                taken_transactions: 1,
                dispatch_state: NetworkOpsDispatchState::Running,
            },
            changed: true,
            fee_change_reported: true,
            entries: vec![AppNetworkOpsAppliedTransactionReport {
                transaction_id: current_id,
                result: Ter::TES_SUCCESS,
                applied: true,
                preamble: NetworkOpsApplyResultPreamble {
                    published: true,
                    malformed: false,
                },
                status_branch: NetworkOpsApplyStatusBranch::Included,
                retry_hold_branch: None,
                local_kept: true,
                relay_branch: NetworkOpsRelayBranch::Relayed { deferred: false },
                current_ledger_state_set: true,
                final_status: TransStatus::INCLUDED,
                submit_result: SubmitResult {
                    applied: true,
                    broadcast: true,
                    queued: false,
                    kept: true,
                },
                current_ledger_state: Some(CurrentLedgerState::new(
                    701,
                    XRPAmount::from_drops(25),
                    11,
                    13,
                )),
            }],
            tail: NetworkOpsApplyBatchTail {
                cleared: 1,
                pending_transactions: 2,
                dispatch_state: NetworkOpsDispatchState::None,
            },
        }
    );
    assert_eq!(
        relayed
            .into_inner()
            .expect("relay mutex must not be poisoned"),
        vec![(current_id, false, 9u32)]
    );
    assert_eq!(ledger_master_runtime.get_local_tx_count(), 1);
    assert_eq!(runtime.pending_transaction_count(), 2);
}

#[test]
fn application_root_forwards_network_ops_apply_pending_owner_path() {
    let mut root = ApplicationRoot::new(1).expect("application root");
    let runtime = root.attach_default_network_ops_runtime();
    let source = account("3434343434343434343434343434343434343434");
    let queued = Arc::new(Mutex::new(Transaction::new(payment_tx(
        source,
        account("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
        1,
        None,
        10,
    ))));
    let retry = Arc::new(Mutex::new(Transaction::new(Arc::new(STTx::new(
        TxType::PAYMENT,
        |tx| {
            tx.set_account_id(get_field_by_symbol("sfAccount"), source);
            tx.set_account_id(
                get_field_by_symbol("sfDestination"),
                account("BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB"),
            );
            tx.set_field_amount(
                get_field_by_symbol("sfAmount"),
                STAmount::new_native(1_000_000, false),
            );
            tx.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::new_native(11, false),
            );
            tx.set_field_u32(get_field_by_symbol("sfSequence"), 2);
            tx.set_field_u32(get_field_by_symbol("sfLastLedgerSequence"), 402);
        },
    )))));
    let fail_hard = Arc::new(Mutex::new(Transaction::new(payment_tx(
        source,
        account("CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC"),
        3,
        None,
        12,
    ))));

    assert!(runtime.stage_transaction(Arc::clone(&queued), false, false, false));
    assert!(runtime.stage_transaction(Arc::clone(&retry), false, false, false));
    assert!(runtime.stage_transaction(Arc::clone(&fail_hard), false, true, true));

    let report = root
        .apply_network_ops_pending_with(
            400,
            Some(401),
            |tx, _flags| {
                let id = tx
                    .lock()
                    .expect("transaction mutex must not be poisoned")
                    .get_id();
                if id
                    == queued
                        .lock()
                        .expect("transaction mutex must not be poisoned")
                        .get_id()
                {
                    return ApplyResult::new(Ter::TER_QUEUED, false, false);
                }
                if id
                    == retry
                        .lock()
                        .expect("transaction mutex must not be poisoned")
                        .get_id()
                {
                    return ApplyResult::new(Ter::TER_RETRY, false, false);
                }
                ApplyResult::new(Ter::TER_RETRY, false, false)
            },
            || {},
            |_tx, _result| {},
            |_tx| {},
            |_tx| false,
            |_tx| None::<u32>,
            |_tx, _deferred, _skip| {},
            |_tx| NetworkOpsCurrentLedgerState {
                fee: XRPAmount::from_drops(30),
                account_seq: 20,
                available_seq: 21,
            },
        )
        .expect("root should forward to network ops runtime");

    assert_eq!(report.entries.len(), 3);
    assert_eq!(
        report.entries[0].status_branch,
        NetworkOpsApplyStatusBranch::Queued
    );
    assert_eq!(
        report.entries[1].retry_hold_branch,
        Some(NetworkOpsRetryHoldBranch::Held {
            ledgers_left: Some(2)
        })
    );
    assert_eq!(
        report.entries[2].retry_hold_branch,
        Some(NetworkOpsRetryHoldBranch::FailHard)
    );
    assert_eq!(runtime.pending_transaction_count(), 0);
    assert_eq!(runtime.submit_held_count(), 0);
}
