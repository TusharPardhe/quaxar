//! `ConsensusDriver` — the single entry point for consensus participation.
//!
//! Replaces the fragmented AppConsensusRuntime + NetworkOpsRuntime + bootstrap
//! timer with a unified architecture matching rippled:
//!
//! - ONE `parking_lot::Mutex` protects the consensus state machine
//! - ONE heartbeat thread (1s) drives `timer_entry`
//! - Proposals arrive DIRECTLY from overlay threads (no batching)
//! - Accept work runs inside on_accept → pending_start_round (existing pattern)
//!
//! See docs/CONSENSUS_REDESIGN.md for the full architecture.

use basics::chrono::NetClockTimePoint;
use consensus::{RclCxLedger, RclCxPeerPos, RclCxTx, RclConsensusAdapter};
use basics::base_uint::Uint256;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

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
