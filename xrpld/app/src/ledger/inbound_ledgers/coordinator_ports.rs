//! M4.2-C1 production adapter ports: the app-side execution layer for the
//! coordinator's write, timer, and phase effects, plus the production assembly
//! of every port into a live [`CoordinatorAdapter`].
//!
//! Ownership boundary: each port owns only resource-local state. The
//! [`CoordinatorWritePort`] owns the per-session persistence FIFO and the
//! NodeStore submission; the [`CoordinatorTimerPort`] owns arming identity
//! tracking over the shared [`WorkerPool`] timer service; the
//! [`CoordinatorPhasePort`] owns the one production mode writer. No port holds
//! a lock while invoking coordinator logic, and completions return as typed
//! events the coordinator validates by exact `SessionRef`/`OperationRef`.
//!
//! Rippled parity notes: the write path mirrors `InboundLedger`'s NodeStore
//! fence ordering (one in-flight persistence ticket per session, the final
//! durability barrier ordered after every accepted write); the phase mapping
//! mirrors `NetworkOPs::setOperatingMode` values; timer wakeups only enqueue a
//! typed completion (`TimeoutCounter` parity), they never run session logic.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use ledger::FetchPackCache;
use nodestore::{NodeObjectType, PersistenceWork};
use overlay::{Peer, SimplePeerSet};

use acquisition::{
    CancellationPort, CoordinatorRunner, DurabilityCompletion, DurabilityOutcome, OperationRef,
    PhasePort, SessionRef, StoredObjectKind, SyncPhase, TimerPort, TimerRequest, WriteBatch,
    WriteCompletion, WriteOutcome,
};

use crate::network::network_ops::{NetworkOpsOperatingMode, SharedNetworkOpsState};

use super::coordinator_adapter::{
    BrokerCancellationDispatcher, BrokerReadPort, BrokerTicketState, CONTROL_EVENT_QUEUE_CAPACITY,
    CoordinatorAdapter, EventSender, OverlayLedgerRequestPort, PACKET_INGRESS_QUEUE_CAPACITY,
};
use super::coordinator_handoff::CoordinatorHandoffPort;
use super::read_broker::NodeReadBroker;
use super::registry::CompletedInboundLedger;
use super::worker_pool::WorkerPool;
use crate::shamap::shamap_store_backend::SHAMapStoreNodeStore;

/// The production instantiation of the coordinator adapter.
pub(crate) type ProductionAdapter = CoordinatorAdapter<
    OverlayLedgerRequestPort,
    BrokerReadPort,
    CoordinatorWritePort,
    CoordinatorTimerPort,
    CoordinatorHandoffPort,
    CoordinatorPhasePort,
    CoordinatorCancellationDispatcher,
>;

// ─── Production adapter convenience (registry wiring) ──────────────────────

impl
    CoordinatorAdapter<
        OverlayLedgerRequestPort,
        BrokerReadPort,
        CoordinatorWritePort,
        CoordinatorTimerPort,
        CoordinatorHandoffPort,
        CoordinatorPhasePort,
        CoordinatorCancellationDispatcher,
    >
{
    /// Register app-side origin metadata before submitting an acquisition
    /// demand. The handoff port binds it to the exact `SessionStarted` effect
    /// before any peer request is dispatched.
    pub(crate) fn register_pending_handoff_origin(
        &mut self,
        target: basics::base_uint::Uint256,
        reason: super::registry::AcquireReason,
        acquisition_id: u64,
    ) {
        self.handoffs
            .register_pending_session_origin(target, reason, acquisition_id);
    }

    /// Clear an origin binding when a peer-capable demand did not create a
    /// session, keeping the handoff port's deferred state bounded.
    pub(crate) fn clear_pending_handoff_origin(&mut self, target: basics::base_uint::Uint256) {
        self.handoffs.clear_pending_session_origin(target);
    }

    /// Reopens an exact handoff after the recipient could not accept it, then
    /// queues a typed retry fact without running the owner under a receiver
    /// lock. A stale session or already-consumed handoff is ignored.
    pub(crate) fn recipient_rejected_durable_handoff(
        &mut self,
        handoff: acquisition::DurableHandoffId,
        session: SessionRef,
    ) -> bool {
        if !self
            .handoffs
            .reopen_after_recipient_rejection(handoff, session)
        {
            return false;
        }
        self.handle_fact(acquisition::AcquisitionEvent::DurableHandoffRejected {
            handoff,
            session,
            reason: acquisition::HandoffRejectReason::RecipientRejected,
        });
        true
    }

    /// Refresh the overlay peer set the request port delivers to. This is the
    /// connectivity fact feed: overlay remains the socket owner, the coordinator
    /// consumes availability as an input fact.
    pub(crate) fn refresh_peers(&self, peers: Vec<Arc<dyn Peer>>) {
        self.requests.refresh_peers(peers);
    }
}

// ─── Write port ─────────────────────────────────────────────────────────────

/// One submitted coordinator write batch retained by a session's persistence
/// FIFO. A batch carries both its write operation and its durability fence
/// operation, so one NodeStore submission settles both completions.
#[derive(Debug, Clone)]
struct PersistenceBatch {
    batch: WriteBatch,
}

/// Per-session persistence FIFO. Exactly one batch is in flight, so a passed
/// durability fence is ordered after every accepted write of the same session.
#[derive(Debug, Default)]
struct SessionPersistence {
    queued: VecDeque<PersistenceBatch>,
    in_flight: Option<OperationRef>,
    // Exact write/fence outcomes that could not enter the bounded coordinator
    // control lane. This is resource-local transport state, not lifecycle
    // state: the coordinator remains the sole validator of each completion.
    completions: VecDeque<acquisition::AcquisitionEvent>,
}

/// All live sessions' persistence state. Resource-local: the coordinator owns
/// write intent and retry policy; this owns only submission accounting.
#[derive(Debug, Default)]
struct CoordinatorPersistence {
    by_session: HashMap<SessionRef, SessionPersistence>,
    // Session IDs with at least one retained completion. The queue provides
    // bounded round-robin replay across sessions while each session's own
    // write/fence order stays intact.
    ready_completion_sessions: VecDeque<SessionRef>,
}

/// The work record the NodeStore scheduler executes. It performs the physical
/// writes and the fence, posts the typed completions, and only then advances
/// the session's FIFO. Holding no port lock during the physical work preserves
/// the "no I/O under a coordinator-adjacent lock" rule.
struct CoordinatorPersistenceWork {
    node_store: SHAMapStoreNodeStore,
    batch: WriteBatch,
    pending: Arc<Mutex<CoordinatorPersistence>>,
    tx: EventSender,
}

impl CoordinatorPersistenceWork {
    fn payload_bytes(batch: &WriteBatch) -> usize {
        batch.nodes().iter().map(|node| node.data().len()).sum()
    }
}

fn nodestore_object_type(kind: StoredObjectKind) -> NodeObjectType {
    match kind {
        StoredObjectKind::Ledger => NodeObjectType::Ledger,
        StoredObjectKind::AccountNode => NodeObjectType::AccountNode,
        StoredObjectKind::TransactionNode => NodeObjectType::TransactionNode,
        StoredObjectKind::Unknown => NodeObjectType::Unknown,
    }
}

impl PersistenceWork for CoordinatorPersistenceWork {
    fn retained_payload_bytes(&self) -> usize {
        Self::payload_bytes(&self.batch)
    }

    fn run(self: Box<Self>) {
        let Self {
            node_store,
            batch,
            pending,
            tx,
        } = *self;
        let session = batch.operation().session();
        let (write_outcome, fence_outcome): (WriteOutcome, Option<DurabilityOutcome>) =
            if node_store.store_generation() != batch.store_generation().get() {
                // A store rotation between admission and execution invalidates
                // this generation-scoped batch.
                (
                    WriteOutcome::Cancelled,
                    batch.fence().map(|_| DurabilityOutcome::Stale),
                )
            } else {
                let mut write_failed = None;
                for node in batch.nodes() {
                    if let Err(error) = match &node_store {
                        SHAMapStoreNodeStore::Single(database) => database.store(
                            nodestore_object_type(node.object_kind()),
                            node.data().to_vec(),
                            *node.key().as_uint256(),
                            batch.ledger_sequence(),
                        ),
                        SHAMapStoreNodeStore::Rotating(database) => database.store(
                            nodestore_object_type(node.object_kind()),
                            node.data().to_vec(),
                            *node.key().as_uint256(),
                            batch.ledger_sequence(),
                        ),
                    } {
                        write_failed = Some(error);
                        break;
                    }
                }
                match (write_failed, batch.fence()) {
                    (Some(_), Some(_)) => (WriteOutcome::Failed, Some(DurabilityOutcome::Failed)),
                    (Some(_), None) => (WriteOutcome::Failed, None),
                    (None, Some(_)) => {
                        let fence = match &node_store {
                            SHAMapStoreNodeStore::Single(database) => database.sync_result(),
                            SHAMapStoreNodeStore::Rotating(database) => database.sync_result(),
                        };
                        match fence {
                            Ok(()) => (WriteOutcome::Accepted, Some(DurabilityOutcome::Passed)),
                            Err(_) => (WriteOutcome::Failed, Some(DurabilityOutcome::Failed)),
                        }
                    }
                    (None, None) => (WriteOutcome::Accepted, None),
                }
            };
        // Retain the exact ordered pair before releasing the FIFO. `try_send`
        // cannot block a NodeStore worker behind the coordinator that drains
        // its own control lane; an owner turn later flushes any retained pair.
        let next = {
            let mut state = pending.lock().expect("coordinator persistence lock");
            let was_empty = {
                let entry = state.by_session.entry(session).or_default();
                let was_empty = entry.completions.is_empty();
                entry
                    .completions
                    .push_back(acquisition::AcquisitionEvent::WriteCompleted(
                        WriteCompletion::new(batch.operation(), write_outcome),
                    ));
                if let (Some(fence), Some(fence_outcome)) = (batch.fence(), fence_outcome) {
                    entry
                        .completions
                        .push_back(acquisition::AcquisitionEvent::DurabilityFenced(
                            DurabilityCompletion::new(fence, fence_outcome),
                        ));
                }
                was_empty
            };
            if was_empty {
                state.ready_completion_sessions.push_back(session);
            }

            // Advance the session FIFO only if this exact operation is still
            // in flight. A cancelled session has no slot; its late completion
            // is still replayed and rejected as stale by the coordinator, but
            // it must not dispatch a successor.
            let entry = state
                .by_session
                .get_mut(&session)
                .expect("persistence entry was inserted above");
            if entry.in_flight != Some(batch.operation()) {
                None
            } else {
                entry.in_flight = None;
                entry.queued.pop_front().map(|command| command.batch)
            }
        };
        flush_persistence_completions(&pending, &tx);
        if let Some(next) = next {
            dispatch_persistence(&node_store, &pending, &tx, next);
        }
    }
}

/// Make one bounded round-robin pass over retained exact completions. Holding
/// the narrow persistence lock only spans a nonblocking channel operation, so
/// no worker or coordinator waits for its own consumer.
fn flush_persistence_completions(pending: &Arc<Mutex<CoordinatorPersistence>>, tx: &EventSender) {
    let mut state = pending.lock().expect("coordinator persistence lock");
    // One pass is bounded by the control lane itself. Requeueing a session after
    // each successful send gives every live session a turn while allowing an
    // empty lane to receive a complete write/fence pair immediately.
    for _ in 0..CONTROL_EVENT_QUEUE_CAPACITY {
        let Some(session) = state.ready_completion_sessions.pop_front() else {
            break;
        };
        let Some(event) = state
            .by_session
            .get_mut(&session)
            .and_then(|entry| entry.completions.pop_front())
        else {
            continue;
        };
        match tx.try_send(event) {
            Ok(()) => {
                let remove = {
                    let entry = state
                        .by_session
                        .get(&session)
                        .expect("completion session remains while replaying");
                    entry.completions.is_empty()
                        && entry.queued.is_empty()
                        && entry.in_flight.is_none()
                };
                if remove {
                    state.by_session.remove(&session);
                } else {
                    state.ready_completion_sessions.push_back(session);
                }
            }
            Err(mpsc::TrySendError::Full(event)) | Err(mpsc::TrySendError::Disconnected(event)) => {
                state
                    .by_session
                    .entry(session)
                    .or_default()
                    .completions
                    .push_front(event);
                state.ready_completion_sessions.push_front(session);
                break;
            }
        }
    }
}

fn dispatch_persistence(
    node_store: &SHAMapStoreNodeStore,
    pending: &Arc<Mutex<CoordinatorPersistence>>,
    tx: &EventSender,
    batch: WriteBatch,
) {
    node_store.schedule_write(Box::new(CoordinatorPersistenceWork {
        node_store: node_store.clone(),
        batch,
        pending: Arc::clone(pending),
        tx: tx.clone(),
    }));
}

/// NodeStore write submission. The port owns physical submission and the
/// per-session FIFO; the coordinator owns write intent, in-flight accounting,
/// and the terminal durability decision.
#[derive(Clone)]
pub(crate) struct CoordinatorWritePort {
    node_store: SHAMapStoreNodeStore,
    pending: Arc<Mutex<CoordinatorPersistence>>,
    tx: EventSender,
}

impl CoordinatorWritePort {
    /// Builds a write port over the NodeStore and the completion channel.
    pub(crate) fn new(node_store: SHAMapStoreNodeStore, tx: EventSender) -> Self {
        Self {
            node_store,
            pending: Arc::new(Mutex::new(CoordinatorPersistence::default())),
            tx,
        }
    }

    /// Release queued and in-flight persistence work of a terminal session.
    /// Already-retained exact completions remain queued so they can be
    /// delivered and rejected as stale rather than silently dropped.
    pub(crate) fn cancel_session(&self, session: SessionRef) {
        let mut state = self.pending.lock().expect("coordinator persistence lock");
        let Some(entry) = state.by_session.get_mut(&session) else {
            return;
        };
        entry.queued.clear();
        entry.in_flight = None;
        if entry.completions.is_empty() {
            state.by_session.remove(&session);
        }
    }
}

impl acquisition::WritePort for CoordinatorWritePort {
    fn submit_write(&mut self, batch: WriteBatch) {
        let session = batch.operation().session();
        let node_store = self.node_store.clone();
        let pending = Arc::clone(&self.pending);
        let tx = self.tx.clone();
        // Claim the one in-flight slot while holding the port lock; release it
        // before the physical submission so an inline scheduler (DummyScheduler
        // in tests) cannot deadlock on the same lock from `run`.
        let command = {
            let mut state = pending.lock().expect("coordinator persistence lock");
            let entry = state.by_session.entry(session).or_default();
            if entry.in_flight.is_some() {
                entry.queued.push_back(PersistenceBatch { batch });
                None
            } else {
                entry.in_flight = Some(batch.operation());
                Some(batch)
            }
        };
        if let Some(batch) = command {
            dispatch_persistence(&node_store, &pending, &tx, batch);
        }
    }

    fn flush_completions(&mut self) {
        flush_persistence_completions(&self.pending, &self.tx);
    }
}

#[cfg(test)]
mod persistence_tests {
    use super::*;
    use acquisition::{
        OperationGeneration, OperationId, OperationKind, PlanEpoch, RunEpoch, SessionId,
        StoreGeneration,
    };
    use basics::base_uint::Uint256;

    fn session() -> SessionRef {
        SessionRef::new(
            RunEpoch::new(1),
            SessionId::new(1),
            Uint256::from(1),
            PlanEpoch::new(1),
            StoreGeneration::new(1),
        )
    }

    #[test]
    fn full_control_queue_retains_ordered_write_and_fence_until_owner_replays_them() {
        let pending = Arc::new(Mutex::new(CoordinatorPersistence::default()));
        let (tx, rx) = mpsc::sync_channel(1);
        tx.send(acquisition::AcquisitionEvent::Heartbeat)
            .expect("fill bounded control queue");
        let session = session();
        let write = OperationRef::new(
            session,
            OperationKind::Write,
            OperationId::new(1),
            OperationGeneration::new(1),
        );
        let fence = OperationRef::new(
            session,
            OperationKind::DurabilityFence,
            OperationId::new(2),
            OperationGeneration::new(1),
        );
        {
            let mut state = pending.lock().expect("persistence lock");
            let entry = state.by_session.entry(session).or_default();
            entry
                .completions
                .push_back(acquisition::AcquisitionEvent::WriteCompleted(
                    WriteCompletion::new(write, WriteOutcome::Accepted),
                ));
            entry
                .completions
                .push_back(acquisition::AcquisitionEvent::DurabilityFenced(
                    DurabilityCompletion::new(fence, DurabilityOutcome::Passed),
                ));
            state.ready_completion_sessions.push_back(session);
        }

        flush_persistence_completions(&pending, &tx);
        assert_eq!(
            pending
                .lock()
                .expect("persistence lock")
                .by_session
                .get(&session)
                .expect("full queue retains session")
                .completions
                .len(),
            2,
            "a full lane must retain both exact completions without blocking"
        );
        assert!(matches!(
            rx.recv().expect("filled event"),
            acquisition::AcquisitionEvent::Heartbeat
        ));

        flush_persistence_completions(&pending, &tx);
        assert!(matches!(
            rx.recv().expect("write completion"),
            acquisition::AcquisitionEvent::WriteCompleted(completion)
                if completion.operation() == write
        ));
        flush_persistence_completions(&pending, &tx);
        assert!(matches!(
            rx.recv().expect("fence completion"),
            acquisition::AcquisitionEvent::DurabilityFenced(completion)
                if completion.operation() == fence
        ));
        assert!(
            pending
                .lock()
                .expect("persistence lock")
                .by_session
                .get(&session)
                .is_none(),
            "all replayed records release their resource-local state"
        );
    }
}

// ─── Timer port ─────────────────────────────────────────────────────────────

/// Timer arming over the shared [`WorkerPool`] timer service. Tracks exactly
/// which operations each session armed so session cancellation can disarm them
/// all; the timer thread only posts typed `TimerFired` completions.
#[derive(Clone)]
pub(crate) struct CoordinatorTimerPort {
    pool: Arc<WorkerPool>,
    completions: super::coordinator_adapter::RetainedControlEvents,
    armed: Arc<Mutex<HashMap<SessionRef, HashSet<OperationRef>>>>,
}

impl CoordinatorTimerPort {
    /// Builds a timer port over the shared timer service.
    pub(crate) fn new(pool: Arc<WorkerPool>, tx: EventSender) -> Self {
        Self {
            pool,
            completions: super::coordinator_adapter::RetainedControlEvents::new(tx),
            armed: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Disarm every timer a session armed. Used by the cancellation
    /// dispatcher; late wakeups for the session are validated by the
    /// coordinator and dropped as stale.
    pub(crate) fn disarm_session(&self, session: SessionRef) {
        let operations = {
            let mut armed = self.armed.lock().expect("timer port armed lock");
            armed.remove(&session).unwrap_or_default()
        };
        for operation in operations {
            self.pool.disarm_coordinator_timer(operation);
        }
    }
}

impl TimerPort for CoordinatorTimerPort {
    fn arm(&mut self, request: TimerRequest) {
        let operation = request.operation();
        let session = operation.session();
        self.armed
            .lock()
            .expect("timer port armed lock")
            .entry(session)
            .or_default()
            .insert(operation);
        self.pool.schedule_coordinator_timer(
            operation,
            request.timer(),
            request.after(),
            self.completions.clone(),
        );
    }

    fn disarm(&mut self, operation: OperationRef) {
        {
            let mut armed = self.armed.lock().expect("timer port armed lock");
            if let Some(set) = armed.get_mut(&operation.session()) {
                set.remove(&operation);
            }
        }
        self.pool.disarm_coordinator_timer(operation);
    }

    fn flush_completions(&mut self) {
        self.completions.flush();
    }
}

// ─── Phase port ─────────────────────────────────────────────────────────────

/// Service-phase publication. The coordinator is the only production writer of
/// the operating mode; `need_network_ledger` is derived from the phase rather
/// than independently mutable. Other legacy writers are removed in M6.
#[derive(Clone)]
pub(crate) struct CoordinatorPhasePort {
    state: Arc<SharedNetworkOpsState>,
}

impl CoordinatorPhasePort {
    /// Builds a phase port over the shared NetworkOps state.
    pub(crate) fn new(state: Arc<SharedNetworkOpsState>) -> Self {
        Self { state }
    }
}

fn network_ops_mode(phase: SyncPhase) -> NetworkOpsOperatingMode {
    match phase {
        SyncPhase::Disconnected => NetworkOpsOperatingMode::Disconnected,
        SyncPhase::Connected => NetworkOpsOperatingMode::Connected,
        SyncPhase::Syncing { .. } => NetworkOpsOperatingMode::Syncing,
        SyncPhase::Tracking { .. } => NetworkOpsOperatingMode::Tracking,
        SyncPhase::Full { .. } => NetworkOpsOperatingMode::Full,
        // Shutdown publishes no further mode; retain the last public mode by
        // holding `Connected` rather than inventing a mode value.
        SyncPhase::Stopping => NetworkOpsOperatingMode::Connected,
    }
}

/// `need_network_ledger` is derived: the node still needs a network ledger
/// whenever it is connected-but-not-full or actively syncing.
fn derived_need_network_ledger(phase: SyncPhase) -> bool {
    matches!(phase, SyncPhase::Connected | SyncPhase::Syncing { .. })
}

impl PhasePort for CoordinatorPhasePort {
    fn set_phase(&mut self, phase: SyncPhase) {
        let reason = match phase {
            SyncPhase::Disconnected => "coordinator-disconnected",
            SyncPhase::Connected => "coordinator-connected",
            SyncPhase::Syncing { .. } => "coordinator-syncing",
            SyncPhase::Tracking { .. } => "coordinator-tracking",
            SyncPhase::Full { .. } => "coordinator-full",
            SyncPhase::Stopping => "coordinator-stopping",
        };
        self.state
            .set_operating_mode_with_reason(network_ops_mode(phase), reason);
        self.state
            .set_need_network_ledger(derived_need_network_ledger(phase));
    }
}

// ─── Cancellation port ──────────────────────────────────────────────────────

/// Session cancellation dispatch: releases brokered read tickets, disarms every
/// timer the session armed, and releases its persistence FIFO. Adapters settle
/// after their resource lock is released; the coordinator ignores late
/// completions for a cancelled session.
#[derive(Clone)]
pub(crate) struct CoordinatorCancellationDispatcher {
    reads: BrokerCancellationDispatcher,
    timers: CoordinatorTimerPort,
    writes: CoordinatorWritePort,
}

impl CoordinatorCancellationDispatcher {
    /// Builds the combined cancellation dispatcher.
    pub(crate) fn new(
        reads: BrokerCancellationDispatcher,
        timers: CoordinatorTimerPort,
        writes: CoordinatorWritePort,
    ) -> Self {
        Self {
            reads,
            timers,
            writes,
        }
    }
}

impl CancellationPort for CoordinatorCancellationDispatcher {
    fn session_cancelled(&mut self, session: SessionRef) {
        self.reads.session_cancelled(session);
        self.timers.disarm_session(session);
        self.writes.cancel_session(session);
    }
}

// ─── Production assembly ────────────────────────────────────────────────────

/// Every resource the production coordinator adapter needs. The registry
/// constructs this in the M4.2-C2 switchover.
pub(crate) struct CoordinatorPortResources {
    /// The coordinator owner built by the caller (run epoch, budgets, plan
    /// seed).
    pub(crate) runner: CoordinatorRunner,
    /// The tracked peer set overlay requests deliver to.
    pub(crate) peers: SimplePeerSet,
    /// The shared NodeStore read broker.
    pub(crate) broker: NodeReadBroker,
    /// Shared broker ticket state (read port + cancellation dispatcher).
    pub(crate) tickets: BrokerTicketState,
    /// The fetch-pack cache by-hash replies populate.
    pub(crate) fetch_pack: Arc<FetchPackCache>,
    /// The NodeStore write target and durability fence.
    pub(crate) node_store: SHAMapStoreNodeStore,
    /// The durable-completion channel LedgerMaster consumes.
    pub(crate) completed_ledgers_tx: mpsc::SyncSender<CompletedInboundLedger>,
    /// The shared timer service.
    pub(crate) timer_pool: Arc<WorkerPool>,
    /// The NetworkOps state the phase port publishes to.
    pub(crate) phase_state: Arc<SharedNetworkOpsState>,
}

/// Builds the fully wired production adapter. The event channel is created
/// first so the read and timer ports share the same sender the adapter drains.
pub(crate) fn build_coordinator_adapter(resources: CoordinatorPortResources) -> ProductionAdapter {
    let CoordinatorPortResources {
        runner,
        peers,
        broker,
        tickets,
        fetch_pack,
        node_store,
        completed_ledgers_tx,
        timer_pool,
        phase_state,
    } = resources;
    let (tx, rx) = mpsc::sync_channel(CONTROL_EVENT_QUEUE_CAPACITY);
    let (packet_tx, packet_rx) = mpsc::sync_channel(PACKET_INGRESS_QUEUE_CAPACITY);
    let reads = BrokerReadPort::new(
        broker.clone(),
        tickets.clone(),
        node_store.clone(),
        tx.clone(),
    );
    let writes = CoordinatorWritePort::new(node_store, tx.clone());
    let timers = CoordinatorTimerPort::new(timer_pool, tx.clone());
    let handoffs = CoordinatorHandoffPort::new(completed_ledgers_tx, tx.clone());
    let phase = CoordinatorPhasePort::new(phase_state);
    let cancellations = CoordinatorCancellationDispatcher::new(
        BrokerCancellationDispatcher::new(broker, tickets),
        timers.clone(),
        writes.clone(),
    );
    CoordinatorAdapter::with_event_channel(
        runner,
        OverlayLedgerRequestPort::new(peers),
        reads,
        writes,
        timers,
        handoffs,
        phase,
        cancellations,
        fetch_pack,
        tx,
        rx,
        packet_tx,
        packet_rx,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    use acquisition::{
        LedgerTarget, OperationKind, PersistNode, PlanEpoch, RunEpoch, SessionId, StoreGeneration,
        WritePort,
    };
    use basics::base_uint::Uint256;
    use basics::basic_config::Section;
    use basics::sha_map_hash::SHAMapHash;
    use nodestore::{
        BatchWriteReport, FetchReport, Manager as _, ManagerImp, NullJournal, Scheduler, Task,
    };

    const SEQ: u32 = 88;

    fn session(n: u64) -> SessionRef {
        SessionRef::new(
            RunEpoch::new(1),
            SessionId::new(n),
            Uint256::from(n),
            PlanEpoch::new(1),
            StoreGeneration::new(1),
        )
    }

    /// Every operation is a distinct identity so the ports can never coalesce
    /// two operations of the same session/kind.
    static NEXT_OPERATION_ID: AtomicU64 = AtomicU64::new(1);

    fn operation(session: SessionRef, kind: OperationKind) -> OperationRef {
        let id = NEXT_OPERATION_ID.fetch_add(1, Ordering::Relaxed);
        OperationRef::new(
            session,
            kind,
            acquisition::OperationId::new(id),
            acquisition::OperationGeneration::new(id),
        )
    }

    /// A deterministic NodeStore scheduler that retains every scheduled task
    /// until the test runs it. Replaces `DummyScheduler` wherever a test must
    /// observe the write-port FIFO between submissions instead of a synchronous
    /// cascade through every queued batch.
    #[derive(Default)]
    struct CaptureScheduler {
        queued: Arc<Mutex<VecDeque<Arc<dyn Task>>>>,
    }

    impl CaptureScheduler {
        fn new() -> Self {
            Self::default()
        }

        fn pending(&self) -> usize {
            self.queued.lock().expect("captured task lock").len()
        }

        fn run_next(&self) -> bool {
            let task = self.queued.lock().expect("captured task lock").pop_front();
            if let Some(task) = task {
                task.perform_scheduled_task();
                true
            } else {
                false
            }
        }
    }

    impl Scheduler for CaptureScheduler {
        fn schedule_task(&self, task: Arc<dyn Task>) {
            self.queued
                .lock()
                .expect("captured task lock")
                .push_back(task);
        }

        fn on_fetch(&self, _report: FetchReport) {}

        fn on_batch_write(&self, _report: BatchWriteReport) {}
    }

    fn memory_node_store_with(path: &str, scheduler: Arc<dyn Scheduler>) -> SHAMapStoreNodeStore {
        let manager = ManagerImp::new();
        let mut config = Section::new("node_db");
        config.set("type", "Memory");
        config.set("path", path);
        let database = manager
            .make_database(0, scheduler, 1, &config, Arc::new(NullJournal))
            .expect("database");
        SHAMapStoreNodeStore::Single(database)
    }

    fn memory_node_store(path: &str) -> SHAMapStoreNodeStore {
        memory_node_store_with(path, Arc::new(nodestore::DummyScheduler))
    }

    #[test]
    fn write_port_stores_nodes_and_reports_write_then_fence() {
        let node_store = memory_node_store("coord-write-port-order");
        let (tx, rx) = mpsc::sync_channel(CONTROL_EVENT_QUEUE_CAPACITY);
        let mut port = CoordinatorWritePort::new(node_store.clone(), tx);

        let session = session(1);
        let write = operation(session, OperationKind::Write);
        let fence = operation(session, OperationKind::DurabilityFence);
        let key = SHAMapHash::new(Uint256::from(0xAA));
        let batch = WriteBatch::new(
            write,
            fence,
            StoreGeneration::new(node_store.store_generation()),
            777,
            vec![PersistNode::new(
                key,
                bytes::Bytes::from_static(&[1, 2, 3]),
                StoredObjectKind::AccountNode,
            )],
        );
        port.submit_write(batch);

        let first = rx
            .recv_timeout(Duration::from_secs(1))
            .expect("write completion");
        match first {
            acquisition::AcquisitionEvent::WriteCompleted(completion) => {
                assert_eq!(completion.operation(), write);
                assert_eq!(completion.outcome(), WriteOutcome::Accepted);
            }
            other => panic!("expected a write completion, got {other:?}"),
        }
        let second = rx
            .recv_timeout(Duration::from_secs(1))
            .expect("fence completion");
        match second {
            acquisition::AcquisitionEvent::DurabilityFenced(completion) => {
                assert_eq!(completion.operation(), fence);
                assert_eq!(completion.outcome(), DurabilityOutcome::Passed);
            }
            other => panic!("expected a fence completion, got {other:?}"),
        }

        let backend = node_store.export_backend().expect("backend");
        let (object, status) = backend.fetch(&Uint256::from(0xAA));
        assert_eq!(status, nodestore::Status::Ok);
        assert_eq!(object.expect("stored node").data().as_slice(), &[1, 2, 3]);
    }

    #[test]
    fn write_port_serializes_batches_per_session() {
        let scheduler = Arc::new(CaptureScheduler::new());
        let node_store = memory_node_store_with("coord-write-port-serialized", scheduler.clone());
        let (tx, rx) = mpsc::sync_channel(CONTROL_EVENT_QUEUE_CAPACITY);
        let mut port = CoordinatorWritePort::new(node_store.clone(), tx);

        let session = session(2);
        let first_write = operation(session, OperationKind::Write);
        let first_fence = operation(session, OperationKind::DurabilityFence);
        let first = WriteBatch::new(
            first_write,
            first_fence,
            StoreGeneration::new(node_store.store_generation()),
            777,
            vec![PersistNode::new(
                SHAMapHash::new(Uint256::from(0x01)),
                bytes::Bytes::from_static(&[1]),
                StoredObjectKind::Unknown,
            )],
        );
        let second_write = operation(session, OperationKind::Write);
        let second_fence = operation(session, OperationKind::DurabilityFence);
        let second = WriteBatch::new(
            second_write,
            second_fence,
            StoreGeneration::new(node_store.store_generation()),
            777,
            vec![PersistNode::new(
                SHAMapHash::new(Uint256::from(0x02)),
                bytes::Bytes::from_static(&[2]),
                StoredObjectKind::Unknown,
            )],
        );
        port.submit_write(first);
        port.submit_write(second);

        // The first batch occupies the one in-flight slot; the second waits in
        // the FIFO until the first batch's fence settles.
        assert_eq!(scheduler.pending(), 1);
        assert!(scheduler.run_next());
        assert_eq!(scheduler.pending(), 1);
        assert!(scheduler.run_next());
        assert_eq!(scheduler.pending(), 0);

        // Exactly one write completion and its fence per batch, in submission
        // order: write1, fence1, write2, fence2.
        let expected = [
            (first_write, false),
            (first_fence, true),
            (second_write, false),
            (second_fence, true),
        ];
        for (index, (operation, is_fence)) in expected.into_iter().enumerate() {
            let event = rx
                .recv_timeout(Duration::from_secs(1))
                .unwrap_or_else(|_| panic!("completion #{index}"));
            match (event, is_fence) {
                (acquisition::AcquisitionEvent::WriteCompleted(completion), false) => {
                    assert_eq!(completion.operation(), operation);
                    assert_eq!(completion.outcome(), WriteOutcome::Accepted);
                }
                (acquisition::AcquisitionEvent::DurabilityFenced(completion), true) => {
                    assert_eq!(completion.operation(), operation);
                    assert_eq!(completion.outcome(), DurabilityOutcome::Passed);
                }
                (other, _) => panic!("expected a completion, got {other:?}"),
            }
        }
    }

    #[test]
    fn write_port_cancellation_releases_queued_persistence() {
        let scheduler = Arc::new(CaptureScheduler::new());
        let node_store = memory_node_store_with("coord-write-port-cancel", scheduler.clone());
        let (tx, rx) = mpsc::sync_channel(CONTROL_EVENT_QUEUE_CAPACITY);
        let mut port = CoordinatorWritePort::new(node_store.clone(), tx);

        let session = session(3);
        let first_write = operation(session, OperationKind::Write);
        let first_fence = operation(session, OperationKind::DurabilityFence);
        let first = WriteBatch::new(
            first_write,
            first_fence,
            StoreGeneration::new(node_store.store_generation()),
            777,
            vec![PersistNode::new(
                SHAMapHash::new(Uint256::from(0x03)),
                bytes::Bytes::from_static(&[3]),
                StoredObjectKind::Unknown,
            )],
        );
        let second = WriteBatch::new(
            operation(session, OperationKind::Write),
            operation(session, OperationKind::DurabilityFence),
            StoreGeneration::new(node_store.store_generation()),
            777,
            vec![PersistNode::new(
                SHAMapHash::new(Uint256::from(0x04)),
                bytes::Bytes::from_static(&[4]),
                StoredObjectKind::Unknown,
            )],
        );
        port.submit_write(first);
        port.submit_write(second);

        // The second batch is still queued behind the in-flight first batch.
        assert_eq!(scheduler.pending(), 1);

        // Cancellation releases the FIFO: the queued successor is never
        // dispatched, so only the in-flight batch's (now stale) completions
        // arrive and no further work is scheduled.
        port.cancel_session(session);
        assert!(scheduler.run_next());
        assert_eq!(scheduler.pending(), 0);

        let first_event = rx
            .recv_timeout(Duration::from_secs(1))
            .expect("in-flight write completion");
        match first_event {
            acquisition::AcquisitionEvent::WriteCompleted(completion) => {
                assert_eq!(completion.operation(), first_write);
            }
            other => panic!("expected a write completion, got {other:?}"),
        }
        let second_event = rx
            .recv_timeout(Duration::from_secs(1))
            .expect("in-flight fence completion");
        match second_event {
            acquisition::AcquisitionEvent::DurabilityFenced(completion) => {
                assert_eq!(completion.operation(), first_fence);
            }
            other => panic!("expected a fence completion, got {other:?}"),
        }

        match rx.recv_timeout(Duration::from_millis(50)) {
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            other => panic!("a cancelled session must emit no successor completion, got {other:?}"),
        }
    }

    #[test]
    fn timer_port_fires_the_exact_armed_operation() {
        let pool = Arc::new(WorkerPool::new_with_manual_timer_for_test(0));
        let (tx, rx) = mpsc::sync_channel(CONTROL_EVENT_QUEUE_CAPACITY);
        let mut port = CoordinatorTimerPort::new(Arc::clone(&pool), tx);

        let session = session(4);
        let operation = operation(session, OperationKind::Timer);
        port.arm(TimerRequest::new(
            operation,
            acquisition::TimerKind::AcquireTimeout,
            Duration::from_secs(60),
        ));
        assert_eq!(
            pool.scheduled_timer_delays_for_test(),
            vec![Duration::from_secs(60)]
        );
        assert_eq!(
            pool.fire_next_timer_for_test(),
            Some(Duration::from_secs(60))
        );
        match rx
            .recv_timeout(Duration::from_secs(1))
            .expect("timer wakeup")
        {
            acquisition::AcquisitionEvent::TimerFired {
                operation: fired,
                timer,
            } => {
                assert_eq!(fired, operation);
                assert_eq!(timer, acquisition::TimerKind::AcquireTimeout);
            }
            other => panic!("expected a timer wakeup, got {other:?}"),
        }
        pool.stop();
    }

    #[test]
    fn timer_port_disarm_removes_only_the_matching_operation() {
        let pool = Arc::new(WorkerPool::new_with_manual_timer_for_test(0));
        let (tx, rx) = mpsc::sync_channel(CONTROL_EVENT_QUEUE_CAPACITY);
        let mut port = CoordinatorTimerPort::new(Arc::clone(&pool), tx);

        let session = session(5);
        let kept = operation(session, OperationKind::Timer);
        let dropped = operation(session, OperationKind::Timer);
        port.arm(TimerRequest::new(
            kept,
            acquisition::TimerKind::ReadRetry,
            Duration::from_secs(60),
        ));
        port.arm(TimerRequest::new(
            dropped,
            acquisition::TimerKind::ReadRetry,
            Duration::from_secs(60),
        ));
        assert_eq!(pool.scheduled_timer_delays_for_test().len(), 2);

        port.disarm(dropped);
        assert_eq!(pool.scheduled_timer_delays_for_test().len(), 1);

        assert_eq!(
            pool.fire_next_timer_for_test(),
            Some(Duration::from_secs(60))
        );
        match rx
            .recv_timeout(Duration::from_secs(1))
            .expect("timer wakeup")
        {
            acquisition::AcquisitionEvent::TimerFired {
                operation: fired, ..
            } => {
                assert_eq!(fired, kept, "the disarmed operation must never fire");
            }
            other => panic!("expected a timer wakeup, got {other:?}"),
        }
        pool.stop();
    }

    #[test]
    fn timer_port_session_cancellation_disarms_every_armed_timer() {
        let pool = Arc::new(WorkerPool::new(0));
        let (tx, _rx) = mpsc::sync_channel(CONTROL_EVENT_QUEUE_CAPACITY);
        let mut port = CoordinatorTimerPort::new(Arc::clone(&pool), tx);

        let session = session(6);
        port.arm(TimerRequest::new(
            operation(session, OperationKind::Timer),
            acquisition::TimerKind::AcquireTimeout,
            Duration::from_secs(1),
        ));
        port.arm(TimerRequest::new(
            operation(session, OperationKind::Timer),
            acquisition::TimerKind::HandoffRetry,
            Duration::from_secs(1),
        ));
        assert_eq!(pool.scheduled_timer_delays_for_test().len(), 2);

        port.disarm_session(session);
        assert!(
            pool.scheduled_timer_delays_for_test().is_empty(),
            "session cancellation must disarm every timer the session armed"
        );
        pool.stop();
    }

    #[test]
    fn phase_port_maps_phase_and_derives_need_network_ledger() {
        let state = Arc::new(SharedNetworkOpsState::new(
            NetworkOpsOperatingMode::Connected,
        ));
        let mut port = CoordinatorPhasePort::new(Arc::clone(&state));

        port.set_phase(SyncPhase::Syncing {
            target: LedgerTarget::new(Uint256::from(9), Some(SEQ)),
        });
        assert_eq!(state.operating_mode(), NetworkOpsOperatingMode::Syncing);
        assert!(
            state.need_network_ledger(),
            "syncing requires a network ledger"
        );

        port.set_phase(SyncPhase::Full {
            lcl: acquisition::LedgerIdentity::new(Uint256::from(9), SEQ),
            published: acquisition::LedgerIdentity::new(Uint256::from(9), SEQ),
        });
        assert_eq!(state.operating_mode(), NetworkOpsOperatingMode::Full);
        assert!(!state.need_network_ledger());

        port.set_phase(SyncPhase::Connected);
        assert_eq!(state.operating_mode(), NetworkOpsOperatingMode::Connected);
        assert!(state.need_network_ledger());
    }

    #[test]
    fn production_assembly_wires_every_port_and_admits() {
        // Assemble the full production adapter and prove the ingress path
        // reaches the owner queue through the typed ports.
        let node_store = memory_node_store("coord-assembly");
        let (completed_tx, _completed_rx) = mpsc::sync_channel(16);
        let broker = NodeReadBroker::new(crate::ReadBrokerConfig::default()).expect("broker");
        let pool = Arc::new(WorkerPool::new(0));
        let phase_state = Arc::new(SharedNetworkOpsState::new(
            NetworkOpsOperatingMode::Disconnected,
        ));
        let peers = SimplePeerSet::new(Vec::new());
        let fetch_pack = Arc::new(FetchPackCache::new(
            1024,
            time::Duration::seconds(600),
            basics::tagged_cache::MonotonicClock::default(),
        ));
        let mut adapter = build_coordinator_adapter(CoordinatorPortResources {
            runner: CoordinatorRunner::new(RunEpoch::new(1)),
            peers,
            broker,
            tickets: BrokerTicketState::default(),
            fetch_pack: Arc::clone(&fetch_pack),
            node_store,
            completed_ledgers_tx: completed_tx,
            timer_pool: pool,
            phase_state,
        });

        let effects = adapter.connectivity(&[1]);
        assert!(
            effects.contains(&acquisition::AcquisitionEffect::SetServicePhase(
                SyncPhase::Connected
            )),
            "the phase port must publish the promoted phase"
        );
        let session = adapter
            .acquire_requested(
                LedgerTarget::new(Uint256::from(9), Some(SEQ)),
                acquisition::AcquireReason::Consensus,
            )
            .iter()
            .find_map(|effect| match effect {
                acquisition::AcquisitionEffect::SendLedgerRequest(request) => {
                    Some(request.session())
                }
                _ => None,
            })
            .expect("a peer request must be emitted");
        assert_eq!(session.target_hash(), Uint256::from(9));
    }

    #[test]
    fn production_replacement_rejects_a_stale_lease_and_late_events() {
        // The full production adapter stack: real broker, phase port, routing
        // snapshot, and admission gates. A replacement for the same target
        // hash must make every old `SessionRef` event (held admission lease,
        // late timer) harmless to the replacement session.
        let node_store = memory_node_store("coord-replacement-stale-lease");
        let (completed_tx, _completed_rx) = mpsc::sync_channel(16);
        let broker = NodeReadBroker::new(crate::ReadBrokerConfig::default()).expect("broker");
        let pool = Arc::new(WorkerPool::new(0));
        let phase_state = Arc::new(SharedNetworkOpsState::new(
            NetworkOpsOperatingMode::Disconnected,
        ));
        let peers = SimplePeerSet::new(Vec::new());
        let fetch_pack = Arc::new(FetchPackCache::new(
            1024,
            time::Duration::seconds(600),
            basics::tagged_cache::MonotonicClock::default(),
        ));
        let mut adapter = build_coordinator_adapter(CoordinatorPortResources {
            runner: CoordinatorRunner::new(RunEpoch::new(1)),
            peers,
            broker,
            tickets: BrokerTicketState::default(),
            fetch_pack: Arc::clone(&fetch_pack),
            node_store,
            completed_ledgers_tx: completed_tx,
            timer_pool: pool,
            phase_state,
        });
        adapter.connectivity(&[1]);

        let session_a = adapter
            .acquire_requested(
                LedgerTarget::new(Uint256::from(9), Some(SEQ)),
                acquisition::AcquireReason::Consensus,
            )
            .iter()
            .find_map(|effect| match effect {
                acquisition::AcquisitionEffect::SendLedgerRequest(request) => {
                    Some(request.session())
                }
                _ => None,
            })
            .expect("first peer request");
        let snapshot_a = adapter.routing_snapshot();
        let route_a = snapshot_a
            .route(&Uint256::from(9))
            .expect("route for the first session");
        assert_eq!(route_a.session(), session_a);

        // Admit a real wire packet through route A's gate and hold the
        // resulting lease/event without letting the owner consume it.
        let message = overlay::TmLedgerData {
            ledger_hash: Uint256::from(9).data().to_vec(),
            ledger_seq: SEQ,
            r#type: 0, // liBASE: no node ids required
            nodes: vec![overlay::message::wire::TmLedgerNode {
                nodedata: vec![1, 2, 3],
                nodeid: None,
                ..overlay::message::wire::TmLedgerNode::default()
            }],
            ..overlay::TmLedgerData::default()
        };
        assert_eq!(
            adapter.route_ledger_data(1, &message),
            super::super::coordinator_adapter::LedgerDataIngressDisposition::Delivered
        );
        let held = match adapter.pending_events().into_iter().next() {
            Some(acquisition::AcquisitionEvent::PacketAdmitted(packet)) => packet,
            other => panic!("expected one admitted packet, got {other:?}"),
        };
        assert_eq!(held.lease().session(), session_a);
        assert_eq!(route_a.gate().current_packets(), 1);

        // A store rotation invalidates session A. Reacquiring the same target
        // then creates session B; an exact duplicate request would correctly
        // coalesce instead of replacing session A.
        adapter.push(acquisition::AcquisitionEvent::StoreRotated(
            StoreGeneration::new(2),
        ));
        assert_eq!(adapter.drain(), 1);
        assert_eq!(adapter.snapshot().sessions_cancelled(), 1);

        let effects_b = adapter.acquire_requested(
            LedgerTarget::new(Uint256::from(9), Some(SEQ)),
            acquisition::AcquireReason::Consensus,
        );
        let session_b = effects_b
            .iter()
            .find_map(|effect| match effect {
                acquisition::AcquisitionEffect::SendLedgerRequest(request) => {
                    Some(request.session())
                }
                _ => None,
            })
            .expect("replacement peer request");
        assert_ne!(session_b, session_a);
        assert_eq!(session_b.target_hash(), Uint256::from(9));
        let snapshot_b = adapter.routing_snapshot();
        let route_b = snapshot_b
            .route(&Uint256::from(9))
            .expect("replacement route");
        assert_eq!(route_b.session(), session_b);
        assert_eq!(
            snapshot_b.session_count(),
            1,
            "only the replacement session is routed/live"
        );
        assert_eq!(adapter.snapshot().sessions_cancelled(), 1);

        // The held lease was minted under session A. Pushing the stale
        // admission through the owner loop must be rejected: it never reaches
        // session B's mailbox, is not credited to any session, and is counted
        // stale.
        adapter.push(acquisition::AcquisitionEvent::PacketAdmitted(held));
        assert_eq!(adapter.drain(), 1);
        assert_eq!(adapter.snapshot().stale_events(), 1);
        assert_eq!(
            adapter.snapshot().packets_admitted(),
            0,
            "the stale packet is admitted to no session"
        );
        assert_eq!(
            adapter.snapshot().phase(),
            &SyncPhase::Syncing {
                target: LedgerTarget::new(Uint256::from(9), Some(SEQ))
            }
        );
        assert_eq!(
            route_b.gate().current_packets(),
            0,
            "session B's gate was never charged"
        );

        // A late timer for session A is equally stale and cannot perturb the
        // replacement session.
        let stale_timer = OperationRef::new(
            session_a,
            OperationKind::Timer,
            acquisition::OperationId::new(1),
            acquisition::OperationGeneration::new(1),
        );
        adapter.push(acquisition::AcquisitionEvent::TimerFired {
            operation: stale_timer,
            timer: acquisition::TimerKind::AcquireTimeout,
        });
        adapter.drain();
        assert_eq!(adapter.snapshot().stale_events(), 2);
        let snapshot_after = adapter.routing_snapshot();
        assert_eq!(
            snapshot_after.session_count(),
            1,
            "late old-session events leave the replacement session untouched"
        );
        assert_eq!(
            snapshot_after
                .route(&Uint256::from(9))
                .expect("replacement route")
                .session(),
            session_b
        );
    }
}
