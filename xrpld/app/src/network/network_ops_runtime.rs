//! App-owned `NetworkOPs` runtime shell above the landed control-flow helpers.
//!
//! This keeps the current finishable owner graph in one runtime object:
//! - shared pending/applying batch state,
//! - app-owned wiring to the shared `NetworkOPs` mode state,
//! - held-transaction reinjection through the app-owned `LedgerMaster` bridge,
//! - and the current `processTransactionSet(...)` / batch-tail queue handoff.
//!
//! The broader open-ledger, TxQ, HashRouter, and relay graph is still a
//! separate slice, so this owner only claims the parts that are now backed by
//! real Rust runtime state instead of detached helpers.

use crate::consensus_ledger_from_ledger;
use crate::ledger::ledger_master_runtime::AppLedgerMasterRuntime;
use crate::ledger::ledger_master_state::SharedLedgerMasterState;
use crate::network::network_ops::{
    NetworkOpsApplyBatchEntry, NetworkOpsApplyBatchStart, NetworkOpsApplyBatchTail,
    NetworkOpsApplyResultPreamble, NetworkOpsApplyStatusBranch, NetworkOpsAsyncDispatch,
    NetworkOpsCurrentLedgerState, NetworkOpsDispatchState, NetworkOpsPreprocessDecision,
    NetworkOpsProcessDispatch, NetworkOpsProcessSetFrontDecision, NetworkOpsProcessSetOwnerSync,
    NetworkOpsRelayBranch, NetworkOpsRetryHoldBranch, NetworkOpsRuntimeState,
    NetworkOpsSubmitFlowOutcome, NetworkOpsSyncDispatch, NetworkOpsTransactionSetOutcome,
    SharedNetworkOpsState, run_networkops_apply_result_preamble,
    run_networkops_apply_status_branch, run_networkops_apply_txq_batch, run_networkops_local_keep,
    run_networkops_preprocess_transaction_gate, run_networkops_process_transaction,
    run_networkops_relay_branch, run_networkops_retry_hold_branch,
    run_networkops_set_current_ledger_state, run_networkops_submit_transaction,
    run_networkops_submit_transaction_gate,
};
use crate::runtime::component_runtime::AppConsensusRuntime;
use crate::tx_queue::transaction::{CurrentLedgerState, SubmitResult, TransStatus, Transaction};
use crate::tx_queue::transaction_master::{SharedTransaction, TransactionMaster};
use basics::base_uint::Uint256;
use basics::sha_map_hash::SHAMapHash;
use consensus::RclCxTx;
use ledger::{CanonicalTXSet, Ledger};
use protocol::{
    Rules, STTx, Ter, XRPAmount, get_field_by_symbol, passes_local_checks, tfInnerBatchTxn,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tx::{
    ApplyFlags, ApplyResult, CheckValidityFacts, CheckValidityResult,
    run_check_validity_with_flag_cache,
};
use xrpl_core::{HashRouter, HashRouterFlags};

pub const APP_NETWORKOPS_MAX_POPPED_TRANSACTIONS: usize = 10;
pub type AppNetworkOpsPendingTransaction = NetworkOpsApplyBatchEntry<SharedTransaction>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppNetworkOpsRuntimeSnapshot {
    pub pending_transactions: usize,
    pub submit_held: usize,
    pub dispatch_state: NetworkOpsDispatchState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppNetworkOpsApplyHeldOutcome {
    pub drained_count: usize,
    pub process_outcome: Option<NetworkOpsTransactionSetOutcome>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppNetworkOpsPreprocessReport {
    pub accepted: bool,
    pub decision: NetworkOpsPreprocessDecision,
    pub transaction_id: Uint256,
    pub status: TransStatus,
    pub result: Ter,
    pub canonicalized: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppNetworkOpsProcessReport {
    pub preprocess: AppNetworkOpsPreprocessReport,
    pub dispatch: NetworkOpsProcessDispatch,
    pub async_dispatch: Option<NetworkOpsAsyncDispatch>,
    pub sync_dispatch: Option<NetworkOpsSyncDispatch>,
    pub pending_transactions: usize,
    pub dispatch_state: NetworkOpsDispatchState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppNetworkOpsSubmitReport {
    pub outcome: NetworkOpsSubmitFlowOutcome,
    pub transaction_id: Uint256,
    pub process_job_added: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppNetworkOpsAppliedTransactionReport {
    pub transaction_id: Uint256,
    pub result: Ter,
    pub applied: bool,
    pub preamble: NetworkOpsApplyResultPreamble,
    pub status_branch: NetworkOpsApplyStatusBranch,
    pub retry_hold_branch: Option<NetworkOpsRetryHoldBranch>,
    pub local_kept: bool,
    pub relay_branch: NetworkOpsRelayBranch,
    pub current_ledger_state_set: bool,
    pub final_status: TransStatus,
    pub submit_result: SubmitResult,
    pub current_ledger_state: Option<CurrentLedgerState>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppNetworkOpsApplyReport {
    pub start: NetworkOpsApplyBatchStart,
    pub changed: bool,
    pub fee_change_reported: bool,
    pub entries: Vec<AppNetworkOpsAppliedTransactionReport>,
    pub tail: NetworkOpsApplyBatchTail,
}

pub struct AppNetworkOpsRuntime {
    state: Mutex<NetworkOpsRuntimeState<AppNetworkOpsPendingTransaction>>,
    network_ops_state: Arc<SharedNetworkOpsState>,
    ledger_master_runtime: Mutex<Arc<AppLedgerMasterRuntime>>,
    hash_router: Arc<HashRouter>,
    transaction_master: Arc<TransactionMaster>,
    ledger_master_state: Arc<SharedLedgerMasterState>,
    consensus_bootstrap_started: AtomicBool,
}

impl std::fmt::Debug for AppNetworkOpsRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppNetworkOpsRuntime")
            .field("snapshot", &self.snapshot())
            .field(
                "held_transactions",
                &self.ledger_master_runtime().held_transaction_count(),
            )
            .field("hash_router_entries", &self.hash_router.entry_count())
            .field(
                "transaction_cache_size",
                &self.transaction_master.get_cache().size(),
            )
            .field(
                "validated_ledger_seq",
                &self.ledger_master_state.validated_ledger_seq(),
            )
            .finish()
    }
}

impl AppNetworkOpsRuntime {
    pub fn new(
        network_ops_state: Arc<SharedNetworkOpsState>,
        ledger_master_runtime: Arc<AppLedgerMasterRuntime>,
        hash_router: Arc<HashRouter>,
        transaction_master: Arc<TransactionMaster>,
        ledger_master_state: Arc<SharedLedgerMasterState>,
    ) -> Self {
        Self::with_state(
            network_ops_state,
            ledger_master_runtime,
            hash_router,
            transaction_master,
            ledger_master_state,
            NetworkOpsRuntimeState::default(),
        )
    }

    pub fn with_state(
        network_ops_state: Arc<SharedNetworkOpsState>,
        ledger_master_runtime: Arc<AppLedgerMasterRuntime>,
        hash_router: Arc<HashRouter>,
        transaction_master: Arc<TransactionMaster>,
        ledger_master_state: Arc<SharedLedgerMasterState>,
        state: NetworkOpsRuntimeState<AppNetworkOpsPendingTransaction>,
    ) -> Self {
        Self {
            state: Mutex::new(state),
            network_ops_state,
            ledger_master_runtime: Mutex::new(ledger_master_runtime),
            hash_router,
            transaction_master,
            ledger_master_state,
            consensus_bootstrap_started: AtomicBool::new(false),
        }
    }

    pub fn network_ops_state(&self) -> Arc<SharedNetworkOpsState> {
        Arc::clone(&self.network_ops_state)
    }

    pub fn ledger_master_runtime(&self) -> Arc<AppLedgerMasterRuntime> {
        Arc::clone(
            &self
                .ledger_master_runtime
                .lock()
                .expect("network ops ledger master runtime mutex must not be poisoned"),
        )
    }

    pub fn set_ledger_master_runtime(
        &self,
        ledger_master_runtime: Arc<AppLedgerMasterRuntime>,
    ) -> Arc<AppLedgerMasterRuntime> {
        let mut current = self
            .ledger_master_runtime
            .lock()
            .expect("network ops ledger master runtime mutex must not be poisoned");
        std::mem::replace(&mut *current, ledger_master_runtime)
    }

    pub fn hash_router(&self) -> Arc<HashRouter> {
        Arc::clone(&self.hash_router)
    }

    pub fn transaction_master(&self) -> Arc<TransactionMaster> {
        Arc::clone(&self.transaction_master)
    }

    pub fn ledger_master_state(&self) -> Arc<SharedLedgerMasterState> {
        Arc::clone(&self.ledger_master_state)
    }

    pub fn snapshot(&self) -> AppNetworkOpsRuntimeSnapshot {
        let state = self
            .state
            .lock()
            .expect("network ops runtime state mutex must not be poisoned");
        AppNetworkOpsRuntimeSnapshot {
            pending_transactions: state.pending_transactions().len(),
            submit_held: state.submit_held().len(),
            dispatch_state: state.dispatch_state(),
        }
    }

    pub fn pending_transaction_count(&self) -> usize {
        self.snapshot().pending_transactions
    }

    /// Clear all pending transactions (called after consensus to prevent
    /// re-applying already-validated txs to the new open ledger).
    pub fn clear_pending_transactions(&self) {
        let mut state = self
            .state
            .lock()
            .expect("network ops runtime state mutex must not be poisoned");
        state.pending_transactions_mut().clear();
    }

    pub fn submit_held_count(&self) -> usize {
        self.snapshot().submit_held
    }

    pub fn dispatch_state(&self) -> NetworkOpsDispatchState {
        self.snapshot().dispatch_state
    }

    pub fn reset_consensus_bootstrap(&self) {
        self.consensus_bootstrap_started
            .store(false, Ordering::Release);
    }

    /// Start a consensus round on `ledger` (its id is used as the new
    /// round's previous-ledger id). Unlike
    /// [`NetworkOpsRuntime::maybe_begin_consensus_from_validated`], this is
    /// NOT gated by the one-shot `consensus_bootstrap_started` flag -- it
    /// is meant to be called once per round, every time a ledger is
    /// accepted, matching the reference's `NetworkOPsImp::endConsensus`
    /// unconditionally calling `beginConsensus` (which in turn calls
    /// `Consensus::startRound`) after `RCLConsensus::Adaptor::onAccept`
    /// finishes building the new ledger. Without this repeated call,
    /// `Consensus::timerEntry` would tick forever on a round already in
    /// the `Accepted` phase (a no-op, per `timerEntry`'s early return) and
    /// the chain would permanently stall after its first ledger.
    pub fn start_next_round(&self, consensus_runtime: &AppConsensusRuntime, ledger: Arc<Ledger>) {
        let now = current_net_time();
        let prev_id = *ledger.header().hash.as_uint256();
        let prev_cx = consensus_ledger_from_ledger(&ledger);
        let runtime = consensus_runtime.clone();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("network ops start-next-round runtime");
        rt.block_on(async {
            runtime.start_round(now, prev_id, prev_cx).await;
        });
    }

    pub fn maybe_begin_consensus_from_validated(
        &self,
        consensus_runtime: &AppConsensusRuntime,
        ledger: Arc<Ledger>,
    ) -> bool {
        if self
            .consensus_bootstrap_started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return false;
        }

        let now = current_net_time();
        let prev_id = *ledger.header().hash.as_uint256();
        let prev_cx = consensus_ledger_from_ledger(&ledger);
        let runtime = consensus_runtime.clone();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("network ops bootstrap consensus runtime");
        rt.block_on(async {
            runtime.start_round(now, prev_id, prev_cx).await;
        });
        true
    }

    pub fn handle_map_complete(
        &self,
        consensus_runtime: &AppConsensusRuntime,
        map_hash: Uint256,
        set: Arc<shamap::sync::SyncTree>,
    ) {
        let mut txs = Vec::<RclCxTx>::new();
        let mut fetch = |_hash: basics::sha_map_hash::SHAMapHash| -> Option<
            basics::memory::intrusive_pointer::SharedIntrusive<
                shamap::nodes::tree_node::SHAMapTreeNode,
            >,
        > { None };
        let _ = set.visit_leaves(&mut fetch, &mut |item: &shamap::item::SHAMapItem| {
            txs.push(RclCxTx { id: item.key() });
        });

        let now = current_net_time();
        let runtime = consensus_runtime.clone();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("network ops mapComplete runtime");
        rt.block_on(async {
            runtime.got_tx_set(now, txs).await;
        });

        tracing::debug!(target: "network",
            "[mapComplete] gotTxSet hash={:02x}{:02x}{:02x}{:02x}",
            map_hash.data()[0],
            map_hash.data()[1],
            map_hash.data()[2],
            map_hash.data()[3],
        );
    }

    pub fn handle_peer_proposal(
        &self,
        consensus_runtime: &AppConsensusRuntime,
        public_key: protocol::PublicKey,
        signature: Vec<u8>,
        suppression_id: Uint256,
        proposal: consensus::ConsensusProposal<protocol::PublicKey, Uint256, Uint256>,
    ) -> bool {
        let now = current_net_time();
        let runtime = consensus_runtime.clone();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("network ops peer proposal runtime");
        rt.block_on(async {
            runtime
                .peer_proposal(now, public_key, signature, suppression_id, proposal)
                .await
        })
    }

    pub fn handle_consensus_timer(&self, consensus_runtime: &AppConsensusRuntime) {
        let now = current_net_time();
        let runtime = consensus_runtime.clone();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("network ops consensus timer runtime");
        rt.block_on(async {
            runtime.timer_tick(now, true).await;
        });
    }

    /// Drain pending proposals without running timer_entry.
    /// Called on every 50ms bootstrap iteration for low-latency proposal feeding.
    pub fn drain_proposals(&self, consensus_runtime: &AppConsensusRuntime) {
        let now = current_net_time();
        let runtime = consensus_runtime.clone();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("network ops drain proposals runtime");
        rt.block_on(async {
            runtime.timer_tick(now, false).await;
        });
    }

    pub fn check_validity(&self, transaction: &STTx, rules: &Rules) -> CheckValidityResult {
        let txid = transaction.get_transaction_id();
        run_check_validity_with_flag_cache(
            validity_facts(transaction),
            rules,
            || self.hash_router.get_flags(txid),
            |flags| {
                if flags != HashRouterFlags::UNDEFINED {
                    let _ = self.hash_router.set_flags(txid, flags);
                }
            },
            || transaction.check_sign(rules),
            || passes_local_checks(transaction),
        )
    }

    pub fn preprocess_transaction(
        &self,
        transaction: &mut SharedTransaction,
    ) -> AppNetworkOpsPreprocessReport {
        let sttx = read_transaction(transaction, |tx| Arc::clone(tx.get_s_transaction()));
        let rules = self.current_rules();
        let txid = sttx.get_transaction_id();
        let decision = run_networkops_preprocess_transaction_gate(
            sttx.is_flag(tfInnerBatchTxn),
            rules.enabled(&protocol::feature_batch()),
            || self.hash_router.get_flags(txid),
            || self.check_validity(sttx.as_ref(), &rules),
        );
        let mut canonicalized = false;
        let accepted = match decision {
            NetworkOpsPreprocessDecision::Continue => {
                self.transaction_master.canonicalize(transaction);
                canonicalized = true;
                true
            }
            NetworkOpsPreprocessDecision::RejectCachedBad => {
                mutate_transaction(transaction, |tx| {
                    tx.set_status(TransStatus::INVALID);
                    tx.set_result(Ter::TEM_BAD_SIGNATURE);
                });
                false
            }
            NetworkOpsPreprocessDecision::RejectInnerBatch => {
                mutate_transaction(transaction, |tx| {
                    tx.set_status(TransStatus::INVALID);
                    tx.set_result(Ter::TEM_INVALID_FLAG);
                });
                let _ = self.hash_router.set_flags(txid, HashRouterFlags::BAD);
                false
            }
            NetworkOpsPreprocessDecision::RejectBadSignature(_) => {
                mutate_transaction(transaction, |tx| {
                    tx.set_status(TransStatus::INVALID);
                    tx.set_result(Ter::TEM_BAD_SIGNATURE);
                });
                let _ = self.hash_router.set_flags(txid, HashRouterFlags::BAD);
                false
            }
        };

        read_transaction(transaction, |tx| AppNetworkOpsPreprocessReport {
            accepted,
            decision: decision.clone(),
            transaction_id: tx.get_id(),
            status: tx.get_status(),
            result: tx.get_result(),
            canonicalized,
        })
    }

    pub fn process_transaction(
        &self,
        transaction: &mut SharedTransaction,
        admin: bool,
        local: bool,
        fail_hard: bool,
        add_batch_job: impl FnOnce() -> bool,
        run_sync_batch: impl FnOnce(),
    ) -> AppNetworkOpsProcessReport {
        let preprocess = self.preprocess_transaction(transaction);
        let mut add_batch_job = Some(add_batch_job);
        let mut run_sync_batch = Some(run_sync_batch);
        let transaction_for_async = Arc::clone(transaction);
        let transaction_for_sync = Arc::clone(transaction);
        let mut async_dispatch = None;
        let mut sync_dispatch = None;

        let dispatch = run_networkops_process_transaction(
            || preprocess.accepted,
            local,
            || {
                sync_dispatch = Some(self.stage_transaction_sync(
                    transaction_for_sync,
                    admin,
                    local,
                    fail_hard,
                ));
                if let Some(run_sync_batch) = run_sync_batch.take() {
                    run_sync_batch();
                }
            },
            || {
                async_dispatch = Some(
                    self.stage_transaction_async(
                        transaction_for_async,
                        admin,
                        local,
                        fail_hard,
                        add_batch_job
                            .take()
                            .expect("async add_batch_job must only be consumed once"),
                    ),
                );
            },
        );

        let snapshot = self.snapshot();
        AppNetworkOpsProcessReport {
            preprocess,
            dispatch,
            async_dispatch,
            sync_dispatch,
            pending_transactions: snapshot.pending_transactions,
            dispatch_state: snapshot.dispatch_state,
        }
    }

    pub fn submit_transaction(
        &self,
        transaction: Arc<STTx>,
        enqueue_process_transaction: impl FnOnce(SharedTransaction) -> bool,
    ) -> AppNetworkOpsSubmitReport {
        let txid = transaction.get_transaction_id();
        let rules = self.current_rules_for_submit();
        let gate_transaction = Arc::clone(&transaction);
        let queued_transaction = Arc::clone(&transaction);
        let mut process_job_added = false;

        let outcome = run_networkops_submit_transaction(
            self.network_ops_state.need_network_ledger(),
            || {
                run_networkops_submit_transaction_gate(
                    gate_transaction.is_flag(tfInnerBatchTxn),
                    rules.enabled(&protocol::feature_batch()),
                    || self.hash_router.get_flags(txid),
                    || Ok(self.check_validity(gate_transaction.as_ref(), &rules)),
                )
            },
            || {},
            || {
                process_job_added =
                    enqueue_process_transaction(shared_transaction(queued_transaction));
            },
        );

        AppNetworkOpsSubmitReport {
            outcome,
            transaction_id: txid,
            process_job_added,
        }
    }

    pub fn process_transaction_set_entrypoint<Input>(
        &self,
        make_load_event: impl FnOnce(),
        inputs: impl IntoIterator<Item = Input>,
        build_decision: impl FnMut(Input) -> NetworkOpsProcessSetFrontDecision<SharedTransaction>,
        trace_invalid_reason: impl FnMut(&str),
        set_bad_flag: impl FnMut(),
        run_sync_batch: impl FnMut(NetworkOpsProcessSetOwnerSync),
    ) -> NetworkOpsTransactionSetOutcome {
        let mut state = self
            .state
            .lock()
            .expect("network ops runtime state mutex must not be poisoned");

        state.process_transaction_set_entrypoint(
            make_load_event,
            inputs,
            build_decision,
            trace_invalid_reason,
            set_bad_flag,
            |tx| transaction_applying(tx),
            |tx| {
                set_transaction_applying(&tx);
                pending_transaction(tx, false, false, false)
            },
            |tx| pending_transaction_applying(tx),
            run_sync_batch,
        )
    }

    pub fn process_transaction_set(
        &self,
        mut set: CanonicalTXSet,
        run_sync_batch: impl FnMut(NetworkOpsProcessSetOwnerSync),
    ) -> NetworkOpsTransactionSetOutcome {
        self.process_transaction_set_entrypoint(
            || {},
            set.drain_ordered(),
            |tx| {
                let mut shared = shared_transaction(tx);
                let preprocess = self.preprocess_transaction(&mut shared);
                if preprocess.accepted {
                    NetworkOpsProcessSetFrontDecision::Candidate(shared)
                } else {
                    NetworkOpsProcessSetFrontDecision::RejectPreprocess
                }
            },
            |_reason| {},
            || {},
            run_sync_batch,
        )
    }

    pub fn apply_held_transactions(
        &self,
        next_open_ledger_parent_hash: SHAMapHash,
        build_decision: impl FnMut(Arc<STTx>) -> NetworkOpsProcessSetFrontDecision<SharedTransaction>,
        trace_invalid_reason: impl FnMut(&str),
        set_bad_flag: impl FnMut(),
        run_sync_batch: impl FnMut(NetworkOpsProcessSetOwnerSync),
    ) -> AppNetworkOpsApplyHeldOutcome {
        let mut build_decision = build_decision;
        let mut trace_invalid_reason = trace_invalid_reason;
        let mut set_bad_flag = set_bad_flag;
        let mut run_sync_batch = run_sync_batch;
        let mut process_outcome = None;

        let drained_count = self.ledger_master_runtime().apply_held_transactions(
            next_open_ledger_parent_hash,
            |mut set| {
                process_outcome = Some(self.process_transaction_set_entrypoint(
                    || {},
                    set.drain_ordered(),
                    |tx| build_decision(tx),
                    |reason| trace_invalid_reason(reason),
                    || set_bad_flag(),
                    |sync| run_sync_batch(sync),
                ));
            },
        );

        AppNetworkOpsApplyHeldOutcome {
            drained_count,
            process_outcome,
        }
    }

    pub fn apply_held_transactions_to_queue(
        &self,
        next_open_ledger_parent_hash: SHAMapHash,
        run_sync_batch: impl FnMut(NetworkOpsProcessSetOwnerSync),
    ) -> AppNetworkOpsApplyHeldOutcome {
        self.apply_held_transactions(
            next_open_ledger_parent_hash,
            |tx| NetworkOpsProcessSetFrontDecision::Candidate(shared_transaction(tx)),
            |_reason| {},
            || {},
            run_sync_batch,
        )
    }

    pub fn promote_included_transaction(&self, transaction: &SharedTransaction) -> usize {
        let current = Arc::clone(
            transaction
                .lock()
                .expect("transaction mutex must not be poisoned")
                .get_s_transaction(),
        );

        let ledger_master_runtime = self.ledger_master_runtime();
        let mut staged = Vec::new();
        for _ in 0..APP_NETWORKOPS_MAX_POPPED_TRANSACTIONS {
            let Some(next) = ledger_master_runtime.pop_acct_transaction(&current) else {
                break;
            };

            let transaction = shared_transaction(next);
            if transaction_applying(&transaction) {
                break;
            }
            set_transaction_applying(&transaction);
            staged.push(transaction);
        }

        if staged.is_empty() {
            return 0;
        }

        let mut state = self
            .state
            .lock()
            .expect("network ops runtime state mutex must not be poisoned");
        for transaction in staged {
            state.push_submit_held(pending_transaction(transaction, false, false, false));
        }

        state.submit_held().len()
    }

    pub fn stage_transaction(
        &self,
        transaction: SharedTransaction,
        admin: bool,
        local: bool,
        fail_hard: bool,
    ) -> bool {
        if transaction_applying(&transaction) {
            return false;
        }

        set_transaction_applying(&transaction);
        let mut state = self
            .state
            .lock()
            .expect("network ops runtime state mutex must not be poisoned");
        state.pending_transactions_mut().push(pending_transaction(
            transaction,
            admin,
            local,
            fail_hard,
        ));
        true
    }

    pub fn begin_apply_batch(
        &self,
    ) -> (
        Vec<AppNetworkOpsPendingTransaction>,
        NetworkOpsApplyBatchStart,
    ) {
        let mut state = self
            .state
            .lock()
            .expect("network ops runtime state mutex must not be poisoned");
        state.begin_apply_batch(|| {})
    }

    pub fn finish_apply_batch(
        &self,
        transactions: &[NetworkOpsApplyBatchEntry<SharedTransaction>],
    ) -> NetworkOpsApplyBatchTail {
        let mut state = self
            .state
            .lock()
            .expect("network ops runtime state mutex must not be poisoned");
        state.finish_apply_batch(
            transactions,
            || {},
            |tx| {
                tx.lock()
                    .expect("transaction mutex must not be poisoned")
                    .clear_applying()
            },
            || {},
        )
    }

    pub fn apply_pending_with<RelaySkip>(
        &self,
        current_ledger_index: u32,
        validated_ledger_index: Option<u32>,
        apply_tx: impl FnMut(&SharedTransaction, ApplyFlags) -> ApplyResult,
        report_fee_change: impl FnMut(),
        publish_proposed: impl FnMut(&SharedTransaction, Ter),
        set_bad_flag: impl FnMut(&SharedTransaction),
        set_held_flag: impl FnMut(&SharedTransaction) -> bool,
        should_relay: impl FnMut(&SharedTransaction) -> Option<RelaySkip>,
        relay: impl FnMut(&SharedTransaction, bool, RelaySkip),
        current_ledger_state: impl FnMut(
            &SharedTransaction,
        ) -> NetworkOpsCurrentLedgerState<XRPAmount, u32>,
    ) -> Option<AppNetworkOpsApplyReport> {
        let mut apply_tx = apply_tx;
        self.apply_pending_batch_with(
            current_ledger_index,
            validated_ledger_index,
            |transactions| run_networkops_apply_txq_batch(transactions, &mut apply_tx),
            report_fee_change,
            publish_proposed,
            set_bad_flag,
            set_held_flag,
            should_relay,
            relay,
            current_ledger_state,
        )
    }

    pub fn apply_pending_batch_with<RelaySkip>(
        &self,
        current_ledger_index: u32,
        validated_ledger_index: Option<u32>,
        apply_batch: impl FnOnce(&mut [AppNetworkOpsPendingTransaction]) -> bool,
        mut report_fee_change: impl FnMut(),
        publish_proposed: impl FnMut(&SharedTransaction, Ter),
        set_bad_flag: impl FnMut(&SharedTransaction),
        set_held_flag: impl FnMut(&SharedTransaction) -> bool,
        should_relay: impl FnMut(&SharedTransaction) -> Option<RelaySkip>,
        relay: impl FnMut(&SharedTransaction, bool, RelaySkip),
        current_ledger_state: impl FnMut(
            &SharedTransaction,
        ) -> NetworkOpsCurrentLedgerState<XRPAmount, u32>,
    ) -> Option<AppNetworkOpsApplyReport> {
        if self.pending_transaction_count() == 0 {
            return None;
        }

        let (mut transactions, start) = self.begin_apply_batch();
        let changed = apply_batch(&mut transactions);
        let fee_change_reported = if changed {
            report_fee_change();
            true
        } else {
            false
        };
        Some(self.finish_applied_pending_batch(
            transactions,
            start,
            changed,
            fee_change_reported,
            current_ledger_index,
            validated_ledger_index,
            publish_proposed,
            set_bad_flag,
            set_held_flag,
            should_relay,
            relay,
            current_ledger_state,
        ))
    }

    fn finish_applied_pending_batch<RelaySkip>(
        &self,
        mut transactions: Vec<AppNetworkOpsPendingTransaction>,
        start: NetworkOpsApplyBatchStart,
        changed: bool,
        fee_change_reported: bool,
        current_ledger_index: u32,
        validated_ledger_index: Option<u32>,
        publish_proposed: impl FnMut(&SharedTransaction, Ter),
        set_bad_flag: impl FnMut(&SharedTransaction),
        set_held_flag: impl FnMut(&SharedTransaction) -> bool,
        should_relay: impl FnMut(&SharedTransaction) -> Option<RelaySkip>,
        relay: impl FnMut(&SharedTransaction, bool, RelaySkip),
        current_ledger_state: impl FnMut(
            &SharedTransaction,
        ) -> NetworkOpsCurrentLedgerState<XRPAmount, u32>,
    ) -> AppNetworkOpsApplyReport {
        let ledger_master_runtime = self.ledger_master_runtime();
        let operating_mode_full = self.network_ops_state.is_full();
        let mut publish_proposed = publish_proposed;
        let mut set_bad_flag = set_bad_flag;
        let mut set_held_flag = set_held_flag;
        let mut should_relay = should_relay;
        let mut relay = relay;
        let mut current_ledger_state = current_ledger_state;
        let mut entries = Vec::with_capacity(transactions.len());

        for entry in transactions.iter_mut() {
            let preamble = run_networkops_apply_result_preamble(
                entry,
                |tx| mutate_transaction(tx, |transaction| transaction.clear_submit_result()),
                |tx, result| publish_proposed(tx, result),
                |tx| mutate_transaction(tx, |transaction| transaction.set_applied()),
                |tx, result| mutate_transaction(tx, |transaction| transaction.set_result(result)),
                |tx| {
                    set_bad_flag(tx);
                },
            );

            let status_branch = run_networkops_apply_status_branch(
                entry,
                |tx| {
                    let _ = self.promote_included_transaction(tx);
                },
                |tx| {
                    mutate_transaction(tx, |transaction| {
                        transaction.set_status(TransStatus::INCLUDED)
                    })
                },
                |tx| {
                    mutate_transaction(tx, |transaction| {
                        transaction.set_status(TransStatus::OBSOLETE)
                    })
                },
                |tx| {
                    mutate_transaction(tx, |transaction| transaction.set_status(TransStatus::HELD))
                },
                |tx| add_held_transaction(&ledger_master_runtime, tx),
                |tx| mutate_transaction(tx, |transaction| transaction.set_queued()),
                |tx| mutate_transaction(tx, |transaction| transaction.set_kept()),
                |tx| {
                    mutate_transaction(tx, |transaction| {
                        transaction.set_status(TransStatus::INVALID)
                    })
                },
            );

            let retry_hold_branch = if status_branch == NetworkOpsApplyStatusBranch::RetryCandidate
            {
                Some(run_networkops_retry_hold_branch(
                    entry,
                    current_ledger_index,
                    last_ledger_sequence(&entry.transaction),
                    |tx| set_held_flag(tx),
                    |tx| {
                        mutate_transaction(tx, |transaction| {
                            transaction.set_status(TransStatus::HELD)
                        })
                    },
                    |tx| add_held_transaction(&ledger_master_runtime, tx),
                    |tx| mutate_transaction(tx, |transaction| transaction.set_kept()),
                ))
            } else {
                None
            };

            let result = entry
                .result
                .expect("network ops apply report requires populated results");
            let local_kept = run_networkops_local_keep(
                entry,
                result,
                |tx| push_local_transaction(&ledger_master_runtime, current_ledger_index, tx),
                |tx| mutate_transaction(tx, |transaction| transaction.set_kept()),
            );
            let relay_branch = run_networkops_relay_branch(
                entry,
                operating_mode_full,
                result,
                inner_batch_flag_set(&entry.transaction),
                |tx| should_relay(tx),
                |tx, deferred, skip| relay(tx, deferred, skip),
                |tx| mutate_transaction(tx, |transaction| transaction.set_broadcast()),
            );
            let current_ledger_state_set = run_networkops_set_current_ledger_state(
                entry,
                validated_ledger_index,
                |tx| current_ledger_state(tx),
                |tx, validated_ledger, state| {
                    mutate_transaction(tx, |transaction| {
                        transaction.set_current_ledger_state(
                            validated_ledger,
                            state.fee,
                            state.account_seq,
                            state.available_seq,
                        )
                    })
                },
            );

            entries.push(read_transaction(&entry.transaction, |transaction| {
                AppNetworkOpsAppliedTransactionReport {
                    transaction_id: transaction.get_id(),
                    result,
                    applied: entry.applied,
                    preamble,
                    status_branch,
                    retry_hold_branch,
                    local_kept,
                    relay_branch,
                    current_ledger_state_set,
                    final_status: transaction.get_status(),
                    submit_result: transaction.get_submit_result(),
                    current_ledger_state: transaction.get_current_ledger_state(),
                }
            }));
        }

        let tail = self.finish_apply_batch(&transactions);
        AppNetworkOpsApplyReport {
            start,
            changed,
            fee_change_reported,
            entries,
            tail,
        }
    }

    fn current_rules(&self) -> Rules {
        self.ledger_master_state
            .closed_ledger()
            .or_else(|| self.ledger_master_state.validated_ledger())
            .map(|ledger| ledger.rules().clone())
            .unwrap_or_else(|| Rules::new(std::iter::empty::<Uint256>()))
    }

    fn current_rules_for_submit(&self) -> Rules {
        self.ledger_master_state
            .validated_ledger()
            .or_else(|| self.ledger_master_state.closed_ledger())
            .map(|ledger| ledger.rules().clone())
            .unwrap_or_else(|| Rules::new(std::iter::empty::<Uint256>()))
    }

    fn stage_transaction_async(
        &self,
        transaction: SharedTransaction,
        admin: bool,
        local: bool,
        fail_hard: bool,
        add_batch_job: impl FnOnce() -> bool,
    ) -> NetworkOpsAsyncDispatch {
        let queued_transaction = Arc::clone(&transaction);
        let applying_transaction = Arc::clone(&transaction);
        let mut state = self
            .state
            .lock()
            .expect("network ops runtime state mutex must not be poisoned");
        state.transaction_async(
            transaction_applying(&transaction),
            pending_transaction(queued_transaction, admin, local, fail_hard),
            || set_transaction_applying(&applying_transaction),
            add_batch_job,
        )
    }

    fn stage_transaction_sync(
        &self,
        transaction: SharedTransaction,
        admin: bool,
        local: bool,
        fail_hard: bool,
    ) -> NetworkOpsSyncDispatch {
        if transaction_applying(&transaction) {
            return NetworkOpsSyncDispatch::ExistingApplying;
        }

        set_transaction_applying(&transaction);
        let mut state = self
            .state
            .lock()
            .expect("network ops runtime state mutex must not be poisoned");
        state.pending_transactions_mut().push(pending_transaction(
            transaction,
            admin,
            local,
            fail_hard,
        ));
        NetworkOpsSyncDispatch::Staged
    }
}

fn current_net_time() -> basics::chrono::NetClockTimePoint {
    basics::chrono::NetClockTimePoint::new(
        (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .saturating_sub(946684800)) as u32,
    )
}

fn shared_transaction(transaction: Arc<STTx>) -> SharedTransaction {
    Arc::new(Mutex::new(Transaction::new(transaction)))
}

fn validity_facts(transaction: &STTx) -> CheckValidityFacts {
    let txn_signature = get_field_by_symbol("sfTxnSignature");
    let signers = get_field_by_symbol("sfSigners");
    CheckValidityFacts {
        inner_batch_flag_set: transaction.is_flag(tfInnerBatchTxn),
        txn_signature_present: transaction.is_field_present(txn_signature),
        signing_pub_key_empty: transaction.get_signing_pub_key().is_empty(),
        signers_present: transaction.is_field_present(signers),
    }
}

fn pending_transaction(
    transaction: SharedTransaction,
    admin: bool,
    local: bool,
    fail_hard: bool,
) -> AppNetworkOpsPendingTransaction {
    NetworkOpsApplyBatchEntry::new(transaction, admin, local, fail_hard)
}

fn transaction_applying(transaction: &SharedTransaction) -> bool {
    transaction
        .lock()
        .expect("transaction mutex must not be poisoned")
        .get_applying()
}

fn pending_transaction_applying(transaction: &AppNetworkOpsPendingTransaction) -> bool {
    transaction_applying(&transaction.transaction)
}

fn set_transaction_applying(transaction: &SharedTransaction) {
    transaction
        .lock()
        .expect("transaction mutex must not be poisoned")
        .set_applying();
}

fn add_held_transaction(
    ledger_master_runtime: &AppLedgerMasterRuntime,
    transaction: &SharedTransaction,
) {
    let sttx = read_transaction(transaction, |tx| Arc::clone(tx.get_s_transaction()));
    ledger_master_runtime.add_held_sttx(sttx);
}

fn push_local_transaction(
    ledger_master_runtime: &AppLedgerMasterRuntime,
    current_ledger_index: u32,
    transaction: &SharedTransaction,
) {
    let sttx = read_transaction(transaction, |tx| Arc::clone(tx.get_s_transaction()));
    ledger_master_runtime.push_local_tx(current_ledger_index, sttx);
}

fn inner_batch_flag_set(transaction: &SharedTransaction) -> bool {
    read_transaction(transaction, |tx| {
        tx.get_s_transaction().is_flag(tfInnerBatchTxn)
    })
}

fn last_ledger_sequence(transaction: &SharedTransaction) -> Option<u32> {
    let field = get_field_by_symbol("sfLastLedgerSequence");
    read_transaction(transaction, |tx| {
        let sttx = tx.get_s_transaction();
        sttx.is_field_present(field)
            .then(|| sttx.get_field_u32(field))
    })
}

fn mutate_transaction<R>(
    transaction: &SharedTransaction,
    update: impl FnOnce(&mut Transaction) -> R,
) -> R {
    let mut guard = transaction
        .lock()
        .expect("transaction mutex must not be poisoned");
    update(&mut guard)
}

fn read_transaction<R>(transaction: &SharedTransaction, read: impl FnOnce(&Transaction) -> R) -> R {
    let guard = transaction
        .lock()
        .expect("transaction mutex must not be poisoned");
    read(&guard)
}

#[cfg(test)]
mod tests {
    use super::{
        APP_NETWORKOPS_MAX_POPPED_TRANSACTIONS, AppNetworkOpsAppliedTransactionReport,
        AppNetworkOpsApplyHeldOutcome, AppNetworkOpsApplyReport, AppNetworkOpsRuntime,
        AppNetworkOpsRuntimeSnapshot, pending_transaction, read_transaction, shared_transaction,
    };
    use crate::ledger::ledger_master_runtime::AppLedgerMasterRuntime;
    use crate::ledger::ledger_master_state::SharedLedgerMasterState;
    use crate::network::network_ops::{
        NetworkOpsApplyBatchStart, NetworkOpsApplyBatchTail, NetworkOpsApplyResultPreamble,
        NetworkOpsApplyStatusBranch, NetworkOpsCurrentLedgerState, NetworkOpsDispatchState,
        NetworkOpsOperatingMode, NetworkOpsProcessSetOwnerSync, NetworkOpsRelayBranch,
        NetworkOpsRetryHoldBranch, NetworkOpsRuntimeState, NetworkOpsTransactionSetOutcome,
        SharedNetworkOpsState,
    };
    use crate::state::time_keeper::TimeKeeper;
    use crate::tx_queue::transaction::{
        CurrentLedgerState, SubmitResult, TransStatus, Transaction,
    };
    use basics::base_uint::Uint256;
    use basics::sha_map_hash::SHAMapHash;
    use protocol::{AccountID, STAmount, STTx, Ter, TxType, XRPAmount, get_field_by_symbol};
    use std::cell::RefCell;
    use std::sync::{Arc, Mutex};
    use tx::{ApplyFlags, ApplyResult};
    use xrpl_core::{HashRouter, HashRouterSetup};

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
        ledger_master_runtime: Arc<AppLedgerMasterRuntime>,
    ) -> AppNetworkOpsRuntime {
        AppNetworkOpsRuntime::new(
            Arc::new(SharedNetworkOpsState::new(mode)),
            ledger_master_runtime,
            Arc::new(HashRouter::new(HashRouterSetup::default())),
            Arc::new(crate::tx_queue::transaction_master::TransactionMaster::new()),
            Arc::new(SharedLedgerMasterState::new(Arc::new(TimeKeeper::new()))),
        )
    }

    #[test]
    fn runtime_applies_held_transactions_into_owned_pending_queue() {
        let ledger_master_runtime = Arc::new(AppLedgerMasterRuntime::default());
        let runtime = runtime(
            NetworkOpsOperatingMode::Full,
            Arc::clone(&ledger_master_runtime),
        );
        let source = account("5555555555555555555555555555555555555555");
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

        let syncs = RefCell::new(Vec::new());
        let outcome = runtime
            .apply_held_transactions_to_queue(SHAMapHash::new(Uint256::from_u64(91)), |sync| {
                syncs.borrow_mut().push(sync)
            });

        assert_eq!(
            outcome,
            AppNetworkOpsApplyHeldOutcome {
                drained_count: 2,
                process_outcome: Some(NetworkOpsTransactionSetOutcome::SyncBatch {
                    added_count: 2,
                }),
            }
        );
        assert_eq!(
            syncs.into_inner(),
            vec![NetworkOpsProcessSetOwnerSync {
                added_count: 2,
                had_pending_before: false,
                has_applying_after_merge: true,
            }]
        );
        assert_eq!(
            runtime.snapshot(),
            AppNetworkOpsRuntimeSnapshot {
                pending_transactions: 2,
                submit_held: 0,
                dispatch_state: NetworkOpsDispatchState::None,
            }
        );

        let (transactions, start) = runtime.begin_apply_batch();
        assert_eq!(start.taken_transactions, 2);
        assert_eq!(start.dispatch_state, NetworkOpsDispatchState::Running);
        assert_eq!(
            transactions
                .iter()
                .map(|tx| {
                    tx.transaction
                        .lock()
                        .expect("transaction mutex must not be poisoned")
                        .get_id()
                })
                .collect::<Vec<_>>(),
            vec![first.get_transaction_id(), second.get_transaction_id()]
        );

        let tail = runtime.finish_apply_batch(&transactions);
        assert_eq!(tail.cleared, 2);
        assert_eq!(tail.pending_transactions, 0);
        assert_eq!(tail.dispatch_state, NetworkOpsDispatchState::None);
    }

    #[test]
    fn runtime_promotes_included_transaction_into_submit_held_then_pending() {
        let ledger_master_runtime = Arc::new(AppLedgerMasterRuntime::default());
        let runtime = runtime(
            NetworkOpsOperatingMode::Full,
            Arc::clone(&ledger_master_runtime),
        );
        let source = account("6666666666666666666666666666666666666666");
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
        let promoted = runtime.promote_included_transaction(&current);
        assert_eq!(promoted, 2);
        assert_eq!(runtime.pending_transaction_count(), 0);
        assert_eq!(runtime.submit_held_count(), 2);

        let tail = runtime.finish_apply_batch(&[pending_transaction(
            Arc::clone(&current),
            false,
            false,
            false,
        )]);
        assert_eq!(tail.cleared, 1);
        assert_eq!(tail.pending_transactions, 2);
        assert_eq!(runtime.submit_held_count(), 0);

        let (transactions, start) = runtime.begin_apply_batch();
        assert_eq!(start.taken_transactions, 2);
        assert_eq!(start.dispatch_state, NetworkOpsDispatchState::Running);
        assert_eq!(
            transactions
                .iter()
                .map(|tx| {
                    let guard = tx
                        .transaction
                        .lock()
                        .expect("transaction mutex must not be poisoned");
                    assert!(guard.get_applying());
                    guard.get_id()
                })
                .collect::<Vec<_>>(),
            vec![next.get_transaction_id(), ticket.get_transaction_id()]
        );
    }

    #[test]
    fn runtime_can_swap_ledger_master_owner_without_losing_queue_state() {
        let original = Arc::new(AppLedgerMasterRuntime::default());
        let replacement = Arc::new(AppLedgerMasterRuntime::default());
        let runtime = AppNetworkOpsRuntime::with_state(
            Arc::new(SharedNetworkOpsState::new(
                NetworkOpsOperatingMode::Tracking,
            )),
            Arc::clone(&original),
            Arc::new(HashRouter::new(HashRouterSetup::default())),
            Arc::new(crate::tx_queue::transaction_master::TransactionMaster::new()),
            Arc::new(SharedLedgerMasterState::new(Arc::new(TimeKeeper::new()))),
            NetworkOpsRuntimeState::new(
                vec![pending_transaction(
                    Arc::new(Mutex::new(Transaction::new(payment_tx(
                        account("7777777777777777777777777777777777777777"),
                        account("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
                        1,
                        None,
                        10,
                    )))),
                    false,
                    false,
                    false,
                )],
                Vec::new(),
                NetworkOpsDispatchState::Scheduled,
            ),
        );

        let previous = runtime.set_ledger_master_runtime(Arc::clone(&replacement));
        assert!(Arc::ptr_eq(&previous, &original));
        assert!(Arc::ptr_eq(&runtime.ledger_master_runtime(), &replacement));
        assert_eq!(runtime.pending_transaction_count(), 1);
        assert_eq!(runtime.dispatch_state(), NetworkOpsDispatchState::Scheduled);
        assert_eq!(APP_NETWORKOPS_MAX_POPPED_TRANSACTIONS, 10);
    }

    #[test]
    fn runtime_apply_pending_runs_cpp_side_effect_order_for_successful_local_tx() {
        let ledger_master_runtime = Arc::new(AppLedgerMasterRuntime::default());
        let runtime = runtime(
            NetworkOpsOperatingMode::Tracking,
            Arc::clone(&ledger_master_runtime),
        );
        let source = account("8888888888888888888888888888888888888888");
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

        let current = shared_transaction(Arc::clone(&current));
        assert!(runtime.stage_transaction(Arc::clone(&current), true, true, false));

        let relayed = RefCell::new(Vec::new());
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
                    relayed.borrow_mut().push((
                        read_transaction(tx, |t| t.get_id()),
                        deferred,
                        skip,
                    ))
                },
                |_tx| NetworkOpsCurrentLedgerState {
                    fee: XRPAmount::from_drops(25),
                    account_seq: 11,
                    available_seq: 13,
                },
            )
            .expect("pending batch should apply");

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
                    transaction_id: read_transaction(&current, |tx| tx.get_id()),
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
            relayed.into_inner(),
            vec![(read_transaction(&current, |tx| tx.get_id()), false, 9u32)]
        );
        assert_eq!(ledger_master_runtime.get_local_tx_count(), 1);
        assert_eq!(runtime.pending_transaction_count(), 2);
        assert_eq!(runtime.submit_held_count(), 0);

        let (promoted, start) = runtime.begin_apply_batch();
        assert_eq!(start.taken_transactions, 2);
        assert_eq!(
            promoted
                .iter()
                .map(|tx| read_transaction(&tx.transaction, |transaction| transaction.get_id()))
                .collect::<Vec<_>>(),
            vec![next.get_transaction_id(), ticket.get_transaction_id()]
        );
    }

    #[test]
    fn runtime_apply_pending_batch_with_exposes_one_caller_owned_batch() {
        let ledger_master_runtime = Arc::new(AppLedgerMasterRuntime::default());
        let runtime = runtime(
            NetworkOpsOperatingMode::Tracking,
            Arc::clone(&ledger_master_runtime),
        );
        let source = account("1212121212121212121212121212121212121212");
        let included = shared_transaction(payment_tx(
            source,
            account("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
            1,
            None,
            10,
        ));
        let queued = shared_transaction(payment_tx(
            source,
            account("BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB"),
            2,
            None,
            11,
        ));

        assert!(runtime.stage_transaction(Arc::clone(&included), false, true, false));
        assert!(runtime.stage_transaction(Arc::clone(&queued), false, false, false));

        let batch_calls = std::cell::Cell::new(0usize);
        let seen_ids = RefCell::new(Vec::new());
        let fee_change_reports = std::cell::Cell::new(0usize);

        let report = runtime
            .apply_pending_batch_with(
                800,
                Some(801),
                |transactions| {
                    batch_calls.set(batch_calls.get() + 1);
                    seen_ids.borrow_mut().extend(
                        transactions
                            .iter()
                            .map(|entry| read_transaction(&entry.transaction, |tx| tx.get_id())),
                    );

                    for (index, entry) in transactions.iter_mut().enumerate() {
                        if index == 0 {
                            entry.result = Some(Ter::TES_SUCCESS);
                            entry.applied = true;
                        } else {
                            entry.result = Some(Ter::TER_QUEUED);
                            entry.applied = false;
                        }
                    }

                    true
                },
                || fee_change_reports.set(fee_change_reports.get() + 1),
                |_tx, _result| {},
                |_tx| {},
                |_tx| false,
                |_tx| None::<u32>,
                |_tx, _deferred, _skip| {},
                |_tx| NetworkOpsCurrentLedgerState {
                    fee: XRPAmount::from_drops(30),
                    account_seq: 9,
                    available_seq: 11,
                },
            )
            .expect("pending batch should apply");

        assert_eq!(batch_calls.get(), 1);
        assert_eq!(
            seen_ids.into_inner(),
            vec![
                read_transaction(&included, |tx| tx.get_id()),
                read_transaction(&queued, |tx| tx.get_id())
            ]
        );
        assert_eq!(fee_change_reports.get(), 1);
        assert_eq!(report.start.taken_transactions, 2);
        assert!(report.changed);
        assert!(report.fee_change_reported);
        assert_eq!(report.entries.len(), 2);
        assert_eq!(report.entries[0].result, Ter::TES_SUCCESS);
        assert!(report.entries[0].applied);
        assert_eq!(
            report.entries[0].status_branch,
            NetworkOpsApplyStatusBranch::Included
        );
        assert_eq!(report.entries[0].final_status, TransStatus::INCLUDED);
        assert_eq!(report.entries[1].result, Ter::TER_QUEUED);
        assert!(!report.entries[1].applied);
        assert_eq!(
            report.entries[1].status_branch,
            NetworkOpsApplyStatusBranch::Queued
        );
        assert_eq!(report.entries[1].final_status, TransStatus::HELD);
        assert_eq!(runtime.pending_transaction_count(), 0);
        assert_eq!(ledger_master_runtime.held_transaction_count(), 1);
    }

    #[test]
    fn runtime_apply_pending_covers_queued_retry_malformed_and_fail_hard_branches() {
        let ledger_master_runtime = Arc::new(AppLedgerMasterRuntime::default());
        let runtime = runtime(
            NetworkOpsOperatingMode::Full,
            Arc::clone(&ledger_master_runtime),
        );
        let source = account("9999999999999999999999999999999999999999");

        let queued = shared_transaction(payment_tx(
            source,
            account("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"),
            1,
            None,
            10,
        ));
        let retry = shared_transaction(Arc::new(STTx::new(TxType::PAYMENT, |tx| {
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
        })));
        let malformed = shared_transaction(payment_tx(
            source,
            account("CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC"),
            3,
            None,
            12,
        ));
        let fail_hard = shared_transaction(payment_tx(
            source,
            account("DDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDD"),
            4,
            None,
            13,
        ));

        assert!(runtime.stage_transaction(Arc::clone(&queued), false, false, false));
        assert!(runtime.stage_transaction(Arc::clone(&retry), false, false, false));
        assert!(runtime.stage_transaction(Arc::clone(&malformed), false, false, false));
        assert!(runtime.stage_transaction(Arc::clone(&fail_hard), false, true, true));

        let bad = RefCell::new(Vec::new());
        let held_flags = RefCell::new(Vec::new());
        let relayed = RefCell::new(Vec::new());
        let report = runtime
            .apply_pending_with(
                400,
                Some(401),
                |tx, flags| {
                    let id = read_transaction(tx, |transaction| transaction.get_id());
                    if id == read_transaction(&queued, |transaction| transaction.get_id()) {
                        return ApplyResult::new(Ter::TER_QUEUED, false, false);
                    }
                    if id == read_transaction(&retry, |transaction| transaction.get_id()) {
                        return ApplyResult::new(Ter::TER_RETRY, false, false);
                    }
                    if id == read_transaction(&malformed, |transaction| transaction.get_id()) {
                        assert_eq!(flags, ApplyFlags::NONE);
                        return ApplyResult::new(Ter::TEM_MALFORMED, false, false);
                    }
                    assert_eq!(flags, ApplyFlags::FAIL_HARD);
                    ApplyResult::new(Ter::TER_RETRY, false, false)
                },
                || {},
                |_tx, _result| {},
                |tx| {
                    bad.borrow_mut()
                        .push(read_transaction(tx, |transaction| transaction.get_id()))
                },
                |tx| {
                    let id = read_transaction(tx, |transaction| transaction.get_id());
                    held_flags.borrow_mut().push(id);
                    false
                },
                |tx| {
                    let id = read_transaction(tx, |transaction| transaction.get_id());
                    if id == read_transaction(&queued, |transaction| transaction.get_id()) {
                        Some(17u32)
                    } else {
                        None
                    }
                },
                |tx, deferred, skip| {
                    relayed.borrow_mut().push((
                        read_transaction(tx, |transaction| transaction.get_id()),
                        deferred,
                        skip,
                    ))
                },
                |_tx| NetworkOpsCurrentLedgerState {
                    fee: XRPAmount::from_drops(30),
                    account_seq: 20,
                    available_seq: 21,
                },
            )
            .expect("pending batch should apply");

        assert!(!report.changed);
        assert!(!report.fee_change_reported);
        assert_eq!(report.tail.pending_transactions, 0);
        assert_eq!(runtime.pending_transaction_count(), 0);
        assert_eq!(runtime.submit_held_count(), 0);
        assert_eq!(ledger_master_runtime.held_transaction_count(), 2);
        assert_eq!(
            bad.into_inner(),
            vec![read_transaction(&malformed, |transaction| transaction.get_id())]
        );
        assert_eq!(held_flags.into_inner(), Vec::<Uint256>::new());
        assert_eq!(
            relayed.into_inner(),
            vec![(
                read_transaction(&queued, |transaction| transaction.get_id()),
                true,
                17u32
            )]
        );

        let queued_report = &report.entries[0];
        assert_eq!(
            queued_report.status_branch,
            NetworkOpsApplyStatusBranch::Queued
        );
        assert_eq!(queued_report.final_status, TransStatus::HELD);
        assert_eq!(
            queued_report.submit_result,
            SubmitResult {
                applied: false,
                broadcast: true,
                queued: true,
                kept: true,
            }
        );

        let retry_report = &report.entries[1];
        assert_eq!(
            retry_report.retry_hold_branch,
            Some(NetworkOpsRetryHoldBranch::Held {
                ledgers_left: Some(2),
            })
        );
        assert_eq!(retry_report.final_status, TransStatus::HELD);

        let malformed_report = &report.entries[2];
        assert!(malformed_report.preamble.malformed);
        assert_eq!(malformed_report.final_status, TransStatus::INVALID);

        let fail_hard_report = &report.entries[3];
        assert_eq!(
            fail_hard_report.retry_hold_branch,
            Some(NetworkOpsRetryHoldBranch::FailHard)
        );
        assert!(!fail_hard_report.local_kept);
        assert_eq!(
            fail_hard_report.relay_branch,
            NetworkOpsRelayBranch::SkippedEligibility
        );
        assert_eq!(fail_hard_report.final_status, TransStatus::NEW);
    }
}
