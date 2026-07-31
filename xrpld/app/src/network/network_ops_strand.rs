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
use crate::consensus::rcl_consensus::{
    ConsensusRunner, PendingAcceptWork, RclConsensusValidationSource,
};
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

/// Prefer the currently advertised LCL if it is cached. If that advertisement
/// advanced while a recovery acquisition was running, use the completed
/// candidate once; `switchLastClosedLedger` still applies currentness and
/// compatibility checks before installation.
fn select_recovery_lcl(
    cached_preferred: Option<Arc<ledger::Ledger>>,
    _completed_consensus_recovery_ledger: &mut Option<Arc<ledger::Ledger>>,
) -> Option<Arc<ledger::Ledger>> {
    // Rippled's checkLastClosedLedger only uses the currently-preferred hash
    // (either from cache or via InboundLedgers::acquire). It never falls back
    // to a prior completed candidate.
    cached_preferred
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
    // A peer closed-ledger advertisement can advance while its full ledger is
    // downloading. Retain the completed candidate so recovery can still
    // evaluate it as a compatible LCL instead of serially chasing each newer
    // advertisement forever.
    let mut completed_consensus_recovery_ledger: Option<Arc<ledger::Ledger>> = None;

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
                    prev_ledger_id,
                    prev_ledger,
                } => {
                    let _lcl_transition_guard = root.lcl_transition_gate().lock();
                    if root
                        .closed_ledger()
                        .is_none_or(|current| *current.header().hash.as_uint256() != prev_ledger_id)
                    {
                        tracing::warn!(
                            target: "consensus",
                            %prev_ledger_id,
                            "discarded stale external start-round command"
                        );
                        continue;
                    }
                    if let Some(inbound_tx) = root.inbound_transactions().lock().ok().as_mut() {
                        inbound_tx.new_round(prev_ledger.seq());
                    }
                    runner.start_round(now, prev_ledger_id, prev_ledger, true);
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
            let prop = consensus::ConsensusProposal::new(
                proposal.previous_ledger,
                proposal.message.propose_seq,
                proposal.current_tx_hash,
                peer_close_time,
                now,
                proposal.public_key,
            );
            let peer_pos = crate::consensus::rcl_cx_peer_pos::RclCxPeerPos::new(
                proposal.public_key,
                proposal.message.signature.clone(),
                proposal.suppression,
                prop,
            );
            let relay = runner.peer_proposal(now, &peer_pos);
            // Match rippled PeerImp::checkPropose: relay trusted proposals
            // to all peers (minus the suppression set from HashRouter).
            if relay {
                if let Some(overlay_runtime) = root.overlay_runtime() {
                    overlay_runtime.overlay().relay_proposal(
                        proposal.message,
                        proposal.suppression,
                        proposal.public_key,
                    );
                }
            }
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
            consensus_rt.update_phase(runner.phase());
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
                consensus_rt.update_phase(runner.phase());
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
            consensus_rt.update_phase(runner.phase());
        }

        // ─── 5. Handle Accepted phase → detect new closed and start_round ─
        //
        // When need_network_ledger is true, the node is acquiring the network
        // ledger and must NOT start new consensus rounds on our local (wrong)
        // ledger. The switchLastClosedLedger block in step 6 handles starting
        // a round on the correct chain once the acquisition completes.
        if runner.phase() == ConsensusPhase::Accepted
            && !scheduler.accept_is_queued()
            && !scheduler.has_pending_accept()
            && !root.need_network_ledger()
        {
            // The recovery path snapshots the LCL and restarts consensus from
            // it, so it must use the same transition gate as JtAccept and
            // switchLastClosedLedger.
            let _lcl_transition_guard = root.lcl_transition_gate().lock();
            if let Some(closed) = root.closed_ledger() {
                let closed_id = *closed.header().hash.as_uint256();
                if last_round_ledger_id != Some(closed_id) {
                    let now = root.shared_time_keeper().close_time();
                    let prev_cx = crate::consensus_ledger_from_ledger(&closed);
                    if let Some(inbound_tx) = root.inbound_transactions().lock().ok().as_mut() {
                        inbound_tx.new_round(closed.header().seq);
                    }
                    runner.start_round(now, closed_id, prev_cx, true);
                    consensus_rt.update_phase(runner.phase());
                    consensus_rt.update_prev_ledger_id(runner.prev_ledger_id());
                    last_round_ledger_id = Some(closed_id);
                    last_timer_tick = Instant::now();
                    tracing::info!(target: "consensus", seq = closed.header().seq,
                        "Consensus started next round on newly accepted ledger");
                }
            }
        }

        // ─── 6a. completion recovery — registry is authoritative ────────
        // The sender is only a wakeup optimization. If it is disconnected or
        // not yet drained, completed acquisition state remains recoverable
        // here and follows the same storeLedger -> checkAccept path.
        if let Some(lm_rt) = root.ledger_master_runtime() {
            let lm = lm_rt.ledger_master();
            for (_, ledger, reason) in
                shared_inbound.poll_results_bounded(MAX_LEDGER_COMPLETIONS_PER_TURN)
            {
                let ledger = Arc::new(ledger);
                if reason == AcquireReason::Consensus && root.need_network_ledger() {
                    completed_consensus_recovery_ledger = Some(Arc::clone(&ledger));
                }
                let inserted = persist_completed_inbound_ledger(&root, &lm, &ledger, reason);
                root.check_accept_hash_seq(*ledger.header().hash.as_uint256(), ledger.header().seq);
                if inserted {
                    root.validations().register_ledger(&ledger);
                    if let Some(ref tx) = event_tx {
                        let _ = tx.try_send(crate::consensus::driver::ConsensusEvent::LedgerDone(
                            Arc::clone(&ledger),
                        ));
                    }
                }
            }
        }

        // ─── 6. checkAccept — matching rippled LedgerMaster::checkAccept ──
        check_accept_and_advance(
            &root,
            &shared_inbound,
            &mut runner,
            &consensus_rt,
            &mut last_round_ledger_id,
            &mut completed_consensus_recovery_ledger,
            configured_ledger_history,
            &mut last_history_tick,
        );

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
                    if completion.reason == AcquireReason::Consensus && root.need_network_ledger() {
                        completed_consensus_recovery_ledger = Some(Arc::clone(&ledger));
                    }
                    let lm = lm_rt.ledger_master();
                    let inserted =
                        persist_completed_inbound_ledger(&root, &lm, &ledger, completion.reason);
                    root.check_accept_hash_seq(
                        *ledger.header().hash.as_uint256(),
                        ledger.header().seq,
                    );
                    if inserted {
                        root.validations().register_ledger(&ledger);
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
                if completion.reason == AcquireReason::Consensus && root.need_network_ledger() {
                    completed_consensus_recovery_ledger = Some(Arc::clone(&ledger));
                }
                let inserted = root.ledger_master_runtime().is_some_and(|lm_rt| {
                    let lm = lm_rt.ledger_master();
                    persist_completed_inbound_ledger(&root, &lm, &ledger, completion.reason)
                });
                root.check_accept_hash_seq(*ledger.header().hash.as_uint256(), ledger.header().seq);
                if inserted {
                    root.validations().register_ledger(&ledger);
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
                // A pending consensus ledger is keyed by hash only. Do not
                // infer its sequence from a peer's independent history range.
                shared_inbound.acquire_closed_ledger_async(hash, AcquireReason::Consensus);
            }
        }

        // ─── 7. Wait for next event (proposal notify or 50ms timeout) ─────
        root.wait_consensus_or_timeout(Duration::from_millis(50));
    }

    tracing::info!(target: "consensus", "NetworkOPs strand stopped");
}

// ─── checkAccept + tryAdvance + operating mode + history ─────────────────────

fn check_accept_and_advance(
    root: &ApplicationRoot,
    shared_inbound: &Arc<InboundLedgers>,
    runner: &mut dyn ConsensusRunner,
    consensus_rt: &AppConsensusRuntime,
    last_round_ledger_id: &mut Option<Uint256>,
    completed_consensus_recovery_ledger: &mut Option<Arc<ledger::Ledger>>,
    configured_ledger_history: u32,
    last_history_tick: &mut Instant,
) {
    let Some(lm_rt) = root.ledger_master_runtime() else {
        return;
    };
    let lm = lm_rt.ledger_master();
    // This pass can install a preferred LCL and immediately reparent the open
    // ledger and consensus runner. Keep the same transition gate as JtAccept
    // for its full duration so validation/acquisition cannot replace the LCL
    // between the installation and that handoff.
    let _lcl_transition_guard = root.lcl_transition_gate().lock();

    // ── switchLastClosedLedger for joining nodes ──────────────────────────
    if root.need_network_ledger()
        && let (Some(our_closed), Some(overlay_rt)) = (root.closed_ledger(), root.overlay_runtime())
    {
        use overlay::Overlay;

        let our_closed_hash = *our_closed.header().hash.as_uint256();
        let previous_closed_hash = *our_closed.header().parent_hash.as_uint256();
        let peers = overlay_rt.overlay().active_peers();
        let mut peer_counts = std::collections::BTreeMap::<Uint256, u32>::new();
        for peer in &peers {
            let hash = peer.closed_ledger_hash();
            if !hash.is_zero() {
                *peer_counts.entry(hash).or_default() += 1;
            }
        }

        // H3: Include our own closed ledger in the tally so getPreferredLCL
        // accounts for our view.  Matches rippled where checkLastClosedLedger
        // counts the node's own closed ledger hash among peers.
        if root.network_ops_operating_mode()
            >= crate::network::network_ops::NetworkOpsOperatingMode::Tracking
        {
            if let Some(our_closed) = root.closed_ledger() {
                let our_hash = *our_closed.header().hash.as_uint256();
                if !our_hash.is_zero() {
                    *peer_counts.entry(our_hash).or_default() += 1;
                }
            }
        }

        // `Validations::getPreferredLCL` is trusted-first, but deliberately
        // falls back to peer LCL counts when no trusted validation exists.
        // Requiring a quorum here stranded a cold node after it had acquired
        // the peer ledger: no validator list meant its completed ledger could
        // never be selected or installed. This is rippled's
        // checkLastClosedLedger/switchLastClosedLedger decision, not a
        // validation-quorum acceptance decision.
        let preferred_hash = root.validations().preferred_lcl(
            &RclValidatedLedger::from_ledger(&our_closed),
            lm.valid_ledger_seq(),
            &peer_counts,
        );
        let should_switch = !preferred_hash.is_zero()
            && preferred_hash != our_closed_hash
            && preferred_hash != previous_closed_hash;

        if should_switch {
            // Request only the preferred LCL, rather than an arbitrary
            // highest-sequence peer report. This preserves trusted-validator
            // preference when it exists and uses the peer-count fallback only
            // when it does not.
            let target = peers
                .iter()
                .map(|peer| peer.closed_ledger_hash())
                .find(|hash| *hash == preferred_hash);
            if let Some(hash) = target
                && !shared_inbound.contains(&hash)
            {
                shared_inbound.acquire_closed_ledger_async(hash, AcquireReason::Consensus);
            }

            // Prefer the presently advertised LCL when it is cached. If it
            // advanced while the previous preferred ledger was downloading,
            // evaluate that just-completed candidate instead. `is_compatible`
            // and `can_be_current` below remain the authority for whether it
            // is safe to install, so this fallback cannot promote a divergent
            // or stale chain merely to escape acquisition churn.
            let network_ledger = select_recovery_lcl(
                lm.ledger_history().get_cached_ledger_by_hash(
                    basics::sha_map_hash::SHAMapHash::new(preferred_hash),
                ),
                completed_consensus_recovery_ledger,
            );
            if let Some(network_ledger) = network_ledger {
                let state_complete = !network_ledger.state_map().is_synching();
                let tx_complete = network_ledger.header().tx_hash.is_zero()
                    || !network_ledger.tx_map().is_synching();
                let can_be_current =
                    lm.can_be_current(network_ledger.as_ref(), root.current_close_time_seconds());
                let compatible = lm.is_compatible(network_ledger.as_ref());
                if state_complete && tx_complete && can_be_current && compatible {
                    let new_seq = network_ledger.header().seq;
                    let new_hash = *network_ledger.header().hash.as_uint256();

                    // `isCompatible` establishes that the candidate is on
                    // the validated chain and `can_be_current` verifies its
                    // close time and sequence are usable. Rippled applies no
                    // additional `new_seq >= valid_ledger_seq` gate: a
                    // compatible preferred LCL can normally trail the most
                    // recently validated ledger.
                    // Switching the local closed ledger and accepting a
                    // fully validated ledger are separate operations.
                    // Let the common checkAccept path decide whether this
                    // candidate has exact-sequence, nUNL-filtered quorum.
                    let trusted_validation_quorum = root
                        .trusted_validation_count_for_ledger(new_hash, new_seq)
                        >= root.validators().quorum();
                    root.check_accept_hash_seq(new_hash, new_seq);
                    let accepted = lm.validated_ledger().is_some_and(|validated| {
                        validated.header().hash == network_ledger.header().hash
                    });
                    if !accepted {
                        // This is a peer-LCL fallback only. It must not
                        // mutate LedgerMaster's validated/published slots
                        // or mark the ledger validated: peer agreement is
                        // not trusted-validation evidence.
                        root.on_closed_ledger(Arc::clone(&network_ledger));
                    }

                    root.set_need_network_ledger(false);

                    // TxQ must observe the new closed ledger before the
                    // re-parented open ledger accepts its queued work.
                    // This is rippled's `processClosedLedger(..., true)`
                    // backstep call in switchLastClosedLedger.
                    root.process_closed_ledger_txq(network_ledger.as_ref(), true);

                    // Rippled parity: rebuild open ledger on the new chain so
                    // local transactions and TxQ state are re-evaluated against
                    // the new parent. Matches NetworkOPs::switchLastClosedLedger
                    // `openLedger_.accept(...)` after chain jump.
                    root.rebuild_open_ledger_after_consensus(
                        new_seq.saturating_add(1),
                        network_ledger.fees().base,
                        new_hash,
                    );

                    // The shared notifier supplies rippled-compatible
                    // network time and the actual advertised ledger range.
                    root.broadcast_consensus_status_change(
                        network_ledger.as_ref(),
                        3, // neSWITCHED_LEDGER
                        true,
                    );

                    // endConsensus promotes the operating mode before
                    // beginConsensus, so a caught-up validator can propose
                    // in this new round rather than first observing it.
                    root.promote_operating_mode_after_accepted_ledger(network_ledger.as_ref());
                    let proposing =
                        root.network_ops_operating_mode() == NetworkOpsOperatingMode::Full;
                    let now = root.shared_time_keeper().close_time();
                    let prev_cx = crate::consensus_ledger_from_ledger(&network_ledger);
                    if let Some(inbound_tx) = root.inbound_transactions().lock().ok().as_mut() {
                        inbound_tx.new_round(network_ledger.header().seq);
                    }
                    runner.start_round(now, new_hash, prev_cx, proposing);

                    // M16: Cycle peer status after chain jump.  Matches rippled's
                    // endConsensus which calls cycleStatus() on peers that still
                    // advertise the now-dead closed ledger, so they re-report
                    // their current state on the next StatusChange message.
                    if let Some(overlay_rt) = root.overlay_runtime() {
                        use overlay::Overlay;
                        let dead_ledger = network_ledger.header().parent_hash;
                        for peer in overlay_rt.overlay().active_peers() {
                            if peer.closed_ledger_hash() == *dead_ledger.as_uint256() {
                                peer.cycle_status();
                            }
                        }
                    }

                    consensus_rt.update_phase(runner.phase());
                    consensus_rt.update_prev_ledger_id(runner.prev_ledger_id());
                    *last_round_ledger_id = Some(new_hash);
                    tracing::info!(
                        target: "consensus",
                        new_seq,
                        %new_hash,
                        trusted_validation_quorum,
                        "Consensus restarted on network chain (switchLastClosedLedger)"
                    );
                } else if !can_be_current || !compatible {
                    tracing::warn!(
                        target: "consensus",
                        seq = network_ledger.header().seq,
                        hash = %preferred_hash,
                        can_be_current,
                        compatible,
                        "Rejected preferred peer LCL that conflicts with validated or quorum-backed history"
                    );
                }
            }
        }
    }

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

    // ── Update complete_ledgers display ──────────────────────────────────
    let complete_range = lm.complete_ledgers();
    let range_str = complete_range.to_string();
    if !range_str.is_empty() {
        root.set_status_rpc_complete_ledgers(Some(range_str));
    }

    // ── Operating mode promotion ─────────────────────────────────────────
    // Match rippled endConsensus (NetworkOPs.cpp:2219-2232):
    // - CONNECTED/SYNCING + !needNetworkLedger → TRACKING
    // - CONNECTED/TRACKING + current ledger fresh → FULL
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

        // Connected/Tracking → Full when current ledger is fresh
        if matches!(
            next_mode,
            NetworkOpsOperatingMode::Connected | NetworkOpsOperatingMode::Tracking
        ) && !need_network
        {
            let fresh = root.closed_ledger().map_or(false, |current| {
                let now_close = root.current_close_time_seconds();
                let parent_close = current.header().close_time;
                let resolution = u32::from(current.header().close_time_resolution);
                now_close < parent_close.saturating_add(resolution.saturating_mul(2))
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
        // Find the first missing ledger scanning backward from valid_seq
        let mut missing_seq = None;
        let earliest_seq = 2u32; // don't go below genesis+1
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
                let lh = lm.ledger_history();

                for seq in (earliest_seq..=missing).rev() {
                    if prefetch_count >= prefetch_limit {
                        break;
                    }
                    if complete.contains(seq) {
                        continue;
                    }
                    // Resolve hash from ledger_history index. If non-zero,
                    // the ledger has been seen before (via validation or
                    // peer report) and we can acquire by hash directly.
                    let sha_hash = lh.get_ledger_hash(seq);
                    if sha_hash.is_zero() {
                        continue;
                    }
                    let hash = *sha_hash.as_uint256();
                    if lh.get_cached_ledger_by_hash(sha_hash).is_some() {
                        continue;
                    }
                    if shared_inbound.has_entry_for_seq_or_hash(seq, &hash) {
                        continue;
                    }
                    shared_inbound.acquire_async(hash, seq, AcquireReason::History);
                    prefetch_count += 1;
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

fn persist_completed_inbound_ledger(
    root: &ApplicationRoot,
    lm: &ledger::LedgerMaster,
    ledger: &Arc<ledger::Ledger>,
    reason: AcquireReason,
) -> bool {
    let normalized = root.ledger_with_node_fetcher(Arc::clone(ledger));
    match reason {
        // `InboundLedger::done` calls `storeLedger` for generic and consensus
        // acquisitions. rippled's `storeLedger` passes
        // `ledger->header().validated` to ledgerHistory_.insert.
        AcquireReason::Consensus | AcquireReason::Generic => {
            let validated = normalized.header().validated;
            !lm.ledger_history().insert(normalized, validated)
        }
        // History is consumed by LedgerMaster's fetch-for-history path. That
        // path always calls `setFullLedger(..., false, false)`: even an
        // already-complete sequence may be a competing hash whose lifecycle
        // effects must be considered before the result is de-duplicated.
        AcquireReason::History => {
            let was_complete = lm.have_ledger(normalized.header().seq);
            let persistence =
                ledger::LedgerPersistence::new(Arc::new(root.build_ledger_persistence_runtime()));
            if let Err(error) =
                lm.set_full_ledger(&persistence, normalized, false, false, None, None)
            {
                tracing::warn!(target: "ledger", ?error, "failed to promote completed history ledger");
                return false;
            }
            !was_complete
        }
    }
}

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
        drain_bounded, persist_completed_inbound_ledger, record_completed_inbound_ledger,
        select_recovery_lcl,
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
            consensus_hash: Uint256::from_u64(70),
            have_correct_lcl: true,
            base_fee_drops: 10,
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
            consensus_hash: Uint256::from_u64(80),
            have_correct_lcl: true,
            base_fee_drops: 10,
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
            consensus_hash: Uint256::from_u64(90),
            have_correct_lcl: true,
            base_fee_drops: 10,
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
    fn select_recovery_lcl_uses_only_cached_preferred_hash() {
        let completed = immutable_ledger(101, 0xA1);
        let mut pending = Some(Arc::clone(&completed));

        // When the preferred hash is not cached, no fallback to a previously
        // completed candidate occurs — matching rippled's checkLastClosedLedger
        // which only acquires/uses the currently-preferred hash.
        assert!(select_recovery_lcl(None, &mut pending).is_none());
        assert!(pending.is_some(), "stale candidate must not be consumed");

        // When the preferred hash IS cached, it is returned directly.
        let newer = immutable_ledger(102, 0xA2);
        let mut pending = Some(completed);
        let selected = select_recovery_lcl(Some(Arc::clone(&newer)), &mut pending)
            .expect("cached preferred LCL should be returned");
        assert_eq!(selected.header().hash, newer.header().hash);
        assert!(pending.is_some(), "stale candidate must not be consumed");
    }

    #[test]
    fn completed_inbound_ledger_is_cached_without_promoting_unvalidated_state() {
        let root = ApplicationRoot::new(0).expect("root should build");
        let master = LedgerMaster::new(MonotonicClock::default(), LedgerMasterConfig::default());
        let current = immutable_ledger(101, 0xA1);

        assert!(root.validated_ledger().is_none());
        assert!(persist_completed_inbound_ledger(
            &root,
            &master,
            &current,
            AcquireReason::Consensus,
        ));
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
