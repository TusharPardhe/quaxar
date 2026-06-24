//! Small managed-component wrappers that let `MainRuntime` coordinate the
//! app-owned service graph like reference `ApplicationImp::start()` / `run()`.

use crate::ledger::ledger_master_runtime::AppLedgerMasterRuntime;
use crate::network::network_ops_runtime::AppNetworkOpsRuntime;
use crate::runtime::main_runtime::ManagedComponent;
use crate::shamap::shamap_store_backend::SHAMapStoreNodeStore;
use crate::state::app_registry::{AppInboundLedgers, AppInboundTransactions};
use basics;
use consensus;
use ledger::LedgerCleaner;
use perflog::{PerfLog, PerfLogImp};
use protocol;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct AppNodeStoreRuntime {
    node_store: SHAMapStoreNodeStore,
    started: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
}

impl std::fmt::Debug for AppNodeStoreRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppNodeStoreRuntime")
            .field("kind", &self.node_store.kind())
            .field("started", &self.started.load(Ordering::Acquire))
            .field("stopped", &self.stopped.load(Ordering::Acquire))
            .finish()
    }
}

impl AppNodeStoreRuntime {
    pub fn new(node_store: SHAMapStoreNodeStore) -> Self {
        Self {
            node_store,
            started: Arc::new(AtomicBool::new(false)),
            stopped: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl ManagedComponent for AppNodeStoreRuntime {
    fn start(&self) -> Result<(), String> {
        if self.stopped.load(Ordering::Acquire) {
            return Err("node store runtime has already been stopped".to_owned());
        }
        self.started.store(true, Ordering::Release);
        Ok(())
    }

    fn stop(&self) {
        if self.stopped.swap(true, Ordering::AcqRel) {
            return;
        }
        match &self.node_store {
            SHAMapStoreNodeStore::Single(database) => database.stop(),
            SHAMapStoreNodeStore::Rotating(database) => database.stop(),
        }
    }

    fn fd_required(&self) -> usize {
        self.node_store.fd_required().max(0) as usize
    }
}

#[derive(Clone)]
pub struct AppLedgerRuntime {
    ledger_cleaner: Arc<LedgerCleaner>,
    inbound_ledgers: AppInboundLedgers,
    inbound_transactions: AppInboundTransactions,
    ledger_replayer: Arc<Mutex<ledger::LedgerReplayer>>,
    ledger_master_runtime: Arc<AppLedgerMasterRuntime>,
    network_ops_runtime: Option<Arc<AppNetworkOpsRuntime>>,
    started: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
}

impl std::fmt::Debug for AppLedgerRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppLedgerRuntime")
            .field(
                "held_transactions",
                &self.ledger_master_runtime.held_transaction_count(),
            )
            .field(
                "has_network_ops_runtime",
                &self.network_ops_runtime.is_some(),
            )
            .field("started", &self.started.load(Ordering::Acquire))
            .field("stopped", &self.stopped.load(Ordering::Acquire))
            .finish()
    }
}

impl AppLedgerRuntime {
    pub fn new(
        ledger_cleaner: Arc<LedgerCleaner>,
        inbound_ledgers: AppInboundLedgers,
        inbound_transactions: AppInboundTransactions,
        ledger_replayer: Arc<Mutex<ledger::LedgerReplayer>>,
        ledger_master_runtime: Arc<AppLedgerMasterRuntime>,
        network_ops_runtime: Option<Arc<AppNetworkOpsRuntime>>,
    ) -> Self {
        Self {
            ledger_cleaner,
            inbound_ledgers,
            inbound_transactions,
            ledger_replayer,
            ledger_master_runtime,
            network_ops_runtime,
            started: Arc::new(AtomicBool::new(false)),
            stopped: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl ManagedComponent for AppLedgerRuntime {
    fn start(&self) -> Result<(), String> {
        if self.stopped.load(Ordering::Acquire) {
            return Err("ledger runtime has already been stopped".to_owned());
        }
        if self.started.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        self.ledger_cleaner.start();
        Ok(())
    }

    fn stop(&self) {
        if self.stopped.swap(true, Ordering::AcqRel) {
            return;
        }
        self.ledger_cleaner.stop();
        self.inbound_ledgers
            .lock()
            .expect("inbound ledgers mutex must not be poisoned")
            .stop();
        self.inbound_transactions
            .lock()
            .expect("inbound transactions mutex must not be poisoned")
            .stop();
        self.ledger_replayer
            .lock()
            .expect("ledger replayer mutex must not be poisoned")
            .stop();
    }
}

#[derive(Clone)]
pub struct AppConsensusRuntime {
    network_ops_runtime: Arc<AppNetworkOpsRuntime>,
    started: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
    runner:
        Arc<tokio::sync::Mutex<Option<Box<dyn crate::consensus::rcl_consensus::ConsensusRunner>>>>,
    /// Pending proposals queued by the bootstrap/main loop for processing
    /// INSIDE timer_tick (before timer_entry runs). This ensures proposals
    /// are in curr_peer_positions when shouldCloseLedger evaluates.
    pending_proposals: Arc<std::sync::Mutex<Vec<PendingProposal>>>,
    /// are acquired from peers. The validation thread consumes this and calls
    /// got_tx_set on the consensus engine.
    map_complete_rx: Arc<
        std::sync::Mutex<
            Option<
                std::sync::mpsc::Receiver<(
                    basics::base_uint::Uint256,
                    std::sync::Arc<shamap::sync::SyncTree>,
                )>,
            >,
        >,
    >,
}

/// A proposal queued for processing inside timer_tick.
pub struct PendingProposal {
    pub now: basics::chrono::NetClockTimePoint,
    pub public_key: protocol::PublicKey,
    pub signature: Vec<u8>,
    pub suppression_id: basics::base_uint::Uint256,
    pub proposal: consensus::ConsensusProposal<
        protocol::PublicKey,
        basics::base_uint::Uint256,
        basics::base_uint::Uint256,
    >,
}

impl std::fmt::Debug for AppConsensusRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppConsensusRuntime")
            .field(
                "pending_transactions",
                &self.network_ops_runtime.pending_transaction_count(),
            )
            .field("started", &self.started.load(Ordering::Acquire))
            .field("stopped", &self.stopped.load(Ordering::Acquire))
            .finish()
    }
}

impl AppConsensusRuntime {
    pub fn new(network_ops_runtime: Arc<AppNetworkOpsRuntime>) -> Self {
        Self {
            network_ops_runtime,
            started: Arc::new(AtomicBool::new(false)),
            stopped: Arc::new(AtomicBool::new(false)),
            runner: Arc::new(tokio::sync::Mutex::new(None)),
            pending_proposals: Arc::new(std::sync::Mutex::new(Vec::new())),
            map_complete_rx: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    /// Set the map_complete receiver (from InboundTransactions channel).
    pub fn set_map_complete_receiver(
        &self,
        rx: std::sync::mpsc::Receiver<(
            basics::base_uint::Uint256,
            std::sync::Arc<shamap::sync::SyncTree>,
        )>,
    ) {
        *self.map_complete_rx.lock().expect("map_complete_rx mutex") = Some(rx);
    }

    /// Take the map_complete receiver (for the validation thread to own).
    pub fn take_map_complete_receiver(
        &self,
    ) -> Option<
        std::sync::mpsc::Receiver<(
            basics::base_uint::Uint256,
            std::sync::Arc<shamap::sync::SyncTree>,
        )>,
    > {
        self.map_complete_rx
            .lock()
            .expect("map_complete_rx mutex")
            .take()
    }

    pub fn set_runner(&self, runner: Box<dyn crate::consensus::rcl_consensus::ConsensusRunner>) {
        *self
            .runner
            .try_lock()
            .expect("set_runner called during init without contention") = Some(runner);
    }

    pub async fn timer_tick(&self, now: basics::chrono::NetClockTimePoint, run_timer: bool) {
        let runner_guard = self.runner.lock().await;
        if let Some(runner) = runner_guard.as_ref() {
            // Always drain pending proposals so they reach consensus within 50ms.
            let proposals: Vec<PendingProposal> = {
                let mut queue = self.pending_proposals.lock().expect("pending_proposals");
                std::mem::take(&mut *queue)
            };
            if !proposals.is_empty() {
                tracing::info!(target: "consensus", count = proposals.len(), "timer_tick: draining pending proposals");
            }
            for p in proposals {
                runner
                    .peer_proposal(
                        p.now,
                        p.public_key,
                        p.signature,
                        p.suppression_id,
                        p.proposal,
                    )
                    .await;
            }

            // Only run the state machine on the 1s boundary.
            if run_timer {
                runner.timer_tick(now).await;

                // Drain again after timer_tick in case a new round started
                // (via pending_start_round) and proposals arrived during the tick.
                let proposals2: Vec<PendingProposal> = {
                    let mut queue = self.pending_proposals.lock().expect("pending_proposals");
                    std::mem::take(&mut *queue)
                };
                for p in proposals2 {
                    runner
                        .peer_proposal(
                            p.now,
                            p.public_key,
                            p.signature,
                            p.suppression_id,
                            p.proposal,
                        )
                        .await;
                }
            }
        }
    }

    /// Queue a proposal for processing inside the next timer_tick.
    /// This is thread-safe and non-blocking.
    pub fn push_proposal(&self, proposal: PendingProposal) {
        self.pending_proposals
            .lock()
            .expect("pending_proposals")
            .push(proposal);
    }

    /// Start a consensus round with the given previous ledger.
    pub async fn start_round(
        &self,
        now: basics::chrono::NetClockTimePoint,
        prev_ledger_id: basics::base_uint::Uint256,
        prev_ledger: consensus::RclCxLedger,
    ) {
        let runner_guard = self.runner.lock().await;
        if let Some(runner) = runner_guard.as_ref() {
            runner.start_round(now, prev_ledger_id, prev_ledger).await;
        }
    }

    /// Get blocking access to the runner (for non-async contexts).
    pub fn runner_blocking(
        &self,
    ) -> tokio::sync::MutexGuard<
        '_,
        Option<Box<dyn crate::consensus::rcl_consensus::ConsensusRunner>>,
    > {
        self.runner.blocking_lock()
    }

    pub async fn peer_proposal(
        &self,
        now: basics::chrono::NetClockTimePoint,
        public_key: protocol::PublicKey,
        signature: Vec<u8>,
        suppression_id: basics::base_uint::Uint256,
        proposal: consensus::ConsensusProposal<
            protocol::PublicKey,
            basics::base_uint::Uint256,
            basics::base_uint::Uint256,
        >,
    ) -> bool {
        let runner_guard = self.runner.lock().await;
        if let Some(runner) = runner_guard.as_ref() {
            runner
                .peer_proposal(now, public_key, signature, suppression_id, proposal)
                .await
        } else {
            false
        }
    }

    pub async fn got_tx_set(
        &self,
        now: basics::chrono::NetClockTimePoint,
        txset: Vec<consensus::RclCxTx>,
    ) {
        let runner_guard = self.runner.lock().await;
        if let Some(runner) = runner_guard.as_ref() {
            runner.got_tx_set(now, txset).await;
        }
    }
}

impl ManagedComponent for AppConsensusRuntime {
    fn start(&self) -> Result<(), String> {
        if self.stopped.load(Ordering::Acquire) {
            return Err("consensus runtime has already been stopped".to_owned());
        }
        self.started.store(true, Ordering::Release);
        Ok(())
    }

    fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
    }
}

#[derive(Clone, Debug, Default)]
pub struct AppValidatorSiteRuntime {
    started: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
}

impl ManagedComponent for AppValidatorSiteRuntime {
    fn start(&self) -> Result<(), String> {
        if self.stopped.load(Ordering::Acquire) {
            return Err("validator site runtime has already been stopped".to_owned());
        }
        self.started.store(true, Ordering::Release);
        Ok(())
    }

    fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
    }
}

#[derive(Clone)]
pub struct AppPerfLogRuntime {
    perf_log: Arc<PerfLogImp>,
    started: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
}

impl std::fmt::Debug for AppPerfLogRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppPerfLogRuntime")
            .field("started", &self.started.load(Ordering::Acquire))
            .field("stopped", &self.stopped.load(Ordering::Acquire))
            .finish()
    }
}

impl AppPerfLogRuntime {
    pub fn new(perf_log: Arc<PerfLogImp>) -> Self {
        Self {
            perf_log,
            started: Arc::new(AtomicBool::new(false)),
            stopped: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl ManagedComponent for AppPerfLogRuntime {
    fn start(&self) -> Result<(), String> {
        if self.stopped.load(Ordering::Acquire) {
            return Err("perf log runtime has already been stopped".to_owned());
        }
        if self.started.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        self.perf_log.start();
        Ok(())
    }

    fn stop(&self) {
        if self.stopped.swap(true, Ordering::AcqRel) {
            return;
        }
        self.perf_log.stop();
    }
}
