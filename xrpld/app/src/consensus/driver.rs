//! `ConsensusDriver` — the single entry point for consensus participation.
//!
//! Replaces the fragmented AppConsensusRuntime + NetworkOpsRuntime + bootstrap
//! timer with a unified architecture matching rippled:
//!
//! - ONE `parking_lot::Mutex` protects the consensus state machine
//! - ONE heartbeat thread (1s) drives `timer_entry`
//! - Proposals arrive DIRECTLY from overlay threads (no batching)
//! - Accept work runs inside on_accept → pending_start_round (existing pattern)
//! - ONE event-loop thread processes validations + ledger completions
//!
//! See docs/CONSENSUS_ARCHITECTURE.md for the full architecture.

use basics::base_uint::Uint256;
use basics::chrono::NetClockTimePoint;
use basics::sha_map_hash::SHAMapHash;
use consensus::{RclCxLedger, RclCxPeerPos, RclCxTx, RclConsensusAdapter};
use ledger::Ledger;
use overlay::QueuedValidation;
use parking_lot::Mutex;
use overlay::Overlay as _;
use protocol::{STValidation, SerialIter};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::consensus::rcl_validations::RclValidationAcceptanceSink;
use crate::ledger::shared_inbound_ledgers::SharedInboundLedgers;
use crate::state::application_root::ApplicationRoot;

// ─── Operating Mode ──────────────────────────────────────────────────────────

/// Operating mode (matches rippled OperatingMode). AtomicU8 for lock-free reads.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum OperatingMode {
    Disconnected = 0,
    Connected = 1,
    Syncing = 2,
    Tracking = 3,
    Full = 4,
}

impl From<u8> for OperatingMode {
    fn from(v: u8) -> Self {
        match v {
            1 => Self::Connected,
            2 => Self::Syncing,
            3 => Self::Tracking,
            4 => Self::Full,
            _ => Self::Disconnected,
        }
    }
}

// ─── Consensus Event ─────────────────────────────────────────────────────────

/// Events dispatched to the consensus driver event-loop thread.
/// Both validations and completed ledgers are processed on the same thread,
/// matching rippled where both run on JobQueue workers serialized by MasterMutex.
pub enum ConsensusEvent {
    Validation(QueuedValidation),
    LedgerDone(Arc<Ledger>),
}

// ─── CheckAcceptSink (ledger promotion on quorum) ────────────────────────────

/// Implements the `RclValidationAcceptanceSink` pattern for the driver thread.
/// When a validation arrives and quorum is met for a ledger we already have,
/// promotes it to validated immediately. If the ledger is missing, triggers
/// acquisition via SharedInboundLedgers.
struct DriverCheckAcceptSink {
    app: ApplicationRoot,
    shared_inbound: Arc<SharedInboundLedgers>,
}

impl RclValidationAcceptanceSink for DriverCheckAcceptSink {
    fn check_accept(&self, hash: Uint256, seq: u32) {
        let valid_ledger_seq = self.app.validated_ledger_seq().unwrap_or(0);

        // Skip if we already validated this or a later sequence.
        if seq != 0 && seq <= valid_ledger_seq {
            return;
        }

        // Skip if our current validated ledger matches this hash.
        let has_local = self
            .app
            .validated_ledger()
            .is_some_and(|l| *l.header().hash.as_uint256() == hash);
        if has_local {
            return;
        }

        // Immediate promotion: if we have the ledger in history AND enough
        // validations, promote to validated RIGHT NOW (matching rippled's
        // event-driven checkAccept that fires on each validation receipt).
        if let Some(lm_rt) = self.app.ledger_master_runtime() {
            let lm = lm_rt.ledger_master();
            if let Some(ledger) =
                lm.get_ledger_by_hash(SHAMapHash::new(hash))
            {
                let validations = self
                    .app
                    .validations()
                    .store()
                    .trusted_for_ledger_by_sequence(hash, seq);
                let val_count = self
                    .app
                    .validators()
                    .negative_unl_filter_validations(validations)
                    .len();
                let needed = if self.app.standalone() {
                    0
                } else {
                    self.app.validators().quorum()
                };
                if val_count >= needed {
                    let mut promoted = self.app.ledger_with_node_fetcher(ledger);
                    {
                        let l = Arc::make_mut(&mut promoted);
                        l.set_validated();
                        l.set_full();
                        l.finalize_immutable_no_setup();
                    }
                    lm.ledger_history()
                        .insert(Arc::clone(&promoted), true);
                    lm.mark_ledger_complete(promoted.header().seq);
                    lm.set_valid_ledger_no_sweep(Arc::clone(&promoted), None, None);
                    if lm.published_ledger().is_none() {
                        lm.set_pub_ledger(Arc::clone(&promoted));
                    }
                    tracing::info!(
                        target: "consensus",
                        seq = promoted.header().seq,
                        hash = %hash,
                        validators = val_count,
                        "Driver: promoted validated ledger"
                    );
                    return;
                }
            }
        }

        if seq != 0 && !hash.is_zero() {
            if let Some(lm_rt) = self.app.ledger_master_runtime() {
                if let Some(sil) = lm_rt.shared_inbound_ledgers.lock()
                    .expect("shared_inbound_ledgers lock").as_ref()
                {
                    sil.acquire(hash, seq);
                } else {
                    self.shared_inbound.acquire(hash, seq);
                }
            } else {
                self.shared_inbound.acquire(hash, seq);
            }
        }

        // Notify overlay of the target validated sequence for tracking mode.
        if seq != 0
            && valid_ledger_seq == 0
        {
            if let Some(overlay) = self.app.overlay_runtime() {
                overlay.overlay().check_tracking(seq);
            }
        }
    }
}

// ─── ConsensusDriver ─────────────────────────────────────────────────────────

/// The consensus driver. Type-parameterized by the RCL adaptor.
pub struct ConsensusDriver<A: RclConsensusAdapter> {
    /// The RclConsensus engine (behind parking_lot::Mutex)
    engine: Mutex<consensus::RclConsensus<A>>,

    /// Operating mode
    mode: AtomicU8,

    /// Last locally-validated sequence (canValidateSeq enforcement)
    last_validated_seq: AtomicU32,

    /// Stop flag for heartbeat
    stop: AtomicBool,

    /// Minimum peer count before consensus ticks
    min_peer_count: usize,
}

impl<A: RclConsensusAdapter> ConsensusDriver<A> {
    /// Create a new driver wrapping an existing RclConsensus engine.
    pub fn new(engine: consensus::RclConsensus<A>, min_peer_count: usize) -> Self {
        Self {
            engine: Mutex::new(engine),
            mode: AtomicU8::new(OperatingMode::Connected as u8),
            last_validated_seq: AtomicU32::new(0),
            stop: AtomicBool::new(false),
            min_peer_count,
        }
    }

    /// Called by the heartbeat thread every 1 second.
    /// Matches rippled's processHeartbeatTimer.
    pub fn heartbeat(&self, now: NetClockTimePoint, peer_count: usize) {
        // Gate: don't tick consensus without peers (prevents solo closing)
        if peer_count < self.min_peer_count {
            self.set_mode(OperatingMode::Disconnected);
            return;
        }

        // Mode promotion: DISCONNECTED → CONNECTED
        if self.mode() == OperatingMode::Disconnected {
            self.set_mode(OperatingMode::Connected);
        }

        // Tick the consensus state machine.
        // RclConsensus::timer_tick is async (due to vestigial RclRoundTimer),
        // so we use a minimal block_on.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("heartbeat tokio runtime");
        let pending_round = rt.block_on(async {
            let mut engine = self.engine.lock();
            let _decision = engine.timer_tick(now).await;
            engine.adaptor().inner.take_pending_start_round()
        });

        // Start next round (re-acquires lock)
        if let Some((round_now, prev_id, prev_cx)) = pending_round {
            self.engine.lock().start_round(round_now, prev_id, prev_cx);
        }
    }

    /// Called from overlay peer handler when a trusted proposal arrives.
    /// No batching — takes the mutex, updates state, returns.
    pub fn peer_proposal(&self, now: NetClockTimePoint, pos: RclCxPeerPos) -> bool {
        self.engine.lock().peer_proposal(now, pos)
    }

    /// Called from overlay when a transaction set is received.
    pub fn got_tx_set(&self, now: NetClockTimePoint, txset: Vec<RclCxTx>) {
        self.engine.lock().got_tx_set(now, txset);
    }

    /// Start the first consensus round (before overlay starts).
    pub fn begin_consensus(&self, now: NetClockTimePoint, prev_id: Uint256, prev: RclCxLedger) {
        self.engine.lock().start_round(now, prev_id, prev);
        tracing::info!(target: "consensus", "ConsensusDriver: initial round started");
    }

    /// Get the current operating mode (lock-free).
    pub fn mode(&self) -> OperatingMode {
        OperatingMode::from(self.mode.load(Ordering::Acquire))
    }

    /// Set operating mode.
    pub fn set_mode(&self, mode: OperatingMode) {
        self.mode.store(mode as u8, Ordering::Release);
    }

    /// canValidateSeq: returns true only for strictly increasing sequences.
    pub fn can_validate_seq(&self, seq: u32) -> bool {
        let current = self.last_validated_seq.load(Ordering::Acquire);
        if seq <= current {
            return false;
        }
        self.last_validated_seq.store(seq, Ordering::Release);
        true
    }

    pub fn last_validated_seq(&self) -> u32 {
        self.last_validated_seq.load(Ordering::Acquire)
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
    }

    pub fn is_stopped(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }

    /// Access the engine under lock (for state queries like consensus_info RPC)
    pub fn with_engine<R>(&self, f: impl FnOnce(&consensus::RclConsensus<A>) -> R) -> R {
        f(&self.engine.lock())
    }
}

// ─── Heartbeat Spawn ─────────────────────────────────────────────────────────

/// Spawn the heartbeat thread. Returns JoinHandle for clean shutdown.
pub fn spawn_heartbeat<A: RclConsensusAdapter + 'static>(
    driver: Arc<ConsensusDriver<A>>,
    get_peer_count: impl Fn() -> usize + Send + 'static,
    get_now: impl Fn() -> NetClockTimePoint + Send + 'static,
) -> JoinHandle<()> {
    thread::Builder::new()
        .name("consensus-heartbeat".into())
        .spawn(move || {
            tracing::info!(target: "consensus", "Heartbeat thread started (1s)");
            loop {
                thread::sleep(Duration::from_secs(1));
                if driver.is_stopped() {
                    break;
                }
                driver.heartbeat(get_now(), get_peer_count());
            }
            tracing::info!(target: "consensus", "Heartbeat thread stopped");
        })
        .expect("spawn consensus-heartbeat")
}

// ─── Event-Loop Spawn ────────────────────────────────────────────────────────

/// Spawn the consensus event-loop thread.
///
/// This thread processes validations and completed ledger results on a single
/// thread, matching rippled where both `JtValidationT` and `JtLedgerData` jobs
/// execute in the same JobQueue (serialized by MasterMutex).
///
/// The thread blocks on `event_rx.recv()` and wakes instantly when a validation
/// or ledger completion arrives. No polling, no sleep.
///
/// # Arguments
///
/// * `app` — The ApplicationRoot (access to validators, validations store, ledger_master, time_keeper)
/// * `shared_inbound` — SharedInboundLedgers for triggering acquisition of missing ledgers
/// * `event_rx` — Unified channel receiving both validations and ledger completions
/// * `stop` — Shared stop flag (same as the ConsensusDriver stop flag)
pub fn spawn_event_loop(
    app: ApplicationRoot,
    shared_inbound: Arc<SharedInboundLedgers>,
    event_rx: Receiver<ConsensusEvent>,
    stop: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::Builder::new()
        .name("consensus-driver".into())
        .spawn(move || {
            tracing::info!(target: "consensus", "Consensus driver thread started (single-owner)");

            let accept_sink = DriverCheckAcceptSink {
                app: app.clone(),
                shared_inbound: Arc::clone(&shared_inbound),
            };

            let mut last_tick = Instant::now();
            let tick_interval = Duration::from_secs(1);

            loop {
                if stop.load(Ordering::Acquire) {
                    break;
                }

                let until_tick = tick_interval.saturating_sub(last_tick.elapsed());
                let timeout = until_tick.min(Duration::from_millis(50));

                match event_rx.recv_timeout(timeout) {
                    Ok(ConsensusEvent::Validation(queued)) => {
                        handle_validation(&app, &accept_sink, &queued);
                    }
                    Ok(ConsensusEvent::LedgerDone(ledger)) => {
                        handle_ledger_done(&app, &accept_sink, &shared_inbound, ledger);
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                }

                // Drain proposals on EVERY iteration (matching rippled where
                // peerProposal runs immediately via JobQueue, not batched).
                drain_proposals(&app);

                if last_tick.elapsed() >= tick_interval {
                    last_tick = Instant::now();
                    tick_timer(&app);
                }
            }

            tracing::info!(target: "consensus", "Consensus driver thread stopped");
        })
        .expect("spawn consensus-driver")
}

fn drain_proposals(app: &ApplicationRoot) {
    let (Some(consensus_rt), Some(network_ops_rt)) =
        (app.consensus_runtime(), app.network_ops_runtime())
    else {
        return;
    };
    network_ops_rt.drain_proposals(consensus_rt.as_ref());
}

fn tick_timer(app: &ApplicationRoot) {
    let (Some(consensus_rt), Some(network_ops_rt)) =
        (app.consensus_runtime(), app.network_ops_runtime())
    else {
        return;
    };

    let peer_count = app
        .overlay_runtime()
        .map(|ort| {
            use overlay::Overlay;
            ort.overlay().active_peers().len()
        })
        .unwrap_or(0);

    let min_peers = {
        let unl_size = app.validators().count();
        if unl_size <= 1 { 1 } else { (unl_size - 1) / 2 }
    };
    if peer_count < min_peers {
        return;
    }

    if let Some(lm_rt) = app.ledger_master_runtime() {
        if let Some(closed) = lm_rt.ledger_master().closed_ledger() {
            network_ops_rt.maybe_begin_consensus_from_validated(
                consensus_rt.as_ref(),
                std::sync::Arc::clone(&closed),
            );
        }
    }

    network_ops_rt.handle_consensus_timer(consensus_rt.as_ref());
}

/// Create the event channel pair for the consensus driver.
/// Returns (sender, receiver) — sender is cloneable for multiple producers.
pub fn consensus_event_channel() -> (Sender<ConsensusEvent>, Receiver<ConsensusEvent>) {
    std::sync::mpsc::channel()
}

// ─── Validation Handler ──────────────────────────────────────────────────────

/// Process a single queued validation.
///
/// Matches rippled's flow:
/// `PeerImp::checkValidation` → `NetworkOPs::recvValidation` →
/// `handleNewValidation` → `checkAccept`
fn handle_validation(
    app: &ApplicationRoot,
    accept_sink: &DriverCheckAcceptSink,
    queued: &QueuedValidation,
) {
    // Deserialize the STValidation from the wire bytes.
    let mut serial = SerialIter::new(&queued.message.validation);
    let parsed = STValidation::from_serial_iter_default_node_id(&mut serial, false);
    let mut validation = match parsed {
        Ok(v) => v,
        Err(e) => {
            tracing::trace!(
                target: "consensus",
                peer = queued.peer_id,
                error = ?e,
                "Failed to deserialize validation"
            );
            return;
        }
    };

    // Set seen_time to now, matching reference which sets it on receipt.
    // Without this, seen_time=0 makes local_age check fail in is_current().
    let now_wall = app.current_close_time_seconds();
    validation.set_seen(now_wall);

    // Adjust our clock from the validator's sign_time so is_current() passes.
    // Validators are authoritative on network time.
    let sign_time = validation.get_sign_time();
    if sign_time > 0 {
        let offset = sign_time as i64 - now_wall as i64;
        app.time_keeper()
            .adjust_close_time(time::Duration::seconds(offset));
    }

    // Feed into the validation store with the acceptance sink.
    // This calls handleNewValidation → validations.add → checkAccept via sink.
    let source = queued.peer_id.to_string();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        app.receive_validation_to_network_ops_with_accept(&mut validation, &source, accept_sink)
    }));
    let report = match result {
        Ok(r) => r,
        Err(e) => {
            let msg = e
                .downcast_ref::<String>()
                .map(|s| s.as_str())
                .or_else(|| e.downcast_ref::<&str>().copied())
                .unwrap_or("unknown");
            tracing::warn!(
                target: "consensus",
                error = msg,
                "Panic in receive_validation_to_network_ops_with_accept"
            );
            return;
        }
    };

    if let Some(ref report) = report {
        tracing::debug!(
            target: "consensus",
            hash = %report.ledger_hash,
            trusted = report.trusted,
            current = report.current,
            bypass = report.bypass_accept,
            "Validation received"
        );
    }

    // Relay trusted validations to other peers (matching rippled relay behavior).
    if let Some(report) = &report
        && report.relay
    {
        if let Some(overlay_rt) = app.overlay_runtime() {
            overlay_rt.overlay().relay_validation(
                queued.message.clone(),
                queued.suppression,
                *validation.get_signer_public(),
            );
        }
    }
}

// ─── Ledger Completion Handler ───────────────────────────────────────────────

/// Process a completed ledger from an InboundLedger worker.
///
/// Matches rippled's flow:
/// `InboundLedger::done` → `storeLedger` → `checkAccept` → promote
fn handle_ledger_done(
    app: &ApplicationRoot,
    accept_sink: &DriverCheckAcceptSink,
    shared_inbound: &Arc<SharedInboundLedgers>,
    ledger: Arc<Ledger>,
) {
    let hash = *ledger.header().hash.as_uint256();
    let seq = ledger.header().seq;

    tracing::info!(
        target: "consensus",
        seq,
        hash = %hash,
        "Driver: received completed ledger from InboundLedger"
    );

    // Store in LedgerHistory so it's visible to all threads immediately.
    if let Some(lm_rt) = app.ledger_master_runtime() {
        let lm = lm_rt.ledger_master();
        lm.ledger_history().insert(Arc::clone(&ledger), false);
    }

    // Mark complete in the SharedInboundLedgers registry.
    shared_inbound.mark_complete(&hash);

    // Call check_acquired() on the validations store.
    // This populates the validation trie for nodes that were waiting on this
    // ledger, so getPreferred/checkAccept can now see them.
    {
        let validations_lock = app.validations().validations();
        let mut validations = validations_lock
            .lock()
            .expect("validations mutex for check_acquired");
        validations.check_acquired();
    }

    // Now run checkAccept: if quorum is met for this ledger, promote it.
    accept_sink.check_accept(hash, seq);
}
