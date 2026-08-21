//! The serialized coordinator runner (M3: typed events, reads, writes, timers).
//!
//! [`CoordinatorRunner`] is the production authority of the acquisition domain.
//! It owns [`CoordinatorState`] on one serialized strand/thread, consumes typed
//! [`AcquisitionEvent`]s, mutates coordinator state, and returns the typed
//! [`AcquisitionEffect`]s for adapters to execute *after* the call returns.
//!
//! Concurrency and ownership rules this runner enforces:
//!
//! * No callback is ever invoked while handling an event: `handle_event` only
//!   reads its owned state and returns a value list. An adapter can therefore
//!   never hold a resource lock while coordinator logic runs.
//! * The runner holds no port references; effects are executed by the caller
//!   after state mutation, and adapters return typed completion events.
//! * A completion may mutate state only when its [`SessionRef`] matches a live
//!   session. Timers are matched exactly against the operation that armed them;
//!   reads/writes/fences/handoffs gain exact in-flight operation matching in M4
//!   when the runner dispatches them.
//! * Cancellation invalidates a session immediately; every late event for it is
//!   stale and ignored (counted for observability).
//! * After [`AcquisitionEvent::Shutdown`] the runner produces no further
//!   effects and every later event is stale.
//!
//! Session lifecycle (tree plan, mailbox, persistence intent) migrates into the
//! coordinator in M4; here the runner owns the typed boundary, service phase,
//! session identity, admission accounting, and staleness/cancellation rules.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use ledger::Ledger;

use basics::base_uint::Uint256;

use crate::effect::AcquisitionEffect;
use crate::event::{AcquisitionEvent, ConsensusTarget};
use crate::handoff::{DurableHandoffAcknowledgement, DurableLedger};
use crate::id::{DurableHandoffId, IdCounter, PeerId, RunEpoch, StoreGeneration};
use crate::identity::{OperationKind, OperationRef, SessionRef};
use crate::ingress::{
    ADMISSION_BYTE_LIMIT, ADMISSION_PACKET_LIMIT, AdmissionBudget, AdmittedLedgerPacket,
};
use crate::io::{DurabilityCompletion, ReadCompletion, ReadPriority, WriteCompletion};
use crate::peer::{LedgerDataRequest, LedgerNodeRequest, PeerAvailabilitySnapshot, PeerRequest};
use crate::phase::{SyncPhase, TransitionFact};
use crate::plan::{
    MAX_TIMEOUT_REPROBES, NullPlanSeed, PlanDurabilityOutcome, PlanNetworkNeed, PlanReadOutcome,
    PlanSeed, PlanTimeout, PlanTurn, PlanWriteOutcome, SessionPlan, TurnContext,
};
use crate::session::{CancelReason, FailureReason, SessionPhase, session_phase_transition};
use crate::target::{AcquireReason, LedgerIdentity, LedgerTarget};
use crate::timer::{TimerKind, TimerRequest};

/// Small fixed delay for one durable-handoff delivery retry. The timer is
/// deliberately coordinator-owned and rearmed only after a later rejection;
/// it avoids immediate retry loops while keeping failed delivery responsive.
pub const HANDOFF_RETRY_DELAY: Duration = Duration::from_millis(100);

/// rippled `InboundLedger::addPeers` begins acquisition through this many
/// scored peers (`kPeerCountStart` in `InboundLedger.cpp`). Keep the
/// coordinator's peer policy at that protocol boundary without reintroducing
/// a second peer-set lifecycle owner.
const INITIAL_PEER_REQUEST_FANOUT: usize = 5;
/// rippled `InboundLedger::addPeers` adds this many scored peers after a
/// no-progress timeout (`kPeerCountAdd`).
const TIMEOUT_PEER_ESCALATION: usize = 3;
/// An acquisition has at most the initial rippled peer fanout plus one
/// `kPeerCountAdd` window for each bounded no-progress timeout. This is a
/// selected-peer window, not an unbounded history of every responder.
const MAX_SELECTED_PEERS: usize = INITIAL_PEER_REQUEST_FANOUT
    + TIMEOUT_PEER_ESCALATION * crate::plan::DEFAULT_MAX_ACQUIRE_TIMEOUTS as usize;
/// rippled switches to `TMGetObjectByHash` only after more than four
/// no-progress timeouts (`kLedgerBecomeAggressiveThreshold`).
const AGGRESSIVE_TIMEOUT_THRESHOLD: u32 = 4;
/// rippled request batch limits from
/// `../rippled/src/xrpld/app/ledger/detail/InboundLedger.cpp`:
/// `kReqNodesReply` for a response-triggered request and `kReqNodes` for
/// blind/local/timeout work.
const REPLY_NODE_REQUEST_BATCH: usize = 128;
const BLIND_NODE_REQUEST_BATCH: usize = 12;
/// Bounded timeout request batch (`kReqNodes`), shared with the plan's local
/// reprobe batch so one timeout interval covers one exact rotating frontier.
const TIMEOUT_FRONTIER_REQUEST_LIMIT: usize = MAX_TIMEOUT_REPROBES;

/// Coordinator-owned budgets for the acquisition domain.
///
/// `Default` reproduces the existing per-ledger mailbox semantics (`128`
/// packets, `4 MiB`) as the admission budget and bounds live sessions so
/// history demand cannot consume unbounded coordinator state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetState {
    max_sessions: usize,
    admission: AdmissionBudget,
    acquire_timeout: Duration,
}

impl BudgetState {
    /// Builds an explicit budget.
    pub const fn new(
        max_sessions: usize,
        admission: AdmissionBudget,
        acquire_timeout: Duration,
    ) -> Self {
        Self {
            max_sessions,
            admission,
            acquire_timeout,
        }
    }

    /// The maximum concurrently live sessions.
    pub const fn max_sessions(self) -> usize {
        self.max_sessions
    }

    /// The per-session admission budget (defaults to the existing mailbox
    /// limits of `128` packets / `4 MiB`).
    pub const fn admission(self) -> AdmissionBudget {
        self.admission
    }

    /// The acquisition attempt deadline before a session fails.
    pub const fn acquire_timeout(self) -> Duration {
        self.acquire_timeout
    }
}

impl Default for BudgetState {
    fn default() -> Self {
        Self::new(
            32,
            AdmissionBudget::new(ADMISSION_PACKET_LIMIT, ADMISSION_BYTE_LIMIT),
            Duration::from_secs(5),
        )
    }
}

/// All mutable acquisition-domain state owned by the coordinator on one owner
/// task (the target shape from `AGENTS.md`).
#[derive(Debug)]
pub struct CoordinatorState {
    phase: SyncPhase,
    run_epoch: RunEpoch,
    sessions: BTreeMap<SessionRef, CoordinatorSession>,
    budgets: BudgetState,
    peer_view: PeerAvailabilitySnapshot,
    /// One bounded exact target demand deferred only because no usable peer
    /// capability existed. It is replayed by this same owner on the next
    /// `PeerCapabilityAvailable` fact; it is never an adapter-side retry.
    deferred_acquire: Option<(LedgerTarget, AcquireReason)>,
    /// Latest preferred-LCL demand that could not obtain capacity after
    /// cancellable Generic/History work was considered. It is coordinator
    /// state, not a registry retry or consensus callback latch.
    deferred_consensus_acquire: Option<LedgerTarget>,
    storage_generation: StoreGeneration,
    /// Latest serialized local LCL fact. A Full identity is refreshed only by
    /// a fresh publication of this exact ledger.
    last_installed_lcl: Option<LedgerIdentity>,
    ids: IdCounter,
}

/// The coordinator-owned lifecycle of one session.
///
/// Owns the [`SessionPlan`] (mailbox, tree engine, read admission, persistence
/// intent, timeout budget, cancellation) plus the pending timer used for exact
/// `TimerFired` matching. No other component may mutate this state.
#[derive(Debug)]
pub struct CoordinatorSession {
    target: LedgerTarget,
    reason: AcquireReason,
    phase: SessionPhase,
    plan: SessionPlan,
    sent_peers: BTreeSet<PeerId>,
    /// Hashes recently requested from the network. This is the coordinator
    /// equivalent of rippled `InboundLedger::recentNodes_`: reply-driven
    /// turns must not re-request an already outstanding missing node.
    recent_node_hashes: BTreeSet<Uint256>,
    pending_timer: Option<(TimerKind, OperationRef)>,
    pending_handoff: Option<DurableHandoffId>,
    // The durable ledger is retained for handoff retry: `plan.durable_ledger()`
    // yields the payload exactly once, and any timer-driven re-publish of the
    // pending handoff id carries the same payload for recipient deduplication.
    durable: Option<Arc<Ledger>>,
}

impl CoordinatorSession {
    fn new(
        target: LedgerTarget,
        reason: AcquireReason,
        peer: PeerId,
        admission: AdmissionBudget,
    ) -> Self {
        let mut sent_peers = BTreeSet::new();
        sent_peers.insert(peer);
        Self {
            target,
            reason,
            phase: SessionPhase::Active,
            plan: SessionPlan::new(admission),
            sent_peers,
            recent_node_hashes: BTreeSet::new(),
            pending_timer: None,
            pending_handoff: None,
            durable: None,
        }
    }

    /// The target being acquired.
    pub const fn target(&self) -> LedgerTarget {
        self.target
    }

    /// Why the target is being acquired.
    pub const fn reason(&self) -> AcquireReason {
        self.reason
    }

    /// The session lifecycle phase.
    pub const fn phase(&self) -> &SessionPhase {
        &self.phase
    }

    /// The coordinator-owned session plan.
    pub const fn plan(&self) -> &SessionPlan {
        &self.plan
    }

    /// The currently accepted packet count (queued mailbox packets).
    pub fn packet_count(&self) -> u64 {
        self.plan.packet_count()
    }

    /// The currently accepted packet bytes (queued mailbox packets).
    pub fn packet_bytes(&self) -> u64 {
        self.plan.packet_bytes()
    }

    /// Bounded selected peers eligible for timeout resends. Reply-driven
    /// requests may use a responding peer once without expanding this window.
    pub fn sent_peers(&self) -> &BTreeSet<PeerId> {
        &self.sent_peers
    }

    /// The timer this session has armed and is waiting on, if any.
    pub const fn pending_timer(&self) -> Option<(TimerKind, OperationRef)> {
        self.pending_timer
    }

    /// The durable handoff id this session is awaiting acknowledgement for,
    /// once its durability fence passed.
    pub const fn pending_handoff(&self) -> Option<DurableHandoffId> {
        self.pending_handoff
    }

    fn can_promote_unknown_sequence(&self, target: LedgerTarget) -> bool {
        // A known sequence strengthens the identity metadata of an active
        // hash-only acquisition; it does not change the session, its rooted
        // plan, or any in-flight operation. Once the Base packet has seeded
        // the plan, replacing this session would discard admitted work and
        // restart the same hash from scratch.
        self.phase == SessionPhase::Active
            && self.target.hash() == target.hash()
            && self.target.sequence().is_none()
            && target.sequence().is_some()
    }

    fn promote_target(&mut self, target: LedgerTarget) {
        self.target = target;
    }
}

/// An immutable snapshot of runner-owned state for observability.
///
/// Admission-gate reservation counters live at ingress and are composed into
/// the adapter-level [`crate::CoordinatorSnapshot`]; this snapshot reports what
/// the runner owns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerSnapshot {
    run_epoch: RunEpoch,
    phase: SyncPhase,
    session_count: usize,
    active_by_reason: BTreeMap<AcquireReason, usize>,
    storage_generation: StoreGeneration,
    peer_count: usize,
    events_handled: u64,
    stale_events: u64,
    rejected_events: u64,
    sessions_started: u64,
    sessions_cancelled: u64,
    cancelled_by_reason: BTreeMap<CancelReason, u64>,
    failed_by_reason: BTreeMap<FailureReason, u64>,
    sessions_completed: u64,
    handoff_rejections: u64,
    peer_requests: u64,
    timers_armed: u64,
    packets_admitted: u64,
    packets_dropped: u64,
    plan_turns: u64,
    fetch_pack_advances: u64,
    shutdown: bool,
}

impl RunnerSnapshot {
    /// The coordinator run epoch.
    pub const fn run_epoch(&self) -> RunEpoch {
        self.run_epoch
    }

    /// The current service phase.
    pub const fn phase(&self) -> &SyncPhase {
        &self.phase
    }

    /// The number of tracked sessions.
    pub const fn session_count(&self) -> usize {
        self.session_count
    }

    /// Live sessions grouped by acquisition reason.
    pub fn active_by_reason(&self) -> &BTreeMap<AcquireReason, usize> {
        &self.active_by_reason
    }

    /// The current NodeStore generation.
    pub const fn storage_generation(&self) -> StoreGeneration {
        self.storage_generation
    }

    /// The number of usable peers in the last connectivity snapshot.
    pub const fn peer_count(&self) -> usize {
        self.peer_count
    }

    /// Events accepted by the runner (including rejected/stale ones).
    pub const fn events_handled(&self) -> u64 {
        self.events_handled
    }

    /// Stale events ignored after cancellation, replacement, or shutdown.
    pub const fn stale_events(&self) -> u64 {
        self.stale_events
    }

    /// Events rejected for a policy reason (no peers, phase rules, capacity).
    pub const fn rejected_events(&self) -> u64 {
        self.rejected_events
    }

    /// Sessions started.
    pub const fn sessions_started(&self) -> u64 {
        self.sessions_started
    }

    /// Sessions cancelled.
    pub const fn sessions_cancelled(&self) -> u64 {
        self.sessions_cancelled
    }

    /// Terminal cancellations grouped by their exact coordinator-owned reason.
    pub fn cancelled_by_reason(&self) -> &BTreeMap<CancelReason, u64> {
        &self.cancelled_by_reason
    }

    /// Terminal failures grouped by their exact coordinator-owned reason.
    pub fn failed_by_reason(&self) -> &BTreeMap<FailureReason, u64> {
        &self.failed_by_reason
    }

    /// Sessions that reached `Complete` after a durable handoff ack.
    pub const fn sessions_completed(&self) -> u64 {
        self.sessions_completed
    }

    /// Durable handoff deliveries rejected by the adapter (channel full or
    /// disconnected); each accepted rejection arms one exact retry timer for
    /// the same id. Duplicate rejections while armed are stale.
    pub const fn handoff_rejections(&self) -> u64 {
        self.handoff_rejections
    }

    /// Peer requests emitted.
    pub const fn peer_requests(&self) -> u64 {
        self.peer_requests
    }

    /// Timers armed.
    pub const fn timers_armed(&self) -> u64 {
        self.timers_armed
    }

    /// Packets routed to sessions.
    pub const fn packets_admitted(&self) -> u64 {
        self.packets_admitted
    }

    /// Packets dropped by the defensive session mailbox bounds.
    pub const fn packets_dropped(&self) -> u64 {
        self.packets_dropped
    }

    /// Bounded tree-plan turns executed across live sessions.
    pub const fn plan_turns(&self) -> u64 {
        self.plan_turns
    }

    /// Turns run in response to a fetch-pack availability fact.
    pub const fn fetch_pack_advances(&self) -> u64 {
        self.fetch_pack_advances
    }

    /// True after a shutdown event was handled.
    pub const fn shutdown(&self) -> bool {
        self.shutdown
    }
}

/// Counter state for runner observability.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct RunnerStats {
    events_handled: u64,
    stale_events: u64,
    rejected_events: u64,
    sessions_started: u64,
    sessions_cancelled: u64,
    cancelled_by_reason: BTreeMap<CancelReason, u64>,
    failed_by_reason: BTreeMap<FailureReason, u64>,
    sessions_completed: u64,
    handoff_rejections: u64,
    peer_requests: u64,
    timers_armed: u64,
    packets_admitted: u64,
    packets_dropped: u64,
    plan_turns: u64,
    fetch_pack_advances: u64,
}

/// The serialized coordinator event loop.
#[derive(Debug)]
pub struct CoordinatorRunner {
    state: CoordinatorState,
    stats: RunnerStats,
    shutdown: bool,
    /// One-shot engine construction port. It runs outside coordinator state
    /// mutation and returns a uniquely owned engine or nothing.
    plan_seed: Box<dyn PlanSeed + Send + Sync>,
}

impl CoordinatorRunner {
    /// A runner with default budgets for a run epoch.
    pub fn new(run_epoch: RunEpoch) -> Self {
        Self::with_budget(run_epoch, BudgetState::default())
    }

    /// A runner seeded into an exact phase with a usable peer, for
    /// deterministic transition tests.
    #[cfg(test)]
    pub(crate) fn with_phase(run_epoch: RunEpoch, phase: SyncPhase) -> Self {
        let mut runner = Self::with_budget(run_epoch, BudgetState::default());
        runner.state.phase = phase;
        runner.state.peer_view = PeerAvailabilitySnapshot::new(vec![PeerId::new(1)]);
        runner
    }

    /// A runner with explicit budgets.
    pub fn with_budget(run_epoch: RunEpoch, budgets: BudgetState) -> Self {
        Self::with_plan_seed(run_epoch, budgets, Box::new(NullPlanSeed))
    }

    /// A runner with explicit budgets and a plan seed. The seed builds a rooted
    /// engine from the first Base/header packet; with `NullPlanSeed` packets
    /// stay in the session mailbox and the session waits on its timer.
    pub fn with_plan_seed(
        run_epoch: RunEpoch,
        budgets: BudgetState,
        plan_seed: Box<dyn PlanSeed + Send + Sync>,
    ) -> Self {
        Self {
            state: CoordinatorState {
                phase: SyncPhase::Disconnected,
                run_epoch,
                sessions: BTreeMap::new(),
                budgets,
                peer_view: PeerAvailabilitySnapshot::new(vec![]),
                deferred_acquire: None,
                deferred_consensus_acquire: None,
                storage_generation: StoreGeneration::new(1),
                last_installed_lcl: None,
                ids: IdCounter::new(),
            },
            stats: RunnerStats::default(),
            shutdown: false,
            plan_seed,
        }
    }

    /// Handles one typed event and returns the effects adapters must execute
    /// after this call returns. Never invokes a port or callback.
    pub fn handle_event(&mut self, event: AcquisitionEvent) -> Vec<AcquisitionEffect> {
        if self.shutdown {
            // After shutdown the coordinator produces no effects; every later
            // event is stale.
            self.stats.stale_events += 1;
            return Vec::new();
        }
        let mut effects = match event {
            AcquisitionEvent::Shutdown => self.on_shutdown(),
            AcquisitionEvent::StartupMode { phase } => self.on_startup_mode(phase),
            AcquisitionEvent::Connectivity(snapshot) => self.on_connectivity(snapshot),
            AcquisitionEvent::AcquireRequested { target, reason } => {
                self.on_acquire(target, reason)
            }
            AcquisitionEvent::ConsensusTarget(target) => self.on_consensus(target),
            AcquisitionEvent::PreferredLclDivergence { target } => {
                self.on_preferred_lcl_divergence(target)
            }
            AcquisitionEvent::BlockedWithNoTarget => self.on_blocked_with_no_target(),
            AcquisitionEvent::PacketAdmitted(packet) => self.on_packet(packet),
            AcquisitionEvent::ReadCompleted(completion) => self.on_read(completion),
            AcquisitionEvent::WriteCompleted(completion) => self.on_write(completion),
            AcquisitionEvent::DurabilityFenced(completion) => self.on_durability(completion),
            AcquisitionEvent::DurableHandoffAcknowledged(ack) => self.on_handoff_ack(ack),
            AcquisitionEvent::DurableHandoffRejected {
                handoff, session, ..
            } => self.on_handoff_rejected(handoff, session),
            AcquisitionEvent::TimerFired { operation, timer } => self.on_timer(operation, timer),
            AcquisitionEvent::LclInstalled(identity) => self.on_lcl_installed(identity),
            AcquisitionEvent::PublicationCommitted { identity, fresh } => {
                self.on_publication(identity, fresh)
            }
            AcquisitionEvent::StoreRotated(generation) => self.on_store_rotated(generation),
            AcquisitionEvent::FetchPackAvailable => self.on_fetch_pack(),
            AcquisitionEvent::Heartbeat => self.on_heartbeat(),
        };
        self.replay_deferred_consensus(&mut effects);
        self.stats.events_handled += 1;
        effects
    }

    /// True only when the coordinator retained the latest capacity-deferred
    /// preferred-LCL demand. Adapters use this disposition to retain the
    /// session-origin binding; they never become a second retry owner.
    pub fn has_deferred_consensus_target(&self, target: LedgerTarget) -> bool {
        self.state.deferred_consensus_acquire == Some(target)
    }

    /// Retain and replay the latest consensus target only when a session slot
    /// is actually free. This is invoked after every fact on the serialized
    /// owner; no registry, adapter, or consensus callback owns a retry loop.
    fn replay_deferred_consensus(&mut self, effects: &mut Vec<AcquisitionEffect>) {
        if self.shutdown
            || !self.state.peer_view.has_usable_peer_capability()
            || self
                .state
                .sessions
                .values()
                .filter(|session| !session.phase.is_terminal())
                .count()
                >= self.state.budgets.max_sessions
        {
            return;
        }
        let Some(target) = self.state.deferred_consensus_acquire.take() else {
            return;
        };
        effects.extend(self.on_acquire(target, AcquireReason::Consensus));
    }

    /// The coordinator run epoch.
    pub const fn run_epoch(&self) -> RunEpoch {
        self.state.run_epoch
    }

    /// The current service phase.
    pub const fn phase(&self) -> &SyncPhase {
        &self.state.phase
    }

    /// The current NodeStore generation.
    pub const fn storage_generation(&self) -> StoreGeneration {
        self.state.storage_generation
    }

    /// The live session for the exact reference, if any.
    pub fn session(&self, session: SessionRef) -> Option<&CoordinatorSession> {
        self.state.sessions.get(&session)
    }

    /// Every routable live session reference, in target/session order.
    ///
    /// Adapters rebuild routing snapshots from this set so a session is routed
    /// exactly while it can accept packets. Sessions awaiting durable handoff
    /// acknowledgement are excluded: admission against them would reserve
    /// capacity the coordinator would reject as stale.
    pub fn live_sessions(&self) -> impl Iterator<Item = SessionRef> + '_ {
        self.state
            .sessions
            .iter()
            .filter(|(_, session)| {
                matches!(
                    session.phase,
                    SessionPhase::Active | SessionPhase::Persisting
                )
            })
            .map(|(session_ref, _)| *session_ref)
    }

    /// Runner-owned observable state.
    pub fn snapshot(&self) -> RunnerSnapshot {
        let mut active_by_reason = BTreeMap::new();
        for session in self.state.sessions.values() {
            if !session.phase.is_terminal() {
                *active_by_reason.entry(session.reason).or_insert(0usize) += 1;
            }
        }
        RunnerSnapshot {
            run_epoch: self.state.run_epoch,
            phase: self.state.phase,
            session_count: self.state.sessions.len(),
            active_by_reason,
            storage_generation: self.state.storage_generation,
            peer_count: self.state.peer_view.peers().len(),
            events_handled: self.stats.events_handled,
            stale_events: self.stats.stale_events,
            rejected_events: self.stats.rejected_events,
            sessions_started: self.stats.sessions_started,
            sessions_cancelled: self.stats.sessions_cancelled,
            cancelled_by_reason: self.stats.cancelled_by_reason.clone(),
            failed_by_reason: self.stats.failed_by_reason.clone(),
            sessions_completed: self.stats.sessions_completed,
            handoff_rejections: self.stats.handoff_rejections,
            peer_requests: self.stats.peer_requests,
            timers_armed: self.stats.timers_armed,
            packets_admitted: self.stats.packets_admitted,
            packets_dropped: self.stats.packets_dropped,
            plan_turns: self.stats.plan_turns,
            fetch_pack_advances: self.stats.fetch_pack_advances,
            shutdown: self.shutdown,
        }
    }

    fn on_shutdown(&mut self) -> Vec<AcquisitionEffect> {
        let mut effects = Vec::new();
        if let Ok(next) = self.state.phase.apply(TransitionFact::Shutdown) {
            self.state.phase = next;
            effects.push(AcquisitionEffect::SetServicePhase(next));
        }
        self.cancel_all(CancelReason::Shutdown, &mut effects);
        self.shutdown = true;
        effects
    }

    /// Seeds the coordinator's initial service phase from the bootstrap startup
    /// intent. Not a transition and never touches a session: it re-publishes
    /// the phase through the phase port so the coordinator is the sole mode
    /// writer from the moment it installs (M6-D). A repeat of the same phase is
    /// a no-op; a different seed re-publishes the new phase.
    fn on_startup_mode(&mut self, phase: SyncPhase) -> Vec<AcquisitionEffect> {
        let mut effects = Vec::new();
        if phase == self.state.phase {
            return effects;
        }
        self.state.phase = phase;
        effects.push(AcquisitionEffect::SetServicePhase(phase));
        effects
    }

    fn on_connectivity(&mut self, snapshot: PeerAvailabilitySnapshot) -> Vec<AcquisitionEffect> {
        let had_peers = self.state.peer_view.has_usable_peer_capability();
        let has_peers = snapshot.has_usable_peer_capability();
        self.state.peer_view = snapshot;
        let mut effects = Vec::new();
        let fact = if has_peers {
            if had_peers {
                // Already connected; an unchanged snapshot changes nothing.
                return effects;
            }
            TransitionFact::PeerCapabilityAvailable
        } else {
            if !had_peers {
                // Already disconnected; an unchanged snapshot changes nothing.
                return effects;
            }
            TransitionFact::PeerCapabilityLost
        };
        if let Ok(next) = self.state.phase.apply(fact) {
            self.state.phase = next;
            effects.push(AcquisitionEffect::SetServicePhase(next));
            if !has_peers {
                // A durable handoff has already crossed the final storage
                // fence. Peer loss may demote service phase, but must not
                // revoke its exact retry/ack state; all other live sessions
                // retain the established real-zero-peer cancellation policy.
                self.cancel_non_durable_for_peer_loss(&mut effects);
            }
        }
        // A target received while peerless is a concrete consensus/recovery
        // fact, not disposable work. rippled resumes timer-driven demand after
        // peer capability returns; replay the coordinator-owned exact target
        // only after the transition fact has established that capability.
        if has_peers && let Some((target, reason)) = self.state.deferred_acquire.take() {
            effects.extend(self.on_acquire(target, reason));
        }
        effects
    }

    fn on_consensus(&mut self, target: ConsensusTarget) -> Vec<AcquisitionEffect> {
        self.on_acquire(target.target(), target.reason())
    }

    /// A preferred-LCL divergence demotion (rippled `consensusViewChange`).
    /// Demotes `Connected/Tracking/Full -> Syncing { target }` without minting a
    /// session: the resident-and-compatible switch path must not start a peer
    /// fetch, and the missing/incomplete path feeds its own `AcquireRequested`
    /// demand. Rejected without usable peers (the transition rules require a
    /// fresh `PeerCapabilityAvailable` fact first) and from phases where the
    /// divergence fact is illegal (`Syncing`/`Disconnected`/`Stopping`).
    fn on_preferred_lcl_divergence(&mut self, target: LedgerTarget) -> Vec<AcquisitionEffect> {
        if !self.state.peer_view.has_usable_peer_capability() {
            self.stats.rejected_events += 1;
            tracing::info!(
                target: "acquisition_trace",
                event = "preferred_lcl_divergence_rejected",
                target_hash = %target.hash(),
                target_seq = ?target.sequence(),
                phase = ?self.state.phase,
                peer_count = self.state.peer_view.peers().len(),
                reason = "no_usable_peers",
                "acquisition trace: preferred-LCL divergence did not enter coordinator"
            );
            return Vec::new();
        }
        let fact = TransitionFact::PreferredLclDivergence { target };
        let mut effects = Vec::new();
        if let Ok(next) = self.state.phase.apply(fact) {
            if next != self.state.phase {
                self.state.phase = next;
                effects.push(AcquisitionEffect::SetServicePhase(next));
            }
        } else {
            self.stats.rejected_events += 1;
        }
        effects
    }

    /// Consensus accepted a round with no usable peer positions while `Full`
    /// (Quaxar-specific `no_consensus_positions` demotion). Demotes
    /// `Full -> Connected` with no concrete target; a later
    /// [`AcquisitionEvent::PreferredLclDivergence`] or
    /// [`AcquisitionEvent::ConsensusTarget`] fact motivates `Connected -> Syncing`.
    fn on_blocked_with_no_target(&mut self) -> Vec<AcquisitionEffect> {
        let fact = TransitionFact::BlockedWithNoTarget;
        let mut effects = Vec::new();
        if let Ok(next) = self.state.phase.apply(fact) {
            if next != self.state.phase {
                self.state.phase = next;
                effects.push(AcquisitionEffect::SetServicePhase(next));
            }
        } else {
            self.stats.rejected_events += 1;
        }
        effects
    }

    fn on_acquire(
        &mut self,
        target: LedgerTarget,
        reason: AcquireReason,
    ) -> Vec<AcquisitionEffect> {
        let mut effects = Vec::new();

        // A newer consensus fact supersedes any earlier capacity-deferred
        // preferred-LCL target. If this exact demand cannot start below, it is
        // retained again as the latest one.
        if reason == AcquireReason::Consensus {
            self.state.deferred_consensus_acquire = None;
        }

        // Retain exactly one concrete demand while peerless. The owner replays
        // it after `PeerCapabilityAvailable`; dropping it would strand recovery
        // until another unrelated validation/consensus event happens to repeat
        // the same target.
        if !self.state.peer_view.has_usable_peer_capability() {
            self.state.deferred_acquire = Some((target, reason));
            tracing::info!(
                target: "acquisition_trace",
                event = "acquire_demand_deferred",
                target_hash = %target.hash(),
                target_seq = ?target.sequence(),
                ?reason,
                phase = ?self.state.phase,
                peer_count = self.state.peer_view.peers().len(),
                "acquisition trace: retained target demand until usable peer capability returns"
            );
            return effects;
        }

        // Resolve duplicate demand before capacity and cancellation. An exact
        // same-target demand coalesces with its live owner, including a
        // DurablePending handoff. The only mutable coalescing case is an
        // unknown-sequence session promoted to a known sequence before its
        // header seeded a plan; later or conflicting requests retain the
        // existing replacement semantics, except DurablePending is never
        // cancelled because its durable result is committed to delivery.
        let same_hash = self.live_sessions_for_hash(target.hash());
        let exact = same_hash.iter().copied().find(|session| {
            self.state
                .sessions
                .get(session)
                .is_some_and(|state| state.target == target)
        });
        let promotion = if exact.is_none() {
            same_hash.iter().copied().find(|session| {
                self.state
                    .sessions
                    .get(session)
                    .is_some_and(|state| state.can_promote_unknown_sequence(target))
            })
        } else {
            None
        };
        // A hash-only demand carries less identity information than an
        // existing same-hash session with a verified sequence. It therefore
        // coalesces with that live owner rather than cancelling it. This is
        // the legacy InboundLedgers one-acquisition-per-hash behavior and
        // avoids consensus's hash-only follow-up thrashing a catch-up request
        // that already knows the sequence.
        let hash_only_coalesce =
            if exact.is_none() && promotion.is_none() && target.sequence().is_none() {
                same_hash.iter().copied().find(|session| {
                    self.state.sessions.get(session).is_some_and(|state| {
                        state.target.hash() == target.hash() && state.target.sequence().is_some()
                    })
                })
            } else {
                None
            };
        let replaceable: Vec<SessionRef> =
            if exact.is_none() && promotion.is_none() && hash_only_coalesce.is_none() {
                same_hash
                    .iter()
                    .copied()
                    .filter(|session| {
                        self.state
                            .sessions
                            .get(session)
                            .is_some_and(|state| state.phase != SessionPhase::DurablePending)
                    })
                    .collect()
            } else {
                Vec::new()
            };
        let continuing_existing =
            exact.is_some() || promotion.is_some() || hash_only_coalesce.is_some();
        // Terminal sessions remain observable for stale-event accounting, but
        // they are no longer live coordinator work and must not consume the
        // bounded concurrent-session budget. Otherwise a sequence of failed
        // targets permanently stalls acquisition after `max_sessions`.
        let live_sessions = self
            .state
            .sessions
            .values()
            .filter(|state| !state.phase.is_terminal())
            .count();
        let mut would_exceed = !continuing_existing
            && live_sessions.saturating_sub(replaceable.len()) >= self.state.budgets.max_sessions;
        // A retained preferred-LCL target reserves the next free slot. Generic
        // or History work may coalesce with an existing owner, but cannot take
        // that slot and permanently strand convergence.
        if reason != AcquireReason::Consensus
            && !continuing_existing
            && self.state.deferred_consensus_acquire.is_some()
        {
            would_exceed = true;
        }
        // Consensus may preempt only a cancellable lower-priority session.
        // DurablePending is intentionally excluded: its completed result and
        // handoff are already committed and must not be revoked.
        if would_exceed && reason == AcquireReason::Consensus {
            let preempt = self
                .state
                .sessions
                .iter()
                .filter(|(_, state)| {
                    matches!(
                        state.reason,
                        AcquireReason::History | AcquireReason::Generic
                    ) && matches!(state.phase, SessionPhase::Active | SessionPhase::Persisting)
                })
                .min_by_key(|(_, state)| match state.reason {
                    AcquireReason::History => 0u8,
                    AcquireReason::Generic => 1u8,
                    AcquireReason::Consensus => 2u8,
                })
                .map(|(session, _)| *session);
            if let Some(preempt) = preempt {
                self.cancel_session(preempt, CancelReason::Explicit, &mut effects);
                would_exceed = false;
            }
        }
        let disposition = if exact.is_some() {
            "coalesced_exact"
        } else if promotion.is_some() {
            "promoted_known_sequence"
        } else if hash_only_coalesce.is_some() {
            "coalesced_hash_only"
        } else if would_exceed {
            "rejected_capacity"
        } else if replaceable.is_empty() {
            "new_session"
        } else {
            "replacing_conflicting_session"
        };
        if matches!(
            disposition,
            "new_session"
                | "replacing_conflicting_session"
                | "promoted_known_sequence"
                | "rejected_capacity"
        ) {
            tracing::info!(
                target: "acquisition_trace",
                event = "acquire_demand_disposition",
                target_hash = %target.hash(),
                target_seq = ?target.sequence(),
                ?reason,
                phase = ?self.state.phase,
                peer_count = self.state.peer_view.peers().len(),
                same_hash_sessions = same_hash.len(),
                replacement_count = replaceable.len(),
                live_sessions,
                max_sessions = self.state.budgets.max_sessions,
                disposition,
                "acquisition trace: target demand disposition"
            );
        } else {
            tracing::debug!(
                target: "acquisition_trace",
                event = "acquire_demand_coalesced",
                target_hash = %target.hash(),
                target_seq = ?target.sequence(),
                ?reason,
                disposition,
                "acquisition trace: target demand reused its existing live session"
            );
        }
        if would_exceed {
            if reason == AcquireReason::Consensus {
                self.state.deferred_consensus_acquire = Some(target);
                let fact = TransitionFact::TargetRequired { target };
                if let Ok(next) = self.state.phase.apply(fact)
                    && next != self.state.phase
                {
                    self.state.phase = next;
                    effects.push(AcquisitionEffect::SetServicePhase(next));
                }
                tracing::info!(
                    target: "acquisition_trace",
                    event = "consensus_demand_deferred_capacity",
                    target_hash = %target.hash(),
                    target_seq = ?target.sequence(),
                    live_sessions,
                    max_sessions = self.state.budgets.max_sessions,
                    "acquisition trace: retained latest preferred-LCL demand until capacity is free"
                );
            } else {
                self.stats.rejected_events += 1;
            }
            return effects;
        }

        // A hash-only consensus lookup issued while Full is commonly the
        // consensus adaptor probing the child it is about to install locally.
        // It is not yet evidence of a *network* acquisition requirement: the
        // normal locally-produced child may receive a matching peer Base reply
        // before it is installed as LCL. That reply confirms availability, not
        // divergence: rippled changes operating mode only from the explicit
        // preferred-LCL reconciliation path. Keep Full for hash-only consensus
        // probes; an actual remote divergence arrives separately as
        // `PreferredLclDivergence` before its acquisition demand. A known
        // target and a non-consensus demand retain the required immediate
        // demotion.
        let defer_hash_only_consensus_probe = reason == AcquireReason::Consensus
            && target.sequence().is_none()
            && matches!(self.state.phase, SyncPhase::Full { .. });

        // Phase transition next: Connected/Syncing/Tracking/Full -> Syncing.
        // A coalesced promotion updates the phase target without minting a
        // second session; an exact duplicate is otherwise a phase no-op.
        let phase_target = hash_only_coalesce
            .and_then(|session| self.state.sessions.get(&session).map(|state| state.target))
            .unwrap_or(target);
        if !defer_hash_only_consensus_probe {
            let fact = TransitionFact::TargetRequired {
                target: phase_target,
            };
            if let Ok(next) = self.state.phase.apply(fact) {
                if next != self.state.phase {
                    self.state.phase = next;
                    effects.push(AcquisitionEffect::SetServicePhase(next));
                }
            } else {
                self.stats.rejected_events += 1;
                return effects;
            }
        }

        if exact.is_some() || hash_only_coalesce.is_some() {
            return effects;
        }
        if let Some(session) = promotion {
            if let Some(session_state) = self.state.sessions.get_mut(&session) {
                session_state.promote_target(target);
            }
            return effects;
        }

        // Conflicting known sequences retain replacement behavior for normal
        // live sessions. DurablePending is intentionally absent from
        // `replaceable`: its handoff remains pending until acknowledgement.
        for old in replaceable {
            self.cancel_session(old, CancelReason::Replaced, &mut effects);
        }

        // Mint the session against the current storage generation.
        let session = SessionRef::new(
            self.state.run_epoch,
            self.state.ids.next_id(),
            target.hash(),
            self.state.ids.next_id(),
            self.state.storage_generation,
        );
        // rippled `InboundLedger::addPeers` starts through five scored peers
        // and triggers each selected peer. The coordinator's availability
        // snapshot is already ordered by the overlay adapter, so retain its
        // order and establish the same bounded initial fanout.
        let initial_peers = self
            .state
            .peer_view
            .peers()
            .iter()
            .copied()
            .take(INITIAL_PEER_REQUEST_FANOUT)
            .collect::<Vec<_>>();
        let peer = initial_peers[0];
        tracing::info!(
            target: "acquisition_trace",
            event = "session_started",
            run_epoch = session.run_epoch().get(),
            session_id = session.session_id().get(),
            target_hash = %session.target_hash(),
            plan_epoch = session.plan_epoch().get(),
            store_generation = session.store_generation().get(),
            target_seq = ?target.sequence(),
            ?reason,
            initial_peer_count = initial_peers.len(),
            initial_peers = ?initial_peers,
            acquire_timeout_ms = self.state.budgets.acquire_timeout.as_millis(),
            "acquisition trace: exact target session started with initial Base fanout"
        );
        let admission = self.state.budgets.admission;
        self.state.sessions.insert(
            session,
            CoordinatorSession::new(target, reason, peer, admission),
        );
        self.stats.sessions_started += 1;
        // Bind delivery metadata to this exact session before the first
        // request. This preserves durable handoff identity when the session
        // was created by a deferred demand replay.
        effects.push(AcquisitionEffect::SessionStarted(session));

        // Request the Base/header ledger packet from each initially acquired
        // peer. An unknown sequence remains a header request with `None`;
        // target hashes are never misframed as tree-node requests.
        for peer in initial_peers {
            if let Some(session_state) = self.state.sessions.get_mut(&session) {
                session_state.sent_peers.insert(peer);
            }
            let operation = OperationRef::new(
                session,
                OperationKind::PeerRequest,
                self.state.ids.next_id(),
                self.state.ids.next_id(),
            );
            self.stats.peer_requests += 1;
            effects.push(AcquisitionEffect::SendLedgerRequest(PeerRequest::new(
                session,
                operation,
                peer,
                LedgerDataRequest::GetLedger {
                    sequence: target.sequence(),
                },
            )));
        }

        // Arm the acquisition deadline. The wakeup returns as a typed
        // `TimerFired` matched exactly against this operation.
        let timer_operation = OperationRef::new(
            session,
            OperationKind::Timer,
            self.state.ids.next_id(),
            self.state.ids.next_id(),
        );
        if let Some(session_state) = self.state.sessions.get_mut(&session) {
            session_state.pending_timer = Some((TimerKind::AcquireTimeout, timer_operation));
        }
        self.stats.timers_armed += 1;
        effects.push(AcquisitionEffect::ArmTimer(TimerRequest::new(
            timer_operation,
            TimerKind::AcquireTimeout,
            self.state.budgets.acquire_timeout,
        )));
        effects
    }

    fn on_packet(&mut self, packet: AdmittedLedgerPacket) -> Vec<AcquisitionEffect> {
        let session = packet.lease().session();
        let mut effects = Vec::new();
        let Some(session_state) = self.state.sessions.get_mut(&session) else {
            self.stats.stale_events += 1;
            return Vec::new();
        };
        if session_state.phase.is_terminal() {
            self.stats.stale_events += 1;
            return Vec::new();
        }
        // A header packet seeds the uniquely owned engine once. The seed runs
        // outside state mutation and returns an engine or nothing.
        let packet_peer = packet.peer_id();
        let header = packet.packet().clone();
        let needs_seed = packet.packet().packet_type == ledger::InboundLedgerDataType::Base
            && session_state.plan.engine().is_none();
        let seeded = if needs_seed {
            self.plan_seed
                .build(session, &header)
                .map(|engine| session_state.plan.install_engine(engine))
                .unwrap_or(false)
        } else {
            false
        };
        if needs_seed {
            tracing::info!(
                target: "acquisition_trace",
                event = "base_seed_evaluated",
                run_epoch = session.run_epoch().get(),
                session_id = session.session_id().get(),
                target_hash = %session.target_hash(),
                plan_epoch = session.plan_epoch().get(),
                store_generation = session.store_generation().get(),
                peer_id = packet_peer.get(),
                packet_nodes = header.nodes.len(),
                seeded,
                "acquisition trace: Base packet evaluated for header/root plan seed"
            );
        }
        // Defensive mailbox bounds: ingress already reserved capacity through
        // the `AdmissionGate`, but a misconfigured or replaying ingress must
        // not overflow coordinator state. The plan's mailbox enforces the same
        // `128`-packet / `4 MiB` semantics.
        if !session_state.plan.push_packet(packet) {
            self.stats.packets_dropped += 1;
            tracing::info!(
                target: "acquisition_trace",
                event = "packet_dropped_mailbox_full",
                run_epoch = session.run_epoch().get(),
                session_id = session.session_id().get(),
                target_hash = %session.target_hash(),
                peer_id = packet_peer.get(),
                mailbox_packets = session_state.plan.packet_count(),
                mailbox_bytes = session_state.plan.packet_bytes(),
                "acquisition trace: admitted packet could not enter bounded session mailbox"
            );
            return Vec::new();
        }
        self.stats.packets_admitted += 1;
        // A Base packet only earns progress after the seed has verified and
        // retained its header/root data. Ordinary nodes earn progress later,
        // only when the engine reports a useful SHAMap addition.
        if seeded {
            session_state.plan.note_progress();
        }
        self.run_plan_turn(session, Some(packet_peer), &mut effects);
        effects
    }

    fn on_timer(&mut self, operation: OperationRef, timer: TimerKind) -> Vec<AcquisitionEffect> {
        let session = operation.session();
        let Some(session_state) = self.state.sessions.get_mut(&session) else {
            self.stats.stale_events += 1;
            return Vec::new();
        };
        if session_state.phase.is_terminal() {
            self.stats.stale_events += 1;
            return Vec::new();
        }
        let Some((expected_kind, expected)) = session_state.pending_timer else {
            self.stats.stale_events += 1;
            return Vec::new();
        };
        if expected_kind != timer || !expected.is_expected_for(&operation) {
            // A rearmed, unknown, or otherwise unmatched timer is stale.
            self.stats.stale_events += 1;
            return Vec::new();
        }
        // Exact identity matched: consume only the timer currently armed for
        // this session. Handoff retry is intentionally distinct from the plan
        // deadline and may re-publish only while the exact handoff remains
        // pending.
        session_state.pending_timer = None;
        // `InboundLedger::onTimer` clears `recentNodes_` for every exact
        // acquisition deadline, including a progress interval. This lets the
        // next trigger re-request a still-retained frontier while the exact
        // SessionRef/OperationRef gate still rejects stale events.
        if timer == TimerKind::AcquireTimeout {
            session_state.recent_node_hashes.clear();
        }
        if timer == TimerKind::HandoffRetry {
            if session_state.phase != SessionPhase::DurablePending
                || session_state.pending_handoff.is_none()
            {
                self.stats.stale_events += 1;
                return Vec::new();
            }
            let Some(ledger) = session_state.durable.clone() else {
                self.stats.stale_events += 1;
                return Vec::new();
            };
            let handoff = session_state
                .pending_handoff
                .expect("checked pending handoff");
            return vec![AcquisitionEffect::PublishDurable(DurableLedger::new(
                handoff, session, ledger,
            ))];
        }
        if timer != TimerKind::AcquireTimeout {
            self.stats.stale_events += 1;
            return Vec::new();
        }
        let effects = Vec::new();
        let Some(session_state) = self.state.sessions.get_mut(&session) else {
            self.stats.stale_events += 1;
            return effects;
        };
        let timeout_before = session_state.plan.timeouts();
        let no_progress_interval = session_state.plan.take_no_progress_interval();
        let seeded_before_timeout = session_state.plan.engine().is_some();
        let pending_network_before_timeout = session_state.plan.pending_network().len();
        let pending_reads_before_timeout = session_state.plan.pending_read_count();
        let read_backlog_before_timeout = session_state.plan.read_backlog_count();
        let timeout = session_state.plan.on_timeout(no_progress_interval);
        let timeout_after = session_state.plan.timeouts();
        match timeout {
            PlanTimeout::Continue => {
                tracing::info!(
                    target: "acquisition_trace",
                    event = "acquisition_timeout_interval",
                    run_epoch = session.run_epoch().get(),
                    session_id = session.session_id().get(),
                    target_hash = %session.target_hash(),
                    plan_epoch = session.plan_epoch().get(),
                    store_generation = session.store_generation().get(),
                    timeout_budget_before = timeout_before,
                    timeout_budget_after = timeout_after,
                    no_progress_interval,
                    recovery_dispatched = no_progress_interval,
                    seeded_before_timeout,
                    pending_network_before_timeout,
                    pending_reads_before_timeout,
                    read_backlog_before_timeout,
                    "acquisition trace: exact session deadline rearmed after one progress or no-progress interval"
                );
                // rippled `InboundLedger::onTimer` only rebuilds demand on a
                // no-progress interval (`rippled/src/xrpld/app/ledger/detail/
                // InboundLedger.cpp`, `onTimer`/`trigger`/`addPeers`). It
                // rechecks local storage, retargets
                // the retained frontier, and expands peer membership without
                // rebuilding the active traversal. The coordinator does the
                // same through brokered reprobes plus fresh exact effects.
                let mut effects = effects;
                if no_progress_interval {
                    self.escalate_timeout_peers(session);
                    if seeded_before_timeout {
                        let nodes = self.next_timeout_frontier_batch(session);
                        self.submit_timeout_reprobes(session, &nodes, &mut effects);
                        self.send_timeout_frontier_requests(
                            session,
                            &nodes,
                            timeout_after,
                            &mut effects,
                        );
                    } else {
                        self.send_base_request(session, &mut effects);
                    }
                }

                // Rearm the deadline with a fresh operation identity and let the
                // plan run again (queued packets may now be feedable).
                let timer_operation = OperationRef::new(
                    session,
                    OperationKind::Timer,
                    self.state.ids.next_id(),
                    self.state.ids.next_id(),
                );
                if let Some(session_state) = self.state.sessions.get_mut(&session) {
                    session_state.pending_timer =
                        Some((TimerKind::AcquireTimeout, timer_operation));
                }
                self.stats.timers_armed += 1;
                effects.push(AcquisitionEffect::ArmTimer(TimerRequest::new(
                    timer_operation,
                    TimerKind::AcquireTimeout,
                    self.state.budgets.acquire_timeout,
                )));
                self.run_plan_turn(session, None, &mut effects);
                effects
            }
            PlanTimeout::Fail => {
                tracing::info!(
                    target: "acquisition_trace",
                    event = "acquisition_timeout_exhausted",
                    run_epoch = session.run_epoch().get(),
                    session_id = session.session_id().get(),
                    target_hash = %session.target_hash(),
                    plan_epoch = session.plan_epoch().get(),
                    store_generation = session.store_generation().get(),
                    timeout_budget_before = timeout_before,
                    timeout_budget_after = timeout_after,
                    no_progress_interval,
                    seeded_before_timeout,
                    pending_network_before_timeout,
                    pending_reads_before_timeout,
                    read_backlog_before_timeout,
                    "acquisition trace: exact session exhausted its no-progress timeout budget on the seventh interval"
                );
                let mut effects = effects;
                self.fail_session(session, FailureReason::AcquisitionTimeout, &mut effects);
                effects
            }
        }
    }

    fn on_read(&mut self, completion: ReadCompletion) -> Vec<AcquisitionEffect> {
        if !matches!(
            completion.operation().kind(),
            OperationKind::Read | OperationKind::RecoveryRead
        ) {
            self.stats.stale_events += 1;
            return Vec::new();
        }
        let session = completion.operation().session();
        let (outcome, pending_reads_after, read_backlog_after) = {
            let Some(session_state) = self.state.sessions.get_mut(&session) else {
                self.stats.stale_events += 1;
                return Vec::new();
            };
            if session_state.phase.is_terminal() {
                self.stats.stale_events += 1;
                return Vec::new();
            }
            let outcome = session_state.plan.on_read(&completion);
            (
                outcome,
                session_state.plan.pending_read_count(),
                session_state.plan.read_backlog_count(),
            )
        };
        tracing::info!(
            target: "acquisition_trace",
            event = "node_store_read_completed",
            run_epoch = session.run_epoch().get(),
            session_id = session.session_id().get(),
            target_hash = %session.target_hash(),
            plan_epoch = session.plan_epoch().get(),
            store_generation = session.store_generation().get(),
            outcome = ?completion.outcome(),
            plan_outcome = ?outcome,
            pending_reads_after,
            read_backlog_after,
            "acquisition trace: brokered NodeStore read completion returned to session"
        );
        match outcome {
            PlanReadOutcome::Applied => {
                let mut effects = Vec::new();
                self.run_plan_turn(session, None, &mut effects);
                effects
            }
            PlanReadOutcome::Stale => {
                self.stats.stale_events += 1;
                Vec::new()
            }
        }
    }

    fn on_write(&mut self, completion: WriteCompletion) -> Vec<AcquisitionEffect> {
        if completion.operation().kind() != OperationKind::Write {
            self.stats.stale_events += 1;
            return Vec::new();
        }
        let session = completion.operation().session();
        let (outcome, persistence_after) = {
            let Some(session_state) = self.state.sessions.get_mut(&session) else {
                self.stats.stale_events += 1;
                return Vec::new();
            };
            if session_state.phase.is_terminal() {
                self.stats.stale_events += 1;
                return Vec::new();
            }
            let outcome = session_state
                .plan
                .on_write(completion.operation(), completion.outcome());
            (outcome, session_state.plan.persistence().label())
        };
        tracing::info!(
            target: "acquisition_trace",
            event = "node_store_write_completed",
            run_epoch = session.run_epoch().get(),
            session_id = session.session_id().get(),
            target_hash = %session.target_hash(),
            plan_epoch = session.plan_epoch().get(),
            store_generation = session.store_generation().get(),
            outcome = ?completion.outcome(),
            plan_outcome = ?outcome,
            persistence_after,
            "acquisition trace: NodeStore write completion returned to session"
        );
        let mut effects = Vec::new();
        match outcome {
            PlanWriteOutcome::IncrementalAccepted => {
                self.run_plan_turn(session, None, &mut effects)
            }
            PlanWriteOutcome::FinalAccepted => {}
            PlanWriteOutcome::Failed(reason) => self.fail_session(session, reason, &mut effects),
            PlanWriteOutcome::Stale => self.stats.stale_events += 1,
        }
        effects
    }

    fn on_durability(&mut self, completion: DurabilityCompletion) -> Vec<AcquisitionEffect> {
        if completion.operation().kind() != OperationKind::DurabilityFence {
            self.stats.stale_events += 1;
            return Vec::new();
        }
        let session = completion.operation().session();
        let outcome = {
            let Some(session_state) = self.state.sessions.get_mut(&session) else {
                self.stats.stale_events += 1;
                return Vec::new();
            };
            if session_state.phase.is_terminal() {
                self.stats.stale_events += 1;
                return Vec::new();
            }
            session_state
                .plan
                .on_durability(completion.operation(), completion.outcome())
        };
        let mut effects = Vec::new();
        match outcome {
            PlanDurabilityOutcome::Durable => {
                // Persisting -> DurablePending: the ledger is durable and is
                // handed off exactly once. It is never normal-adoptable now.
                let durable = SessionPhase::DurablePending;
                let Some(session_state) = self.state.sessions.get_mut(&session) else {
                    self.stats.stale_events += 1;
                    return effects;
                };
                if !session_phase_transition(&session_state.phase, &durable) {
                    self.stats.stale_events += 1;
                    return effects;
                }
                session_state.phase = durable;
                let Some(ledger) = session_state.plan.durable_ledger() else {
                    // A passed fence without a materialized ledger cannot hand
                    // off; terminalize so no durable result is lost.
                    self.fail_session(session, FailureReason::DurabilityFenceFailed, &mut effects);
                    return effects;
                };
                let handoff = self.state.ids.next_id::<DurableHandoffId>();
                tracing::info!(
                    target: "acquisition_trace",
                    event = "durability_fence_passed",
                    run_epoch = session.run_epoch().get(),
                    session_id = session.session_id().get(),
                    target_hash = %session.target_hash(),
                    plan_epoch = session.plan_epoch().get(),
                    store_generation = session.store_generation().get(),
                    handoff = handoff.get(),
                    ledger_hash = %ledger.header().hash,
                    ledger_seq = ledger.header().seq,
                    "acquisition trace: final durability fence passed; publishing exact durable handoff"
                );
                session_state.pending_handoff = Some(handoff);
                // The acquisition deadline is no longer meaningful once the
                // fence passed; a later rejected handoff arms its own exact
                // retry timer instead.
                session_state.pending_timer = None;
                session_state.durable = Some(Arc::clone(&ledger));
                effects.push(AcquisitionEffect::PublishDurable(DurableLedger::new(
                    handoff, session, ledger,
                )));
            }
            PlanDurabilityOutcome::Failed(reason) => {
                self.fail_session(session, reason, &mut effects)
            }
            PlanDurabilityOutcome::Stale => self.stats.stale_events += 1,
        }
        effects
    }

    fn on_handoff_ack(&mut self, ack: DurableHandoffAcknowledgement) -> Vec<AcquisitionEffect> {
        let session = ack.session();
        let Some(session_state) = self.state.sessions.get_mut(&session) else {
            self.stats.stale_events += 1;
            return Vec::new();
        };
        if session_state.phase.is_terminal() {
            self.stats.stale_events += 1;
            return Vec::new();
        }
        if session_state.pending_handoff != Some(ack.handoff()) {
            // A stale or unknown handoff id never finalizes a session: an
            // earlier delivery may have been retried, but only the exact
            // pending id authorizes `DurablePending -> Complete`.
            self.stats.stale_events += 1;
            return Vec::new();
        }
        let complete = SessionPhase::Complete;
        if session_phase_transition(&session_state.phase, &complete) {
            tracing::info!(
                target: "acquisition_trace",
                event = "durable_handoff_acknowledged",
                run_epoch = session.run_epoch().get(),
                session_id = session.session_id().get(),
                target_hash = %session.target_hash(),
                plan_epoch = session.plan_epoch().get(),
                store_generation = session.store_generation().get(),
                handoff = ack.handoff().get(),
                "acquisition trace: durable handoff recipient completed persistence, registration, and acceptance dispatch"
            );
            session_state.phase = complete;
            session_state.pending_handoff = None;
            session_state.pending_timer = None;
            session_state.durable = None;
            self.stats.sessions_completed += 1;
        }
        Vec::new()
    }

    /// Schedules one exact retry for a rejected durable handoff. Duplicate
    /// rejection events are stale while that retry is armed; the retry timer,
    /// not the rejection callback, performs the re-publish.
    fn on_handoff_rejected(
        &mut self,
        handoff: DurableHandoffId,
        session: SessionRef,
    ) -> Vec<AcquisitionEffect> {
        let valid = self
            .state
            .sessions
            .get(&session)
            .is_some_and(|session_state| {
                session_state.phase == SessionPhase::DurablePending
                    && session_state.pending_handoff == Some(handoff)
                    && session_state.pending_timer.is_none()
            });
        if !valid {
            self.stats.stale_events += 1;
            return Vec::new();
        }
        let operation = OperationRef::new(
            session,
            OperationKind::Timer,
            self.state.ids.next_id(),
            self.state.ids.next_id(),
        );
        let session_state = self
            .state
            .sessions
            .get_mut(&session)
            .expect("validated live handoff session");
        session_state.pending_timer = Some((TimerKind::HandoffRetry, operation));
        self.stats.handoff_rejections += 1;
        self.stats.timers_armed += 1;
        vec![AcquisitionEffect::ArmTimer(TimerRequest::new(
            operation,
            TimerKind::HandoffRetry,
            HANDOFF_RETRY_DELAY,
        ))]
    }

    fn on_lcl_installed(&mut self, identity: LedgerIdentity) -> Vec<AcquisitionEffect> {
        // NetworkOps republishes the current LCL during routine maintenance.
        // It is a meaningful causal fact only when the identity changes or it
        // changes coordinator state; logging each unchanged, nonmatching fact
        // at INFO would overwhelm the target-acquisition trace.
        let lcl_changed = self.state.last_installed_lcl != Some(identity);
        let phase_before = lcl_changed.then(|| format!("{:?}", self.state.phase));
        // This fact is serialized by NetworkOps and is the exact local-LCL
        // counterpart that authorizes a later in-place Full refresh.
        self.state.last_installed_lcl = Some(identity);
        // The LCL-install fact legalizes `Syncing -> Tracking` for the acquired
        // target and `Connected -> Tracking` for a locally resident preferred
        // LCL installed without acquisition (rippled switchLastClosedLedger
        // clearing needNetworkLedger). While Tracking, normal in-place
        // consensus keeps installing newer LCLs; refresh that identity so the
        // corresponding publication can satisfy `Tracking -> Full`.
        let mut effects = Vec::new();
        match self.state.phase {
            SyncPhase::Syncing { target } if target.hash() == identity.hash() => {
                let fact = TransitionFact::TargetInstalledAsLcl { lcl: identity };
                if let Ok(next) = self.state.phase.apply(fact) {
                    self.state.phase = next;
                    effects.push(AcquisitionEffect::SetServicePhase(next));
                }
            }
            SyncPhase::Connected => {
                let fact = TransitionFact::TargetInstalledAsLcl { lcl: identity };
                if let Ok(next) = self.state.phase.apply(fact) {
                    self.state.phase = next;
                    effects.push(AcquisitionEffect::SetServicePhase(next));
                }
            }
            SyncPhase::Tracking { lcl } if identity.sequence() > lcl.sequence() => {
                let next = SyncPhase::Tracking { lcl: identity };
                self.state.phase = next;
                effects.push(AcquisitionEffect::SetServicePhase(next));
            }
            // Full stays Full across normal local LCL advancement. The paired
            // fresh publication below refreshes both identities atomically;
            // emitting Tracking here would create a misleading per-ledger
            // Full -> Tracking -> Full churn. This matches rippled
            // NetworkOPsImp::endConsensus, which only promotes
            // CONNECTED/SYNCING to TRACKING and CONNECTED/TRACKING to FULL
            // after a non-abnormal checkLastClosedLedger pass
            // (../rippled/src/xrpld/app/misc/NetworkOPs.cpp).
            SyncPhase::Full { .. } => {}
            _ => {}
        }
        // A local LCL installation satisfies any pre-fence session for the
        // exact same target. Cancel it now so a stale timer cannot later turn
        // a locally successful consensus round into an acquisition timeout.
        // `cancel_session` rejects DurablePending, preserving the unique
        // durability handoff protocol for coordinator-owned completions.
        for session in self.live_sessions_for_hash(identity.hash()) {
            self.cancel_session(session, CancelReason::LclInstalled, &mut effects);
        }
        if lcl_changed || !effects.is_empty() {
            tracing::info!(
                target: "acquisition_trace",
                event = "lcl_installed_fact_applied",
                lcl_hash = %identity.hash(),
                lcl_seq = identity.sequence(),
                lcl_changed,
                phase_before = ?phase_before,
                phase_after = ?self.state.phase,
                emitted_effects = effects.len(),
                "acquisition trace: LCL installation fact changed coordinator state"
            );
        }
        effects
    }

    /// A publication advance. `Tracking -> Full` requires a fresh publication
    /// anchor whose contiguity to the tracked LCL has already been proven by
    /// the NetworkOps adapter. The anchor may be an earlier ledger: rippled
    /// `NetworkOPsImp::endConsensus` checks open-ledger freshness, not equality
    /// between the publication head and the newly installed LCL. While already
    /// `Full`, a newer matching fresh publication refreshes the LCL/published
    /// identities in place: normal local consensus must not emit a redundant
    /// phase cycle. A non-fresh publication is a no-op in either case.
    fn on_publication(&mut self, identity: LedgerIdentity, fresh: bool) -> Vec<AcquisitionEffect> {
        match self.state.phase {
            SyncPhase::Full { lcl, published }
                if fresh
                    && self.state.last_installed_lcl == Some(identity)
                    && identity.sequence() > lcl.sequence()
                    && identity.sequence() > published.sequence() =>
            {
                // This publication proves the next locally installed LCL is
                // contiguous and fresh. Refresh Full's exact identities
                // without routing through a visible intermediate mode. The
                // exact LCL equality is the Rust-owned equivalent of rippled's
                // endConsensus operating on its current closed ledger.
                self.state.phase = SyncPhase::Full {
                    lcl: identity,
                    published: identity,
                };
                Vec::new()
            }
            // NetworkOps only emits this fact after proving that `identity`
            // is the tracked LCL or a contiguous published ancestor. The
            // sequence guard makes a newer or unrelated publication harmless
            // even if an adapter regresses.
            SyncPhase::Tracking { lcl } if fresh && identity.sequence() <= lcl.sequence() => {
                let fact = TransitionFact::ChainContiguous {
                    lcl,
                    published: identity,
                };
                if let Ok(next) = self.state.phase.apply(fact) {
                    self.state.phase = next;
                    return vec![AcquisitionEffect::SetServicePhase(next)];
                }
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn on_store_rotated(&mut self, generation: StoreGeneration) -> Vec<AcquisitionEffect> {
        if generation == self.state.storage_generation {
            return Vec::new();
        }
        self.state.storage_generation = generation;
        let mut effects = Vec::new();
        let stale: Vec<SessionRef> = self
            .state
            .sessions
            .keys()
            .filter(|session| session.store_generation() != generation)
            .copied()
            .collect();
        for session in stale {
            self.cancel_session(session, CancelReason::StoreRotated, &mut effects);
        }
        effects
    }

    /// A fetch-pack pass added by-hash node data to the shared fetch-pack
    /// cache. The fact names no session: re-advance every live session so its
    /// traversal's resident lookup can resolve the newly resident by-hash nodes
    /// without waiting for a peer reply (rippled `gotFetchPack` parity). Each
    /// re-advance is bounded by the same plan-turn limits as any other event;
    /// sessions without a rooted engine or already terminal are not touched.
    fn on_fetch_pack(&mut self) -> Vec<AcquisitionEffect> {
        let live: Vec<SessionRef> = self
            .state
            .sessions
            .iter()
            .filter(|(_, state)| !state.phase.is_terminal())
            .map(|(session, _)| *session)
            .collect();
        let mut effects = Vec::new();
        for session in live {
            let has_engine =
                self.state.sessions.get(&session).is_some_and(|state| {
                    !state.phase.is_terminal() && state.plan.engine().is_some()
                });
            if !has_engine {
                continue;
            }
            self.stats.fetch_pack_advances += 1;
            self.run_plan_turn(session, None, &mut effects);
        }
        effects
    }

    /// A periodic heartbeat. Mirrors rippled's `processHeartbeatTimer` mode
    /// reassertion: re-publishes the current phase so the phase port re-runs
    /// the validated-ledger-age / blocked normalization on `Connected` and
    /// `Syncing`. Recovery from `Disconnected` is owned by the connectivity
    /// fact, and `Tracking`/`Full` are never reasserted, matching
    /// `heartbeat_operating_mode_reassertion`.
    fn on_heartbeat(&mut self) -> Vec<AcquisitionEffect> {
        let mut effects = Vec::new();
        match self.state.phase {
            SyncPhase::Connected | SyncPhase::Syncing { .. } => {
                effects.push(AcquisitionEffect::SetServicePhase(self.state.phase));
            }
            _ => {}
        }
        effects
    }

    /// Runs one bounded plan turn for `session` and appends its work effects.
    /// The tree engine is advanced only on this owner task; work commands are
    /// returned after state mutation and executed by adapters.
    fn run_plan_turn(
        &mut self,
        session: SessionRef,
        reply_peer: Option<PeerId>,
        effects: &mut Vec<AcquisitionEffect>,
    ) {
        self.stats.plan_turns += 1;
        let turn = {
            let CoordinatorState { sessions, ids, .. } = &mut self.state;
            let Some(session_state) = sessions.get_mut(&session) else {
                return;
            };
            if session_state.phase.is_terminal() {
                return;
            }
            let mut ctx = TurnContext {
                session,
                store_generation: session.store_generation(),
                priority: Self::read_priority(session_state.reason),
                ids,
            };
            session_state.plan.run_turn(&mut ctx)
        };
        match turn {
            PlanTurn::Continue => {}
            PlanTurn::Reads(requests) => {
                tracing::info!(
                    target: "acquisition_trace",
                    event = "node_store_reads_submitted",
                    run_epoch = session.run_epoch().get(),
                    session_id = session.session_id().get(),
                    target_hash = %session.target_hash(),
                    plan_epoch = session.plan_epoch().get(),
                    store_generation = session.store_generation().get(),
                    reads = requests.len(),
                    "acquisition trace: session requested brokered NodeStore reads before peer frontier work"
                );
                for request in requests {
                    effects.push(AcquisitionEffect::SubmitRead(request));
                }
            }
            PlanTurn::Network(nodes) => {
                // rippled `InboundLedger::filterNodes` records requested hashes
                // and suppresses an all-duplicate set on a reply trigger. This
                // matters after the initial five-peer header fanout: each Base
                // reply can otherwise discover the same frontier and multiply
                // requests before the first node reply attaches it. Timeout
                // work intentionally remains eligible to retry known hashes.
                let nodes = if reply_peer.is_some() {
                    let Some(session_state) = self.state.sessions.get_mut(&session) else {
                        return;
                    };
                    nodes
                        .into_iter()
                        .filter(|node| session_state.recent_node_hashes.insert(node.hash()))
                        .collect::<Vec<_>>()
                } else {
                    if let Some(session_state) = self.state.sessions.get_mut(&session) {
                        session_state
                            .recent_node_hashes
                            .extend(nodes.iter().map(|node| node.hash()));
                    }
                    nodes
                };
                if nodes.is_empty() {
                    return;
                }
                let sequence = self
                    .state
                    .sessions
                    .get(&session)
                    .and_then(|state| state.plan.ledger_sequence());
                // Rippled's normal trigger pins reply-driven follow-up work
                // to the responding peer. A local/read/fetch-pack wake has no
                // responding peer, so its `PeerSet::sendRequest(..., nullptr)`
                // equivalent broadcasts only to the session's acquired peers.
                let peers = match reply_peer {
                    Some(peer) => vec![peer],
                    None => self
                        .state
                        .sessions
                        .get(&session)
                        .map(|state| state.sent_peers.iter().copied().collect::<Vec<_>>())
                        .unwrap_or_default(),
                };
                // Rippled's normal trigger requests the missing SHAMap
                // locations through TMGetLedger. Generic by-hash requests are
                // its aggressive timeout fallback, not the default path.
                let Some(sequence) = sequence else {
                    return;
                };
                let state_node_count = nodes
                    .iter()
                    .filter(|node| node.kind() == ledger::TreeKind::State)
                    .count();
                let transaction_node_count = nodes.len().saturating_sub(state_node_count);
                tracing::info!(
                    target: "acquisition_trace",
                    event = "network_frontier_requested",
                    run_epoch = session.run_epoch().get(),
                    session_id = session.session_id().get(),
                    target_hash = %session.target_hash(),
                    plan_epoch = session.plan_epoch().get(),
                    store_generation = session.store_generation().get(),
                    ledger_sequence = sequence,
                    reply_peer = ?reply_peer.map(PeerId::get),
                    peer_count = peers.len(),
                    nodes = nodes.len(),
                    state_nodes = state_node_count,
                    transaction_nodes = transaction_node_count,
                    "acquisition trace: requesting current SHAMap frontier from selected peers"
                );
                let request_batch = if reply_peer.is_some() {
                    REPLY_NODE_REQUEST_BATCH
                } else {
                    BLIND_NODE_REQUEST_BATCH
                };
                // rippled `InboundLedger::filterNodes` limits a response
                // trigger to `kReqNodesReply` (128) and blind/local work to
                // `kReqNodes` (12); chunk effects without truncating the exact
                // retained frontier.
                for kind in [ledger::TreeKind::State, ledger::TreeKind::Transaction] {
                    let node_ids = nodes
                        .iter()
                        .filter(|node| node.kind() == kind)
                        .map(|node| node.node_id())
                        .collect::<Vec<_>>();
                    for node_ids in node_ids.chunks(request_batch) {
                        for peer in &peers {
                            let operation = OperationRef::new(
                                session,
                                OperationKind::PeerRequest,
                                self.state.ids.next_id(),
                                self.state.ids.next_id(),
                            );
                            // A reply-driven request is pinned to the responding
                            // peer, but it must not turn an unbounded history of
                            // responders into timeout resend recipients.
                            self.stats.peer_requests += 1;
                            effects.push(AcquisitionEffect::SendLedgerRequest(PeerRequest::new(
                                session,
                                operation,
                                *peer,
                                LedgerDataRequest::GetLedgerNodes {
                                    kind,
                                    node_ids: node_ids.to_vec(),
                                    sequence,
                                },
                            )));
                        }
                    }
                }
            }
            PlanTurn::Persist(batch) => {
                tracing::info!(
                    target: "acquisition_trace",
                    event = "persistence_batch_submitted",
                    run_epoch = session.run_epoch().get(),
                    session_id = session.session_id().get(),
                    target_hash = %session.target_hash(),
                    plan_epoch = session.plan_epoch().get(),
                    store_generation = session.store_generation().get(),
                    ledger_sequence = batch.ledger_sequence(),
                    nodes = batch.nodes().len(),
                    payload_bytes = batch.payload_bytes(),
                    final_batch = batch.requires_fence(),
                    "acquisition trace: accepted SHAMap nodes submitted for persistence"
                );
                if batch.requires_fence() {
                    // Active -> Persisting: the tree is structurally complete
                    // and the final batch must pass its durability fence before
                    // any handoff. Incremental accepted-node batches leave the
                    // session active and carry no fence.
                    let persisting = SessionPhase::Persisting;
                    if let Some(session_state) = self.state.sessions.get_mut(&session)
                        && session_phase_transition(&session_state.phase, &persisting)
                    {
                        session_state.phase = persisting;
                    }
                }
                effects.push(AcquisitionEffect::SubmitWrite(batch));
            }
            PlanTurn::Invalid => {
                self.fail_session(session, FailureReason::InvalidTreePlan, effects);
            }
        }
    }

    /// Terminalizes a live session with a failure reason: the session phase
    /// becomes `Failed`, its plan is cancelled, and a cancellation effect is
    /// emitted so adapters release their resource-local state.
    fn fail_session(
        &mut self,
        session: SessionRef,
        reason: FailureReason,
        effects: &mut Vec<AcquisitionEffect>,
    ) {
        let Some(session_state) = self.state.sessions.get_mut(&session) else {
            return;
        };
        if session_state.phase.is_terminal() {
            return;
        }
        let failed = SessionPhase::Failed { reason };
        if session_phase_transition(&session_state.phase, &failed) {
            tracing::info!(
                target: "acquisition_trace",
                event = "session_failed",
                run_epoch = session.run_epoch().get(),
                session_id = session.session_id().get(),
                target_hash = %session.target_hash(),
                plan_epoch = session.plan_epoch().get(),
                store_generation = session.store_generation().get(),
                ?reason,
                phase_before = ?session_state.phase,
                plan_runs = session_state.plan.runs(),
                timeout_count = session_state.plan.timeouts(),
                pending_network = session_state.plan.pending_network().len(),
                pending_reads = session_state.plan.pending_read_count(),
                read_backlog = session_state.plan.read_backlog_count(),
                persistence = session_state.plan.persistence().label(),
                "acquisition trace: session terminalized with failure"
            );
            session_state.phase = failed;
            session_state.pending_timer = None;
            session_state.plan.cancel();
            self.stats.sessions_cancelled += 1;
            *self.stats.failed_by_reason.entry(reason).or_insert(0) += 1;
            effects.push(AcquisitionEffect::CancelSession(session));
        }
    }

    /// Adds at most `TIMEOUT_PEER_ESCALATION` currently available, previously
    /// unselected peers without exceeding the per-session selected-peer
    /// window. The set is the coordinator's replacement for rippled's PeerSet
    /// membership; it never owns transport or retains every responder.
    fn escalate_timeout_peers(&mut self, session: SessionRef) {
        let additions = self
            .state
            .sessions
            .get(&session)
            .filter(|state| !state.phase.is_terminal())
            .map(|state| {
                let remaining = MAX_SELECTED_PEERS.saturating_sub(state.sent_peers.len());
                self.state
                    .peer_view
                    .peers()
                    .iter()
                    .copied()
                    .filter(|peer| !state.sent_peers.contains(peer))
                    .take(TIMEOUT_PEER_ESCALATION.min(remaining))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if let Some(state) = self.state.sessions.get_mut(&session) {
            state.sent_peers.extend(additions);
        }
    }

    /// Selects one rotating exact frontier batch for one no-progress timeout.
    /// The resulting records are reused for both local reprobes and peer
    /// resends, so neither port can repeatedly select only the first batch.
    fn next_timeout_frontier_batch(&mut self, session: SessionRef) -> Vec<PlanNetworkNeed> {
        self.state
            .sessions
            .get_mut(&session)
            .filter(|state| !state.phase.is_terminal())
            .map(|state| state.plan.next_timeout_recovery_batch())
            .unwrap_or_default()
    }

    /// Reprobe one exact timeout frontier batch through the asynchronous
    /// NodeStore port. No plan or coordinator lock performs physical storage
    /// I/O.
    fn submit_timeout_reprobes(
        &mut self,
        session: SessionRef,
        nodes: &[PlanNetworkNeed],
        effects: &mut Vec<AcquisitionEffect>,
    ) {
        let requests = {
            let CoordinatorState { sessions, ids, .. } = &mut self.state;
            let Some(state) = sessions.get_mut(&session) else {
                return;
            };
            if state.phase.is_terminal() {
                return;
            }
            let mut ctx = TurnContext {
                session,
                store_generation: session.store_generation(),
                priority: Self::read_priority(state.reason),
                ids,
            };
            state.plan.reprobe_network_batch(nodes, &mut ctx)
        };
        for request in requests {
            effects.push(AcquisitionEffect::SubmitRead(request));
        }
    }

    /// Resends the retained exact frontier with fresh operation identities.
    /// Normal retries preserve node ids and tree kind via `GetLedgerNodes`;
    /// only the rippled `> 4` timeout threshold switches to bounded by-hash
    /// requests. The session/plan/store identity is unchanged throughout.
    fn send_timeout_frontier_requests(
        &mut self,
        session: SessionRef,
        nodes: &[PlanNetworkNeed],
        timeout_count: u32,
        effects: &mut Vec<AcquisitionEffect>,
    ) {
        debug_assert!(nodes.len() <= TIMEOUT_FRONTIER_REQUEST_LIMIT);
        let Some((sequence, peers)) = self.state.sessions.get(&session).and_then(|state| {
            (!state.phase.is_terminal()).then(|| {
                (
                    state.plan.ledger_sequence(),
                    state.sent_peers.iter().copied().collect::<Vec<_>>(),
                )
            })
        }) else {
            return;
        };
        let Some(sequence) = sequence else {
            return;
        };
        if nodes.is_empty() || peers.is_empty() {
            return;
        }
        let aggressive = timeout_count > AGGRESSIVE_TIMEOUT_THRESHOLD;
        if aggressive {
            for kind in [ledger::TreeKind::State, ledger::TreeKind::Transaction] {
                let nodes = nodes
                    .iter()
                    .filter(|node| node.kind() == kind)
                    .map(|node| LedgerNodeRequest::new(node.hash(), kind))
                    .collect::<Vec<_>>();
                if nodes.is_empty() {
                    continue;
                }
                for peer in &peers {
                    let operation = OperationRef::new(
                        session,
                        OperationKind::PeerRequest,
                        self.state.ids.next_id(),
                        self.state.ids.next_id(),
                    );
                    self.stats.peer_requests += 1;
                    effects.push(AcquisitionEffect::SendLedgerRequest(PeerRequest::new(
                        session,
                        operation,
                        *peer,
                        LedgerDataRequest::GetNodes {
                            nodes: nodes.clone(),
                            sequence: Some(sequence),
                        },
                    )));
                }
            }
            return;
        }
        for kind in [ledger::TreeKind::State, ledger::TreeKind::Transaction] {
            let node_ids = nodes
                .iter()
                .filter(|node| node.kind() == kind)
                .map(|node| node.node_id())
                .collect::<Vec<_>>();
            if node_ids.is_empty() {
                continue;
            }
            for peer in &peers {
                let operation = OperationRef::new(
                    session,
                    OperationKind::PeerRequest,
                    self.state.ids.next_id(),
                    self.state.ids.next_id(),
                );
                self.stats.peer_requests += 1;
                effects.push(AcquisitionEffect::SendLedgerRequest(PeerRequest::new(
                    session,
                    operation,
                    *peer,
                    LedgerDataRequest::GetLedgerNodes {
                        kind,
                        node_ids: node_ids.clone(),
                        sequence,
                    },
                )));
            }
        }
    }

    /// Reissues an unseeded Base/header request on an exact live session.
    ///
    /// The target and session identity never change across retries. Prefer a
    /// peer that has not received a request from this session yet, then cycle
    /// over the current availability snapshot. This models rippled's timeout
    /// trigger plus peer-set expansion without restoring the legacy peer-set
    /// lifecycle as a second owner.
    fn send_base_request(&mut self, session: SessionRef, effects: &mut Vec<AcquisitionEffect>) {
        let Some((target, sent_peers)) = self.state.sessions.get(&session).and_then(|state| {
            (!state.phase.is_terminal() && state.plan.engine().is_none())
                .then(|| (state.target, state.sent_peers.clone()))
        }) else {
            return;
        };
        for peer in sent_peers {
            let operation = OperationRef::new(
                session,
                OperationKind::PeerRequest,
                self.state.ids.next_id(),
                self.state.ids.next_id(),
            );
            // rippled `PeerSet::sendRequest(..., nullptr)` reaches every
            // current selected peer. A timeout retry must not privilege
            // BTreeSet's first id and strand other usable peers.
            self.stats.peer_requests += 1;
            effects.push(AcquisitionEffect::SendLedgerRequest(PeerRequest::new(
                session,
                operation,
                peer,
                LedgerDataRequest::GetLedger {
                    sequence: target.sequence(),
                },
            )));
        }
    }

    /// Derives the NodeStore read admission priority from the acquisition
    /// reason: consensus/validation/recovery demand preempts history fill.
    const fn read_priority(reason: AcquireReason) -> ReadPriority {
        match reason {
            AcquireReason::Consensus => ReadPriority::Consensus,
            AcquireReason::Generic | AcquireReason::History => ReadPriority::History,
        }
    }

    fn live_sessions_for_hash(&self, hash: Uint256) -> Vec<SessionRef> {
        self.state
            .sessions
            .iter()
            .filter(|(session, state)| session.target_hash() == hash && !state.phase.is_terminal())
            .map(|(session, _)| *session)
            .collect()
    }

    fn cancel_non_durable_for_peer_loss(&mut self, effects: &mut Vec<AcquisitionEffect>) {
        let live: Vec<SessionRef> = self
            .state
            .sessions
            .iter()
            .filter(|(_, state)| {
                !state.phase.is_terminal() && state.phase != SessionPhase::DurablePending
            })
            .map(|(session, _)| *session)
            .collect();
        for session in live {
            self.cancel_session(session, CancelReason::PeerLoss, effects);
        }
    }

    fn cancel_all(&mut self, reason: CancelReason, effects: &mut Vec<AcquisitionEffect>) {
        let live: Vec<SessionRef> = self
            .state
            .sessions
            .iter()
            .filter(|(_, state)| !state.phase.is_terminal())
            .map(|(session, _)| *session)
            .collect();
        for session in live {
            self.cancel_session(session, reason, effects);
        }
    }

    fn cancel_session(
        &mut self,
        session: SessionRef,
        reason: CancelReason,
        effects: &mut Vec<AcquisitionEffect>,
    ) {
        let Some(session_state) = self.state.sessions.get_mut(&session) else {
            return;
        };
        if session_state.phase.is_terminal() {
            return;
        }
        let cancelled = SessionPhase::Cancelled { reason };
        if session_phase_transition(&session_state.phase, &cancelled) {
            tracing::info!(
                target: "acquisition_trace",
                event = "session_cancelled",
                run_epoch = session.run_epoch().get(),
                session_id = session.session_id().get(),
                target_hash = %session.target_hash(),
                plan_epoch = session.plan_epoch().get(),
                store_generation = session.store_generation().get(),
                ?reason,
                phase_before = ?session_state.phase,
                plan_runs = session_state.plan.runs(),
                timeout_count = session_state.plan.timeouts(),
                pending_network = session_state.plan.pending_network().len(),
                pending_reads = session_state.plan.pending_read_count(),
                read_backlog = session_state.plan.read_backlog_count(),
                persistence = session_state.plan.persistence().label(),
                "acquisition trace: session cancelled before durable completion"
            );
            session_state.phase = cancelled;
            session_state.pending_timer = None;
            session_state.plan.cancel();
            self.stats.sessions_cancelled += 1;
            *self.stats.cancelled_by_reason.entry(reason).or_insert(0) += 1;
            effects.push(AcquisitionEffect::CancelSession(session));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TreeEngine;
    use crate::TreePlanId;
    use crate::handoff::HandoffRejectReason;
    use crate::id::{DurableHandoffId, OperationGeneration, OperationId, PlanEpoch, SessionId};
    use crate::ingress::{AdmissionGate, BackpressureOutcome};
    use crate::io::{ReadOutcome, WriteOutcome};
    use crate::plan::{PlanNetworkNeed, PlanReadNeed, ScriptedEngine, ScriptedStep};
    use basics::sha_map_hash::SHAMapHash;
    use shamap::node_id::SHAMapNodeId;
    use std::collections::VecDeque;
    use std::sync::Arc;

    fn connect(runner: &mut CoordinatorRunner) {
        let effects = runner.handle_event(AcquisitionEvent::Connectivity(
            PeerAvailabilitySnapshot::new(vec![PeerId::new(1)]),
        ));
        assert_eq!(
            effects,
            vec![AcquisitionEffect::SetServicePhase(SyncPhase::Connected)]
        );
    }

    fn acquire(runner: &mut CoordinatorRunner, seq: u32) -> SessionRef {
        let effects = runner.handle_event(AcquisitionEvent::AcquireRequested {
            target: target(seq),
            reason: AcquireReason::Consensus,
        });
        peer_request_session(&effects)
    }

    fn peer_request_session(effects: &[AcquisitionEffect]) -> SessionRef {
        effects
            .iter()
            .find_map(|effect| match effect {
                AcquisitionEffect::SendLedgerRequest(request) => Some(request.session()),
                _ => None,
            })
            .expect("a peer request effect must be emitted")
    }

    fn timer_operation(effects: &[AcquisitionEffect]) -> OperationRef {
        effects
            .iter()
            .find_map(|effect| match effect {
                AcquisitionEffect::ArmTimer(request) => Some(request.clone().operation()),
                _ => None,
            })
            .expect("an arm timer effect must be emitted")
    }

    fn target(seq: u32) -> LedgerTarget {
        LedgerTarget::new(Uint256::from(u64::from(seq)), Some(seq))
    }

    fn identity(seq: u32) -> LedgerIdentity {
        LedgerIdentity::new(Uint256::from(u64::from(seq)), seq)
    }

    fn admitted_packet(
        session: SessionRef,
        gate_budget: AdmissionBudget,
        bytes: u64,
    ) -> AdmittedLedgerPacket {
        admitted_packet_from_peer(session, PeerId::new(1), gate_budget, bytes)
    }

    fn admitted_packet_from_peer(
        session: SessionRef,
        peer: PeerId,
        gate_budget: AdmissionBudget,
        bytes: u64,
    ) -> AdmittedLedgerPacket {
        let gate = Arc::new(AdmissionGate::new(gate_budget, session));
        let lease = match gate.try_reserve(1, bytes) {
            BackpressureOutcome::Admitted(lease) => lease,
            other => panic!("expected admission, got {other:?}"),
        };
        AdmittedLedgerPacket::new(
            lease,
            session,
            peer,
            ledger::InboundLedgerPacket::new(
                ledger::InboundLedgerDataType::Base,
                vec![ledger::InboundLedgerNodeData::new(
                    None,
                    vec![0; bytes as usize],
                )],
            ),
        )
        .expect("matching lease must admit")
    }

    #[test]
    fn connectivity_establishes_connected_then_demotes_to_disconnected() {
        let mut runner = CoordinatorRunner::new(RunEpoch::new(1));
        assert_eq!(runner.phase(), &SyncPhase::Disconnected);

        // An empty snapshot changes nothing.
        let effects = runner.handle_event(AcquisitionEvent::Connectivity(
            PeerAvailabilitySnapshot::new(vec![]),
        ));
        assert!(effects.is_empty());
        assert_eq!(runner.phase(), &SyncPhase::Disconnected);

        // A usable peer promotes Disconnected -> Connected.
        connect(&mut runner);

        // An unchanged snapshot changes nothing.
        let effects = runner.handle_event(AcquisitionEvent::Connectivity(
            PeerAvailabilitySnapshot::new(vec![PeerId::new(1)]),
        ));
        assert!(effects.is_empty());
        assert_eq!(runner.phase(), &SyncPhase::Connected);
    }

    #[test]
    fn startup_mode_seeds_and_republishes_the_initial_phase() {
        let mut runner = CoordinatorRunner::new(RunEpoch::new(1));

        // Networked startup intent seeds Connected before any peer capability
        // exists and publishes it through the phase port.
        let effects = runner.handle_event(AcquisitionEvent::StartupMode {
            phase: SyncPhase::Connected,
        });
        assert_eq!(
            effects,
            vec![AcquisitionEffect::SetServicePhase(SyncPhase::Connected)]
        );
        assert_eq!(runner.phase(), &SyncPhase::Connected);

        // Repeating the same seed is a no-op.
        let effects = runner.handle_event(AcquisitionEvent::StartupMode {
            phase: SyncPhase::Connected,
        });
        assert!(effects.is_empty());
        assert_eq!(runner.phase(), &SyncPhase::Connected);

        // A different seed re-publishes the new phase without touching state.
        let effects = runner.handle_event(AcquisitionEvent::StartupMode {
            phase: SyncPhase::Full {
                lcl: identity(5),
                published: identity(5),
            },
        });
        assert_eq!(
            effects,
            vec![AcquisitionEffect::SetServicePhase(SyncPhase::Full {
                lcl: identity(5),
                published: identity(5),
            })]
        );
        assert_eq!(runner.snapshot().session_count(), 0);
        assert_eq!(runner.snapshot().rejected_events(), 0);
    }

    #[test]
    fn startup_mode_seed_is_not_a_transition_and_never_mints_a_session() {
        let mut runner = CoordinatorRunner::new(RunEpoch::new(1));

        // A start_valid seed lands on Full directly, as the legacy bootstrap
        // write did, without any transition-table fact or session mint.
        let effects = runner.handle_event(AcquisitionEvent::StartupMode {
            phase: SyncPhase::Full {
                lcl: identity(5),
                published: identity(5),
            },
        });
        assert_eq!(
            effects,
            vec![AcquisitionEffect::SetServicePhase(SyncPhase::Full {
                lcl: identity(5),
                published: identity(5),
            })]
        );
        assert_eq!(
            runner.phase(),
            &SyncPhase::Full {
                lcl: identity(5),
                published: identity(5)
            }
        );
        assert_eq!(runner.snapshot().session_count(), 0);
        assert_eq!(runner.snapshot().rejected_events(), 0);
    }

    #[test]
    fn peer_loss_demotes_to_disconnected_and_cancels_sessions() {
        let mut runner = CoordinatorRunner::new(RunEpoch::new(1));
        connect(&mut runner);
        let session = acquire(&mut runner, 10);
        assert_eq!(runner.phase(), &SyncPhase::Syncing { target: target(10) });

        let effects = runner.handle_event(AcquisitionEvent::Connectivity(
            PeerAvailabilitySnapshot::new(vec![]),
        ));
        assert!(effects.contains(&AcquisitionEffect::SetServicePhase(SyncPhase::Disconnected)));
        assert!(effects.contains(&AcquisitionEffect::CancelSession(session)));
        assert_eq!(
            runner.session(session).expect("cancelled session").phase(),
            &SessionPhase::Cancelled {
                reason: CancelReason::PeerLoss
            }
        );
        assert_eq!(runner.phase(), &SyncPhase::Disconnected);
        assert_eq!(runner.snapshot().sessions_cancelled(), 1);
    }

    #[test]
    fn acquire_from_connected_emits_peer_request_and_timer() {
        let mut runner = CoordinatorRunner::new(RunEpoch::new(1));
        connect(&mut runner);

        let effects = runner.handle_event(AcquisitionEvent::AcquireRequested {
            target: target(9),
            reason: AcquireReason::Consensus,
        });
        assert!(
            effects.contains(&AcquisitionEffect::SetServicePhase(SyncPhase::Syncing {
                target: target(9)
            }))
        );

        let request = effects
            .iter()
            .find_map(|effect| match effect {
                AcquisitionEffect::SendLedgerRequest(request) => Some(request),
                _ => None,
            })
            .expect("a peer request effect must be emitted");
        assert_eq!(request.peer_id(), PeerId::new(1));
        assert_eq!(
            request.request(),
            &LedgerDataRequest::GetLedger { sequence: Some(9) }
        );
        let session = request.session();
        assert_eq!(session.target_hash(), Uint256::from(9));
        assert_eq!(session.run_epoch(), RunEpoch::new(1));

        let timer = effects
            .iter()
            .find_map(|effect| match effect {
                AcquisitionEffect::ArmTimer(request) => Some(request),
                _ => None,
            })
            .expect("an arm timer effect must be emitted");
        assert_eq!(timer.clone().timer(), TimerKind::AcquireTimeout);
        assert_eq!(timer.clone().operation().session(), session);

        assert_eq!(
            runner.session(session).expect("live session").phase(),
            &SessionPhase::Active
        );
        let snapshot = runner.snapshot();
        assert_eq!(
            snapshot.active_by_reason().get(&AcquireReason::Consensus),
            Some(&1)
        );
        assert_eq!(snapshot.peer_requests(), 1);
        assert_eq!(snapshot.timers_armed(), 1);
        assert_eq!(snapshot.session_count(), 1);
    }

    #[test]
    fn acquire_while_disconnected_replays_exactly_once_when_peers_return() {
        let mut runner = CoordinatorRunner::new(RunEpoch::new(1));
        let target = target(10);

        let deferred = runner.handle_event(AcquisitionEvent::AcquireRequested {
            target,
            reason: AcquireReason::Consensus,
        });
        assert!(deferred.is_empty());
        assert_eq!(runner.phase(), &SyncPhase::Disconnected);
        assert_eq!(runner.snapshot().session_count(), 0);
        assert_eq!(runner.snapshot().rejected_events(), 0);

        // rippled's heartbeat resumes acquisition-demand processing after peer
        // capability returns (NetworkOPs.cpp:1230-1301); a resolver miss is
        // then routed through InboundLedgers::acquire
        // (LedgerMaster.cpp:886-916). Keep the exact demand on this sole
        // coordinator owner until the capability fact arrives.
        let replay = runner.handle_event(AcquisitionEvent::Connectivity(
            PeerAvailabilitySnapshot::new(vec![PeerId::new(1)]),
        ));
        assert_eq!(
            replay
                .iter()
                .filter(|effect| matches!(effect, AcquisitionEffect::SetServicePhase(_)))
                .count(),
            2,
            "reconnect must publish Connected then Syncing"
        );
        assert!(
            replay.contains(&AcquisitionEffect::SetServicePhase(SyncPhase::Syncing {
                target
            }))
        );
        let session = peer_request_session(&replay);
        let session_started_at = replay
            .iter()
            .position(|effect| matches!(effect, AcquisitionEffect::SessionStarted(started) if *started == session))
            .expect("replay must bind the exact session before dispatch");
        let request_at = replay
            .iter()
            .position(|effect| matches!(effect, AcquisitionEffect::SendLedgerRequest(request) if request.session() == session))
            .expect("replay must emit one peer request");
        assert!(session_started_at < request_at);
        assert_eq!(session.target_hash(), target.hash());
        assert_eq!(
            replay
                .iter()
                .filter(|effect| matches!(effect, AcquisitionEffect::SendLedgerRequest(_)))
                .count(),
            1
        );
        assert_eq!(
            replay
                .iter()
                .filter(|effect| matches!(
                    effect,
                    AcquisitionEffect::ArmTimer(request)
                        if request.timer() == TimerKind::AcquireTimeout
                            && request.operation().session() == session
                ))
                .count(),
            1
        );
        assert_eq!(runner.phase(), &SyncPhase::Syncing { target });
        assert_eq!(runner.snapshot().session_count(), 1);
        assert_eq!(runner.snapshot().peer_requests(), 1);
        assert_eq!(runner.snapshot().timers_armed(), 1);

        let duplicate_connectivity = runner.handle_event(AcquisitionEvent::Connectivity(
            PeerAvailabilitySnapshot::new(vec![PeerId::new(1)]),
        ));
        assert!(duplicate_connectivity.is_empty());
        assert_eq!(runner.snapshot().session_count(), 1);
        assert_eq!(runner.snapshot().peer_requests(), 1);
        assert_eq!(runner.snapshot().timers_armed(), 1);
    }

    #[test]
    fn consensus_target_creates_a_consensus_session() {
        let mut runner = CoordinatorRunner::new(RunEpoch::new(1));
        connect(&mut runner);
        let effects = runner.handle_event(AcquisitionEvent::ConsensusTarget(ConsensusTarget::new(
            target(5),
            AcquireReason::Consensus,
        )));
        let session = peer_request_session(&effects);
        assert_eq!(
            runner.session(session).expect("live session").reason(),
            AcquireReason::Consensus
        );
        assert_eq!(runner.phase(), &SyncPhase::Syncing { target: target(5) });
    }

    #[test]
    fn preferred_lcl_divergence_demotes_full_and_tracking_to_syncing_without_a_session() {
        for phase in [
            SyncPhase::Full {
                lcl: identity(1),
                published: identity(1),
            },
            SyncPhase::Tracking { lcl: identity(1) },
            SyncPhase::Connected,
        ] {
            let mut runner = CoordinatorRunner::with_phase(RunEpoch::new(1), phase);
            let effects =
                runner.handle_event(AcquisitionEvent::PreferredLclDivergence { target: target(9) });
            assert_eq!(
                runner.phase(),
                &SyncPhase::Syncing { target: target(9) },
                "phase {:?} must demote to Syncing",
                phase
            );
            assert_eq!(
                effects,
                vec![AcquisitionEffect::SetServicePhase(SyncPhase::Syncing {
                    target: target(9)
                })]
            );
            // The divergence fact never mints a session: the acquisition demand
            // arrives as a separate AcquireRequested fact.
            assert_eq!(runner.snapshot().session_count(), 0);
        }
    }

    #[test]
    fn preferred_lcl_divergence_does_not_mint_a_session_for_a_resident_switch() {
        let mut runner = CoordinatorRunner::new(RunEpoch::new(1));
        connect(&mut runner);
        let effects =
            runner.handle_event(AcquisitionEvent::PreferredLclDivergence { target: target(9) });
        assert_eq!(runner.phase(), &SyncPhase::Syncing { target: target(9) });
        assert!(
            !effects
                .iter()
                .any(|effect| matches!(effect, AcquisitionEffect::SendLedgerRequest(_)))
        );
        assert_eq!(runner.snapshot().peer_requests(), 0);
        assert_eq!(runner.snapshot().session_count(), 0);
    }

    #[test]
    fn preferred_lcl_divergence_while_syncing_or_disconnected_is_rejected() {
        for phase in [
            SyncPhase::Disconnected,
            SyncPhase::Syncing { target: target(1) },
        ] {
            let mut runner = CoordinatorRunner::with_phase(RunEpoch::new(1), phase);
            let rejected = runner.snapshot().rejected_events();
            let effects =
                runner.handle_event(AcquisitionEvent::PreferredLclDivergence { target: target(9) });
            assert!(effects.is_empty(), "phase {:?} must reject the fact", phase);
            assert_eq!(runner.phase(), &phase, "phase {:?} must not change", phase);
            assert_eq!(runner.snapshot().rejected_events(), rejected + 1);
        }
    }

    #[test]
    fn preferred_lcl_divergence_requires_usable_peers() {
        let mut runner = CoordinatorRunner::new(RunEpoch::new(1));
        let effects =
            runner.handle_event(AcquisitionEvent::PreferredLclDivergence { target: target(9) });
        assert!(effects.is_empty());
        assert_eq!(runner.phase(), &SyncPhase::Disconnected);
        assert_eq!(runner.snapshot().rejected_events(), 1);
    }

    #[test]
    fn preferred_lcl_divergence_then_acquire_mints_the_session() {
        let mut runner = CoordinatorRunner::new(RunEpoch::new(1));
        connect(&mut runner);
        let _effects =
            runner.handle_event(AcquisitionEvent::PreferredLclDivergence { target: target(9) });
        assert_eq!(runner.phase(), &SyncPhase::Syncing { target: target(9) });
        // The strand's missing/incomplete path feeds the acquisition demand
        // after the demotion; the coordinator then mints the session.
        let acquire = runner.handle_event(AcquisitionEvent::AcquireRequested {
            target: target(9),
            reason: AcquireReason::Consensus,
        });
        let session = peer_request_session(&acquire);
        assert_eq!(session.target_hash(), Uint256::from(9));
        assert_eq!(runner.phase(), &SyncPhase::Syncing { target: target(9) });
        assert_eq!(runner.snapshot().session_count(), 1);
        assert_eq!(runner.snapshot().rejected_events(), 0);
    }

    #[test]
    fn blocked_with_no_target_demotes_full_to_connected() {
        let mut runner = CoordinatorRunner::with_phase(
            RunEpoch::new(1),
            SyncPhase::Full {
                lcl: identity(1),
                published: identity(1),
            },
        );
        let effects = runner.handle_event(AcquisitionEvent::BlockedWithNoTarget);
        assert_eq!(
            effects,
            vec![AcquisitionEffect::SetServicePhase(SyncPhase::Connected)]
        );
        assert_eq!(runner.phase(), &SyncPhase::Connected);
        assert_eq!(runner.snapshot().rejected_events(), 0);
    }

    #[test]
    fn blocked_with_no_target_is_rejected_outside_full() {
        for phase in [
            SyncPhase::Connected,
            SyncPhase::Syncing { target: target(1) },
            SyncPhase::Tracking { lcl: identity(1) },
            SyncPhase::Disconnected,
        ] {
            let mut runner = CoordinatorRunner::with_phase(RunEpoch::new(1), phase);
            let rejected = runner.snapshot().rejected_events();
            let effects = runner.handle_event(AcquisitionEvent::BlockedWithNoTarget);
            assert!(effects.is_empty(), "phase {:?} must reject the fact", phase);
            assert_eq!(runner.phase(), &phase, "phase {:?} must not change", phase);
            assert_eq!(runner.snapshot().rejected_events(), rejected + 1);
        }
    }

    #[test]
    fn repeated_targets_coalesce_without_weakening_known_sequence_and_safe_unknown_sequence_promotes()
     {
        let mut runner = CoordinatorRunner::new(RunEpoch::new(1));
        connect(&mut runner);

        let effects_a = runner.handle_event(AcquisitionEvent::AcquireRequested {
            target: target(9),
            reason: AcquireReason::Consensus,
        });
        let session_a = peer_request_session(&effects_a);
        let timer_a = timer_operation(&effects_a);

        // The same exact target coalesces with the active owner: it produces no
        // cancellation, second peer request, or replacement session.
        let effects_b = runner.handle_event(AcquisitionEvent::AcquireRequested {
            target: target(9),
            reason: AcquireReason::Consensus,
        });
        assert!(!effects_b.contains(&AcquisitionEffect::CancelSession(session_a)));
        assert!(
            !effects_b
                .iter()
                .any(|effect| matches!(effect, AcquisitionEffect::SendLedgerRequest(_)))
        );
        assert_eq!(runner.snapshot().session_count(), 1);
        assert_eq!(runner.snapshot().sessions_cancelled(), 0);
        assert_eq!(
            runner
                .session(session_a)
                .expect("coalesced session")
                .phase(),
            &SessionPhase::Active
        );

        // A hash-only follow-up is weaker than the live verified target. It
        // must coalesce without replacing the session, changing its target,
        // or weakening the coordinator's syncing target.
        let known = LedgerTarget::new(Uint256::from(98), Some(98));
        let known_effects = runner.handle_event(AcquisitionEvent::AcquireRequested {
            target: known,
            reason: AcquireReason::Generic,
        });
        let known_session = peer_request_session(&known_effects);
        let hash_only_effects = runner.handle_event(AcquisitionEvent::AcquireRequested {
            target: LedgerTarget::new(Uint256::from(98), None),
            reason: AcquireReason::Consensus,
        });
        assert!(hash_only_effects.iter().all(|effect| {
            !matches!(
                effect,
                AcquisitionEffect::CancelSession(_) | AcquisitionEffect::SendLedgerRequest(_)
            )
        }));
        assert_eq!(
            runner
                .session(known_session)
                .expect("known session")
                .target(),
            known
        );
        assert_eq!(runner.phase(), &SyncPhase::Syncing { target: known });
        assert_eq!(runner.snapshot().sessions_cancelled(), 0);

        // Unknown sequence can promote safely without changing the same-hash
        // session. The same session is retained and the coordinator phase
        // gains the newly known sequence without a replacement send.
        let unknown = LedgerTarget::new(Uint256::from(99), None);
        let unknown_effects = runner.handle_event(AcquisitionEvent::AcquireRequested {
            target: unknown,
            reason: AcquireReason::Consensus,
        });
        let unknown_session = peer_request_session(&unknown_effects);
        assert!(unknown_effects.iter().any(|effect| {
            matches!(
                effect,
                AcquisitionEffect::SendLedgerRequest(request)
                    if request.request() == &LedgerDataRequest::GetLedger { sequence: None }
            )
        }));
        let promotion = runner.handle_event(AcquisitionEvent::AcquireRequested {
            target: LedgerTarget::new(Uint256::from(99), Some(99)),
            reason: AcquireReason::Consensus,
        });
        assert!(promotion.iter().all(|effect| {
            !matches!(
                effect,
                AcquisitionEffect::CancelSession(_) | AcquisitionEffect::SendLedgerRequest(_)
            )
        }));
        assert_eq!(
            runner
                .session(unknown_session)
                .expect("promoted")
                .target()
                .sequence(),
            Some(99)
        );
        assert_eq!(runner.snapshot().sessions_cancelled(), 0);
        assert_eq!(timer_a.session(), session_a);
    }

    #[test]
    fn known_sequence_promotes_a_hash_only_session_after_base_seeds_its_plan() {
        let budget = BudgetState::new(8, AdmissionBudget::new(4, 1024), Duration::from_secs(1));
        let mut runner = CoordinatorRunner::with_plan_seed(
            RunEpoch::new(1),
            budget,
            Box::new(ScriptedSeed::new(vec![ScriptedStep::NeedsNetwork(vec![])])),
        );
        connect(&mut runner);

        let hash_only = LedgerTarget::new(Uint256::from(99), None);
        let initial = runner.handle_event(AcquisitionEvent::AcquireRequested {
            target: hash_only,
            reason: AcquireReason::Consensus,
        });
        let session = peer_request_session(&initial);
        let packet = admitted_packet(session, AdmissionBudget::new(1, 256), 8);
        let _ = runner.handle_event(AcquisitionEvent::PacketAdmitted(packet));
        assert!(
            runner
                .session(session)
                .expect("seeded session")
                .plan()
                .engine()
                .is_some()
        );

        let known = LedgerTarget::new(Uint256::from(99), Some(99));
        let effects = runner.handle_event(AcquisitionEvent::AcquireRequested {
            target: known,
            reason: AcquireReason::Generic,
        });
        assert!(effects.iter().all(|effect| {
            !matches!(
                effect,
                AcquisitionEffect::CancelSession(_) | AcquisitionEffect::SendLedgerRequest(_)
            )
        }));
        assert_eq!(runner.session(session).expect("promoted").target(), known);
        assert_eq!(runner.phase(), &SyncPhase::Syncing { target: known });
        assert_eq!(runner.snapshot().sessions_started(), 1);
        assert_eq!(runner.snapshot().sessions_cancelled(), 0);
    }

    #[test]
    fn live_sessions_excludes_terminal_sessions() {
        let mut runner = CoordinatorRunner::new(RunEpoch::new(1));
        connect(&mut runner);
        assert_eq!(
            runner.live_sessions().collect::<Vec<_>>(),
            Vec::<SessionRef>::new()
        );

        let session = acquire(&mut runner, 10);
        assert_eq!(runner.live_sessions().collect::<Vec<_>>(), vec![session]);

        // Peer loss cancels the session; the terminal session must leave the
        // routing set while remaining observable as a stale target.
        let effects = runner.handle_event(AcquisitionEvent::Connectivity(
            PeerAvailabilitySnapshot::new(vec![]),
        ));
        assert!(effects.contains(&AcquisitionEffect::CancelSession(session)));
        assert_eq!(
            runner.live_sessions().collect::<Vec<_>>(),
            Vec::<SessionRef>::new()
        );
        assert!(runner.session(session).is_some());
    }

    #[test]
    fn store_rotation_cancels_old_generation_and_isolates_new_reads() {
        let mut runner = CoordinatorRunner::new(RunEpoch::new(1));
        connect(&mut runner);
        let session_a = acquire(&mut runner, 10);
        assert_eq!(session_a.store_generation(), StoreGeneration::new(1));

        let effects = runner.handle_event(AcquisitionEvent::StoreRotated(StoreGeneration::new(2)));
        assert!(effects.contains(&AcquisitionEffect::CancelSession(session_a)));
        assert_eq!(runner.storage_generation(), StoreGeneration::new(2));
        assert_eq!(
            runner
                .session(session_a)
                .expect("cancelled session")
                .phase(),
            &SessionPhase::Cancelled {
                reason: CancelReason::StoreRotated
            }
        );

        assert_eq!(
            runner.snapshot().cancelled_by_reason(),
            &BTreeMap::from([(CancelReason::StoreRotated, 1)])
        );

        // A session minted after the rotation is isolated to generation 2.
        let session_b = acquire(&mut runner, 11);
        assert_eq!(session_b.store_generation(), StoreGeneration::new(2));
        assert_ne!(session_b, session_a);

        // A duplicate rotation is idempotent.
        let effects = runner.handle_event(AcquisitionEvent::StoreRotated(StoreGeneration::new(2)));
        assert!(effects.is_empty());
    }

    #[test]
    fn packet_admission_respects_session_mailbox_bounds() {
        let budget = BudgetState::new(8, AdmissionBudget::new(4, 100), Duration::from_secs(1));
        let mut runner = CoordinatorRunner::with_budget(RunEpoch::new(1), budget);
        connect(&mut runner);
        let session = acquire(&mut runner, 10);

        // A lease the ingress gate granted but whose byte count exceeds the
        // coordinator's defensive mailbox budget is dropped without mutation.
        let packet = admitted_packet(session, AdmissionBudget::new(1, 256), 200);
        let effects = runner.handle_event(AcquisitionEvent::PacketAdmitted(packet));
        assert!(effects.is_empty());
        assert_eq!(runner.snapshot().packets_dropped(), 1);
        assert_eq!(
            runner
                .session(session)
                .expect("live session")
                .packet_count(),
            0
        );

        // A within-budget lease is routed and accounted.
        let packet = admitted_packet(session, AdmissionBudget::new(1, 64), 8);
        let effects = runner.handle_event(AcquisitionEvent::PacketAdmitted(packet));
        assert!(effects.is_empty());
        assert_eq!(runner.snapshot().packets_admitted(), 1);
        assert_eq!(
            runner
                .session(session)
                .expect("live session")
                .packet_count(),
            1
        );
        assert_eq!(
            runner
                .session(session)
                .expect("live session")
                .packet_bytes(),
            8
        );
    }

    #[test]
    fn timer_fired_matches_the_exact_expected_operation() {
        let mut runner = CoordinatorRunner::new(RunEpoch::new(1));
        connect(&mut runner);
        let effects = acquire_with_effects(&mut runner, 10);
        let session = peer_request_session(&effects);
        let timer_op = timer_operation(&effects);

        // The exact armed operation fires once and re-arms the deadline with a
        // fresh operation identity (the plan consumed one timeout budget).
        let effects = runner.handle_event(AcquisitionEvent::TimerFired {
            operation: timer_op,
            timer: TimerKind::AcquireTimeout,
        });
        let retry = effects
            .iter()
            .find_map(|effect| match effect {
                AcquisitionEffect::SendLedgerRequest(request) => Some(request),
                _ => None,
            })
            .expect("an unseeded acquisition must retry its Base request");
        assert_eq!(retry.session(), session);
        assert_eq!(retry.peer_id(), PeerId::new(1));
        assert_eq!(
            retry.request(),
            &LedgerDataRequest::GetLedger { sequence: Some(10) }
        );
        let rearm = effects
            .iter()
            .find_map(|effect| match effect {
                AcquisitionEffect::ArmTimer(request) => Some(request.clone().operation()),
                _ => None,
            })
            .expect("the deadline must be rearmed");
        assert_ne!(rearm, timer_op);
        assert_eq!(runner.snapshot().stale_events(), 0);

        // The consumed wakeup cannot fire again (the pending timer was rearmed
        // with a new identity, so the old operation is stale).
        let effects = runner.handle_event(AcquisitionEvent::TimerFired {
            operation: timer_op,
            timer: TimerKind::AcquireTimeout,
        });
        assert!(effects.is_empty());
        assert_eq!(runner.snapshot().stale_events(), 1);

        // A different timer kind on a fresh session's armed timer is stale.
        let effects_b = acquire_with_effects(&mut runner, 11);
        let timer_op_b = timer_operation(&effects_b);
        let effects = runner.handle_event(AcquisitionEvent::TimerFired {
            operation: timer_op_b,
            timer: TimerKind::ReadRetry,
        });
        assert!(effects.is_empty());
        assert_eq!(runner.snapshot().stale_events(), 2);
    }

    fn acquire_with_effects(runner: &mut CoordinatorRunner, seq: u32) -> Vec<AcquisitionEffect> {
        runner.handle_event(AcquisitionEvent::AcquireRequested {
            target: target(seq),
            reason: AcquireReason::Consensus,
        })
    }

    #[test]
    fn unseeded_base_timeout_cycles_after_initial_peer_fanout() {
        let mut runner = CoordinatorRunner::new(RunEpoch::new(1));
        let _ = runner.handle_event(AcquisitionEvent::Connectivity(
            PeerAvailabilitySnapshot::new(vec![PeerId::new(1), PeerId::new(2)]),
        ));
        let initial = acquire_with_effects(&mut runner, 10);
        let session = peer_request_session(&initial);
        let initial_peers = initial
            .iter()
            .filter_map(|effect| match effect {
                AcquisitionEffect::SendLedgerRequest(request) => Some(request.peer_id()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(initial_peers, vec![PeerId::new(1), PeerId::new(2)]);
        let timer = timer_operation(&initial);

        let effects = runner.handle_event(AcquisitionEvent::TimerFired {
            operation: timer,
            timer: TimerKind::AcquireTimeout,
        });
        let retry_peers = effects
            .iter()
            .filter_map(|effect| match effect {
                AcquisitionEffect::SendLedgerRequest(request) => Some(request),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            retry_peers.len(),
            2,
            "timeout must reach every selected peer"
        );
        assert!(retry_peers.iter().all(|retry| {
            retry.session() == session
                && retry.request() == &LedgerDataRequest::GetLedger { sequence: Some(10) }
        }));
        assert_eq!(
            retry_peers
                .iter()
                .map(|retry| retry.peer_id())
                .collect::<Vec<_>>(),
            vec![PeerId::new(1), PeerId::new(2)]
        );
        assert_eq!(runner.snapshot().peer_requests(), 4);
        assert_eq!(
            runner.session(session).expect("live session").sent_peers(),
            &BTreeSet::from([PeerId::new(1), PeerId::new(2)])
        );
    }

    #[test]
    fn progress_suppresses_one_interval_without_resetting_timeout_budget_or_peer_window() {
        let mut runner = CoordinatorRunner::new(RunEpoch::new(1));
        let _ = runner.handle_event(AcquisitionEvent::Connectivity(
            PeerAvailabilitySnapshot::new((1..=100).map(PeerId::new).collect()),
        ));
        let initial = acquire_with_effects(&mut runner, 10);
        let session = peer_request_session(&initial);
        let mut effects = initial;

        // A no-progress interval consumes budget and can expand the bounded
        // selected window. A progress interval only rearms: it does not reset
        // the already consumed rippled timeout count.
        for expected_timeouts in 1..=3 {
            effects = runner.handle_event(AcquisitionEvent::TimerFired {
                operation: timer_operation(&effects),
                timer: TimerKind::AcquireTimeout,
            });
            assert_eq!(
                runner
                    .session(session)
                    .expect("live session")
                    .plan()
                    .timeouts(),
                expected_timeouts
            );
            assert!(
                runner
                    .session(session)
                    .expect("live session")
                    .sent_peers()
                    .len()
                    <= MAX_SELECTED_PEERS
            );

            runner
                .state
                .sessions
                .get_mut(&session)
                .expect("live session")
                .plan
                .note_progress();
            effects = runner.handle_event(AcquisitionEvent::TimerFired {
                operation: timer_operation(&effects),
                timer: TimerKind::AcquireTimeout,
            });
            assert_eq!(
                runner
                    .session(session)
                    .expect("live session")
                    .plan()
                    .timeouts(),
                expected_timeouts,
                "a progress interval must not erase prior no-progress budget"
            );
        }
    }

    #[test]
    fn seeded_no_progress_timeout_reprobes_and_resends_exact_frontier() {
        let budget = BudgetState::new(8, AdmissionBudget::new(4, 1024), Duration::from_secs(1));
        let node = PlanNetworkNeed::new(
            SHAMapNodeId::default(),
            Uint256::from(0x77),
            ledger::TreeKind::Transaction,
        );
        let mut runner = CoordinatorRunner::with_plan_seed(
            RunEpoch::new(1),
            budget,
            Box::new(ScriptedSeed::new(vec![ScriptedStep::NeedsNetworkWithKind(
                vec![node],
            )])),
        );
        let _ = runner.handle_event(AcquisitionEvent::Connectivity(
            PeerAvailabilitySnapshot::new((1..=6).map(PeerId::new).collect()),
        ));
        let initial = acquire_with_effects(&mut runner, 10);
        let session = peer_request_session(&initial);
        let initial_timer = timer_operation(&initial);
        let seed = runner.handle_event(AcquisitionEvent::PacketAdmitted(admitted_packet(
            session,
            AdmissionBudget::new(1, 256),
            8,
        )));
        let initial_frontier = seed
            .iter()
            .find_map(|effect| match effect {
                AcquisitionEffect::SendLedgerRequest(request)
                    if matches!(request.request(), LedgerDataRequest::GetLedgerNodes { .. }) =>
                {
                    Some(request.clone())
                }
                _ => None,
            })
            .expect("seeded plan must request its normal frontier");

        // The Base seed was useful, so rippled's `wasProgress` interval only
        // rearms; it must not manufacture a timeout recovery request yet.
        let first = runner.handle_event(AcquisitionEvent::TimerFired {
            operation: initial_timer,
            timer: TimerKind::AcquireTimeout,
        });
        assert!(read_effects(&first).is_empty());
        assert!(
            !first
                .iter()
                .any(|effect| matches!(effect, AcquisitionEffect::SendLedgerRequest(_)))
        );

        // The next quiet interval is a seeded no-progress timeout. It submits
        // bounded async local reprobes and resends the exact transaction-node
        // frontier with fresh operation identities, including one bounded peer
        // escalation from the initial five peers to peer six.
        let second = runner.handle_event(AcquisitionEvent::TimerFired {
            operation: timer_operation(&first),
            timer: TimerKind::AcquireTimeout,
        });
        let reads = read_effects(&second);
        assert_eq!(reads.len(), 1);
        assert_eq!(reads[0].key(), SHAMapHash::new(Uint256::from(0x77)));
        assert_eq!(reads[0].operation().session(), session);
        let normal = second
            .iter()
            .filter_map(|effect| match effect {
                AcquisitionEffect::SendLedgerRequest(request)
                    if matches!(request.request(), LedgerDataRequest::GetLedgerNodes { .. }) =>
                {
                    Some(request)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(normal.len(), 6);
        assert!(normal.iter().all(|request| {
            request.operation() != initial_frontier.operation()
                && matches!(
                    request.request(),
                    LedgerDataRequest::GetLedgerNodes {
                        kind: ledger::TreeKind::Transaction,
                        node_ids,
                        sequence: 1,
                    } if node_ids == &vec![SHAMapNodeId::default()]
                )
        }));

        // After rippled's `timeouts > 4` threshold the same retained frontier
        // switches to bounded by-hash requests, still with fresh operations.
        let mut effects = second;
        for _ in 0..3 {
            effects = runner.handle_event(AcquisitionEvent::TimerFired {
                operation: timer_operation(&effects),
                timer: TimerKind::AcquireTimeout,
            });
        }
        assert!(effects.iter().any(|effect| {
            matches!(
                effect,
                AcquisitionEffect::SendLedgerRequest(request)
                    if matches!(request.request(), LedgerDataRequest::GetNodes { nodes, sequence: Some(1) }
                        if nodes == &vec![crate::peer::LedgerNodeRequest::new(
                            Uint256::from(0x77),
                            ledger::TreeKind::Transaction,
                        )])
            )
        }));
    }

    #[test]
    fn consensus_capacity_demand_is_retained_replayed_and_preempts_lower_priority_work() {
        let budget = BudgetState::new(1, AdmissionBudget::new(4, 1024), Duration::from_secs(1));
        let mut runner = CoordinatorRunner::with_budget(RunEpoch::new(1), budget);
        connect(&mut runner);
        let first = acquire_with_effects(&mut runner, 1);
        let first_session = peer_request_session(&first);

        // A second consensus target cannot displace a consensus owner, so the
        // coordinator retains the latest preferred-LCL demand rather than
        // silently rejecting it.
        let deferred = runner.handle_event(AcquisitionEvent::AcquireRequested {
            target: target(2),
            reason: AcquireReason::Consensus,
        });
        assert!(
            !deferred
                .iter()
                .any(|effect| matches!(effect, AcquisitionEffect::SessionStarted(_)))
        );
        assert!(runner.has_deferred_consensus_target(target(2)));
        assert_eq!(runner.phase(), &SyncPhase::Syncing { target: target(2) });

        // Generic pressure cannot consume the reserved next slot.
        let generic = runner.handle_event(AcquisitionEvent::AcquireRequested {
            target: target(3),
            reason: AcquireReason::Generic,
        });
        assert!(
            !generic
                .iter()
                .any(|effect| matches!(effect, AcquisitionEffect::SessionStarted(_)))
        );
        assert!(runner.has_deferred_consensus_target(target(2)));

        // A terminal capacity release replays the retained demand on the same
        // owner with a new session/store generation; stale old events remain
        // unable to mutate it.
        let replay = runner.handle_event(AcquisitionEvent::StoreRotated(StoreGeneration::new(2)));
        assert!(replay.contains(&AcquisitionEffect::CancelSession(first_session)));
        let replayed = replay
            .iter()
            .find_map(|effect| match effect {
                AcquisitionEffect::SessionStarted(session) => Some(*session),
                _ => None,
            })
            .expect("free capacity must replay the retained consensus target");
        assert_eq!(replayed.target_hash(), target(2).hash());
        assert_eq!(replayed.store_generation(), StoreGeneration::new(2));
        assert!(!runner.has_deferred_consensus_target(target(2)));

        // When lower-priority work is the occupant, consensus starts now by
        // cancelling only the pre-fence Generic session, never a handoff.
        let mut priority = CoordinatorRunner::with_budget(RunEpoch::new(2), budget);
        connect(&mut priority);
        let generic = priority.handle_event(AcquisitionEvent::AcquireRequested {
            target: target(4),
            reason: AcquireReason::Generic,
        });
        let generic_session = peer_request_session(&generic);
        let consensus = priority.handle_event(AcquisitionEvent::AcquireRequested {
            target: target(5),
            reason: AcquireReason::Consensus,
        });
        assert!(consensus.contains(&AcquisitionEffect::CancelSession(generic_session)));
        assert!(consensus.iter().any(|effect| {
            matches!(effect, AcquisitionEffect::SessionStarted(session) if session.target_hash() == target(5).hash())
        }));
    }

    #[test]
    fn reply_driven_network_request_uses_the_replying_peer() {
        let budget = BudgetState::new(8, AdmissionBudget::new(4, 1024), Duration::from_secs(1));
        let mut runner = CoordinatorRunner::with_plan_seed(
            RunEpoch::new(1),
            budget,
            Box::new(ScriptedSeed::new(vec![ScriptedStep::NeedsNetwork(vec![(
                SHAMapNodeId::default(),
                Uint256::from(3),
            )])])),
        );
        let _ = runner.handle_event(AcquisitionEvent::Connectivity(
            PeerAvailabilitySnapshot::new(vec![PeerId::new(1), PeerId::new(2), PeerId::new(3)]),
        ));
        let initial = acquire_with_effects(&mut runner, 10);
        let session = peer_request_session(&initial);

        let effects = runner.handle_event(AcquisitionEvent::PacketAdmitted(
            admitted_packet_from_peer(session, PeerId::new(3), AdmissionBudget::new(1, 256), 8),
        ));
        let requests = effects
            .iter()
            .filter_map(|effect| match effect {
                AcquisitionEffect::SendLedgerRequest(request) => Some(request),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].peer_id(), PeerId::new(3));
        assert!(matches!(
            requests[0].request(),
            LedgerDataRequest::GetLedgerNodes { .. }
        ));
    }

    #[test]
    fn reply_driven_duplicate_nodes_are_suppressed_after_header_fanout() {
        let budget = BudgetState::new(8, AdmissionBudget::new(4, 1024), Duration::from_secs(1));
        let node = (SHAMapNodeId::default(), Uint256::from(3));
        let mut runner = CoordinatorRunner::with_plan_seed(
            RunEpoch::new(1),
            budget,
            Box::new(ScriptedSeed::new(vec![
                ScriptedStep::NeedsNetwork(vec![node]),
                ScriptedStep::NeedsNetwork(vec![node]),
            ])),
        );
        let _ = runner.handle_event(AcquisitionEvent::Connectivity(
            PeerAvailabilitySnapshot::new(vec![PeerId::new(1), PeerId::new(2)]),
        ));
        let session = peer_request_session(&acquire_with_effects(&mut runner, 10));

        let first = runner.handle_event(AcquisitionEvent::PacketAdmitted(
            admitted_packet_from_peer(session, PeerId::new(1), AdmissionBudget::new(1, 256), 8),
        ));
        assert_eq!(
            first
                .iter()
                .filter(|effect| matches!(effect, AcquisitionEffect::SendLedgerRequest(_)))
                .count(),
            1
        );

        // A second header reply from another initial-fanout peer discovers the
        // same missing node. As in rippled `filterNodes(..., Reply)`, it must
        // not emit a duplicate request or add that peer to the request set.
        let duplicate = runner.handle_event(AcquisitionEvent::PacketAdmitted(
            admitted_packet_from_peer(session, PeerId::new(2), AdmissionBudget::new(1, 256), 8),
        ));
        assert!(
            duplicate
                .iter()
                .all(|effect| !matches!(effect, AcquisitionEffect::SendLedgerRequest(_)))
        );
        assert_eq!(runner.snapshot().peer_requests(), 3); // two Base + one node
        assert_eq!(
            runner.session(session).expect("live session").sent_peers(),
            &BTreeSet::from([PeerId::new(1), PeerId::new(2)])
        );
    }

    #[test]
    fn terminal_sessions_do_not_consume_live_session_capacity() {
        let budget = BudgetState::new(1, AdmissionBudget::default(), Duration::from_secs(1));
        let mut runner = CoordinatorRunner::with_budget(RunEpoch::new(1), budget);
        connect(&mut runner);
        let first = acquire(&mut runner, 1);

        let _ = runner.handle_event(AcquisitionEvent::Connectivity(
            PeerAvailabilitySnapshot::new(vec![]),
        ));
        assert!(
            runner
                .session(first)
                .expect("terminal session")
                .phase()
                .is_terminal()
        );
        let _ = runner.handle_event(AcquisitionEvent::Connectivity(
            PeerAvailabilitySnapshot::new(vec![PeerId::new(1)]),
        ));

        let effects = acquire_with_effects(&mut runner, 2);
        assert!(effects.iter().any(|effect| {
            matches!(effect, AcquisitionEffect::SendLedgerRequest(request)
                if request.session().target_hash() == Uint256::from(2))
        }));
        assert_eq!(runner.snapshot().rejected_events(), 0);
    }

    #[test]
    fn completions_require_exact_session_liveness_and_kind() {
        let mut runner = CoordinatorRunner::new(RunEpoch::new(1));
        connect(&mut runner);
        let session = acquire(&mut runner, 10);

        // A read completion whose operation is not the exact in-flight read is
        // stale even for a live session: a target hash alone never authorizes
        // a result.
        let read = OperationRef::new(
            session,
            OperationKind::Read,
            OperationId::new(1),
            OperationGeneration::new(1),
        );
        runner.handle_event(AcquisitionEvent::ReadCompleted(ReadCompletion::new(
            read,
            ReadOutcome::Settled { node: None },
        )));
        assert_eq!(runner.snapshot().stale_events(), 1);

        // A completion with the wrong operation kind is stale.
        let wrong_kind = OperationRef::new(
            session,
            OperationKind::Timer,
            OperationId::new(1),
            OperationGeneration::new(1),
        );
        runner.handle_event(AcquisitionEvent::ReadCompleted(ReadCompletion::new(
            wrong_kind,
            ReadOutcome::Settled { node: None },
        )));
        assert_eq!(runner.snapshot().stale_events(), 2);

        // A completion for an unknown session is stale.
        let foreign = SessionRef::new(
            RunEpoch::new(1),
            SessionId::new(999),
            Uint256::from(999),
            PlanEpoch::new(1),
            StoreGeneration::new(1),
        );
        let read = OperationRef::new(
            foreign,
            OperationKind::Read,
            OperationId::new(1),
            OperationGeneration::new(1),
        );
        runner.handle_event(AcquisitionEvent::ReadCompleted(ReadCompletion::new(
            read,
            ReadOutcome::Settled { node: None },
        )));
        assert_eq!(runner.snapshot().stale_events(), 3);
    }

    #[test]
    fn durability_fence_and_handoff_ack_require_a_live_session() {
        let mut runner = CoordinatorRunner::new(RunEpoch::new(1));
        connect(&mut runner);
        let session = acquire(&mut runner, 10);

        // A fence whose operation is not an exact in-flight fence is stale
        // even for a live session: a target hash alone never authorizes a
        // result, and durability never adopts provisionally.
        let fence = OperationRef::new(
            session,
            OperationKind::DurabilityFence,
            OperationId::new(1),
            OperationGeneration::new(1),
        );
        runner.handle_event(AcquisitionEvent::DurabilityFenced(
            DurabilityCompletion::new(fence, crate::io::DurabilityOutcome::Passed),
        ));
        assert_eq!(runner.snapshot().stale_events(), 1);

        // A handoff ack for a session that never issued a handoff is stale:
        // only the exact pending handoff id authorizes finalization.
        runner.handle_event(AcquisitionEvent::DurableHandoffAcknowledged(
            DurableHandoffAcknowledgement::new(DurableHandoffId::new(1), session),
        ));
        assert_eq!(runner.snapshot().stale_events(), 2);
        assert_eq!(runner.snapshot().sessions_completed(), 0);

        // After cancellation the same facts are stale.
        let effects = runner.handle_event(AcquisitionEvent::Connectivity(
            PeerAvailabilitySnapshot::new(vec![]),
        ));
        assert!(effects.contains(&AcquisitionEffect::CancelSession(session)));
        runner.handle_event(AcquisitionEvent::DurabilityFenced(
            DurabilityCompletion::new(fence, crate::io::DurabilityOutcome::Passed),
        ));
        assert_eq!(runner.snapshot().stale_events(), 3);
    }

    #[test]
    fn shutdown_produces_stopping_and_no_further_effects() {
        let mut runner = CoordinatorRunner::new(RunEpoch::new(1));
        connect(&mut runner);
        let session = acquire(&mut runner, 10);

        let effects = runner.handle_event(AcquisitionEvent::Shutdown);
        assert!(effects.contains(&AcquisitionEffect::SetServicePhase(SyncPhase::Stopping)));
        assert!(effects.contains(&AcquisitionEffect::CancelSession(session)));
        assert_eq!(runner.phase(), &SyncPhase::Stopping);
        assert!(runner.snapshot().shutdown());

        // Post-shutdown events produce no effects and are stale.
        let effects = runner.handle_event(AcquisitionEvent::AcquireRequested {
            target: target(20),
            reason: AcquireReason::Consensus,
        });
        assert!(effects.is_empty());
        let effects = runner.handle_event(AcquisitionEvent::Connectivity(
            PeerAvailabilitySnapshot::new(vec![PeerId::new(1)]),
        ));
        assert!(effects.is_empty());
        // Post-shutdown events are stale, so the peer view is frozen at its
        // last accepted snapshot.
        assert_eq!(runner.snapshot().stale_events(), 2);
        assert_eq!(runner.snapshot().peer_count(), 1);
    }

    #[test]
    fn max_sessions_backpressure_rejects_excess_demand() {
        let budget = BudgetState::new(1, AdmissionBudget::default(), Duration::from_secs(1));
        let mut runner = CoordinatorRunner::with_budget(RunEpoch::new(1), budget);
        connect(&mut runner);
        acquire(&mut runner, 1);

        let effects = runner.handle_event(AcquisitionEvent::AcquireRequested {
            target: target(2),
            reason: AcquireReason::Consensus,
        });
        assert!(effects.is_empty());
        assert_eq!(runner.snapshot().session_count(), 1);
        assert_eq!(runner.snapshot().rejected_events(), 1);
    }

    #[test]
    fn local_lcl_install_drives_connected_to_tracking_without_acquisition() {
        let mut runner = CoordinatorRunner::new(RunEpoch::new(1));
        connect(&mut runner);
        assert_eq!(runner.phase(), &SyncPhase::Connected);

        // A locally resident preferred LCL installed while Connected (no
        // acquisition was needed) drives Connected -> Tracking, matching
        // rippled switchLastClosedLedger clearing needNetworkLedger.
        let effects = runner.handle_event(AcquisitionEvent::LclInstalled(identity(9)));
        assert_eq!(
            effects,
            vec![AcquisitionEffect::SetServicePhase(SyncPhase::Tracking {
                lcl: identity(9)
            })]
        );

        // Tracking -> Full still requires a fresh matching publication.
        let effects = runner.handle_event(AcquisitionEvent::PublicationCommitted {
            identity: identity(9),
            fresh: false,
        });
        assert!(effects.is_empty());
        assert_eq!(runner.phase(), &SyncPhase::Tracking { lcl: identity(9) });
        let effects = runner.handle_event(AcquisitionEvent::PublicationCommitted {
            identity: identity(9),
            fresh: true,
        });
        assert_eq!(
            effects,
            vec![AcquisitionEffect::SetServicePhase(SyncPhase::Full {
                lcl: identity(9),
                published: identity(9)
            })]
        );
    }

    #[test]
    fn hash_only_consensus_probe_for_locally_installed_lcl_does_not_demote_full() {
        let mut runner = CoordinatorRunner::with_phase(
            RunEpoch::new(1),
            SyncPhase::Full {
                lcl: identity(9),
                published: identity(9),
            },
        );
        // Full already has peer capability in production. This establishes the
        // same peer view without treating a redundant capability fact as a
        // service-phase transition.
        let effects = runner.handle_event(AcquisitionEvent::Connectivity(
            PeerAvailabilitySnapshot::new(vec![PeerId::new(1)]),
        ));
        assert!(effects.is_empty());

        let target = LedgerTarget::new(Uint256::from(10), None);
        let effects = runner.handle_event(AcquisitionEvent::AcquireRequested {
            target,
            reason: AcquireReason::Consensus,
        });
        let session = peer_request_session(&effects);
        assert!(
            !effects.iter().any(|effect| matches!(
                effect,
                AcquisitionEffect::SetServicePhase(SyncPhase::Syncing { .. })
            )),
            "a hash-only local consensus probe must not demote Full before remote work exists"
        );
        assert_eq!(
            runner.phase(),
            &SyncPhase::Full {
                lcl: identity(9),
                published: identity(9),
            }
        );

        // A matching peer Base reply can arrive before the local consensus
        // child is installed. It proves the object is available but not that
        // the preferred LCL diverged. In rippled, only consensusViewChange's
        // explicit divergence path changes mode, so this must remain Full.
        let effects = runner.handle_event(AcquisitionEvent::PacketAdmitted(admitted_packet(
            session,
            AdmissionBudget::new(1, 256),
            8,
        )));
        assert!(
            !effects
                .iter()
                .any(|effect| matches!(effect, AcquisitionEffect::SetServicePhase(_))),
            "a peer reply for a hash-only local consensus probe must not demote Full"
        );
        assert_eq!(
            runner.phase(),
            &SyncPhase::Full {
                lcl: identity(9),
                published: identity(9),
            }
        );

        // The exact locally produced child is then installed. It cancels the
        // pre-fence probe while Full remains stable; its matching fresh
        // publication refreshes Full's identities in place without a visible
        // mode transition.
        let effects = runner.handle_event(AcquisitionEvent::LclInstalled(identity(10)));
        assert!(
            !effects
                .iter()
                .any(|effect| matches!(effect, AcquisitionEffect::SetServicePhase(_))),
            "a newer local LCL must not emit a redundant Full -> Tracking transition"
        );
        assert!(effects.contains(&AcquisitionEffect::CancelSession(session)));
        assert_eq!(
            runner.phase(),
            &SyncPhase::Full {
                lcl: identity(9),
                published: identity(9),
            }
        );
        assert_eq!(
            runner
                .snapshot()
                .cancelled_by_reason()
                .get(&CancelReason::LclInstalled),
            Some(&1)
        );

        let effects = runner.handle_event(AcquisitionEvent::PublicationCommitted {
            identity: identity(10),
            fresh: true,
        });
        assert!(effects.is_empty());
        assert_eq!(
            runner.phase(),
            &SyncPhase::Full {
                lcl: identity(10),
                published: identity(10),
            }
        );
    }

    #[test]
    fn full_identity_refresh_requires_the_exact_local_lcl_fact() {
        let mut runner = CoordinatorRunner::with_phase(
            RunEpoch::new(1),
            SyncPhase::Full {
                lcl: identity(9),
                published: identity(9),
            },
        );

        // A publication by itself never authorizes a Full identity jump.
        let effects = runner.handle_event(AcquisitionEvent::PublicationCommitted {
            identity: identity(10),
            fresh: true,
        });
        assert!(effects.is_empty());
        assert_eq!(
            runner.phase(),
            &SyncPhase::Full {
                lcl: identity(9),
                published: identity(9),
            }
        );

        // The same fresh publication updates Full only after NetworkOps has
        // supplied the exact serialized local-LCL fact.
        let _ = runner.handle_event(AcquisitionEvent::LclInstalled(identity(10)));
        let effects = runner.handle_event(AcquisitionEvent::PublicationCommitted {
            identity: identity(10),
            fresh: true,
        });
        assert!(effects.is_empty());
        assert_eq!(
            runner.phase(),
            &SyncPhase::Full {
                lcl: identity(10),
                published: identity(10),
            }
        );
    }

    #[test]
    fn lcl_installed_and_publication_drive_tracking_and_full() {
        let mut runner = CoordinatorRunner::new(RunEpoch::new(1));
        connect(&mut runner);
        let session = acquire(&mut runner, 9);
        assert_eq!(runner.phase(), &SyncPhase::Syncing { target: target(9) });

        // LCL installation for a hash that is not the syncing target is ignored.
        let effects = runner.handle_event(AcquisitionEvent::LclInstalled(identity(99)));
        assert!(effects.is_empty());
        assert_eq!(runner.phase(), &SyncPhase::Syncing { target: target(9) });

        // The matching LCL install drives Syncing -> Tracking and cancels the
        // no-longer-needed pre-fence acquisition before its timeout can fire.
        let effects = runner.handle_event(AcquisitionEvent::LclInstalled(identity(9)));
        assert!(
            effects.contains(&AcquisitionEffect::SetServicePhase(SyncPhase::Tracking {
                lcl: identity(9)
            }))
        );
        assert!(effects.contains(&AcquisitionEffect::CancelSession(session)));
        assert_eq!(
            runner
                .snapshot()
                .cancelled_by_reason()
                .get(&CancelReason::LclInstalled),
            Some(&1)
        );

        // A newer chain head cannot be a contiguous published anchor for the
        // tracked LCL and is ignored.
        let effects = runner.handle_event(AcquisitionEvent::PublicationCommitted {
            identity: identity(99),
            fresh: true,
        });
        assert!(effects.is_empty());

        // A stale (non-fresh) publication never promotes Tracking -> Full even
        // for the matching chain head; the coordinator owns the freshness gate.
        let effects = runner.handle_event(AcquisitionEvent::PublicationCommitted {
            identity: identity(9),
            fresh: false,
        });
        assert!(effects.is_empty());
        assert_eq!(runner.phase(), &SyncPhase::Tracking { lcl: identity(9) });

        // The matching, fresh publication drives Tracking -> Full.
        let effects = runner.handle_event(AcquisitionEvent::PublicationCommitted {
            identity: identity(9),
            fresh: true,
        });
        assert_eq!(
            effects,
            vec![AcquisitionEffect::SetServicePhase(SyncPhase::Full {
                lcl: identity(9),
                published: identity(9)
            })]
        );
    }

    #[test]
    fn fresh_contiguous_published_anchor_behind_lcl_promotes_full() {
        let mut runner = CoordinatorRunner::new(RunEpoch::new(1));
        connect(&mut runner);
        acquire(&mut runner, 10);
        let _ = runner.handle_event(AcquisitionEvent::LclInstalled(identity(10)));

        // NetworkOps has proven ledger 9 is the contiguous published anchor of
        // the installed LCL. This is the post-restart condition from rippled
        // endConsensus: publication may lag the closed ledger while the open
        // ledger is fresh, and the node must still regain proposing mode.
        let effects = runner.handle_event(AcquisitionEvent::PublicationCommitted {
            identity: identity(9),
            fresh: true,
        });
        assert_eq!(
            effects,
            vec![AcquisitionEffect::SetServicePhase(SyncPhase::Full {
                lcl: identity(10),
                published: identity(9),
            })]
        );
    }

    #[test]
    fn newer_lcl_refreshes_tracking_identity_before_publication() {
        let mut runner = CoordinatorRunner::new(RunEpoch::new(1));
        connect(&mut runner);
        acquire(&mut runner, 9);
        let _ = runner.handle_event(AcquisitionEvent::LclInstalled(identity(9)));

        let effects = runner.handle_event(AcquisitionEvent::LclInstalled(identity(10)));
        assert_eq!(
            effects,
            vec![AcquisitionEffect::SetServicePhase(SyncPhase::Tracking {
                lcl: identity(10)
            })]
        );
        assert_eq!(runner.phase(), &SyncPhase::Tracking { lcl: identity(10) });

        let effects = runner.handle_event(AcquisitionEvent::PublicationCommitted {
            identity: identity(10),
            fresh: true,
        });
        assert_eq!(
            effects,
            vec![AcquisitionEffect::SetServicePhase(SyncPhase::Full {
                lcl: identity(10),
                published: identity(10),
            })]
        );
    }

    #[test]
    fn heartbeat_republishes_only_normalizable_phases() {
        let mut runner = CoordinatorRunner::new(RunEpoch::new(1));

        // Disconnected: no re-publish; recovery is owned by the connectivity
        // fact, not the heartbeat.
        let effects = runner.handle_event(AcquisitionEvent::Heartbeat);
        assert!(effects.is_empty());
        assert_eq!(runner.phase(), &SyncPhase::Disconnected);

        // Connected re-publishes so the phase port re-applies validated-age
        // normalization (rippled processHeartbeatTimer parity).
        connect(&mut runner);
        let effects = runner.handle_event(AcquisitionEvent::Heartbeat);
        assert_eq!(
            effects,
            vec![AcquisitionEffect::SetServicePhase(SyncPhase::Connected)]
        );
        assert_eq!(runner.phase(), &SyncPhase::Connected);

        // Syncing re-publishes for the same reason.
        acquire(&mut runner, 9);
        let effects = runner.handle_event(AcquisitionEvent::Heartbeat);
        assert_eq!(
            effects,
            vec![AcquisitionEffect::SetServicePhase(SyncPhase::Syncing {
                target: target(9)
            })]
        );

        // Tracking and Full are never reasserted (matching
        // heartbeat_operating_mode_reassertion).
        let _ = runner.handle_event(AcquisitionEvent::LclInstalled(identity(9)));
        let effects = runner.handle_event(AcquisitionEvent::Heartbeat);
        assert!(effects.is_empty());
        assert_eq!(runner.phase(), &SyncPhase::Tracking { lcl: identity(9) });
        let _ = runner.handle_event(AcquisitionEvent::PublicationCommitted {
            identity: identity(9),
            fresh: true,
        });
        let effects = runner.handle_event(AcquisitionEvent::Heartbeat);
        assert!(effects.is_empty());
        assert_eq!(
            runner.phase(),
            &SyncPhase::Full {
                lcl: identity(9),
                published: identity(9)
            }
        );
    }

    #[test]
    fn events_handled_counts_every_dispatched_event() {
        let mut runner = CoordinatorRunner::new(RunEpoch::new(1));
        connect(&mut runner);
        acquire(&mut runner, 1);
        acquire(&mut runner, 2);
        assert_eq!(runner.snapshot().events_handled(), 3);
        assert_eq!(runner.snapshot().sessions_started(), 2);
    }

    #[test]
    fn session_retains_the_timer_it_armed() {
        let mut runner = CoordinatorRunner::new(RunEpoch::new(1));
        connect(&mut runner);
        let effects = acquire_with_effects(&mut runner, 7);
        let session = peer_request_session(&effects);
        let timer_op = timer_operation(&effects);
        let (kind, pending) = runner
            .session(session)
            .expect("live session")
            .pending_timer()
            .expect("a timer must be armed");
        assert_eq!(kind, TimerKind::AcquireTimeout);
        assert_eq!(pending, timer_op);
    }

    /// A deterministic one-shot [`PlanSeed`]: the first Base packet builds a
    /// [`ScriptedEngine`] whose scripted steps run once per advance.
    #[derive(Debug, Clone)]
    struct ScriptedSeed {
        steps: VecDeque<ScriptedStep>,
        durable: Option<Arc<ledger::Ledger>>,
    }

    impl ScriptedSeed {
        fn new(steps: Vec<ScriptedStep>) -> Self {
            Self {
                steps: steps.into(),
                durable: None,
            }
        }

        /// Attaches a durable ledger the built engine yields at the M5 handoff.
        fn with_durable_ledger(mut self, ledger: Arc<ledger::Ledger>) -> Self {
            self.durable = Some(ledger);
            self
        }
    }

    impl PlanSeed for ScriptedSeed {
        fn build(
            &mut self,
            session: SessionRef,
            _header: &ledger::InboundLedgerPacket,
        ) -> Option<Box<dyn TreeEngine + Send + Sync>> {
            let engine = ScriptedEngine::new(
                TreePlanId::new(session.session_id().get() + 1),
                std::mem::take(&mut self.steps),
                Vec::new(),
            )
            .with_persistence_sequence(1);
            let engine = match self.durable.take() {
                Some(durable) => engine.with_durable_ledger(durable),
                None => engine,
            };
            Some(Box::new(engine))
        }
    }

    fn read_effects(effects: &[AcquisitionEffect]) -> Vec<crate::io::ReadRequest> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                AcquisitionEffect::SubmitRead(request) => Some(request.clone()),
                _ => None,
            })
            .collect()
    }

    fn write_batch(effects: &[AcquisitionEffect]) -> crate::io::WriteBatch {
        effects
            .iter()
            .find_map(|effect| match effect {
                AcquisitionEffect::SubmitWrite(batch) => Some(batch.clone()),
                _ => None,
            })
            .expect("a write batch effect must be emitted")
    }

    fn durable_handoff(effects: &[AcquisitionEffect]) -> DurableLedger {
        effects
            .iter()
            .find_map(|effect| match effect {
                AcquisitionEffect::PublishDurable(ledger) => Some(ledger.clone()),
                _ => None,
            })
            .expect("a durable handoff effect must be emitted")
    }

    fn immutable_ledger(seq: u32) -> Arc<ledger::Ledger> {
        let header = ledger::LedgerHeader {
            seq,
            ..ledger::LedgerHeader::default()
        };
        let mut ledger = ledger::Ledger::new(header, false);
        ledger.set_immutable(true);
        Arc::new(ledger)
    }

    #[test]
    fn base_packet_seeds_the_plan_and_drives_a_read() {
        let budget = BudgetState::new(8, AdmissionBudget::new(4, 1024), Duration::from_secs(1));
        let mut runner = CoordinatorRunner::with_plan_seed(
            RunEpoch::new(1),
            budget,
            Box::new(ScriptedSeed::new(vec![ScriptedStep::NeedsReads(vec![
                PlanReadNeed::new(
                    SHAMapHash::new(Uint256::from(7)),
                    10,
                    SHAMapNodeId::default(),
                    0,
                ),
            ])])),
        );
        connect(&mut runner);
        let session = acquire(&mut runner, 10);

        let packet = admitted_packet(session, AdmissionBudget::new(1, 256), 8);
        let effects = runner.handle_event(AcquisitionEvent::PacketAdmitted(packet));
        let reads = read_effects(&effects);
        assert_eq!(reads.len(), 1);
        assert_eq!(reads[0].operation().kind(), OperationKind::Read);
        assert_eq!(reads[0].key(), SHAMapHash::new(Uint256::from(7)));
        assert_eq!(reads[0].ledger_sequence(), 10);
        // The header packet was fed and consumed; no queue remains.
        assert_eq!(runner.session(session).expect("live").packet_count(), 0);
        assert_eq!(runner.snapshot().plan_turns(), 1);
    }

    #[test]
    fn read_completion_must_match_the_exact_inflight_operation() {
        let budget = BudgetState::new(8, AdmissionBudget::new(4, 1024), Duration::from_secs(1));
        let mut runner = CoordinatorRunner::with_plan_seed(
            RunEpoch::new(1),
            budget,
            Box::new(ScriptedSeed::new(vec![ScriptedStep::NeedsReads(vec![
                PlanReadNeed::new(
                    SHAMapHash::new(Uint256::from(7)),
                    10,
                    SHAMapNodeId::default(),
                    0,
                ),
            ])])),
        );
        connect(&mut runner);
        let session = acquire(&mut runner, 10);
        let packet = admitted_packet(session, AdmissionBudget::new(1, 256), 8);
        let effects = runner.handle_event(AcquisitionEvent::PacketAdmitted(packet));
        let reads = read_effects(&effects);
        let inflight = reads[0].operation();

        // A different read operation for the same session is stale: a target
        // hash alone never authorizes a result.
        let wrong = OperationRef::new(
            session,
            OperationKind::Read,
            OperationId::new(999),
            OperationGeneration::new(1),
        );
        runner.handle_event(AcquisitionEvent::ReadCompleted(ReadCompletion::new(
            wrong,
            ReadOutcome::Settled { node: None },
        )));
        assert_eq!(runner.snapshot().stale_events(), 1);

        // The exact in-flight operation applies and the plan advances.
        runner.handle_event(AcquisitionEvent::ReadCompleted(ReadCompletion::new(
            inflight,
            ReadOutcome::Settled { node: None },
        )));
        assert_eq!(runner.snapshot().stale_events(), 1);
        assert_eq!(runner.snapshot().plan_turns(), 2);
    }

    #[test]
    fn fetch_pack_fact_re_advances_live_sessions_only() {
        let budget = BudgetState::new(8, AdmissionBudget::new(4, 1024), Duration::from_secs(1));
        let mut runner = CoordinatorRunner::with_plan_seed(
            RunEpoch::new(1),
            budget,
            Box::new(ScriptedSeed::new(vec![
                ScriptedStep::NeedsNetwork(vec![(SHAMapNodeId::default(), Uint256::from(3))]),
                ScriptedStep::NeedsReads(vec![PlanReadNeed::new(
                    SHAMapHash::new(Uint256::from(7)),
                    10,
                    SHAMapNodeId::default(),
                    0,
                )]),
            ])),
        );
        connect(&mut runner);
        let session = acquire(&mut runner, 10);

        // A fetch-pack fact before any live session has a rooted engine is a
        // no-op: there is nothing to re-advance.
        let effects = runner.handle_event(AcquisitionEvent::FetchPackAvailable);
        assert!(effects.is_empty());
        assert_eq!(runner.snapshot().fetch_pack_advances(), 0);

        // Seed the engine with a header packet; the first advance announces
        // network candidates and blocks on them.
        let packet = admitted_packet(session, AdmissionBudget::new(1, 256), 8);
        let effects = runner.handle_event(AcquisitionEvent::PacketAdmitted(packet));
        assert!(
            effects
                .iter()
                .any(|effect| matches!(effect, AcquisitionEffect::SendLedgerRequest(_))),
            "the first advance announces network candidates"
        );
        assert_eq!(runner.snapshot().plan_turns(), 1);

        // The fetch-pack fact names no session but re-advances every live
        // session, so a session blocked on network needs takes another bounded
        // traversal pass and can announce freshly resolvable work.
        let effects = runner.handle_event(AcquisitionEvent::FetchPackAvailable);
        let reads = read_effects(&effects);
        assert_eq!(reads.len(), 1);
        assert_eq!(reads[0].key(), SHAMapHash::new(Uint256::from(7)));
        assert_eq!(runner.snapshot().fetch_pack_advances(), 1);
        assert_eq!(runner.snapshot().plan_turns(), 2);

        // After the session is cancelled a later fetch-pack fact is a no-op.
        let cancel = runner.handle_event(AcquisitionEvent::Shutdown);
        assert!(
            cancel.iter().any(|effect| {
                matches!(effect, AcquisitionEffect::CancelSession(s) if *s == session)
            }),
            "shutdown cancels the live session"
        );
        assert!(
            runner
                .handle_event(AcquisitionEvent::FetchPackAvailable)
                .is_empty()
        );
    }

    #[test]
    fn run_plan_turn_batches_reply_node_requests_at_rippled_128_limit() {
        let needs = (1..=129u64)
            .map(|hash| {
                PlanNetworkNeed::new(
                    SHAMapNodeId::default(),
                    Uint256::from(hash),
                    ledger::TreeKind::State,
                )
            })
            .collect::<Vec<_>>();
        let budget = BudgetState::new(8, AdmissionBudget::new(4, 1024), Duration::from_secs(1));
        let mut runner = CoordinatorRunner::with_plan_seed(
            RunEpoch::new(1),
            budget,
            Box::new(ScriptedSeed::new(vec![ScriptedStep::NeedsNetworkWithKind(
                needs,
            )])),
        );
        connect(&mut runner);
        let session = acquire(&mut runner, 10);
        let effects = runner.handle_event(AcquisitionEvent::PacketAdmitted(admitted_packet(
            session,
            AdmissionBudget::new(1, 256),
            8,
        )));
        let batches = effects
            .iter()
            .filter_map(|effect| match effect {
                AcquisitionEffect::SendLedgerRequest(request)
                    if matches!(request.request(), LedgerDataRequest::GetLedgerNodes { .. }) =>
                {
                    match request.request() {
                        LedgerDataRequest::GetLedgerNodes { node_ids, .. } => Some(node_ids.len()),
                        _ => unreachable!("matched ledger-node request"),
                    }
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(batches, vec![128, 1]);
    }

    #[test]
    fn run_plan_turn_batches_non_reply_node_requests_at_rippled_12_limit() {
        let needs = (1..=13u64)
            .map(|hash| {
                PlanNetworkNeed::new(
                    SHAMapNodeId::default(),
                    Uint256::from(hash),
                    ledger::TreeKind::State,
                )
            })
            .collect::<Vec<_>>();
        let budget = BudgetState::new(8, AdmissionBudget::new(4, 1024), Duration::from_secs(1));
        let mut runner = CoordinatorRunner::with_plan_seed(
            RunEpoch::new(1),
            budget,
            Box::new(ScriptedSeed::new(vec![
                ScriptedStep::NeedsNetworkWithKind(vec![]),
                ScriptedStep::NeedsNetworkWithKind(needs),
            ])),
        );
        connect(&mut runner);
        let session = acquire(&mut runner, 10);
        let _ = runner.handle_event(AcquisitionEvent::PacketAdmitted(admitted_packet(
            session,
            AdmissionBudget::new(1, 256),
            8,
        )));
        let effects = runner.handle_event(AcquisitionEvent::FetchPackAvailable);
        let batches = effects
            .iter()
            .filter_map(|effect| match effect {
                AcquisitionEffect::SendLedgerRequest(request)
                    if matches!(request.request(), LedgerDataRequest::GetLedgerNodes { .. }) =>
                {
                    match request.request() {
                        LedgerDataRequest::GetLedgerNodes { node_ids, .. } => Some(node_ids.len()),
                        _ => unreachable!("matched ledger-node request"),
                    }
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(batches, vec![12, 1]);
    }

    #[test]
    fn durable_fence_hands_off_once_and_ack_completes() {
        let budget = BudgetState::new(8, AdmissionBudget::new(4, 1024), Duration::from_secs(1));
        let ledger = immutable_ledger(10);
        let seed = ScriptedSeed::new(vec![ScriptedStep::Complete])
            .with_durable_ledger(Arc::clone(&ledger));
        let mut runner =
            CoordinatorRunner::with_plan_seed(RunEpoch::new(1), budget, Box::new(seed));
        connect(&mut runner);
        let session = acquire(&mut runner, 10);
        let packet = admitted_packet(session, AdmissionBudget::new(1, 256), 8);
        let effects = runner.handle_event(AcquisitionEvent::PacketAdmitted(packet));
        let batch = write_batch(&effects);
        assert_eq!(batch.operation().kind(), OperationKind::Write);
        assert_eq!(
            batch.fence().expect("final batch fence").kind(),
            OperationKind::DurabilityFence
        );
        assert!(matches!(
            runner.session(session).expect("live").phase(),
            SessionPhase::Persisting
        ));

        // A write completion that is not the exact in-flight write is stale.
        let wrong = OperationRef::new(
            session,
            OperationKind::Write,
            OperationId::new(999),
            OperationGeneration::new(1),
        );
        runner.handle_event(AcquisitionEvent::WriteCompleted(WriteCompletion::new(
            wrong,
            WriteOutcome::Accepted,
        )));
        assert_eq!(runner.snapshot().stale_events(), 1);

        // The exact write completion moves the persistence state to FencePending.
        let effects = runner.handle_event(AcquisitionEvent::WriteCompleted(WriteCompletion::new(
            batch.operation(),
            WriteOutcome::Accepted,
        )));
        assert!(effects.is_empty());

        // A fence completion for a different operation is stale.
        let wrong_fence = OperationRef::new(
            session,
            OperationKind::DurabilityFence,
            OperationId::new(999),
            OperationGeneration::new(1),
        );
        runner.handle_event(AcquisitionEvent::DurabilityFenced(
            DurabilityCompletion::new(wrong_fence, crate::io::DurabilityOutcome::Passed),
        ));
        assert_eq!(runner.snapshot().stale_events(), 2);

        // The exact fence moves Persisting -> DurablePending and emits exactly
        // one PublishDurable handoff: a unique id plus the durable ledger. The
        // ledger is never normal-adoptable before this fence.
        let effects = runner.handle_event(AcquisitionEvent::DurabilityFenced(
            DurabilityCompletion::new(
                batch.fence().expect("final batch fence"),
                crate::io::DurabilityOutcome::Passed,
            ),
        ));
        let published = durable_handoff(&effects);
        assert!(Arc::ptr_eq(published.ledger(), &ledger));
        assert!(!published.handoff().is_invalid());
        assert!(matches!(
            runner.session(session).expect("live").phase(),
            SessionPhase::DurablePending
        ));

        // A handoff ack carrying a different id is stale and cannot finalize.
        runner.handle_event(AcquisitionEvent::DurableHandoffAcknowledged(
            DurableHandoffAcknowledgement::new(DurableHandoffId::new(999), session),
        ));
        assert_eq!(runner.snapshot().stale_events(), 3);
        assert!(matches!(
            runner.session(session).expect("live").phase(),
            SessionPhase::DurablePending
        ));

        // The exact pending handoff id authorizes DurablePending -> Complete:
        // one logical terminal durable result for this successful session.
        runner.handle_event(AcquisitionEvent::DurableHandoffAcknowledged(
            DurableHandoffAcknowledgement::new(published.handoff(), session),
        ));
        assert!(matches!(
            runner.session(session).expect("live").phase(),
            SessionPhase::Complete
        ));
        assert_eq!(runner.snapshot().sessions_completed(), 1);
    }

    #[test]
    fn peer_loss_preserves_durable_pending_handoff_retry_state() {
        let budget = BudgetState::new(8, AdmissionBudget::new(4, 1024), Duration::from_secs(1));
        let ledger = immutable_ledger(12);
        let seed = ScriptedSeed::new(vec![ScriptedStep::Complete])
            .with_durable_ledger(Arc::clone(&ledger));
        let mut runner =
            CoordinatorRunner::with_plan_seed(RunEpoch::new(1), budget, Box::new(seed));
        connect(&mut runner);
        let session = acquire(&mut runner, 12);
        let effects = runner.handle_event(AcquisitionEvent::PacketAdmitted(admitted_packet(
            session,
            AdmissionBudget::new(1, 256),
            8,
        )));
        let batch = write_batch(&effects);
        runner.handle_event(AcquisitionEvent::WriteCompleted(WriteCompletion::new(
            batch.operation(),
            WriteOutcome::Accepted,
        )));
        let effects = runner.handle_event(AcquisitionEvent::DurabilityFenced(
            DurabilityCompletion::new(
                batch.fence().expect("final batch fence"),
                crate::io::DurabilityOutcome::Passed,
            ),
        ));
        let published = durable_handoff(&effects);
        let retry_effects = runner.handle_event(AcquisitionEvent::DurableHandoffRejected {
            handoff: published.handoff(),
            session,
            reason: HandoffRejectReason::ChannelFull,
        });
        let retry = timer_operation(&retry_effects);

        let effects = runner.handle_event(AcquisitionEvent::Connectivity(
            PeerAvailabilitySnapshot::new(vec![]),
        ));
        assert!(effects.contains(&AcquisitionEffect::SetServicePhase(SyncPhase::Disconnected)));
        assert!(
            !effects.contains(&AcquisitionEffect::CancelSession(session)),
            "peer loss must not revoke a post-fence durable handoff"
        );
        assert!(matches!(
            runner.session(session).expect("durable session").phase(),
            SessionPhase::DurablePending
        ));
        assert_eq!(
            runner
                .session(session)
                .expect("durable session")
                .pending_timer(),
            Some((TimerKind::HandoffRetry, retry))
        );
        assert_eq!(
            runner.handle_event(AcquisitionEvent::TimerFired {
                operation: retry,
                timer: TimerKind::HandoffRetry,
            }),
            vec![AcquisitionEffect::PublishDurable(published)]
        );
    }

    #[test]
    fn handoff_rejection_arms_one_exact_retry_before_republish() {
        let budget = BudgetState::new(8, AdmissionBudget::new(4, 1024), Duration::from_secs(1));
        let ledger = immutable_ledger(11);
        let seed = ScriptedSeed::new(vec![ScriptedStep::Complete])
            .with_durable_ledger(Arc::clone(&ledger));
        let mut runner =
            CoordinatorRunner::with_plan_seed(RunEpoch::new(1), budget, Box::new(seed));
        connect(&mut runner);
        let session = acquire(&mut runner, 11);
        let packet = admitted_packet(session, AdmissionBudget::new(1, 256), 8);
        let effects = runner.handle_event(AcquisitionEvent::PacketAdmitted(packet));
        let batch = write_batch(&effects);
        runner.handle_event(AcquisitionEvent::WriteCompleted(WriteCompletion::new(
            batch.operation(),
            WriteOutcome::Accepted,
        )));
        let effects = runner.handle_event(AcquisitionEvent::DurabilityFenced(
            DurabilityCompletion::new(
                batch.fence().expect("final batch fence"),
                crate::io::DurabilityOutcome::Passed,
            ),
        ));
        let published = durable_handoff(&effects);
        assert!(matches!(
            runner.session(session).expect("live").phase(),
            SessionPhase::DurablePending
        ));
        // A repeated exact demand coalesces even after durability: it cannot
        // cancel or replace a committed handoff awaiting acknowledgement.
        let duplicate = runner.handle_event(AcquisitionEvent::AcquireRequested {
            target: target(11),
            reason: AcquireReason::Consensus,
        });
        assert!(duplicate.iter().all(|effect| {
            !matches!(
                effect,
                AcquisitionEffect::CancelSession(_) | AcquisitionEffect::SendLedgerRequest(_)
            )
        }));
        assert!(matches!(
            runner.session(session).expect("live").phase(),
            SessionPhase::DurablePending
        ));

        // A rejection carrying a different handoff id is stale and never
        // re-publishes: only the exact pending id authorizes retry.
        let effects = runner.handle_event(AcquisitionEvent::DurableHandoffRejected {
            handoff: DurableHandoffId::new(777),
            session,
            reason: HandoffRejectReason::ChannelFull,
        });
        assert!(effects.is_empty());
        assert_eq!(runner.snapshot().stale_events(), 1);

        // The exact pending id arms one fresh retry timer; rejection itself
        // cannot re-publish. A duplicate rejection while that timer is armed is
        // stale, preventing retry storms.
        let effects = runner.handle_event(AcquisitionEvent::DurableHandoffRejected {
            handoff: published.handoff(),
            session,
            reason: HandoffRejectReason::ChannelFull,
        });
        let retry = timer_operation(&effects);
        assert!(
            effects
                .iter()
                .all(|effect| !matches!(effect, AcquisitionEffect::PublishDurable(_)))
        );
        assert_eq!(
            runner.session(session).expect("live").pending_timer(),
            Some((TimerKind::HandoffRetry, retry))
        );
        assert_eq!(runner.snapshot().handoff_rejections(), 1);
        let duplicate = runner.handle_event(AcquisitionEvent::DurableHandoffRejected {
            handoff: published.handoff(),
            session,
            reason: HandoffRejectReason::ChannelFull,
        });
        assert!(duplicate.is_empty());
        assert_eq!(runner.snapshot().stale_events(), 2);

        // Only the exact retry wakeup re-publishes the same durable handoff.
        let effects = runner.handle_event(AcquisitionEvent::TimerFired {
            operation: retry,
            timer: TimerKind::HandoffRetry,
        });
        assert_eq!(
            effects,
            vec![AcquisitionEffect::PublishDurable(published.clone())]
        );
        let stale_retry = runner.handle_event(AcquisitionEvent::TimerFired {
            operation: retry,
            timer: TimerKind::HandoffRetry,
        });
        assert!(stale_retry.is_empty());
        assert_eq!(runner.snapshot().stale_events(), 3);
        assert!(matches!(
            runner.session(session).expect("live").phase(),
            SessionPhase::DurablePending
        ));

        // The eventual ack still finalizes exactly once; the retry did not
        // duplicate adoption.
        runner.handle_event(AcquisitionEvent::DurableHandoffAcknowledged(
            DurableHandoffAcknowledgement::new(published.handoff(), session),
        ));
        assert!(matches!(
            runner.session(session).expect("live").phase(),
            SessionPhase::Complete
        ));
        assert_eq!(runner.snapshot().sessions_completed(), 1);
    }

    #[test]
    fn write_failure_terminalizes_without_adoption() {
        let budget = BudgetState::new(8, AdmissionBudget::new(4, 1024), Duration::from_secs(1));
        let mut runner = CoordinatorRunner::with_plan_seed(
            RunEpoch::new(1),
            budget,
            Box::new(ScriptedSeed::new(vec![ScriptedStep::Complete])),
        );
        connect(&mut runner);
        let session = acquire(&mut runner, 10);
        let packet = admitted_packet(session, AdmissionBudget::new(1, 256), 8);
        let effects = runner.handle_event(AcquisitionEvent::PacketAdmitted(packet));
        let batch = write_batch(&effects);

        let effects = runner.handle_event(AcquisitionEvent::WriteCompleted(WriteCompletion::new(
            batch.operation(),
            WriteOutcome::Failed,
        )));
        assert!(effects.contains(&AcquisitionEffect::CancelSession(session)));
        assert!(matches!(
            runner.session(session).expect("live").phase(),
            SessionPhase::Failed { .. }
        ));
        assert_eq!(
            runner.snapshot().failed_by_reason(),
            &BTreeMap::from([(FailureReason::WriteFailure, 1)])
        );

        // A late fence for the cancelled session is stale.
        runner.handle_event(AcquisitionEvent::DurabilityFenced(
            DurabilityCompletion::new(
                batch.fence().expect("final batch fence"),
                crate::io::DurabilityOutcome::Passed,
            ),
        ));
        assert_eq!(runner.snapshot().stale_events(), 1);
    }

    #[test]
    fn unseeded_base_packet_does_not_reset_the_acquisition_timeout_budget() {
        let mut runner = CoordinatorRunner::new(RunEpoch::new(1));
        connect(&mut runner);
        let effects = acquire_with_effects(&mut runner, 10);
        let session = peer_request_session(&effects);
        let mut timer = timer_operation(&effects);

        // Consume six no-progress recovery intervals, leaving the seventh
        // interval to fail. An unseeded/invalid Base packet is merely admitted;
        // it must not reset this budget because no verified header was retained.
        for _ in 0..6 {
            let effects = runner.handle_event(AcquisitionEvent::TimerFired {
                operation: timer,
                timer: TimerKind::AcquireTimeout,
            });
            timer = timer_operation(&effects);
        }
        let packet = admitted_packet(session, AdmissionBudget::new(1, 256), 8);
        let _ = runner.handle_event(AcquisitionEvent::PacketAdmitted(packet));

        let effects = runner.handle_event(AcquisitionEvent::TimerFired {
            operation: timer,
            timer: TimerKind::AcquireTimeout,
        });
        assert!(effects.contains(&AcquisitionEffect::CancelSession(session)));
        assert!(
            runner
                .session(session)
                .expect("session")
                .phase()
                .is_terminal()
        );
    }

    #[test]
    fn acquire_timeout_rearms_then_fails_after_the_budget() {
        let mut runner = CoordinatorRunner::new(RunEpoch::new(1));
        connect(&mut runner);
        let effects = acquire_with_effects(&mut runner, 10);
        let session = peer_request_session(&effects);
        let mut timer_op = timer_operation(&effects);

        // DEFAULT_MAX_ACQUIRE_TIMEOUTS permits six no-progress recovery
        // intervals. The seventh fires terminalize the session.
        for _ in 0..6 {
            let effects = runner.handle_event(AcquisitionEvent::TimerFired {
                operation: timer_op,
                timer: TimerKind::AcquireTimeout,
            });
            let rearm = effects
                .iter()
                .find_map(|effect| match effect {
                    AcquisitionEffect::ArmTimer(request) => Some(request.clone().operation()),
                    _ => None,
                })
                .expect("the deadline must be rearmed");
            assert_ne!(rearm, timer_op);
            timer_op = rearm;
        }
        assert!(
            !runner
                .session(session)
                .expect("live session")
                .phase()
                .is_terminal()
        );

        let effects = runner.handle_event(AcquisitionEvent::TimerFired {
            operation: timer_op,
            timer: TimerKind::AcquireTimeout,
        });
        assert!(effects.contains(&AcquisitionEffect::CancelSession(session)));
        assert!(matches!(
            runner.session(session).expect("live").phase(),
            SessionPhase::Failed { .. }
        ));
    }
}
