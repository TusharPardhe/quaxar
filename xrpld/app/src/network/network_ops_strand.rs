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

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use basics::base_uint::Uint256;
use consensus::algorithm::ConsensusPhase;

use crate::consensus::rcl_consensus::ConsensusRunner;
use crate::ledger::inbound_ledgers::{AcquireReason, InboundLedgers};
use crate::network::network_ops::NetworkOpsOperatingMode;
use crate::runtime::component_runtime::{AppConsensusRuntime, ConsensusCommand};
use crate::ApplicationRoot;

use overlay::inbound::QueuedProposal;

/// Dependencies the strand needs (passed at construction).
pub struct NetworkOpsStrandDeps {
    pub root: ApplicationRoot,
    pub consensus_rt: Arc<AppConsensusRuntime>,
    pub shared_inbound: Arc<InboundLedgers>,
    pub configured_ledger_history: u32,
    /// Consensus event channel sender for LedgerDone events from storeLedger drain.
    pub event_tx: Option<std::sync::mpsc::Sender<crate::consensus::driver::ConsensusEvent>>,
    /// Receiver for completed ledgers from shared_inbound acquisition.
    pub shared_completed_rx: Option<std::sync::mpsc::Receiver<Arc<ledger::Ledger>>>,
}

/// External handle to the running strand. Drop to stop.
pub struct NetworkOpsStrand {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    /// Send proposals from the overlay to the strand.
    pub proposal_tx: std::sync::mpsc::Sender<QueuedProposal>,
    /// Send tx-set completions to the strand.
    pub txset_tx: std::sync::mpsc::Sender<(Uint256, Arc<shamap::sync::SyncTree>)>,
    /// Send commands (StartRound, Stop) to the strand.
    pub command_tx: std::sync::mpsc::Sender<ConsensusCommand>,
}

impl NetworkOpsStrand {
    /// Spawn the strand thread. Takes ownership of the consensus runner.
    pub fn spawn(deps: NetworkOpsStrandDeps) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let (proposal_tx, proposal_rx) = std::sync::mpsc::channel::<QueuedProposal>();
        let (txset_tx, txset_rx) =
            std::sync::mpsc::channel::<(Uint256, Arc<shamap::sync::SyncTree>)>();
        let (command_tx, command_rx) = std::sync::mpsc::channel::<ConsensusCommand>();

        // Wire the command sender to the consensus runtime so external code
        // (e.g. validation event loop) can issue StartRound commands.
        deps.consensus_rt.set_cmd_sender(command_tx.clone());

        let stop_clone = Arc::clone(&stop);
        let thread = thread::Builder::new()
            .name("networkops-strand".into())
            .spawn(move || {
                strand_loop(deps, stop_clone, proposal_rx, txset_rx, command_rx);
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
        let _ = self.command_tx.send(ConsensusCommand::Stop);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for NetworkOpsStrand {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = self.command_tx.send(ConsensusCommand::Stop);
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

    let mut consensus_started = false;
    let mut last_timer_tick = Instant::now();
    let mut last_round_ledger_id: Option<Uint256> = None;
    let mut last_history_tick = Instant::now();
    // fillInProgress tracking: prevents starting a new history fill while one is
    // active at the same or higher sequence (matching rippled LedgerMaster::mFillInProgress).
    let mut fill_in_progress: Option<u32> = None;

    // Detect startup: always start consensus immediately on the closed
    // ledger, matching rippled's Application::run() which calls
    // beginConsensus(closedLedger.hash) unconditionally.
    if let Some(closed) = root.closed_ledger() {
        let now = root.shared_time_keeper().close_time();
        let prev_id = *closed.header().hash.as_uint256();
        let prev_cx = crate::consensus_ledger_from_ledger(&closed);
        runner.start_round(now, prev_id, prev_cx, true);
        consensus_rt.update_phase(runner.phase());
        consensus_rt.update_prev_ledger_id(runner.prev_ledger_id());
        consensus_started = true;
        last_round_ledger_id = Some(prev_id);
        last_timer_tick = Instant::now();
        tracing::info!(target: "consensus", seq = closed.header().seq,
            "Consensus started on closed ledger (matching rippled beginConsensus)");
    }

    // ═══════════════════════════════════════════════════════════════════════
    // MAIN STRAND LOOP — matches rippled's NetworkOPs::heartbeatTimer
    // ═══════════════════════════════════════════════════════════════════════
    while !stop.load(Ordering::Acquire) {
        // ─── 1. Process external commands ─────────────────────────────────
        while let Ok(cmd) = command_rx.try_recv() {
            match cmd {
                ConsensusCommand::StartRound {
                    now,
                    prev_ledger_id,
                    prev_ledger,
                } => {
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

        // ─── 2. Drain proposals → peer_proposal() ────────────────────────
        while let Ok(proposal) = proposal_rx.try_recv() {
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
            runner.peer_proposal(now, &peer_pos);
        }
        consensus_rt.update_phase(runner.phase());
        consensus_rt.update_prev_ledger_id(runner.prev_ledger_id());

        // ─── 3. Drain tx-set completions → got_tx_set() ──────────────────
        while let Ok((hash, set)) = txset_rx.try_recv() {
            let now = root.shared_time_keeper().close_time();
            let tx_set = consensus::RclTxSet::from_parts(
                set.root(),
                Arc::clone(runner.adaptor.tx_set_cache()),
                set.backed(),
                0,
            );
            runner.got_tx_set(now, tx_set);
            consensus_rt.update_phase(runner.phase());
            tracing::debug!(target: "consensus", %hash, "strand: got_tx_set processed");
        }

        // Also drain from the map_complete receiver if available
        if let Some(ref rx) = map_complete_rx {
            while let Ok((hash, set)) = rx.try_recv() {
                let now = root.shared_time_keeper().close_time();
                let tx_set = consensus::RclTxSet::from_parts(
                    set.root(),
                    Arc::clone(runner.adaptor.tx_set_cache()),
                    set.backed(),
                    0,
                );
                runner.got_tx_set(now, tx_set);
                consensus_rt.update_phase(runner.phase());
                tracing::debug!(target: "consensus", %hash, "strand: got_tx_set (map_complete)");
            }
        }

        // ─── 4. Timer tick every 1s → timer_entry + execute_accept ────────
        if last_timer_tick.elapsed() >= Duration::from_secs(1) {
            let now = root.shared_time_keeper().close_time();
            if let Some(work) = runner.timer_tick(now) {
                runner.execute_accept(now, work);
                last_round_ledger_id = Some(runner.prev_ledger_id());
            }
            consensus_rt.update_phase(runner.phase());
            consensus_rt.update_prev_ledger_id(runner.prev_ledger_id());
            last_timer_tick = Instant::now();
        }

        // ─── 5. Handle Accepted phase → detect new closed and start_round ─
        //
        // When need_network_ledger is true, the node is acquiring the network
        // ledger and must NOT start new consensus rounds on our local (wrong)
        // ledger. The switchLastClosedLedger block in step 6 handles starting
        // a round on the correct chain once the acquisition completes.
        if runner.phase() == ConsensusPhase::Accepted && !root.need_network_ledger() {
            if let Some(closed) = root.closed_ledger() {
                let closed_id = *closed.header().hash.as_uint256();
                if last_round_ledger_id != Some(closed_id) {
                    let now = root.shared_time_keeper().close_time();
                    let prev_cx = crate::consensus_ledger_from_ledger(&closed);
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

        // ─── 6. checkAccept — matching rippled LedgerMaster::checkAccept ──
        check_accept_and_advance(
            &root,
            &shared_inbound,
            &mut runner,
            &consensus_rt,
            &mut last_round_ledger_id,
            configured_ledger_history,
            &mut last_history_tick,
            &mut fill_in_progress,
        );

        // ─── 6b. storeLedger drain — completed InboundLedger results ─────
        // Moved from the polling loop: drain completed_ledgers_rx and
        // shared_completed_rx into LedgerHistory, matching rippled's
        // storeLedger path.
        if let Some(lm_rt) = root.ledger_master_runtime() {
            let rx_guard = lm_rt.completed_ledgers_rx.lock().expect("completed_ledgers_rx");
            if let Some(rx) = rx_guard.as_ref() {
                while let Ok(ledger) = rx.try_recv() {
                    let inserted = lm_rt.ledger_master().ledger_history().insert(Arc::clone(&ledger), true);
                    if inserted {
                        if let Some(ref tx) = event_tx {
                            let _ = tx.send(crate::consensus::driver::ConsensusEvent::LedgerDone(Arc::clone(&ledger)));
                        }
                    }
                }
            }
        }
        if let Some(ref rx) = shared_completed_rx {
            while let Ok(ledger) = rx.try_recv() {
                let ledger_seq = ledger.header().seq;
                // Clear fillInProgress when the fill acquisition completes
                if fill_in_progress == Some(ledger_seq) {
                    fill_in_progress = None;
                }
                if let Some(lm_rt) = root.ledger_master_runtime() {
                    let lm = lm_rt.ledger_master();
                    lm.ledger_history().insert(Arc::clone(&ledger), true);
                    if ledger_seq > 0 {
                        lm.mark_ledger_complete(ledger_seq);
                    }
                }
                if let Some(ref tx) = event_tx {
                    let _ = tx.send(crate::consensus::driver::ConsensusEvent::LedgerDone(ledger));
                }
            }
        }

        // ─── 6c. pending_consensus_ledger → acquire_async ────────────────
        if let Some(lm_rt) = root.ledger_master_runtime() {
            let pending = lm_rt.take_pending_consensus_ledger();
            if let Some(hash) = pending {
                shared_inbound.acquire_async(hash, 0, AcquireReason::Consensus);
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
    configured_ledger_history: u32,
    last_history_tick: &mut Instant,
    fill_in_progress: &mut Option<u32>,
) {
    let Some(lm_rt) = root.ledger_master_runtime() else {
        return;
    };
    let lm = lm_rt.ledger_master();
    let quorum = root.validators().quorum();

    // ── switchLastClosedLedger for joining nodes ──────────────────────────
    if root.need_network_ledger() {
        let our_closed_hash = root
            .closed_ledger()
            .map(|l| *l.header().hash.as_uint256())
            .unwrap_or_default();

        if let Some(overlay_rt) = root.overlay_runtime() {
            use overlay::Overlay;
            let peers = overlay_rt.overlay().active_peers();
            let acquired_hashes: Vec<_> = peers
                .iter()
                .map(|p| p.closed_ledger_hash())
                .filter(|h| !h.is_zero() && *h != our_closed_hash)
                .collect();

            // Trigger acquisition for the highest-sequence peer hash we don't
            // have locally. This matches rippled where InboundLedgers::acquire
            // is called from checkLastClosedLedger when the network ledger
            // differs from ours. We pick ONE target (highest seq) and stick
            // with it until complete or swept.
            if !acquired_hashes.is_empty() {
                let target = peers
                    .iter()
                    .filter_map(|p| {
                        let h = p.closed_ledger_hash();
                        let (_, seq) = p.ledger_range();
                        (!h.is_zero() && h != our_closed_hash && seq > 1)
                            .then_some((seq, h))
                    })
                    .max_by_key(|(seq, _)| *seq);

                if let Some((seq, hash)) = target {
                    if !shared_inbound.contains(&hash) {
                        shared_inbound.acquire_async(hash, seq, AcquireReason::Consensus);
                    }
                }
            }

            let mut best_validated: Option<Arc<ledger::Ledger>> = None;
            for hash in &acquired_hashes {
                let candidate = lm
                    .ledger_history()
                    .get_cached_ledger_by_hash(basics::sha_map_hash::SHAMapHash::new(*hash));
                if let Some(candidate) = candidate {
                    let candidate_hash = *candidate.header().hash.as_uint256();
                    let val_count = root.validations().num_trusted_for_ledger(candidate_hash);
                    if val_count >= quorum {
                        if best_validated
                            .as_ref()
                            .map_or(true, |b| candidate.header().seq > b.header().seq)
                        {
                            best_validated = Some(candidate);
                        }
                    }
                }
            }

            if let Some(network_ledger) = best_validated {
                let state_complete = !network_ledger.state_map().is_synching();
                let tx_complete = network_ledger.header().tx_hash.is_zero()
                    || !network_ledger.tx_map().is_synching();
                if state_complete && tx_complete {
                    let new_seq = network_ledger.header().seq;
                    let new_hash = *network_ledger.header().hash.as_uint256();
                    let mut l = (*network_ledger).clone();
                    l.set_validated();
                    let validated = Arc::new(l);
                    lm.set_valid_ledger_no_sweep(Arc::clone(&validated), None, None);
                    lm.mark_ledger_complete(validated.header().seq);
                    root.note_validated_ledger_for_sync(Arc::clone(&validated));
                    root.on_closed_ledger(Arc::clone(&validated));
                    root.set_need_network_ledger(false);
                    root.try_advance_publication();
                    root.promote_operating_mode_after_accepted_ledger(&validated);
                    // Restart consensus on the new chain
                    let now = root.shared_time_keeper().close_time();
                    let prev_cx = crate::consensus_ledger_from_ledger(&validated);
                    runner.start_round(now, new_hash, prev_cx, true);
                    consensus_rt.update_phase(runner.phase());
                    consensus_rt.update_prev_ledger_id(runner.prev_ledger_id());
                    *last_round_ledger_id = Some(new_hash);
                    tracing::info!(target: "consensus", new_seq, %new_hash,
                        "Consensus restarted on network chain (switchLastClosedLedger)");
                }
            }
        }
    }

    // ── checkAccept: promote closed ledger if quorum validations ──────────
    if let Some(closed) = root.closed_ledger() {
        let closed_seq = closed.header().seq;
        if closed_seq > lm.valid_ledger_seq() {
            let closed_hash = *closed.header().hash.as_uint256();
            let val_count = root.validations().num_trusted_for_ledger(closed_hash);
            if val_count >= quorum {
                let mut l = (*closed).clone();
                l.set_validated();
                let validated = Arc::new(l);
                lm.set_valid_ledger_no_sweep(Arc::clone(&validated), None, None);
                root.note_validated_ledger_for_sync(Arc::clone(&validated));
                lm.mark_ledger_complete(validated.header().seq);
                root.set_need_network_ledger(false);
            }
        }
    }

    // ── tryAdvance: burst through consecutive validated ledgers ───────────
    let mut advanced = 0u32;
    loop {
        let next_seq = lm.valid_ledger_seq() + 1;
        let Some(candidate) = lm.ledger_history().get_cached_ledger_by_seq(next_seq) else {
            break;
        };
        let candidate_hash = *candidate.header().hash.as_uint256();
        let val_count = root.validations().num_trusted_for_ledger(candidate_hash);
        if val_count < quorum {
            break;
        }
        let mut l = (*candidate).clone();
        l.set_validated();
        let validated = Arc::new(l);
        lm.set_valid_ledger_no_sweep(Arc::clone(&validated), None, None);
        root.note_validated_ledger_for_sync(Arc::clone(&validated));
        lm.mark_ledger_complete(validated.header().seq);
        root.set_need_network_ledger(false);
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
        if matches!(
            next_mode,
            NetworkOpsOperatingMode::Connected | NetworkOpsOperatingMode::Tracking
        ) && !need_network
        {
            let valid_seq = lm.valid_ledger_seq();
            let fresh = root.published_ledger().map_or(false, |pub_ledger| {
                let now_close = root.current_close_time_seconds();
                let pub_close = pub_ledger.header().close_time;
                let resolution = u32::from(pub_ledger.header().close_time_resolution);
                now_close < pub_close.saturating_add(resolution.saturating_mul(2))
            });
            let have_prev = valid_seq > 1 && lm.have_ledger(valid_seq - 1);
            if fresh || have_prev {
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
    // We also gate on fillInProgress (don't start a new fill if one is active
    // at a lower sequence).

    let valid_seq = lm.valid_ledger_seq();
    let pub_seq = lm
        .published_ledger()
        .map(|l| l.header().seq)
        .unwrap_or(0);

    // Condition 1: not standalone (always true here — strand only spawns for overlay mode)
    // Condition 2: fee load — skip if fee track reports overload
    let fee_overloaded = root.load_fee_track_loaded_local();
    // Condition 3: publication caught up to validation
    let publication_caught_up = valid_seq == pub_seq;
    // Condition 4: validated ledger age < 60s
    let validated_ledger_fresh = root.validated_ledger_age_seconds() < 60;
    // Condition 5: NodeStore write pressure
    // TODO(rippled-parity): Replace with `root.node_store_write_load() < 8192` once
    // ApplicationRoot exposes `Database::get_write_load()` through a convenience method.
    // The nodestore already implements get_write_load() but it is not yet wired through
    // ApplicationRoot. For now we approximate using active acquisition count which
    // correlates with write pressure during initial sync.
    let write_pressure_ok = shared_inbound.active_count() < 8;

    let can_acquire_history = !fee_overloaded
        && publication_caught_up
        && validated_ledger_fresh
        && write_pressure_ok
        && valid_seq > 1
        && last_history_tick.elapsed() >= Duration::from_secs(3);

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
                // fillInProgress gate: don't start a new fill if one is active
                // at a lower or equal sequence (matching rippled mFillInProgress).
                let fill_ok = fill_in_progress.map_or(true, |fip| missing > fip);
                if fill_ok {
                    let parent_hash = lm
                        .ledger_history()
                        .get_cached_ledger_by_seq(missing + 1)
                        .map(|l| *l.header().parent_hash.as_uint256());
                    if let Some(hash) = parent_hash {
                        if !hash.is_zero() {
                            let sha_hash = basics::sha_map_hash::SHAMapHash::new(hash);
                            if lm
                                .ledger_history()
                                .get_cached_ledger_by_hash(sha_hash)
                                .is_none()
                            {
                                *fill_in_progress = Some(missing);
                                shared_inbound.acquire_async(hash, missing, AcquireReason::History);
                            }
                        }
                    }
                }
            }
        }
    }
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
