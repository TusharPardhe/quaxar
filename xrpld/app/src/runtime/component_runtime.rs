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
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// AppNodeStoreRuntime (unchanged)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// AppLedgerRuntime (unchanged)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// AppConsensusRuntime (rewritten for single-strand model)
// ---------------------------------------------------------------------------

/// Command sent from external code to the consensus strand thread.
pub enum ConsensusCommand {
    StartRound {
        now: basics::chrono::NetClockTimePoint,
        prev_ledger_id: basics::base_uint::Uint256,
        prev_ledger: consensus::RclCxLedger,
    },
    Stop,
}

/// The consensus runtime for the single-strand model. The consensus runner
/// (`AppConsensus`) lives directly on the strand thread's stack — it is NOT
/// stored here. This struct only provides:
/// - External accessors (phase, prev_ledger_id) via atomics/mutex
/// - A command channel to the strand thread
/// - The map_complete receiver handoff
/// - Temporary storage of the runner during construction (before the strand
///   thread takes ownership)
#[derive(Clone)]
pub struct AppConsensusRuntime {
    started: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
    /// 0=Open, 1=Establish, 2=Accepted — updated by the strand thread
    phase: Arc<AtomicU8>,
    /// Updated by the strand thread after each state transition
    prev_ledger_id: Arc<parking_lot::Mutex<basics::base_uint::Uint256>>,
    /// Channel to send commands to the strand thread
    cmd_tx: Arc<Mutex<Option<std::sync::mpsc::Sender<ConsensusCommand>>>>,
    /// Receiver for map-complete events (tx-set acquisitions)
    map_complete_rx: Arc<
        Mutex<
            Option<
                std::sync::mpsc::Receiver<(
                    basics::base_uint::Uint256,
                    Arc<shamap::sync::SyncTree>,
                )>,
            >,
        >,
    >,
    /// Temporary storage for the runner before the strand thread takes it
    runner_storage: Arc<Mutex<Option<crate::consensus::rcl_consensus::AppConsensus>>>,
}

impl std::fmt::Debug for AppConsensusRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppConsensusRuntime")
            .field("started", &self.started.load(Ordering::Acquire))
            .field("stopped", &self.stopped.load(Ordering::Acquire))
            .field("phase", &self.phase.load(Ordering::Acquire))
            .finish()
    }
}

impl AppConsensusRuntime {
    pub fn new() -> Self {
        Self {
            started: Arc::new(AtomicBool::new(false)),
            stopped: Arc::new(AtomicBool::new(false)),
            phase: Arc::new(AtomicU8::new(2)), // Accepted initially
            prev_ledger_id: Arc::new(parking_lot::Mutex::new(basics::base_uint::Uint256::zero())),
            cmd_tx: Arc::new(Mutex::new(None)),
            map_complete_rx: Arc::new(Mutex::new(None)),
            runner_storage: Arc::new(Mutex::new(None)),
        }
    }

    /// Store the runner temporarily. The strand thread will take it via `take_runner`.
    pub fn set_runner(&self, runner: crate::consensus::rcl_consensus::AppConsensus) {
        *self.runner_storage.lock().expect("runner_storage mutex") = Some(runner);
    }

    /// Take the runner for the strand thread to own. Called exactly once.
    pub fn take_runner(&self) -> Option<crate::consensus::rcl_consensus::AppConsensus> {
        self.runner_storage
            .lock()
            .expect("runner_storage mutex")
            .take()
    }

    /// Set the command sender (strand thread provides this after starting).
    pub fn set_cmd_sender(&self, tx: std::sync::mpsc::Sender<ConsensusCommand>) {
        *self.cmd_tx.lock().expect("cmd_tx mutex") = Some(tx);
    }

    /// Send a start-round command to the strand thread.
    pub fn send_start_round(
        &self,
        now: basics::chrono::NetClockTimePoint,
        prev_ledger_id: basics::base_uint::Uint256,
        prev_ledger: consensus::RclCxLedger,
    ) {
        if let Some(tx) = self.cmd_tx.lock().expect("cmd_tx mutex").as_ref() {
            let _ = tx.send(ConsensusCommand::StartRound {
                now,
                prev_ledger_id,
                prev_ledger,
            });
        }
    }

    /// Send a stop command to the strand thread.
    pub fn send_stop(&self) {
        if let Some(tx) = self.cmd_tx.lock().expect("cmd_tx mutex").as_ref() {
            let _ = tx.send(ConsensusCommand::Stop);
        }
    }

    /// Set the map_complete receiver (from InboundTransactions channel).
    pub fn set_map_complete_receiver(
        &self,
        rx: std::sync::mpsc::Receiver<(basics::base_uint::Uint256, Arc<shamap::sync::SyncTree>)>,
    ) {
        *self.map_complete_rx.lock().expect("map_complete_rx mutex") = Some(rx);
    }

    /// Take the map_complete receiver (for the strand thread to own).
    pub fn take_map_complete_receiver(
        &self,
    ) -> Option<std::sync::mpsc::Receiver<(basics::base_uint::Uint256, Arc<shamap::sync::SyncTree>)>>
    {
        self.map_complete_rx
            .lock()
            .expect("map_complete_rx mutex")
            .take()
    }

    /// The current round's phase, readable from any thread.
    pub fn phase(&self) -> consensus::algorithm::ConsensusPhase {
        match self.phase.load(Ordering::Acquire) {
            0 => consensus::algorithm::ConsensusPhase::Open,
            1 => consensus::algorithm::ConsensusPhase::Establish,
            _ => consensus::algorithm::ConsensusPhase::Accepted,
        }
    }

    /// Update phase from the strand thread.
    pub fn update_phase(&self, phase: consensus::algorithm::ConsensusPhase) {
        let val = match phase {
            consensus::algorithm::ConsensusPhase::Open => 0,
            consensus::algorithm::ConsensusPhase::Establish => 1,
            consensus::algorithm::ConsensusPhase::Accepted => 2,
        };
        self.phase.store(val, Ordering::Release);
    }

    /// The previous ledger hash the current consensus round is building on.
    pub fn prev_ledger_id(&self) -> basics::base_uint::Uint256 {
        *self.prev_ledger_id.lock()
    }

    /// Update prev_ledger_id from the strand thread.
    pub fn update_prev_ledger_id(&self, id: basics::base_uint::Uint256) {
        *self.prev_ledger_id.lock() = id;
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
        self.send_stop();
    }
}

// ---------------------------------------------------------------------------
// AppValidatorSiteRuntime (unchanged)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// AppPerfLogRuntime (unchanged)
// ---------------------------------------------------------------------------

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
