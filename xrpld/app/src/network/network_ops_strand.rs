//! NetworkOPs timer strand — matching rippled's `NetworkOPsImp` strand model.
//!
//! ONE dedicated thread exclusively owns the `ConsensusRunner` and drives:
//! - `peer_proposal()` — proposals from overlay peers
//! - `timer_entry()` — 1-second consensus timer tick
//! - `execute_accept()` — build accepted ledger when consensus closes
//! - `got_tx_set()` — tx-set completion from InboundTransactions
//! - `start_round()` — begin next consensus round
//! - `checkAccept()` — promote ledger to validated when quorum met
//! - `tryAdvance()` — publish validated ledgers and trigger history fill
//! - Operating mode promotion (Connected → Tracking → Full)
//!
//! This matches rippled's single-strand guarantee: only ONE thread ever
//! accesses the consensus state machine. No mutex protects it because only
//! this thread touches it.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use basics::base_uint::Uint256;
use consensus::algorithm::ConsensusPhase;

use crate::ApplicationRoot;
use crate::consensus::rcl_consensus::{ConsensusRunner, PendingAcceptWork};
use crate::consensus::rcl_validation::RclValidatedLedger;
use crate::job::job_queue::JobQueue;
use crate::job::job_types::JobType;
use crate::ledger::inbound_ledgers::{AcquireReason, InboundLedgers};
use crate::network::network_ops::NetworkOpsOperatingMode;
use crate::runtime::component_runtime::{AppConsensusRuntime, ConsensusCommand};

use overlay::inbound::QueuedProposal;

// History acquisition is retried promptly after the registry finishes a
// ledger. InboundLedgers deduplicates by hash/sequence, as rippled does.
const HISTORY_BACKFILL_RETRY_INTERVAL: Duration = Duration::from_millis(200);
/// Rippled expires a fetch-pack request after one second so a silent peer
/// cannot permanently suppress a new request for the same missing ledger.
const HISTORY_FETCH_PACK_STALE_AFTER: Duration = Duration::from_secs(1);

// A polling turn must always return to the heartbeat scheduler. The overlay
// channels can be continuously non-empty under peer load; draining either one
// without a budget would otherwise defer the next JtNetopTimer forever.
// A command burst should not defer a heartbeat indefinitely either.
const MAX_COMMANDS_PER_TURN: usize = 64;
const MAX_PROPOSALS_PER_TURN: usize = 64;
const MAX_TXSET_COMPLETIONS_PER_TURN: usize = 64;
const MAX_MAP_COMPLETIONS_PER_TURN: usize = 64;
const MAX_LEDGER_COMPLETIONS_PER_TURN: usize = 64;
const MAX_STRAND_INGRESS_QUEUE: usize = 1_024;
const MAX_STRAND_COMMAND_QUEUE: usize = 128;
/// Bound per-pass LCL diagnostics during persistent WrongLedger recovery.
/// State transitions (switches and rejections) remain unsampled.
const LCL_AUDIT_SAMPLE_INTERVAL: Duration = Duration::from_secs(5);

struct LclAuditSampler {
    last_emitted: Instant,
}

impl LclAuditSampler {
    fn new() -> Self {
        Self {
            last_emitted: Instant::now() - LCL_AUDIT_SAMPLE_INTERVAL,
        }
    }

    fn should_emit(&mut self) -> bool {
        if self.last_emitted.elapsed() < LCL_AUDIT_SAMPLE_INTERVAL {
            return false;
        }
        self.last_emitted = Instant::now();
        true
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PreferredLclReconciliation {
    NoChange,
    Pending,
    Switched,
}

/// Result of the strand-owned second phase of inbound completion. A result is
/// acknowledged only after its intended lifecycle transition is durable or
/// intentionally cache-only; retry retains the acquisition in the registry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CompletionPersistence {
    inserted: bool,
    acknowledged: bool,
}

/// Typed JobQueue handoff for work that must ultimately mutate consensus.
///
/// Rippled schedules heartbeats as `JtNetopTimer` and accepted-ledger work as
/// `JtAccept`. Quaxar's runner is deliberately owned only by the strand, so
/// the queued closure cannot call it directly. Instead each typed job sends a
/// command to that sole owner. This retains the queue's existing priority and
/// limit policy without adding a second scheduler or permitting concurrent
/// `timer_tick`/`execute_accept` calls.
#[derive(Clone)]
struct ConsensusJobScheduler {
    job_queue: JobQueue,
    command_tx: std::sync::mpsc::SyncSender<ConsensusCommand>,
    heartbeat_queued: Arc<AtomicBool>,
    accept_queued: Arc<AtomicBool>,
    /// Accepted-ledger work remains here until a JtAccept worker has
    /// successfully handed it to the strand command queue. A full bounded
    /// queue must delay, never discard, the `doAccept` transition.
    pending_accept: Arc<Mutex<Option<PendingAcceptWork>>>,
}

impl ConsensusJobScheduler {
    fn new(job_queue: JobQueue, command_tx: std::sync::mpsc::SyncSender<ConsensusCommand>) -> Self {
        Self {
            job_queue,
            command_tx,
            heartbeat_queued: Arc::new(AtomicBool::new(false)),
            accept_queued: Arc::new(AtomicBool::new(false)),
            pending_accept: Arc::new(Mutex::new(None)),
        }
    }

    /// Queue at most one outstanding heartbeat. The flag remains set until
    /// the command is consumed by the strand, which prevents timer-job pileup
    /// when workers are temporarily saturated.
    fn schedule_heartbeat(&self) -> bool {
        if self
            .heartbeat_queued
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return false;
        }

        let command_tx = self.command_tx.clone();
        let heartbeat_queued = Arc::clone(&self.heartbeat_queued);
        if self
            .job_queue
            .add_job(JobType::JtNetopTimer, "NetHeart", move || {
                if command_tx.try_send(ConsensusCommand::Heartbeat).is_err() {
                    heartbeat_queued.store(false, Ordering::Release);
                }
            })
        {
            true
        } else {
            self.heartbeat_queued.store(false, Ordering::Release);
            false
        }
    }

    /// Queue at most one accepted-ledger handoff. `execute_accept` stays on
    /// the strand so its full `doAccept → endConsensus → startRound` ordering
    /// is serialized with timer, proposal, and tx-set mutations.
    fn schedule_accept(&self, work: PendingAcceptWork) -> bool {
        let mut pending = self.pending_accept.lock().expect("pending accept lock");
        if pending.is_some() {
            return false;
        }
        *pending = Some(work);
        drop(pending);
        self.schedule_pending_accept()
    }

    /// Retry an accept handoff retained because the bounded command queue or
    /// the JobQueue was saturated. The work is removed only after `try_send`
    /// succeeds, so accepted-phase recovery cannot race ahead of it.
    fn schedule_pending_accept(&self) -> bool {
        if self
            .accept_queued
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return false;
        }

        let command_tx = self.command_tx.clone();
        let accept_queued = Arc::clone(&self.accept_queued);
        let pending_accept = Arc::clone(&self.pending_accept);
        if self
            .job_queue
            .add_job(JobType::JtAccept, "AcceptLedger", move || {
                let work = pending_accept.lock().expect("pending accept lock").take();
                let Some(work) = work else {
                    accept_queued.store(false, Ordering::Release);
                    return;
                };
                match command_tx.try_send(ConsensusCommand::Accept(work)) {
                    Ok(()) => {}
                    Err(std::sync::mpsc::TrySendError::Full(ConsensusCommand::Accept(work)))
                    | Err(std::sync::mpsc::TrySendError::Disconnected(ConsensusCommand::Accept(
                        work,
                    ))) => {
                        *pending_accept.lock().expect("pending accept lock") = Some(work);
                        accept_queued.store(false, Ordering::Release);
                    }
                    Err(_) => unreachable!("only Accept commands are sent here"),
                }
            })
        {
            true
        } else {
            self.accept_queued.store(false, Ordering::Release);
            false
        }
    }

    /// True while accepted work has been captured (schedule_accept) but the
    /// JtAccept job has not yet handed it back to the strand as a command.
    /// This is the async window between scheduling and execution that
    /// reconciliation/StartRound must not race ahead of.
    fn has_pending_accept(&self) -> bool {
        self.pending_accept
            .lock()
            .expect("pending accept lock")
            .is_some()
    }

    fn heartbeat_consumed(&self) {
        self.heartbeat_queued.store(false, Ordering::Release);
    }

    fn accept_consumed(&self) {
        self.accept_queued.store(false, Ordering::Release);
    }

    fn accept_is_queued(&self) -> bool {
        self.accept_queued.load(Ordering::Acquire)
    }
}

/// Drain at most `budget` messages so a saturated ingress channel cannot
/// monopolize the consensus owner.
fn drain_bounded<T>(
    receiver: &std::sync::mpsc::Receiver<T>,
    budget: usize,
    mut handle: impl FnMut(T),
) {
    for _ in 0..budget {
        match receiver.try_recv() {
            Ok(value) => handle(value),
            Err(
                std::sync::mpsc::TryRecvError::Empty | std::sync::mpsc::TryRecvError::Disconnected,
            ) => break,
        }
    }
}

fn announce_completed_tx_set(root: &ApplicationRoot, hash: Uint256) {
    use overlay::Overlay;

    let Some(overlay_runtime) = root.overlay_runtime() else {
        return;
    };
    let message = overlay::ProtocolMessage::new(overlay::ProtocolPayload::HaveSet(
        overlay::TmHaveTransactionSet {
            status: 1, // protocol::tsHAVE
            hash: hash.data().to_vec(),
        },
    ));
    overlay_runtime.overlay().broadcast(&message);
}

/// Dependencies the strand needs (passed at construction).
pub struct NetworkOpsStrandDeps {
    pub root: ApplicationRoot,
    pub consensus_rt: Arc<AppConsensusRuntime>,
    pub shared_inbound: Arc<InboundLedgers>,
    pub configured_ledger_history: u32,
    /// Consensus event channel sender for LedgerDone events from storeLedger drain.
    pub event_tx: Option<std::sync::mpsc::SyncSender<crate::consensus::driver::ConsensusEvent>>,
    /// Receiver for completed ledgers from shared_inbound acquisition.
    pub shared_completed_rx:
        Option<std::sync::mpsc::Receiver<crate::ledger::inbound_ledgers::CompletedInboundLedger>>,
}

/// External handle to the running strand. Drop to stop.
pub struct NetworkOpsStrand {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    /// Send proposals from the overlay to the strand.
    pub proposal_tx: std::sync::mpsc::SyncSender<QueuedProposal>,
    /// Send tx-set completions to the strand.
    pub txset_tx: std::sync::mpsc::SyncSender<(Uint256, Arc<shamap::sync::SyncTree>)>,
    /// Send commands (StartRound, Stop) to the strand.
    pub command_tx: std::sync::mpsc::SyncSender<ConsensusCommand>,
}

impl NetworkOpsStrand {
    /// Spawn the strand thread. Takes ownership of the consensus runner.
    pub fn spawn(deps: NetworkOpsStrandDeps) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let (proposal_tx, proposal_rx) =
            std::sync::mpsc::sync_channel::<QueuedProposal>(MAX_STRAND_INGRESS_QUEUE);
        let (txset_tx, txset_rx) = std::sync::mpsc::sync_channel::<(
            Uint256,
            Arc<shamap::sync::SyncTree>,
        )>(MAX_STRAND_INGRESS_QUEUE);
        let (command_tx, command_rx) =
            std::sync::mpsc::sync_channel::<ConsensusCommand>(MAX_STRAND_COMMAND_QUEUE);

        // Wire the command sender to the consensus runtime so external code
        // (e.g. validation event loop) can issue StartRound commands.
        deps.consensus_rt.set_cmd_sender(command_tx.clone());

        let stop_clone = Arc::clone(&stop);
        let strand_command_tx = command_tx.clone();
        let thread = thread::Builder::new()
            .name("networkops-strand".into())
            .spawn(move || {
                strand_loop(
                    deps,
                    stop_clone,
                    proposal_rx,
                    txset_rx,
                    command_rx,
                    strand_command_tx,
                );
            })
            .expect("failed to spawn networkops-strand thread");

        Self {
            stop,
            thread: Some(thread),
            proposal_tx,
            txset_tx,
            command_tx,
        }
    }

    /// Signal the strand to stop and wait for the thread to exit.
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = self.command_tx.try_send(ConsensusCommand::Stop);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for NetworkOpsStrand {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = self.command_tx.try_send(ConsensusCommand::Stop);
        // Don't join on drop — just signal.
    }
}

// ─── Strand thread body ──────────────────────────────────────────────────────

fn strand_loop(
    deps: NetworkOpsStrandDeps,
    stop: Arc<AtomicBool>,
    proposal_rx: std::sync::mpsc::Receiver<QueuedProposal>,
    txset_rx: std::sync::mpsc::Receiver<(Uint256, Arc<shamap::sync::SyncTree>)>,
    command_rx: std::sync::mpsc::Receiver<ConsensusCommand>,
    command_tx: std::sync::mpsc::SyncSender<ConsensusCommand>,
) {
    // Elevate thread priority — consensus must never be starved by RPC load.
    #[cfg(unix)]
    unsafe {
        libc::setpriority(0, 0, -15);
    }

    tracing::info!(target: "consensus", "NetworkOPs strand running");

    let NetworkOpsStrandDeps {
        root,
        consensus_rt,
        shared_inbound,
        configured_ledger_history,
        event_tx,
        shared_completed_rx,
    } = deps;

    // Take the consensus runner — it now lives exclusively on this thread.
    let mut runner = match consensus_rt.take_runner() {
        Some(r) => r,
        None => {
            tracing::error!(target: "consensus", "No consensus runner available, exiting strand");
            return;
        }
    };

    // Take the map-complete receiver for tx-set acquisitions.
    let map_complete_rx = consensus_rt.take_map_complete_receiver();

    let scheduler = ConsensusJobScheduler::new(root.job_queue().clone(), command_tx);
    let mut consensus_started = false;
    let mut last_timer_tick = Instant::now();
    let mut last_round_ledger_id: Option<Uint256> = None;
    let mut last_history_tick = Instant::now();
    let mut history_fetch_pack: Option<(u32, Instant)> = None;
    // Match rippled's `acquiringLedger_`: only issue ONE acquireAsync per
    // unique preferred-LCL hash. Prevents flooding peers with parallel
    // Sample repeating LCL diagnostics while leaving recovery decisions and
    // state-transition logs complete.
    let mut lcl_audit_sampler = LclAuditSampler::new();

    // Detect startup: always start consensus immediately on the closed
    // ledger, matching rippled's Application::run() which calls
    // beginConsensus(closedLedger.hash) unconditionally.
    {
        let _startup_lcl_transition_guard = root.lcl_transition_gate().lock();
        if let Some(closed) = root.closed_ledger() {
            let now = root.shared_time_keeper().close_time();
            let prev_id = *closed.header().hash.as_uint256();
            let prev_cx = crate::consensus_ledger_from_ledger(&closed);
            if let Some(inbound_tx) = root.inbound_transactions().lock().ok().as_mut() {
                inbound_tx.new_round(closed.header().seq);
            }
            runner.start_round(now, prev_id, prev_cx, true);
            consensus_rt.update_phase(runner.phase());
            consensus_rt.update_prev_ledger_id(runner.prev_ledger_id());
            consensus_started = true;
            last_round_ledger_id = Some(prev_id);
            last_timer_tick = Instant::now();
            tracing::info!(target: "consensus", seq = closed.header().seq,
            "Consensus started on closed ledger (matching rippled beginConsensus)");
        }
    }

    // ═══════════════════════════════════════════════════════════════════════
    // MAIN STRAND LOOP — matches rippled's NetworkOPs::heartbeatTimer
    // ═══════════════════════════════════════════════════════════════════════
    while !stop.load(Ordering::Acquire) {
        // Retry retained accept work before doing any fallback that could
        // start a new round. A full command queue delays acceptance but never
        // permits the accepted-phase recovery path to bypass it.
        if scheduler.has_pending_accept() && !scheduler.accept_is_queued() {
            let _ = scheduler.schedule_pending_accept();
        }

        // ─── 1. Process serialized commands ──────────────────────────────
        // Worker jobs and external callers can enqueue commands, but only this
        // thread owns `runner`, so timer, accept, proposal, and round changes
        // cannot mutate consensus concurrently.
        for _ in 0..MAX_COMMANDS_PER_TURN {
            let Ok(cmd) = command_rx.try_recv() else {
                break;
            };
            match cmd {
                ConsensusCommand::Heartbeat => {
                    scheduler.heartbeat_consumed();
                    let now = root.shared_time_keeper().close_time();
                    if let Some(work) = runner.timer_tick(now)
                        && !scheduler.schedule_accept(work)
                    {
                        // A second accept result cannot normally occur before
                        // the first is consumed: timer_entry remains in
                        // Accepted. Never execute it inline on this path.
                        tracing::error!(target: "consensus", "failed to queue JtAccept handoff");
                    }
                    consensus_rt.update_phase(runner.phase());
                    consensus_rt.update_prev_ledger_id(runner.prev_ledger_id());
                    last_timer_tick = Instant::now();
                }
                ConsensusCommand::Accept(work) => {
                    // This command was emitted by a JtAccept job. Execute the
                    // whole accept/end/start transition on the one owner.
                    let now = root.shared_time_keeper().close_time();
                    runner.execute_accept(now, work);
                    scheduler.accept_consumed();
                    consensus_rt.update_phase(runner.phase());
                    consensus_rt.update_prev_ledger_id(runner.prev_ledger_id());
                    last_round_ledger_id = Some(runner.prev_ledger_id());
                }
                ConsensusCommand::StartRound {
                    now,
                    network_closed,
                    prev_ledger,
                } => {
                    let _lcl_transition_guard = root.lcl_transition_gate().lock();
                    let local_prev_id = prev_ledger.id();
                    if runner.phase() != ConsensusPhase::Accepted
                        || scheduler.accept_is_queued()
                        || scheduler.has_pending_accept()
                        || network_closed != local_prev_id
                        || root.closed_ledger().is_none_or(|current| {
                            *current.header().hash.as_uint256() != local_prev_id
                        })
                    {
                        tracing::warn!(
                            target: "consensus",
                            %local_prev_id,
                            %network_closed,
                            accept_is_queued = scheduler.accept_is_queued(),
                            has_pending_accept = scheduler.has_pending_accept(),
                            "discarded stale or recovery-conflicting external start-round command"
                        );
                        continue;
                    }
                    if let Some(inbound_tx) = root.inbound_transactions().lock().ok().as_mut() {
                        inbound_tx.new_round(prev_ledger.seq());
                    }
                    runner.start_round(now, network_closed, prev_ledger, true);
                    consensus_rt.update_phase(runner.phase());
                    consensus_rt.update_prev_ledger_id(runner.prev_ledger_id());
                    consensus_started = true;
                    last_round_ledger_id = Some(runner.prev_ledger_id());
                    last_timer_tick = Instant::now();
                    tracing::info!(target: "consensus", "Consensus started via external command");
                }
                ConsensusCommand::Stop => {
                    tracing::info!(target: "consensus", "Strand received Stop command");
                    return;
                }
            }
        }

        if !consensus_started {
            // Should not happen — consensus starts unconditionally above.
            thread::sleep(Duration::from_millis(50));
            continue;
        }

        // ─── 1b. Mode demotion on insufficient peers (matching rippled processHeartbeatTimer)
        if let Some(overlay_rt) = root.overlay_runtime() {
            use overlay::Overlay;
            let num_peers = overlay_rt.overlay().size();
            let min_peers: usize = 1; // rippled default minPeerCount_
            let current_mode = root.network_ops_state().operating_mode();
            if num_peers < min_peers {
                if current_mode != NetworkOpsOperatingMode::Disconnected {
                    root.set_network_ops_operating_mode(NetworkOpsOperatingMode::Disconnected);
                    tracing::warn!(target: "consensus", num_peers, min_peers, "Peer count below minimum — mode set to DISCONNECTED");
                }
                // Skip consensus timer when disconnected (matching rippled)
                root.wait_consensus_or_timeout(Duration::from_millis(500));
                continue;
            } else if current_mode == NetworkOpsOperatingMode::Disconnected {
                root.set_network_ops_operating_mode(NetworkOpsOperatingMode::Connected);
                tracing::info!(target: "consensus", num_peers, "Peer count sufficient — mode set to CONNECTED");
            }
        }

        // ─── 2. Schedule the 1s heartbeat before processing peer ingress ──
        // `JtNetopTimer` has the reference priority/limit. The job only
        // hands off a command; `timer_tick` remains serialized on this strand.
        if last_timer_tick.elapsed() >= Duration::from_secs(1) {
            let _ = scheduler.schedule_heartbeat();
        }

        // ─── 3. Drain a bounded proposal slice → peer_proposal() ─────────
        drain_bounded(&proposal_rx, MAX_PROPOSALS_PER_TURN, |proposal| {
            let now = root.shared_time_keeper().close_time();
            let peer_close_time =
                basics::chrono::NetClockTimePoint::new(proposal.message.close_time);
            // Match PeerImp::onMessage: the consensus position is keyed by
            // the validator's master key, while `RclCxPeerPos::public_key`
            // retains the signing key used to verify and relay the proposal.
            let master_key = root.manifest_cache().get_master_key(&proposal.public_key);
            let prop = consensus::ConsensusProposal::new(
                proposal.previous_ledger,
                proposal.message.propose_seq,
                proposal.current_tx_hash,
                peer_close_time,
                now,
                master_key,
            );
            let peer_pos = crate::consensus::rcl_cx_peer_pos::RclCxPeerPos::new(
                proposal.public_key,
                proposal.message.signature.clone(),
                proposal.suppression,
                prop,
            );
            runner.peer_proposal(now, &peer_pos);
        });
        consensus_rt.update_phase(runner.phase());
        consensus_rt.update_prev_ledger_id(runner.prev_ledger_id());

        // ─── 4. Drain a bounded tx-set completion slice → got_tx_set() ───
        drain_bounded(&txset_rx, MAX_TXSET_COMPLETIONS_PER_TURN, |(hash, set)| {
            let now = root.shared_time_keeper().close_time();
            let tx_set = consensus::RclTxSet::from_parts(
                set.root(),
                Arc::clone(runner.adaptor.tx_set_cache()),
                set.backed(),
                0,
            );
            runner.got_tx_set(now, tx_set);
            announce_completed_tx_set(&root, hash);
            consensus_rt.update_phase(runner.phase());
            tracing::debug!(target: "consensus", %hash, "strand: got_tx_set processed");
        });

        // Also drain a bounded slice from the map-complete receiver.
        if let Some(ref rx) = map_complete_rx {
            drain_bounded(rx, MAX_MAP_COMPLETIONS_PER_TURN, |(hash, set)| {
                let now = root.shared_time_keeper().close_time();
                let tx_set = consensus::RclTxSet::from_parts(
                    set.root(),
                    Arc::clone(runner.adaptor.tx_set_cache()),
                    set.backed(),
                    0,
                );
                runner.got_tx_set(now, tx_set);
                announce_completed_tx_set(&root, hash);
                consensus_rt.update_phase(runner.phase());
                tracing::debug!(target: "consensus", %hash, "strand: got_tx_set (map_complete)");
            });
        }

        // Durable recovery for completion notifications that could not enter
        // the bounded map-complete channel. These are coalesced by tx-set hash
        // in InboundTransactions and drained every strand turn.
        let pending_map_completions = root
            .inbound_transactions()
            .lock()
            .ok()
            .map(|mut inbound| inbound.take_pending_map_completions(MAX_MAP_COMPLETIONS_PER_TURN))
            .unwrap_or_default();
        for (hash, set) in pending_map_completions {
            let now = root.shared_time_keeper().close_time();
            let tx_set = consensus::RclTxSet::from_parts(
                set.root(),
                Arc::clone(runner.adaptor.tx_set_cache()),
                set.backed(),
                0,
            );
            runner.got_tx_set(now, tx_set);
            announce_completed_tx_set(&root, hash);
            consensus_rt.update_phase(runner.phase());
            tracing::debug!(target: "consensus", %hash, "strand: got_tx_set (durable map completion)");
        }

        // ─── 5. Persist inbound completion before LCL reconciliation ────
        // ─── 6a. completion recovery — registry is authoritative ────────
        // The sender is only a wakeup optimization. If it is disconnected or
        // not yet drained, completed acquisition state remains recoverable
        // here and follows the same storeLedger -> checkAccept path.
        let mut registry_completion_count = 0usize;
        if let Some(lm_rt) = root.ledger_master_runtime() {
            let lm = lm_rt.ledger_master();
            let registry_completions =
                shared_inbound.poll_results_bounded(MAX_LEDGER_COMPLETIONS_PER_TURN);
            registry_completion_count = registry_completions.len();
            if registry_completion_count != 0 {
                tracing::info!(
                    target: "lcl_trace",
                    event = "completion_batch_polled",
                    source = "registry_poll",
                    count = registry_completion_count,
                    runner_phase = ?runner.phase(),
                    "LCL trace: completed inbound ledgers are ready for persistence"
                );
            }
            for (_, ledger, reason) in registry_completions {
                let ledger = Arc::new(ledger);
                let persisted = persist_completed_inbound_ledger(&root, &lm, &ledger, reason);
                trace_completed_inbound_handoff("registry_poll", &lm, &ledger, reason, persisted);
                root.check_accept_hash_seq(*ledger.header().hash.as_uint256(), ledger.header().seq);
                if persisted.acknowledged {
                    shared_inbound.acknowledge_completed(ledger.header().hash.as_uint256());
                }
                // Always register with the validations adaptor regardless of
                // whether LedgerHistory already had this ledger. A ledger can
                // be in LedgerHistory but absent from the adaptor's local map,
                // which would leave check_acquired unable to resolve it.
                root.validations().register_ledger(&ledger);
                if persisted.inserted {
                    if let Some(ref tx) = event_tx {
                        let _ = tx.try_send(crate::consensus::driver::ConsensusEvent::LedgerDone(
                            Arc::clone(&ledger),
                        ));
                    }
                }
            }
        }

        // ─── 6b. storeLedger drain — completed InboundLedger results ─────
        // Moved from the polling loop: drain completed_ledgers_rx and
        // shared_completed_rx into LedgerHistory, matching rippled's
        // storeLedger path.
        if let Some(lm_rt) = root.ledger_master_runtime() {
            let rx_guard = lm_rt
                .completed_ledgers_rx
                .lock()
                .expect("completed_ledgers_rx");
            if let Some(rx) = rx_guard.as_ref() {
                drain_bounded(rx, MAX_LEDGER_COMPLETIONS_PER_TURN, |completion| {
                    let ledger = completion.ledger;
                    let lm = lm_rt.ledger_master();
                    let persisted =
                        persist_completed_inbound_ledger(&root, &lm, &ledger, completion.reason);
                    trace_completed_inbound_handoff(
                        "completed_ledgers_rx",
                        &lm,
                        &ledger,
                        completion.reason,
                        persisted,
                    );
                    root.check_accept_hash_seq(
                        *ledger.header().hash.as_uint256(),
                        ledger.header().seq,
                    );
                    if persisted.acknowledged {
                        shared_inbound.acknowledge_completed(ledger.header().hash.as_uint256());
                    }
                    root.validations().register_ledger(&ledger);
                    if persisted.inserted {
                        if let Some(ref tx) = event_tx {
                            let _ =
                                tx.try_send(crate::consensus::driver::ConsensusEvent::LedgerDone(
                                    Arc::clone(&ledger),
                                ));
                        }
                    }
                });
            }
        }
        if let Some(ref rx) = shared_completed_rx {
            drain_bounded(rx, MAX_LEDGER_COMPLETIONS_PER_TURN, |completion| {
                let ledger = completion.ledger;
                let persisted = root.ledger_master_runtime().map(|lm_rt| {
                    let lm = lm_rt.ledger_master();
                    let persisted =
                        persist_completed_inbound_ledger(&root, &lm, &ledger, completion.reason);
                    trace_completed_inbound_handoff(
                        "shared_completed_rx",
                        &lm,
                        &ledger,
                        completion.reason,
                        persisted,
                    );
                    persisted
                });
                root.check_accept_hash_seq(*ledger.header().hash.as_uint256(), ledger.header().seq);
                if persisted.is_some_and(|result| result.acknowledged) {
                    shared_inbound.acknowledge_completed(ledger.header().hash.as_uint256());
                }
                root.validations().register_ledger(&ledger);
                if persisted.is_some_and(|result| result.inserted) {
                    if let Some(ref tx) = event_tx {
                        let _ = tx
                            .try_send(crate::consensus::driver::ConsensusEvent::LedgerDone(ledger));
                    }
                }
            });
        }

        // ─── 6c. pending_consensus_ledger → acquire_async ────────────────
        if let Some(lm_rt) = root.ledger_master_runtime() {
            let pending = lm_rt.take_pending_consensus_ledger();
            if let Some(hash) = pending {
                tracing::info!(
                    target: "lcl_trace",
                    event = "pending_consensus_target_acquire",
                    %hash,
                    "LCL trace: pending consensus ledger is being acquired"
                );
                // A pending consensus ledger is keyed by hash only. Do not
                // infer its sequence from a peer's independent history range.
                shared_inbound.acquire_closed_ledger_async(hash, AcquireReason::Consensus);
            }
        }

        // ─── 7. endConsensus/checkLastClosedLedger → beginConsensus ────
        // Capture the phase before reconciliation can begin a WrongLedger or
        // replacement round. This is the sole endConsensus cadence token for
        // peer-status cycling, mode promotion, and the ordinary next round.
        let end_consensus_pass = should_run_end_consensus_reconciliation(
            runner.phase(),
            scheduler.accept_is_queued(),
            scheduler.has_pending_accept(),
        );
        if registry_completion_count != 0 && !end_consensus_pass {
            tracing::info!(
                target: "lcl_trace",
                event = "reconcile_deferred_after_completion",
                completion_count = registry_completion_count,
                runner_phase = ?runner.phase(),
                accept_queued = scheduler.accept_is_queued(),
                pending_accept = scheduler.has_pending_accept(),
                "LCL trace: completion persisted but preferred-LCL reconciliation awaits Accepted"
            );
        }
        if end_consensus_pass {
            tracing::info!(
                target: "lcl_trace",
                event = "end_consensus_reconcile_enter",
                runner_phase = ?runner.phase(),
                accept_queued = scheduler.accept_is_queued(),
                pending_accept = scheduler.has_pending_accept(),
                "LCL trace: entering preferred-LCL reconciliation"
            );
            // endConsensus invalidates obsolete peer status before evaluating
            // the preferred LCL, whether or not this pass later performs a jump.
            cycle_obsolete_peer_statuses(&root);
        }

        // All inbound sources above persist first. A completed ledger is
        // immediately cache history; generic WrongLedger recovery may consume
        // it regardless of later preferred-LCL observations.
        let reconciliation = if end_consensus_pass {
            reconcile_preferred_lcl(
                &root,
                &shared_inbound,
                &mut runner,
                &consensus_rt,
                &mut last_round_ledger_id,
                &mut lcl_audit_sampler,
            )
        } else {
            PreferredLclReconciliation::NoChange
        };
        if end_consensus_pass {
            tracing::info!(
                target: "lcl_trace",
                event = "end_consensus_reconcile_outcome",
                reconciliation = ?reconciliation,
                runner_phase = ?runner.phase(),
                current_mode = ?root.network_ops_operating_mode(),
                need_network_ledger = root.need_network_ledger(),
                "LCL trace: preferred-LCL reconciliation completed"
            );
        }

        // `checkAccept`/publication/history do not select, acquire, install,
        // or clear recovery intent. Mode advancement is the sole exception:
        // rippled performs it inside endConsensus only when checkLastClosedLedger
        // reports no abnormal ledger change, before beginConsensus.
        check_accept_and_advance(
            &root,
            &shared_inbound,
            configured_ledger_history,
            &mut last_history_tick,
            &mut history_fetch_pack,
            should_promote_operating_mode_at_end_consensus(end_consensus_pass, reconciliation),
        );

        // Only a no-change endConsensus pass may begin the ordinary next
        // round. A missing preferred target already began generic consensus
        // with that target (WrongLedger); a switch already began exactly one
        // replacement round (normal/SwitchedLedger handling stays generic).
        if reconciliation == PreferredLclReconciliation::NoChange
            && end_consensus_pass
            && !scheduler.accept_is_queued()
            && !scheduler.has_pending_accept()
        {
            let _lcl_transition_guard = root.lcl_transition_gate().lock();
            if let Some(closed) = root.closed_ledger() {
                let closed_id = *closed.header().hash.as_uint256();
                if last_round_ledger_id != Some(closed_id) {
                    let now = root.shared_time_keeper().close_time();
                    let prev_cx = crate::consensus_ledger_from_ledger(&closed);
                    if let Some(inbound_tx) = root.inbound_transactions().lock().ok().as_mut() {
                        inbound_tx.new_round(closed.header().seq);
                    }
                    let proposing =
                        root.network_ops_operating_mode() == NetworkOpsOperatingMode::Full;
                    runner.start_round(now, closed_id, prev_cx, proposing);
                    consensus_rt.update_phase(runner.phase());
                    consensus_rt.update_prev_ledger_id(runner.prev_ledger_id());
                    last_round_ledger_id = Some(closed_id);
                    last_timer_tick = Instant::now();
                    tracing::info!(target: "consensus", seq = closed.header().seq,
                        "Consensus started next round after checkLastClosedLedger");
                    tracing::info!(
                        target: "lcl_trace",
                        event = "ordinary_consensus_round_started",
                        local_lcl_hash = %closed_id,
                        local_lcl_seq = closed.header().seq,
                        started_prev_ledger = %runner.prev_ledger_id(),
                        proposing,
                        current_mode = ?root.network_ops_operating_mode(),
                        need_network_ledger = root.need_network_ledger(),
                        "LCL trace: ordinary consensus round started after no reconciliation change"
                    );
                    tracing::debug!(
                        target: "lcl_audit",
                        local_lcl_hash = %closed_id,
                        local_lcl_seq = closed.header().seq,
                        started_prev_ledger = %runner.prev_ledger_id(),
                        operating_mode = ?root.network_ops_operating_mode(),
                        need_network_ledger = root.need_network_ledger(),
                        "LCL_AUDIT ordinary consensus round restarted after no LCL switch"
                    );
                }
            }
        }

        // ─── 8. Wait for next event (proposal notify or 50ms timeout) ─────
        root.wait_consensus_or_timeout(Duration::from_millis(50));
    }

    tracing::info!(target: "consensus", "NetworkOPs strand stopped");
}

// ─── checkLastClosedLedger / switchLastClosedLedger ─────────────────────────

/// Matches `NetworkOPsImp::endConsensus`: peers that still report our closed
/// ledger's predecessor must refresh status before policy chooses the next LCL.
fn cycle_obsolete_peer_statuses(root: &ApplicationRoot) {
    let Some(closed) = root.closed_ledger() else {
        return;
    };
    let obsolete = *closed.header().parent_hash.as_uint256();
    if let Some(overlay_rt) = root.overlay_runtime() {
        use overlay::Overlay;
        for peer in overlay_rt.overlay().active_peers() {
            if peer.closed_ledger_hash() == obsolete {
                peer.cycle_status();
            }
        }
    }
}

/// Rippled invokes `checkLastClosedLedger` only from `endConsensus`. Keep
/// preferred-target selection and pending-target replacement at that same
/// Accepted boundary: an acquisition completion may populate the cache at any
/// time, but it cannot supersede recovery intent until the next endConsensus
/// pass re-evaluates peer and validation evidence.
fn should_reconcile_preferred_lcl(phase: ConsensusPhase) -> bool {
    phase == ConsensusPhase::Accepted
}

/// Rippled's endConsensus exists only after doAccept returns. Quaxar's
/// JtAccept-to-strand handoff has an async window between `schedule_accept`
/// and the command reaching the strand; suppress the equivalent pass while
/// accepted work is in that window (queued or awaiting queue capacity).
fn should_run_end_consensus_reconciliation(
    phase: ConsensusPhase,
    accept_is_queued: bool,
    has_pending_accept: bool,
) -> bool {
    should_reconcile_preferred_lcl(phase) && !accept_is_queued && !has_pending_accept
}

/// Rippled advances from CONNECTED/SYNCING to TRACKING/FULL in endConsensus
/// only if checkLastClosedLedger did not report an abnormal ledger change.
fn should_promote_operating_mode_at_end_consensus(
    end_consensus_pass: bool,
    reconciliation: PreferredLclReconciliation,
) -> bool {
    end_consensus_pass && reconciliation == PreferredLclReconciliation::NoChange
}

/// Match rippled `checkLastClosedLedger`: choose the preferred LCL only for
/// this endConsensus pass, switch it if it is resident and admissible, and
/// otherwise leave generic Consensus to perform WrongLedger/GetConsL1
/// recovery.  InboundLedgers owns fetching and caching; it never transfers an
/// exact-target ownership token to NetworkOPs.
fn reconcile_preferred_lcl(
    root: &ApplicationRoot,
    shared_inbound: &Arc<InboundLedgers>,
    runner: &mut dyn ConsensusRunner,
    consensus_rt: &AppConsensusRuntime,
    last_round_ledger_id: &mut Option<Uint256>,
    audit_sampler: &mut LclAuditSampler,
) -> PreferredLclReconciliation {
    if !should_reconcile_preferred_lcl(runner.phase()) {
        return PreferredLclReconciliation::NoChange;
    }

    let _lcl_transition_guard = root.lcl_transition_gate().lock();
    let Some(lm_rt) = root.ledger_master_runtime() else {
        return PreferredLclReconciliation::NoChange;
    };
    let lm = lm_rt.ledger_master();
    let Some(our_closed) = root.closed_ledger() else {
        return PreferredLclReconciliation::NoChange;
    };

    let our_hash = *our_closed.header().hash.as_uint256();
    let parent_hash = *our_closed.header().parent_hash.as_uint256();
    // NOTE: Rippled's checkLastClosedLedger (NetworkOPs.cpp:1902-2005) has NO
    // local-parent-residency precondition. It always computes preferredLCL and
    // attempts recovery regardless of whether the parent is locally cached.
    // A previous gate here returned NoChange when the parent couldn't be
    // resolved, which permanently blocked preferred-LCL recovery whenever
    // the parent was swept from ledger_history (the common case during
    // catch-up). Removed to match rippled.

    use overlay::Overlay;
    let peers = root
        .overlay_runtime()
        .map(|overlay_rt| overlay_rt.overlay().active_peers())
        .unwrap_or_default();
    let mut peer_counts = std::collections::BTreeMap::<Uint256, u32>::new();
    peer_counts.entry(our_hash).or_insert(0);
    for peer in &peers {
        let hash = peer.closed_ledger_hash();
        if !hash.is_zero() {
            *peer_counts.entry(hash).or_default() += 1;
        }
    }
    if root.network_ops_operating_mode() >= NetworkOpsOperatingMode::Tracking {
        *peer_counts.entry(our_hash).or_default() += 1;
    }

    let preference_diagnostic = root.validations().preferred_lcl_diagnostic(
        &RclValidatedLedger::from_ledger(&our_closed),
        lm.valid_ledger_seq(),
        &peer_counts,
    );
    let preferred_hash = preference_diagnostic.selected;
    tracing::info!(
        target: "lcl_trace",
        event = "preferred_lcl_selected",
        local_lcl_hash = %our_hash,
        local_lcl_seq = our_closed.header().seq,
        preferred_lcl_hash = %preferred_hash,
        peer_count = peers.len(),
        selected_trusted_validation_count = root.validations().num_trusted_for_ledger(preferred_hash),
        selected_peer_lcl_support = peer_counts.get(&preferred_hash).copied().unwrap_or_default(),
        validation_selection_source = ?preference_diagnostic.selection_source,
        validation_working_source = ?preference_diagnostic.working_source,
        "LCL trace: preferred-LCL selection evaluated"
    );
    let emit_audit = audit_sampler.should_emit();
    if emit_audit {
        let inbound_lifecycle = shared_inbound.lifecycle_snapshot();
        let validated_anchor = lm
            .validated_ledger()
            .map(|ledger| (*ledger.header().hash.as_uint256(), ledger.header().seq));
        let last_valid_anchor = lm.last_valid_ledger();
        tracing::info!(
            target: "lcl_audit",
            local_lcl_hash = %our_hash,
            local_lcl_seq = our_closed.header().seq,
            local_parent_hash = %parent_hash,
            operating_mode = ?root.network_ops_operating_mode(),
            need_network_ledger = root.need_network_ledger(),
            peer_count = peers.len(),
            peer_lcl_counts = ?peer_counts,
            preferred_lcl_hash = %preferred_hash,
            selected_trusted_validation_count = root.validations().num_trusted_for_ledger(preferred_hash),
            selected_peer_lcl_support = peer_counts.get(&preferred_hash).copied().unwrap_or_default(),
            validation_working_source = ?preference_diagnostic.working_source,
            validation_selection_source = ?preference_diagnostic.selection_source,
            validation_trie_preferred = ?preference_diagnostic.trie_preferred,
            validation_acquiring_preferred = ?preference_diagnostic.acquiring_preferred,
            validation_working_preferred = ?preference_diagnostic.working_preferred,
            validation_peer_preferred = ?preference_diagnostic.peer_preferred,
            validation_current_count = preference_diagnostic.current_validation_count,
            validation_current_trusted_count = preference_diagnostic.current_trusted_count,
            validation_current_trusted_full_count = preference_diagnostic.current_trusted_full_count,
            validation_trie_ledger_count = preference_diagnostic.trie_ledger_count,
            validation_acquiring_entry_count = preference_diagnostic.acquiring_entry_count,
            validation_acquiring_waiter_count = preference_diagnostic.acquiring_waiter_count,
            validation_peer_lcl_entry_count = preference_diagnostic.peer_lcl_entry_count,
            inbound_lifecycle = ?inbound_lifecycle,
            ?validated_anchor,
            ?last_valid_anchor,
            "LCL_AUDIT preferred-LCL selection sampled"
        );
    }

    // Rippled does not switch back to its immediate predecessor. A zero
    // preference is likewise not an actionable recovery target.
    if preferred_hash.is_zero() || preferred_hash == parent_hash {
        tracing::info!(
            target: "lcl_trace",
            event = "preferred_lcl_not_actionable",
            preferred_lcl_hash = %preferred_hash,
            parent_hash = %parent_hash,
            "LCL trace: preferred LCL is zero or the local parent"
        );
        return PreferredLclReconciliation::NoChange;
    }
    if preferred_hash == our_hash {
        tracing::info!(
            target: "lcl_trace",
            event = "preferred_lcl_already_local",
            preferred_lcl_hash = %preferred_hash,
            local_lcl_seq = our_closed.header().seq,
            "LCL trace: preferred LCL already matches local closed ledger"
        );
        return PreferredLclReconciliation::NoChange;
    }

    if matches!(
        root.network_ops_operating_mode(),
        NetworkOpsOperatingMode::Tracking | NetworkOpsOperatingMode::Full
    ) {
        root.set_network_ops_operating_mode(NetworkOpsOperatingMode::Connected);
    }

    let candidate =
        root.resolve_ledger_by_hash(basics::sha_map_hash::SHAMapHash::new(preferred_hash));
    let Some(candidate) = candidate else {
        // Rippled re-invokes InboundLedgers::acquire(hash, 0, CONSENSUS) on
        // every endConsensus pass (NetworkOPs.cpp:1979-1981) unconditionally.
        // Deduplication happens INSIDE acquire(): ledgers_.find(hash) returns
        // the existing entry without creating a new one. Our registry does
        // the same via entries.get_mut(&hash). Match rippled exactly.
        tracing::info!(
            target: "lcl_trace",
            event = "preferred_lcl_resolver_miss",
            preferred_lcl_hash = %preferred_hash,
            local_lcl_hash = %our_hash,
            local_lcl_seq = our_closed.header().seq,
            "LCL trace: preferred LCL is not resolver-visible; requesting consensus acquisition"
        );
        shared_inbound.acquire_closed_ledger_async(preferred_hash, AcquireReason::Consensus);
        shared_inbound.record_recovery_lcl_decision(
            preferred_hash,
            None,
            "check_last_closed_ledger",
            "requested",
        );
        if emit_audit {
            tracing::info!(
                target: "lcl_audit",
                local_lcl_hash = %our_hash,
                local_lcl_seq = our_closed.header().seq,
                preferred_lcl_hash = %preferred_hash,
                "LCL_AUDIT preferred-LCL candidate is not cached"
            );
        }

        // endConsensus -> beginConsensus preserves the actual local LCL as
        // the ledger object while naming the desired network LCL by hash.
        // Generic Consensus then owns its normal WrongLedger/GetConsL1 path.
        let now = root.shared_time_keeper().close_time();
        let prev_cx = crate::consensus_ledger_from_ledger(&our_closed);
        if let Some(inbound_tx) = root.inbound_transactions().lock().ok().as_mut() {
            inbound_tx.new_round(our_closed.header().seq);
        }
        runner.start_round(now, preferred_hash, prev_cx, false);
        consensus_rt.update_phase(runner.phase());
        consensus_rt.update_prev_ledger_id(runner.prev_ledger_id());
        *last_round_ledger_id = Some(preferred_hash);
        return PreferredLclReconciliation::Pending;
    };

    let candidate_hash = *candidate.header().hash.as_uint256();
    if candidate_hash != preferred_hash {
        tracing::warn!(
            target: "lcl_trace",
            event = "preferred_lcl_hash_mismatch",
            preferred_lcl_hash = %preferred_hash,
            candidate_hash = %candidate_hash,
            candidate_seq = candidate.header().seq,
            "LCL trace: resolver returned a ledger with the wrong hash"
        );
        shared_inbound.record_recovery_lcl_decision(
            preferred_hash,
            Some(candidate.as_ref()),
            "check_last_closed_ledger",
            "rejected_hash_mismatch",
        );
        return PreferredLclReconciliation::Pending;
    }

    let state_complete = !candidate.state_map().is_synching();
    let tx_complete = candidate.header().tx_hash.is_zero() || !candidate.tx_map().is_synching();
    let can_be_current = lm.can_be_current(candidate.as_ref(), root.current_close_time_seconds());
    let compatibility_audit = lm.compatibility_audit(candidate.as_ref());
    let compatible = compatibility_audit.compatible();
    if emit_audit {
        tracing::info!(
            target: "lcl_audit",
            target_hash = %preferred_hash,
            candidate_hash = %candidate_hash,
            candidate_seq = candidate.header().seq,
            candidate_parent_hash = %candidate.header().parent_hash,
            state_complete,
            tx_complete,
            can_be_current,
            compatible,
            compatibility = ?compatibility_audit,
            "LCL_AUDIT preferred-LCL candidate admission sampled"
        );
    }
    if !state_complete || !tx_complete {
        tracing::info!(
            target: "lcl_trace",
            event = "preferred_lcl_incomplete",
            preferred_lcl_hash = %preferred_hash,
            candidate_seq = candidate.header().seq,
            state_complete,
            tx_complete,
            "LCL trace: resolver-visible preferred LCL is incomplete"
        );
        shared_inbound.record_recovery_lcl_decision(
            preferred_hash,
            Some(candidate.as_ref()),
            "check_last_closed_ledger",
            "selected_but_incomplete",
        );
        return PreferredLclReconciliation::Pending;
    }
    if !can_be_current || !compatible {
        tracing::warn!(
            target: "lcl_trace",
            event = "preferred_lcl_rejected",
            preferred_lcl_hash = %preferred_hash,
            candidate_hash = %candidate_hash,
            candidate_seq = candidate.header().seq,
            can_be_current,
            compatible,
            compatibility = ?compatibility_audit,
            "LCL trace: resolver-visible preferred LCL failed admission"
        );
        // Match rippled: keep our current LCL as the networkClosed output for
        // this pass; do not create or retire a separate exact-target state.
        shared_inbound.record_recovery_lcl_decision(
            preferred_hash,
            Some(candidate.as_ref()),
            "check_last_closed_ledger",
            "rejected_incompatible_or_stale",
        );
        tracing::warn!(
            target: "lcl_audit",
            target_hash = %preferred_hash,
            candidate_hash = %candidate_hash,
            candidate_seq = candidate.header().seq,
            can_be_current,
            compatible,
            compatibility = ?compatibility_audit,
            local_lcl_hash = %our_hash,
            local_lcl_seq = our_closed.header().seq,
            "LCL_AUDIT preferred-LCL candidate rejected"
        );
        return PreferredLclReconciliation::NoChange;
    }

    switch_last_closed_ledger(
        root,
        shared_inbound,
        runner,
        consensus_rt,
        last_round_ledger_id,
        preferred_hash,
        candidate,
    );
    PreferredLclReconciliation::Switched
}

/// Commit the resident, currently preferred LCL. Inbound completion only
/// populates cache/history; this endConsensus pass remains the sole policy
/// point that installs an admissible LCL.
fn switch_last_closed_ledger(
    root: &ApplicationRoot,
    shared_inbound: &Arc<InboundLedgers>,
    runner: &mut dyn ConsensusRunner,
    consensus_rt: &AppConsensusRuntime,
    last_round_ledger_id: &mut Option<Uint256>,
    target: Uint256,
    ledger: Arc<ledger::Ledger>,
) {
    let new_hash = *ledger.header().hash.as_uint256();
    debug_assert_eq!(new_hash, target);
    let new_seq = ledger.header().seq;
    tracing::info!(
        target: "lcl_audit",
        target_hash = %target,
        new_hash = %new_hash,
        new_seq,
        prior_closed = ?root.closed_ledger().map(|closed| {
            (*closed.header().hash.as_uint256(), closed.header().seq)
        }),
        "LCL_AUDIT preferred-LCL switch admitted"
    );
    tracing::info!(
        target: "lcl_trace",
        event = "preferred_lcl_switch_admitted",
        target_hash = %target,
        new_hash = %new_hash,
        new_seq,
        prior_closed = ?root.closed_ledger().map(|closed| {
            (*closed.header().hash.as_uint256(), closed.header().seq)
        }),
        "LCL trace: switching to resolver-visible preferred LCL"
    );

    // This matches rippled switchLastClosedLedger: the visible waiting state
    // clears only after a real LCL jump, never on a target replacement.
    root.set_need_network_ledger(false);
    root.process_closed_ledger_txq(ledger.as_ref(), true);
    root.rebuild_open_ledger_after_consensus(
        new_seq.saturating_add(1),
        ledger.fees().base,
        new_hash,
    );
    root.on_closed_ledger(Arc::clone(&ledger));
    root.broadcast_consensus_status_change(ledger.as_ref(), 3, true);

    root.check_accept_hash_seq(new_hash, new_seq);
    let proposing = root.network_ops_operating_mode() == NetworkOpsOperatingMode::Full;
    let now = root.shared_time_keeper().close_time();
    let prev_cx = crate::consensus_ledger_from_ledger(&ledger);
    if let Some(inbound_tx) = root.inbound_transactions().lock().ok().as_mut() {
        inbound_tx.new_round(new_seq);
    }
    runner.start_round(now, new_hash, prev_cx, proposing);
    consensus_rt.update_phase(runner.phase());
    consensus_rt.update_prev_ledger_id(runner.prev_ledger_id());
    *last_round_ledger_id = Some(new_hash);

    shared_inbound.record_recovery_lcl_decision(
        target,
        Some(ledger.as_ref()),
        "check_last_closed_ledger",
        "installed",
    );
    tracing::info!(target: "consensus", new_seq, %new_hash,
        "switchLastClosedLedger installed current preferred LCL");
}

// ─── checkAccept + tryAdvance + operating mode + history ─────────────────────

fn check_accept_and_advance(
    root: &ApplicationRoot,
    shared_inbound: &Arc<InboundLedgers>,
    configured_ledger_history: u32,
    last_history_tick: &mut Instant,
    history_fetch_pack: &mut Option<(u32, Instant)>,
    allow_mode_promotion: bool,
) {
    let Some(lm_rt) = root.ledger_master_runtime() else {
        return;
    };
    let lm = lm_rt.ledger_master();
    // Preferred-LCL selection and installation were completed by the
    // strand-owned reconciliation before this maintenance pass. Keep the
    // existing transition gate for validation/publication consistency only.
    let _lcl_transition_guard = root.lcl_transition_gate().lock();
    let published_before = root.published_ledger_seq();

    // ── checkAccept: the closed ledger may now have reached quorum ───────
    if let Some(closed) = root.closed_ledger() {
        root.check_accept_hash_seq(*closed.header().hash.as_uint256(), closed.header().seq);
    }

    // ── tryAdvance: burst through consecutive quorum-backed ledgers ───────
    let mut advanced = 0u32;
    loop {
        let next_seq = lm.valid_ledger_seq() + 1;
        let Some(candidate) = lm.ledger_history().get_cached_ledger_by_seq(next_seq) else {
            break;
        };
        let candidate_hash = *candidate.header().hash.as_uint256();
        root.check_accept_hash_seq(candidate_hash, next_seq);
        if lm.valid_ledger_seq() < next_seq {
            break;
        }
        advanced += 1;
    }
    if advanced > 0 {
        tracing::info!(target: "consensus", advanced, new_valid_seq = lm.valid_ledger_seq(), "tryAdvance burst");
    }

    // ── tryAdvance publication ────────────────────────────────────────────
    root.try_advance_publication();
    // rippled LedgerMaster::doAdvance clears needNetworkLedger_ unconditionally
    // after finding/publishing new validated ledgers (LedgerMaster.cpp:1945-1966).
    // Clear whenever we have a validated published ledger, not only when this
    // specific strand pass observed a publication delta (eliminates race with
    // external validation paths publishing before the strand snapshots).
    if root.need_network_ledger()
        && root
            .published_ledger()
            .is_some_and(|ledger| ledger.header().seq <= lm.valid_ledger_seq())
    {
        root.set_need_network_ledger(false);
    }

    // ── Update complete_ledgers display ──────────────────────────────────
    let complete_range = lm.complete_ledgers();
    let range_str = complete_range.to_string();
    if !range_str.is_empty() {
        root.set_status_rpc_complete_ledgers(Some(range_str));
    }

    // ── Operating mode promotion ─────────────────────────────────────────
    // This is reached only from a captured endConsensus pass with no preferred
    // LCL change. Do not let routine Open/Establish maintenance or a switching
    // pass promote the node, matching NetworkOPsImp::endConsensus.
    // Quaxar's AppOpenLedgerView does not retain the reference open ledger's
    // parentCloseTime. Until that exact timestamp is modeled, keep the
    // need-network-ledger guard here to prevent a freshly seeded local genesis
    // LCL from appearing network-fresh and falsely entering FULL.
    if allow_mode_promotion
        && !root.need_network_ledger()
        && root
            .published_ledger()
            .or_else(|| root.closed_ledger())
            .is_some()
    {
        let current_mode = root.network_ops_state().operating_mode();
        let need_network = root.need_network_ledger();
        let mut next_mode = current_mode;

        // Connected/Syncing → Tracking
        if matches!(
            next_mode,
            NetworkOpsOperatingMode::Connected | NetworkOpsOperatingMode::Syncing
        ) && !need_network
        {
            next_mode = NetworkOpsOperatingMode::Tracking;
        }

        // Connected/Tracking → Full when published ledger is fresh
        // rippled (NetworkOPs.cpp:2219-2230) does NOT gate this on needNetworkLedger.
        if matches!(
            next_mode,
            NetworkOpsOperatingMode::Connected | NetworkOpsOperatingMode::Tracking
        ) {
            // rippled (NetworkOPs.cpp:2226-2230): uses the current open ledger's
            // parentCloseTime (= LCL close time) and closeTimeResolution.
            // auto current = ledgerMaster_.getCurrentLedger();
            // if (now < current->header().parentCloseTime + 2 * closeTimeResolution)
            let fresh = root.closed_ledger().map_or(false, |lcl| {
                let now_close = root.current_close_time_seconds();
                let lcl_close = lcl.header().close_time;
                let resolution = u32::from(lcl.header().close_time_resolution);
                now_close < lcl_close.saturating_add(resolution.saturating_mul(2))
            });
            if fresh {
                next_mode = NetworkOpsOperatingMode::Full;
            }
        }

        if next_mode != current_mode {
            tracing::info!(target: "app", ?current_mode, ?next_mode, "strand: operating mode promoted");
            root.set_network_ops_operating_mode(next_mode);
        }
    }

    // ── History backfill — full rippled doAdvance/fetchForHistory parity ────
    //
    // rippled's LedgerMaster::doAdvance only attempts history acquisition when
    // ALL of these conditions are satisfied:
    //   1. !standalone
    //   2. Local fee load is not excessive
    //   3. Publication queue not backed up (pubLedgerSeq == validLedgerSeq)
    //   4. Validated ledger age < 1 minute (node is in sync)
    //   5. NodeStore write load < 8192
    //
    // Then within that gate, shouldAcquire checks:
    //   6. candidateLedger >= currentLedger (may be the current ledger)
    //   7. currentLedger - candidateLedger <= ledgerHistory (within config range)
    //   8. candidateLedger >= minimumOnline (if known)
    //
    // InboundLedgers deduplicates active history requests by hash and
    // sequence. rippled's fillInProgress_ is for local SQL tryFill work, not
    // a lock held while a remote History acquisition is in flight.

    let valid_seq = lm.valid_ledger_seq();
    let pub_seq = lm
        .published_ledger()
        .map(|ledger| ledger.header().seq)
        .unwrap_or(0);

    // Condition 1: not standalone (always true here — strand only spawns for overlay mode)
    // Condition 2: fee load — skip if fee track reports overload
    let fee_overloaded = root.load_fee_track_loaded_local();
    // Condition 3: publication caught up to validation
    let publication_caught_up = valid_seq == pub_seq;
    // Condition 4: validated ledger age < 60s
    let validated_ledger_fresh = root.validated_ledger_age_seconds() < 60;
    // Condition 5: NodeStore write pressure. This is the same persistence
    // backlog metric and threshold as rippled's
    // `app_.getNodeStore().getWriteLoad() < kMaxWriteLoadAcquire`.
    let write_pressure_ok = root.node_store_write_load() < 8192;

    // rippled's InboundLedgers::acquire rejects History while the node needs
    // a network ledger. This strand is the sole History acquisition caller.
    let can_acquire_history = !root.need_network_ledger()
        && !fee_overloaded
        && publication_caught_up
        && validated_ledger_fresh
        && write_pressure_ok
        && valid_seq > 1
        && last_history_tick.elapsed() >= HISTORY_BACKFILL_RETRY_INTERVAL;

    if can_acquire_history {
        *last_history_tick = Instant::now();

        let complete = lm.complete_ledgers();
        // Do not fetch below the configured NodeStore retention floor.
        // This matches nodeStore().earliestLedgerSeq(), falling back to the
        // historical genesis+1 guard only when no node store is attached.
        let earliest_seq = root
            .minimum_online_seq()
            .unwrap_or(2)
            .max(lm.earliest_fetch(configured_ledger_history));
        // Find the first missing ledger scanning backward from valid_seq
        let mut missing_seq = None;
        for seq in (earliest_seq..valid_seq).rev() {
            if !complete.contains(seq) {
                missing_seq = Some(seq);
                break;
            }
        }

        if let Some(missing) = missing_seq {
            // shouldAcquire gate: is the missing ledger within our configured range?
            let should_acquire = should_acquire_history(
                valid_seq,
                configured_ledger_history,
                missing,
                root.minimum_online_seq(),
            );

            if should_acquire {
                // Rippled parity: batch prefetch up to ledgerFetchSize (256)
                // consecutive missing ledgers going backward from `missing`.
                // Matches LedgerMaster::doAdvance prefetch loop.
                let prefetch_limit = configured_ledger_history.min(256);
                let mut prefetch_count = 0u32;
                let mut primary_history_pending = false;

                for seq in (earliest_seq..=missing).rev() {
                    if prefetch_count >= prefetch_limit {
                        break;
                    }
                    if complete.contains(seq) {
                        continue;
                    }
                    // `getLedgerHashForHistory` first consults its local
                    // history index, then derives the exact canonical hash
                    // from the current validated ledger's skip list. Without
                    // the second source a fresh node that has only its latest
                    // validated ledger cannot begin backfill at all: no
                    // earlier sequence has been inserted into the local
                    // by-sequence cache yet.
                    let Some(sha_hash) = history_hash_for_seq(&lm, seq) else {
                        continue;
                    };
                    let hash = *sha_hash.as_uint256();

                    // A provider hit is already an exact, immutable ledger.
                    // Treat it as trusted History material through the same
                    // ancestry/persistence path as an inbound completion; do
                    // not merely cache-and-skip it, or the missing range can
                    // remain incomplete forever.
                    if let Some(ledger) = root.resolve_ledger_by_hash(sha_hash) {
                        let _ = persist_completed_inbound_ledger(
                            root,
                            &lm,
                            &ledger,
                            AcquireReason::History,
                        );
                        continue;
                    }
                    if seq == missing {
                        primary_history_pending = true;
                    }
                    if shared_inbound.has_entry_for_seq_or_hash(seq, &hash) {
                        continue;
                    }
                    shared_inbound.acquire_async(hash, seq, AcquireReason::History);
                    prefetch_count += 1;
                }

                if primary_history_pending {
                    request_history_fetch_pack(
                        root,
                        &lm,
                        missing,
                        configured_ledger_history,
                        history_fetch_pack,
                    );
                }

                if prefetch_count > 1 {
                    tracing::debug!(
                        target: "history",
                        missing,
                        prefetched = prefetch_count,
                        "batch prefetch of consecutive missing ledgers"
                    );
                }
            }
        }
    }
}

fn trace_completed_inbound_handoff(
    source: &'static str,
    lm: &ledger::LedgerMaster,
    ledger: &Arc<ledger::Ledger>,
    reason: AcquireReason,
    persisted: CompletionPersistence,
) {
    let cache_visible_after = lm
        .ledger_history()
        .get_cached_ledger_by_hash(ledger.header().hash)
        .is_some();
    tracing::info!(
        target: "lcl_trace",
        event = "inbound_completion_persisted",
        source,
        reason = ?reason,
        ledger_hash = %ledger.header().hash,
        ledger_seq = ledger.header().seq,
        immutable = ledger.is_immutable(),
        state_synching = ledger.state_map().is_synching(),
        tx_synching = ledger.tx_map().is_synching(),
        cache_visible_after,
        inserted = persisted.inserted,
        acknowledged = persisted.acknowledged,
        "LCL trace: inbound ledger completion persisted before adoption"
    );
}

fn persist_completed_inbound_ledger(
    root: &ApplicationRoot,
    lm: &ledger::LedgerMaster,
    ledger: &Arc<ledger::Ledger>,
    reason: AcquireReason,
) -> CompletionPersistence {
    let normalized = root.ledger_with_node_fetcher(Arc::clone(ledger));
    match reason {
        // `InboundLedger::done` calls `storeLedger` for generic and consensus
        // acquisitions. It preserves the header's existing validated state
        // and never inserts an unvalidated fork by sequence or into
        // `completeLedgers`.
        AcquireReason::Consensus | AcquireReason::Generic => CompletionPersistence {
            inserted: !lm.ledger_history().insert(normalized, false),
            acknowledged: true,
        },
        // History completion is only trusted-chain material when its exact
        // hash is anchored by the current validated ledger's skip list. A
        // completed peer response alone is otherwise just cacheable data and
        // must not reach setFullLedger's validated/complete persistence path.
        AcquireReason::History => {
            if !is_validated_history_ancestor(lm, normalized.as_ref()) {
                tracing::warn!(
                    target: "history",
                    seq = normalized.header().seq,
                    hash = %normalized.header().hash,
                    "completed history ledger is not anchored to validated chain; caching only"
                );
                return CompletionPersistence {
                    inserted: !lm.ledger_history().insert(normalized, false),
                    acknowledged: true,
                };
            }

            let normalized_seq = normalized.header().seq;
            let was_complete = lm.have_ledger(normalized_seq);
            let persistence =
                ledger::LedgerPersistence::new(root.build_ledger_persistence_runtime());
            match lm.set_full_ledger(
                &persistence,
                Arc::clone(&normalized),
                false,
                false,
                None,
                None,
            ) {
                Ok(true) => {
                    // `set_full_ledger(..., is_current = false)` deliberately
                    // does not update LedgerHistory's sequence index. Keep
                    // this trusted historical ledger cacheable by exact hash
                    // for subsequent contiguous materialization/publication,
                    // but do not mark it as a new validated head.
                    lm.ledger_history().insert(normalized, false);
                    CompletionPersistence {
                        inserted: !was_complete,
                        acknowledged: true,
                    }
                }
                Ok(false) => {
                    // set_full_ledger records the range before its persistence
                    // result is known. Remove that claim so a failed durable
                    // write cannot make history look complete or suppress a
                    // later retry.
                    lm.clear_ledger(normalized_seq);
                    tracing::warn!(target: "ledger", "trusted history ledger was not durably saved");
                    CompletionPersistence {
                        inserted: false,
                        acknowledged: false,
                    }
                }
                Err(error) => {
                    tracing::warn!(target: "ledger", ?error, "failed to materialize trusted history ledger");
                    CompletionPersistence {
                        inserted: false,
                        acknowledged: false,
                    }
                }
            }
        }
    }
}

fn request_history_fetch_pack(
    root: &ApplicationRoot,
    lm: &ledger::LedgerMaster,
    missing: u32,
    fetch_depth: u32,
    in_flight: &mut Option<(u32, Instant)>,
) {
    if let Some((requested_seq, issued_at)) = *in_flight {
        if requested_seq == missing && issued_at.elapsed() <= HISTORY_FETCH_PACK_STALE_AFTER {
            return;
        }
        *in_flight = None;
    }

    // Mirror `getFetchPack`: do not ask for a predecessor below local storage
    // retention, and use the exact hash for the ledger immediately above the
    // gap as the peer's `have` anchor.
    let earliest = crate::ledger::loaded_ledger_runtime::AppLoadedLedgerRuntime::from_root(root)
        .map(|loaded| loaded.earliest_ledger_seq())
        .unwrap_or(1)
        .max(lm.earliest_fetch(fetch_depth));
    if missing <= earliest {
        return;
    }
    let Some(have_hash) = history_hash_for_seq(lm, missing.saturating_add(1)) else {
        return;
    };

    use overlay::Overlay;
    let peer = root
        .overlay_runtime()
        .map(|runtime| runtime.overlay().active_peers())
        .unwrap_or_default()
        .into_iter()
        .filter(|peer| peer.has_range(missing, missing.saturating_add(1)))
        .max_by_key(|peer| peer.score(true));
    let Some(peer) = peer else {
        return;
    };

    peer.send(overlay::Message::new(
        ledger::make_fetch_pack_request(have_hash),
        None,
    ));
    *in_flight = Some((missing, Instant::now()));
    tracing::debug!(target: "history", missing, %have_hash, peer = peer.id(),
        "requested history fetch pack");
}

/// Resolve the canonical hash for a history candidate without selecting a
/// fork by sequence. Only the validated ledger's skip list and the validated
/// history index are eligible sources; arbitrary completed acquisitions must
/// never steer trusted-history backfill.
fn history_hash_for_seq(
    lm: &ledger::LedgerMaster,
    seq: u32,
) -> Option<basics::sha_map_hash::SHAMapHash> {
    let history = lm.ledger_history();
    let indexed = history.get_ledger_hash(seq);
    if !indexed.is_zero() {
        return Some(indexed);
    }

    let hash = lm
        .validated_ledger()
        .and_then(|validated| {
            if validated.header().seq == seq {
                Some(validated.header().hash)
            } else {
                validated.hash_of_seq(seq, &ledger::NullLedgerJournal)
            }
        })
        .filter(|hash| !hash.is_zero());

    hash
}

/// A history fetch may be persisted as trusted full history only if a current
/// validated anchor names this exact hash at the candidate sequence. This is
/// the local equivalent of rippled's doAdvance/fetchForHistory chain proof.
fn is_validated_history_ancestor(lm: &ledger::LedgerMaster, ledger: &ledger::Ledger) -> bool {
    let Some(validated) = lm.validated_ledger() else {
        return false;
    };
    let candidate_seq = ledger.header().seq;
    if candidate_seq > validated.header().seq {
        return false;
    }
    if candidate_seq == validated.header().seq {
        return ledger.header().hash == validated.header().hash;
    }
    validated
        .hash_of_seq(candidate_seq, &ledger::NullLedgerJournal)
        .is_some_and(|hash| hash == ledger.header().hash)
}

#[cfg(test)]
fn record_completed_inbound_ledger(
    lm: &ledger::LedgerMaster,
    ledger: &Arc<ledger::Ledger>,
) -> bool {
    !lm.ledger_history().insert(Arc::clone(ledger), false)
}

/// Matches rippled's static `shouldAcquire()` helper in LedgerMaster.cpp.
///
/// Returns true if `candidate_ledger` should be fetched from the network
/// given the current validated sequence and configured history depth.
fn should_acquire_history(
    current_ledger: u32,
    ledger_history: u32,
    candidate_ledger: u32,
    minimum_online: Option<u32>,
) -> bool {
    // Fetch if it may be the current ledger
    if candidate_ledger >= current_ledger {
        return true;
    }
    // Fetch if within configured history range
    if ledger_history == u32::MAX {
        // "full" history — always acquire
        return true;
    }
    if current_ledger - candidate_ledger <= ledger_history {
        return true;
    }
    // Fetch if at or above the minimum online boundary (SHAMapStore retention)
    if let Some(min) = minimum_online {
        if candidate_ledger >= min {
            return true;
        }
    }
    // Otherwise don't acquire
    false
}

#[cfg(test)]
mod tests {
    use super::{
        ConsensusJobScheduler, MAX_LEDGER_COMPLETIONS_PER_TURN, MAX_PROPOSALS_PER_TURN,
        PreferredLclReconciliation, drain_bounded, persist_completed_inbound_ledger,
        record_completed_inbound_ledger, should_promote_operating_mode_at_end_consensus,
        should_reconcile_preferred_lcl, should_run_end_consensus_reconciliation,
    };
    use crate::ApplicationRoot;
    use crate::consensus::rcl_consensus::PendingAcceptWork;
    use crate::job::job_queue::JobQueue;
    use crate::job::job_types::JobType;
    use crate::ledger::inbound_ledgers::AcquireReason;
    use crate::runtime::component_runtime::ConsensusCommand;
    use basics::base_uint::Uint256;
    use basics::sha_map_hash::SHAMapHash;
    use basics::tagged_cache::MonotonicClock;
    use consensus::algorithm::ConsensusPhase;
    use ledger::{Ledger, LedgerHeader, LedgerMaster, LedgerMasterConfig, calculate_ledger_hash};
    use std::sync::Arc;
    use std::sync::mpsc;
    use std::time::Duration;

    fn immutable_ledger(seq: u32, parent_fill: u8) -> Arc<Ledger> {
        let mut header = LedgerHeader {
            seq,
            parent_hash: SHAMapHash::new(Uint256::from_array([parent_fill; 32])),
            close_time: seq.saturating_add(100),
            close_time_resolution: 30,
            ..LedgerHeader::default()
        };
        header.hash = calculate_ledger_hash(&header);
        let mut state_tree = shamap::mutation::MutableTree::new(seq);
        state_tree
            .add_item(
                shamap::tree_node::SHAMapNodeType::AccountState,
                shamap::item::SHAMapItem::new(
                    Uint256::from_u64(u64::from(seq)),
                    vec![parent_fill; 128],
                ),
            )
            .expect("state entry should insert");
        let mut ledger = Ledger::from_maps(
            header,
            shamap::sync::SyncTree::from_root_with_type(
                state_tree.root(),
                shamap::sync::SHAMapType::State,
                false,
                seq,
                shamap::sync::SyncState::Immutable,
            ),
            shamap::sync::SyncTree::new_with_type(
                shamap::sync::SHAMapType::Transaction,
                false,
                seq,
            ),
        );
        ledger.set_immutable(true);
        Arc::new(ledger)
    }

    #[test]
    fn preferred_lcl_reconciliation_runs_only_at_end_consensus() {
        assert!(!should_reconcile_preferred_lcl(ConsensusPhase::Open));
        assert!(!should_reconcile_preferred_lcl(ConsensusPhase::Establish));
        assert!(should_reconcile_preferred_lcl(ConsensusPhase::Accepted));
        assert!(!should_run_end_consensus_reconciliation(
            ConsensusPhase::Accepted,
            true,
            false
        ));
        assert!(!should_run_end_consensus_reconciliation(
            ConsensusPhase::Accepted,
            false,
            true
        ));
        assert!(should_run_end_consensus_reconciliation(
            ConsensusPhase::Accepted,
            false,
            false
        ));
    }

    #[test]
    fn operating_mode_promotion_requires_no_end_consensus_ledger_change() {
        assert!(!should_promote_operating_mode_at_end_consensus(
            false,
            PreferredLclReconciliation::NoChange,
        ));
        assert!(!should_promote_operating_mode_at_end_consensus(
            true,
            PreferredLclReconciliation::Pending,
        ));
        assert!(!should_promote_operating_mode_at_end_consensus(
            true,
            PreferredLclReconciliation::Switched,
        ));
        assert!(should_promote_operating_mode_at_end_consensus(
            true,
            PreferredLclReconciliation::NoChange,
        ));
    }

    #[test]
    fn heartbeat_job_runs_under_ingress_flood_and_ingress_drain_is_bounded() {
        let queue = JobQueue::new(1);
        let (command_tx, command_rx) = mpsc::sync_channel(128);
        let scheduler = ConsensusJobScheduler::new(queue.clone(), command_tx);
        let (gate_started_tx, gate_started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();

        // Hold the only worker so the full flood and the timer job are queued
        // together. JtNetopTimer must then win over JtTransaction work.
        assert!(queue.add_job(JobType::JtAdmin, "test-gate", move || {
            gate_started_tx.send(()).expect("gate start receiver");
            release_rx.recv().expect("gate release sender");
        }));
        gate_started_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("gate should occupy the worker");

        for _ in 0..MAX_PROPOSALS_PER_TURN * 4 {
            assert!(queue.add_job(JobType::JtTransaction, "ingress", || {}));
        }
        assert!(scheduler.schedule_heartbeat());
        assert_eq!(queue.job_count(JobType::JtNetopTimer), 1);

        release_tx.send(()).expect("release gate");
        assert!(matches!(
            command_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("JtNetopTimer must run before ingress flood"),
            ConsensusCommand::Heartbeat
        ));

        let (ingress_tx, ingress_rx) = mpsc::channel();
        for value in 0..=MAX_PROPOSALS_PER_TURN {
            ingress_tx.send(value).expect("ingress receiver");
        }
        let mut processed = Vec::new();
        drain_bounded(&ingress_rx, MAX_PROPOSALS_PER_TURN, |value| {
            processed.push(value);
        });
        assert_eq!(processed.len(), MAX_PROPOSALS_PER_TURN);
        assert_eq!(ingress_rx.try_recv(), Ok(MAX_PROPOSALS_PER_TURN));

        let (completion_tx, completion_rx) = mpsc::channel();
        for value in 0..=MAX_LEDGER_COMPLETIONS_PER_TURN {
            completion_tx.send(value).expect("completion receiver");
        }
        let mut completions = Vec::new();
        drain_bounded(&completion_rx, MAX_LEDGER_COMPLETIONS_PER_TURN, |value| {
            completions.push(value);
        });
        assert_eq!(completions.len(), MAX_LEDGER_COMPLETIONS_PER_TURN);
        assert_eq!(
            completion_rx.try_recv(),
            Ok(MAX_LEDGER_COMPLETIONS_PER_TURN)
        );

        queue.rendezvous();
        queue.stop();
    }

    #[test]
    fn accept_handoff_is_jtaccept_and_coalesces_before_strand_mutation() {
        let queue = JobQueue::new(1);
        let (command_tx, command_rx) = mpsc::sync_channel(128);
        let scheduler = ConsensusJobScheduler::new(queue.clone(), command_tx);
        let (gate_started_tx, gate_started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();

        assert!(queue.add_job(JobType::JtAdmin, "test-gate", move || {
            gate_started_tx.send(()).expect("gate start receiver");
            release_rx.recv().expect("gate release sender");
        }));
        gate_started_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("gate should occupy the worker");

        let work = PendingAcceptWork {
            parent_ledger: immutable_ledger(6, 0x70),
            closed_seq: 7,
            close_time: 700,
            close_resolution: 30,
            correct_close_time: true,
            close_time_adjustment_seconds: None,
            consensus_hash: Uint256::from_u64(70),
            have_correct_lcl: true,
            consensus_succeeded: true,
            base_fee_drops: 10,
            rejected_dispute_retries: Vec::new(),
            txns: Vec::new(),
            validation: None,
        };
        assert!(scheduler.schedule_accept(work));
        assert!(scheduler.accept_is_queued());
        assert_eq!(queue.job_count(JobType::JtAccept), 1);

        // An accepted consensus phase cannot queue a second mutable accept
        // transition before the first JtAccept command reaches the strand.
        assert!(!scheduler.schedule_accept(PendingAcceptWork {
            parent_ledger: immutable_ledger(7, 0x80),
            closed_seq: 8,
            close_time: 800,
            close_resolution: 30,
            correct_close_time: true,
            close_time_adjustment_seconds: None,
            consensus_hash: Uint256::from_u64(80),
            have_correct_lcl: true,
            consensus_succeeded: true,
            base_fee_drops: 10,
            rejected_dispute_retries: Vec::new(),
            txns: Vec::new(),
            validation: None,
        }));
        assert_eq!(queue.job_count(JobType::JtAccept), 1);

        release_tx.send(()).expect("release gate");
        match command_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("JtAccept command should reach the strand")
        {
            ConsensusCommand::Accept(work) => assert_eq!(work.closed_seq, 7),
            _ => panic!("expected JtAccept handoff"),
        }
        // The strand clears this only after it serially executes the complete
        // accept/endConsensus/startRound transition.
        scheduler.accept_consumed();
        assert!(!scheduler.accept_is_queued());

        queue.rendezvous();
        queue.stop();
    }

    #[test]
    fn accept_handoff_retries_after_full_command_queue_without_losing_work() {
        let queue = JobQueue::new(1);
        let (command_tx, command_rx) = mpsc::sync_channel(1);
        command_tx
            .send(ConsensusCommand::Heartbeat)
            .expect("test command queue should accept filler");
        let scheduler = ConsensusJobScheduler::new(queue.clone(), command_tx);
        let work = PendingAcceptWork {
            parent_ledger: immutable_ledger(8, 0x90),
            closed_seq: 9,
            close_time: 900,
            close_resolution: 30,
            correct_close_time: true,
            close_time_adjustment_seconds: None,
            consensus_hash: Uint256::from_u64(90),
            have_correct_lcl: true,
            consensus_succeeded: true,
            base_fee_drops: 10,
            rejected_dispute_retries: Vec::new(),
            txns: Vec::new(),
            validation: None,
        };

        assert!(scheduler.schedule_accept(work));
        queue.rendezvous();
        assert!(scheduler.has_pending_accept());
        assert!(!scheduler.accept_is_queued());
        assert!(matches!(
            command_rx.try_recv(),
            Ok(ConsensusCommand::Heartbeat)
        ));

        assert!(scheduler.schedule_pending_accept());
        match command_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("retained accept must reach the strand after capacity returns")
        {
            ConsensusCommand::Accept(work) => assert_eq!(work.closed_seq, 9),
            _ => panic!("expected retained JtAccept handoff"),
        }
        assert!(!scheduler.has_pending_accept());
        scheduler.accept_consumed();
        queue.stop();
    }

    #[test]
    fn completed_inbound_ledger_is_cached_without_promoting_unvalidated_state() {
        let root = ApplicationRoot::new(0).expect("root should build");
        let master = LedgerMaster::new(MonotonicClock::default(), LedgerMasterConfig::default());
        let current = immutable_ledger(101, 0xA1);

        assert!(root.validated_ledger().is_none());
        assert!(
            persist_completed_inbound_ledger(&root, &master, &current, AcquireReason::Consensus)
                .inserted
        );
        assert!(master.validated_ledger().is_none());
        assert!(root.validated_ledger().is_none());
        assert_eq!(
            master
                .ledger_history()
                .get_cached_ledger_by_hash(current.header().hash)
                .expect("completed ledger should be available to checkAccept")
                .header()
                .seq,
            101
        );
    }

    #[test]
    fn completed_inbound_ledgers_are_cached_without_marking_complete() {
        let master = LedgerMaster::new(MonotonicClock::default(), LedgerMasterConfig::default());
        let newer = immutable_ledger(101, 0xA1);
        let older = immutable_ledger(100, 0xA0);

        assert!(record_completed_inbound_ledger(&master, &newer));
        assert!(record_completed_inbound_ledger(&master, &older));

        assert!(master.complete_ledgers().empty());
        assert!(
            master
                .ledger_history()
                .get_cached_ledger_by_hash(older.header().hash)
                .is_some_and(|ledger| ledger.header().hash == older.header().hash)
        );
    }
}
