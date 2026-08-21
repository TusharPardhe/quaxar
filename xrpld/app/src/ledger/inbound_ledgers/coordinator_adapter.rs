//! M4.2-B coordinator adapter: hosts the serialized [`CoordinatorRunner`] and
//! binds the acquisition ports to app resources.
//!
//! Ownership boundary: this adapter is the production host of the coordinator
//! runner, which is the single acquisition session lifecycle owner. The adapter
//! owns:
//!
//! * the [`CoordinatorRunner`] (built by the caller with whatever [`PlanSeed`]
//!   the switchover needs);
//! * a typed [`AcquisitionEvent`] queue (bounded packet admission happens at
//!   the per-session [`AdmissionGate`]; control completions are never dropped);
//! * a published immutable [`Arc<RoutingSnapshot>`] that overlay ingress uses
//!   to look up a session route and reserve admission without touching mutable
//!   coordinator state;
//! * the fetch-pack cache that by-hash (`TMGetObjectByHash`) replies populate.
//!
//! No port invokes coordinator logic while holding an adapter lock, and
//! completions return as typed events the owner drains with
//! [`CoordinatorAdapter::drain`].
//!
//! ## Rippled parity notes
//!
//! * `TMGetLedger` replies (`TmLedgerData`) carry node ids on the wire;
//!   rippled's `getSHAMapNodeID` (`LedgerNodeHelpers.cpp`) emits the `nodeid`
//!   field for inner nodes and derives leaf ids from the node key. The Quaxar
//!   wire `TmLedgerNode` carries `nodeid`; the adapter preserves it verbatim
//!   into [`InboundLedgerNodeData`]. The engine's Base path applies the header
//!   and root by node data alone (`InboundLedger.cpp` root handling), and its
//!   state/tx path requires every node id (a missing id is rejected the same
//!   way rippled rejects a bad node).
//! * `TMGetObjectByHash` replies are fetch-pack data keyed by hash, never
//!   routed to a session mailbox and never carrying node ids
//!   (`PeerImp::processGetObjectByHash` -> `addFetchPack`); the SHAMap sync
//!   filter consumes them by hash during traversal. Quaxar mirrors this with
//!   [`CoordinatorAdapter::stash_fetch_pack`].
//!
//! References: `rippled/src/xrpld/app/ledger/detail/InboundLedger.cpp`,
//! `rippled/src/xrpld/app/ledger/detail/LedgerNodeHelpers.cpp`,
//! `rippled/src/xrpld/peer/PeerImp.cpp` (`processGetObjectByHash`).

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, RwLock};

use basics::base_uint::Uint256;
use basics::sha_map_hash::SHAMapHash;
use bytes::Bytes;
use ledger::{
    FetchPackCache, InboundLedgerDataType, InboundLedgerNodeData, InboundLedgerObjectType,
    InboundLedgerPacket, make_get_ledger_with_node_ids, make_inbound_needed_by_hash_request,
};
use overlay::{Peer, PeerId as OverlayPeerId, PeerSet, ProtocolMessage, SimplePeerSet};

use acquisition::{
    AcquisitionEffect, AcquisitionEvent, AdmissionBudget, AdmissionGate, AdmittedLedgerPacket,
    BackpressureOutcome, CancellationPort, CoordinatorPorts, CoordinatorRunner, HandoffPort,
    LedgerDataRequest, LedgerRequestPort, PeerAvailabilitySnapshot, PeerId, PeerRequest, PhasePort,
    ReadCompletion, ReadOutcome, ReadPort, ReadRequest, RouteEntry, RoutingGeneration,
    RoutingSnapshot, RunEpoch, SessionPhase, SessionRef, TimerPort, WritePort,
};

use super::read_broker::{
    NodeReadBroker, ReadAdmission, ReadKey, ReadOutcome as BrokerReadOutcome, ReadReady,
    ReadReadySink, ReadTicket,
};
use crate::shamap::shamap_store_backend::SHAMapStoreNodeStore;

/// The wire `liBASE` request type: a full-ledger header request.
const LI_BASE: i32 = 0;
const LI_TX_NODE: i32 = 1;
const LI_AS_NODE: i32 = 2;

/// Lifecycle, cancellation, handoff, write, and timer facts use this bounded
/// reserved channel. Producers use `send`, applying backpressure instead of
/// dropping an exact terminal fact; packet ingress has a distinct channel and
/// cannot consume this capacity.
pub(crate) type EventSender = mpsc::SyncSender<AcquisitionEvent>;
pub(crate) type EventReceiver = mpsc::Receiver<AcquisitionEvent>;

/// Exact control events produced off the coordinator owner thread. A producer
/// never waits on the owner that consumes this bounded lane: full events remain
/// in this resource-local FIFO until its port is flushed from an owner turn.
#[derive(Clone)]
pub(crate) struct RetainedControlEvents {
    tx: EventSender,
    pending: Arc<Mutex<VecDeque<AcquisitionEvent>>>,
}

impl RetainedControlEvents {
    pub(crate) fn new(tx: EventSender) -> Self {
        Self {
            tx,
            pending: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    pub(crate) fn push(&self, event: AcquisitionEvent) {
        let mut pending = self.pending.lock().expect("retained control events lock");
        pending.push_back(event);
        Self::flush_locked(&self.tx, &mut pending);
    }

    pub(crate) fn flush(&self) {
        let mut pending = self.pending.lock().expect("retained control events lock");
        Self::flush_locked(&self.tx, &mut pending);
    }

    fn flush_locked(tx: &EventSender, pending: &mut VecDeque<AcquisitionEvent>) {
        while let Some(event) = pending.pop_front() {
            match tx.try_send(event) {
                Ok(()) => {}
                Err(mpsc::TrySendError::Full(event))
                | Err(mpsc::TrySendError::Disconnected(event)) => {
                    pending.push_front(event);
                    break;
                }
            }
        }
    }
}

/// Reserved lifecycle capacity. This queue is separate from the packet lane,
/// so normal packet pressure cannot displace cancellation, write/fence, timer,
/// or durable-handoff facts.
pub(crate) const CONTROL_EVENT_QUEUE_CAPACITY: usize = 256;

/// Packet ingress is isolated from control/completion facts in a bounded
/// channel. Only overlay admission uses this sender; it must use `try_send` so
/// a full ingress queue settles its admission lease and defers the frame.
type PacketEventSender = mpsc::SyncSender<AcquisitionEvent>;
type PacketEventReceiver = mpsc::Receiver<AcquisitionEvent>;

/// Global retained packet events between overlay ingress and the serialized
/// owner. This is intentionally below the per-session 128-packet admission
/// gate so queue-full handling is exercised before a single route consumes its
/// whole lease budget.
pub(crate) const PACKET_INGRESS_QUEUE_CAPACITY: usize = 64;

/// One owner-loop pass gives completion/control facts priority, but remains
/// bounded so a continuously ready channel cannot monopolize the runner.
const CONTROL_EVENTS_PER_DRAIN: usize = 32;

/// One owner-loop pass then advances a bounded packet slice, preserving packet
/// progress without allowing ingress to starve lifecycle control facts.
const PACKET_EVENTS_PER_DRAIN: usize = 32;

/// The disposition of one wire ledger-data reply for tracing and overlay
/// backpressure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LedgerDataIngressDisposition {
    /// The packet was admitted and enqueued for the owner loop.
    Delivered,
    /// Admission capacity is exhausted; the overlay defers the frame. A
    /// deferred packet has no actor-side effect.
    Deferred,
    /// No route exists for the ledger hash (unknown session or terminal).
    Unmatched,
    /// The coordinator is shutting down or the session is terminal.
    Terminal,
    /// The wire packet could not be decoded.
    Invalid,
}

/// Bounded per-session ingress accounting and routing generation state.
///
/// `packets_invalid` and `fetch_pack_stashes` are incremented today but not
/// yet read; they feed the M6/M7 metrics emission (required observability:
/// packet admissions/deferrals/drops by disposition).
#[derive(Debug, Default)]
struct AdapterStats {
    packets_admitted: u64,
    packets_deferred: u64,
    packets_unmatched: u64,
    #[allow(dead_code)]
    packets_invalid: u64,
    packets_terminal: u64,
    #[allow(dead_code)]
    fetch_pack_stashes: u64,
}

/// Immutable overlay-ingress capability. It owns no coordinator lifecycle
/// state: it only reads the published route snapshot, reserves the exact
/// route's admission gate, and queues a typed packet event. Keeping this
/// separate from [`CoordinatorAdapter`] lets a peer reply synchronously while
/// request-effect dispatch still holds the adapter's mutable owner lock.
#[derive(Clone)]
pub(crate) struct CoordinatorIngress {
    routing_snapshot: Arc<RwLock<Arc<RoutingSnapshot>>>,
    packet_tx: PacketEventSender,
    stats: Arc<Mutex<AdapterStats>>,
}

impl CoordinatorIngress {
    /// Route a wire `TmLedgerData` reply without accessing mutable coordinator
    /// state. The selected route and its gate remain valid through the moved
    /// admission lease even if the owner replaces the route before draining.
    pub(crate) fn route_ledger_data(
        &self,
        peer_id: OverlayPeerId,
        message: &overlay::TmLedgerData,
    ) -> LedgerDataIngressDisposition {
        let Some(packet_type) = map_wire_ledger_type(message.r#type) else {
            return LedgerDataIngressDisposition::Invalid;
        };
        let Some(hash) = Uint256::from_slice(&message.ledger_hash) else {
            return LedgerDataIngressDisposition::Invalid;
        };
        let mut nodes = Vec::with_capacity(message.nodes.len());
        for (packet_index, node) in message.nodes.iter().enumerate() {
            let Some(data) = decode_wire_ledger_node(node, packet_type, packet_index) else {
                return LedgerDataIngressDisposition::Invalid;
            };
            nodes.push(data);
        }
        self.admit(hash, InboundLedgerPacket::new(packet_type, nodes), peer_id)
    }

    fn admit(
        &self,
        hash: Uint256,
        packet: InboundLedgerPacket,
        peer_id: OverlayPeerId,
    ) -> LedgerDataIngressDisposition {
        let snapshot = Arc::clone(
            &self
                .routing_snapshot
                .read()
                .expect("coordinator routing snapshot read"),
        );
        let Some(route) = snapshot.route(&hash) else {
            self.stats
                .lock()
                .expect("adapter stats lock")
                .packets_unmatched += 1;
            return LedgerDataIngressDisposition::Unmatched;
        };
        let bytes = packet
            .nodes
            .iter()
            .map(|node| node.node_data.len() as u64)
            .sum::<u64>();
        match route.gate().try_reserve(packet.nodes.len() as u64, bytes) {
            BackpressureOutcome::Admitted(lease) => {
                let admitted = match AdmittedLedgerPacket::new(
                    lease,
                    route.session(),
                    PeerId::new(u64::from(peer_id)),
                    packet,
                ) {
                    Ok(packet) => packet,
                    Err(_) => return LedgerDataIngressDisposition::Invalid,
                };
                match self
                    .packet_tx
                    .try_send(AcquisitionEvent::PacketAdmitted(admitted))
                {
                    Ok(()) => {
                        self.stats
                            .lock()
                            .expect("adapter stats lock")
                            .packets_admitted += 1;
                        LedgerDataIngressDisposition::Delivered
                    }
                    Err(mpsc::TrySendError::Full(error)) => {
                        if let AcquisitionEvent::PacketAdmitted(mut packet) = error {
                            let _ = packet.settle();
                        }
                        self.stats
                            .lock()
                            .expect("adapter stats lock")
                            .packets_deferred += 1;
                        LedgerDataIngressDisposition::Deferred
                    }
                    Err(mpsc::TrySendError::Disconnected(error)) => {
                        if let AcquisitionEvent::PacketAdmitted(mut packet) = error {
                            let _ = packet.settle();
                        }
                        self.stats
                            .lock()
                            .expect("adapter stats lock")
                            .packets_terminal += 1;
                        LedgerDataIngressDisposition::Terminal
                    }
                }
            }
            BackpressureOutcome::Deferred => {
                self.stats
                    .lock()
                    .expect("adapter stats lock")
                    .packets_deferred += 1;
                LedgerDataIngressDisposition::Deferred
            }
            BackpressureOutcome::Rejected(_) => {
                self.stats
                    .lock()
                    .expect("adapter stats lock")
                    .packets_terminal += 1;
                LedgerDataIngressDisposition::Terminal
            }
        }
    }
}

/// The coordinator adapter: hosts the runner and its event queue, publishes the
/// routing snapshot, and adapts overlay/data completions into typed events.
///
/// Generic over the seven port types so deterministic tests inject
/// `acquisition::fake::*` ports and production instantiates the app ports below.
pub(crate) struct CoordinatorAdapter<R, RD, WR, T, H, P, C> {
    runner: CoordinatorRunner,
    pub(crate) requests: R,
    reads: RD,
    writes: WR,
    timers: T,
    pub(crate) handoffs: H,
    phase: P,
    cancellations: C,
    tx: EventSender,
    rx: EventReceiver,
    // Retained solely for deterministic adapter tests that inject packets through
    // the same isolated packet lane as overlay ingress.
    #[cfg_attr(not(test), allow(dead_code))]
    packet_tx: PacketEventSender,
    packet_rx: PacketEventReceiver,
    routes: BTreeMap<Uint256, RouteEntry>,
    ingress: CoordinatorIngress,
    routing_generation: u64,
    fetch_pack: Arc<FetchPackCache>,
    /// Exact target hashes whose coordinator sessions terminally failed. The
    /// registry drains this bounded-per-owner-turn set after releasing the
    /// coordinator lock and records rippled-compatible failure cooldowns.
    /// Cancellations remain excluded: only `SessionPhase::Failed` represents
    /// an acquisition failure eligible for history re-admission suppression.
    terminal_failures: BTreeSet<Uint256>,
}

impl<R, RD, WR, T, H, P, C> CoordinatorAdapter<R, RD, WR, T, H, P, C>
where
    R: LedgerRequestPort,
    RD: ReadPort,
    WR: WritePort,
    T: TimerPort,
    H: HandoffPort,
    P: PhasePort,
    C: CancellationPort,
{
    /// Builds an adapter around an externally supplied event channel, so the
    /// production wiring can hand the same sender to the read/timer ports
    /// before the adapter exists.
    pub(crate) fn with_event_channel(
        runner: CoordinatorRunner,
        requests: R,
        reads: RD,
        writes: WR,
        timers: T,
        handoffs: H,
        phase: P,
        cancellations: C,
        #[cfg_attr(not(test), allow(dead_code))]
        // read via test-only stash_fetch_pack until M6-C wiring
        fetch_pack: Arc<FetchPackCache>,
        tx: EventSender,
        rx: EventReceiver,
        packet_tx: PacketEventSender,
        packet_rx: PacketEventReceiver,
    ) -> Self {
        let ingress = CoordinatorIngress {
            routing_snapshot: Arc::new(RwLock::new(Arc::new(RoutingSnapshot::new(
                RoutingGeneration::new(0),
                BTreeMap::new(),
            )))),
            packet_tx: packet_tx.clone(),
            stats: Arc::new(Mutex::new(AdapterStats::default())),
        };
        let mut adapter = Self {
            runner,
            requests,
            reads,
            writes,
            timers,
            handoffs,
            phase,
            cancellations,
            tx,
            rx,
            packet_tx,
            packet_rx,
            routes: BTreeMap::new(),
            ingress,
            routing_generation: 0,
            fetch_pack,
            terminal_failures: BTreeSet::new(),
        };
        adapter.publish_routing();
        adapter
    }

    /// The immutable routing snapshot overlay ingress reads. Routes are cached
    /// per session and refreshed after every event; a gate is never rebuilt
    /// while its session still has in-flight leases.
    #[allow(dead_code)] // overlay ingress consumes the snapshot in M4.2-C3
    pub(crate) fn routing_snapshot(&self) -> Arc<RoutingSnapshot> {
        Arc::clone(
            &self
                .ingress
                .routing_snapshot
                .read()
                .expect("coordinator routing snapshot read"),
        )
    }

    /// Clone the immutable overlay-ingress capability. It remains usable while
    /// the owner is dispatching effects under its mutable adapter lock.
    pub(crate) fn ingress(&self) -> CoordinatorIngress {
        self.ingress.clone()
    }

    /// A cloneable sender for typed completions (reads, writes, timers,
    /// handoffs) posted from worker threads. Backpressure is lossless: the
    /// reserved control channel blocks the producer rather than discarding an
    /// exact lifecycle completion. Reserved for M6-C wiring of completion ports.
    #[allow(dead_code)]
    pub(crate) fn event_sender(&self) -> EventSender {
        self.tx.clone()
    }

    /// The coordinator run epoch, for cross-thread identity checks.
    /// Reserved for M6-C completion-identity checks.
    #[allow(dead_code)]
    pub(crate) const fn run_epoch(&self) -> RunEpoch {
        self.runner.run_epoch()
    }

    /// Return and clear exact hashes whose sessions failed since the last
    /// drain. Callers must consume this only after releasing any coordinator
    /// lock; failure recording is registry-owned resource state, not a second
    /// session lifecycle authority.
    pub(crate) fn take_terminal_failures(&mut self) -> Vec<Uint256> {
        std::mem::take(&mut self.terminal_failures)
            .into_iter()
            .collect()
    }

    /// Handle one typed event and dispatch its effects through the ports.
    /// Effects are executed only after the runner has mutated its state, so no
    /// port observes coordinator state mid-mutation.
    pub(crate) fn handle_fact(&mut self, mut event: AcquisitionEvent) -> Vec<AcquisitionEffect> {
        if let AcquisitionEvent::PacketAdmitted(packet) = &mut event {
            // The moved packet holds the exact gate that reserved it. Settle it
            // before planning so route replacement can never release through a
            // newer session's gate; Drop is only a defensive no-op fallback.
            let _ = packet.settle();
        }
        let effects = self.runner.handle_event(event);
        self.note_terminal_failures(&effects);
        // A request can synchronously produce an overlay reply. Publish the
        // route created by this event before dispatching its outbound effects,
        // otherwise that reply sees the previous snapshot and is rejected.
        self.refresh_routes();
        self.dispatch(&effects);
        effects
    }

    /// Advance one bounded owner-loop slice. Control/completion facts always
    /// run first, then packet ingress gets a bounded turn; neither queue is
    /// drained to empty so continuous traffic cannot monopolize the owner.
    /// Returns the number of facts handled.
    pub(crate) fn drain(&mut self) -> usize {
        let mut handled = 0;
        for _ in 0..CONTROL_EVENTS_PER_DRAIN {
            // A production write port retains exact write/fence completions
            // when the bounded control lane is full. Flush after each dequeue
            // so one newly free slot promptly advances that FIFO without ever
            // blocking a NodeStore worker on the coordinator owner.
            self.reads.flush_completions();
            self.writes.flush_completions();
            self.timers.flush_completions();
            let Ok(event) = self.rx.try_recv() else {
                break;
            };
            handled += 1;
            self.handle_fact(event);
        }
        self.reads.flush_completions();
        self.writes.flush_completions();
        self.timers.flush_completions();
        for _ in 0..PACKET_EVENTS_PER_DRAIN {
            let Ok(event) = self.packet_rx.try_recv() else {
                break;
            };
            handled += 1;
            self.handle_fact(event);
        }
        handled
    }

    /// Enqueue a fact only for deterministic adapter tests. Production owner
    /// facts use `handle_fact`; cross-thread control producers use a retained
    /// nonblocking port rather than this potentially blocking test helper.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn push(&self, event: AcquisitionEvent) {
        match event {
            AcquisitionEvent::PacketAdmitted(packet) => {
                match self
                    .packet_tx
                    .try_send(AcquisitionEvent::PacketAdmitted(packet))
                {
                    Ok(()) => {}
                    Err(mpsc::TrySendError::Full(AcquisitionEvent::PacketAdmitted(mut packet)))
                    | Err(mpsc::TrySendError::Disconnected(AcquisitionEvent::PacketAdmitted(
                        mut packet,
                    ))) => {
                        let _ = packet.settle();
                    }
                    Err(_) => unreachable!("packet queue only receives packet events"),
                }
            }
            event => {
                let _ = self.tx.send(event);
            }
        }
    }

    /// Try to enqueue a control fact without waiting for the owner that drains
    /// this bounded queue. Callers on the NetworkOps strand retain exact facts
    /// and retry them on a later turn when this reports `false`.
    pub(crate) fn try_push_control(&self, event: AcquisitionEvent) -> bool {
        !matches!(
            self.tx.try_send(event),
            Err(mpsc::TrySendError::Full(_)) | Err(mpsc::TrySendError::Disconnected(_))
        )
    }

    /// Test-only: consume queued events without dispatching them, preserving
    /// the owner-loop priority order (control before packet ingress).
    #[cfg(test)]
    pub(crate) fn pending_events(&mut self) -> Vec<AcquisitionEvent> {
        self.rx
            .try_iter()
            .chain(self.packet_rx.try_iter())
            .collect()
    }

    /// The runner's observable snapshot (counters, phase, sessions).
    pub(crate) fn snapshot(&self) -> acquisition::RunnerSnapshot {
        self.runner.snapshot()
    }

    /// True when the runner retained a latest consensus demand because all
    /// non-durable capacity was occupied. The adapter uses this only to retain
    /// origin metadata for the eventual owner replay.
    pub(crate) fn has_deferred_consensus_target(&self, target: acquisition::LedgerTarget) -> bool {
        self.runner.has_deferred_consensus_target(target)
    }

    /// Report a usable-peer snapshot (overlay connectivity fact).
    pub(crate) fn connectivity(&mut self, snapshot: &[OverlayPeerId]) -> Vec<AcquisitionEffect> {
        let peers = snapshot
            .iter()
            .map(|&id| PeerId::new(u64::from(id)))
            .collect::<Vec<_>>();
        self.handle_fact(AcquisitionEvent::Connectivity(
            PeerAvailabilitySnapshot::new(peers),
        ))
    }

    /// Report an acquisition demand fact.
    pub(crate) fn acquire_requested(
        &mut self,
        target: acquisition::LedgerTarget,
        reason: acquisition::AcquireReason,
    ) -> Vec<AcquisitionEffect> {
        self.handle_fact(AcquisitionEvent::AcquireRequested { target, reason })
    }

    /// Report a preferred-LCL divergence fact (rippled `consensusViewChange`).
    /// Demotes `Connected/Tracking/Full -> Syncing { target }` without minting a
    /// session; the missing/incomplete path reports its own `acquire_requested`
    /// demand, and a resident-and-compatible switch performs no peer fetch.
    pub(crate) fn preferred_lcl_divergence(
        &mut self,
        target: acquisition::LedgerTarget,
    ) -> Vec<AcquisitionEffect> {
        self.handle_fact(AcquisitionEvent::PreferredLclDivergence { target })
    }

    /// Report a no-consensus-positions fact (Quaxar-specific). Demotes
    /// `Full -> Connected` when consensus accepted a round with no usable peer
    /// positions; no session and no target are named.
    pub(crate) fn blocked_with_no_target(&mut self) -> Vec<AcquisitionEffect> {
        self.handle_fact(AcquisitionEvent::BlockedWithNoTarget)
    }

    /// Route a wire `TmLedgerData` reply through the cloneable immutable
    /// ingress capability. The production overlay holds that capability
    /// independently, so an inline reply cannot re-enter this mutable owner.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn route_ledger_data(
        &self,
        peer_id: OverlayPeerId,
        message: &overlay::TmLedgerData,
    ) -> LedgerDataIngressDisposition {
        self.ingress.route_ledger_data(peer_id, message)
    }

    /// Stash a `TMGetObjectByHash` reply into the fetch-pack cache. By-hash
    /// replies never carry node ids and never enter a session mailbox; the
    /// SHAMap sync filter consumes them by hash during traversal (rippled
    /// `addFetchPack` parity). Wiring to overlay ingress lands with M6-C.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn stash_fetch_pack(&self, hash: Uint256, data: Bytes) {
        self.ingress
            .stats
            .lock()
            .expect("adapter stats lock")
            .fetch_pack_stashes += 1;
        self.fetch_pack.add_fetch_pack(hash, data.to_vec());
    }

    /// Observe terminal effects after the runner has applied the event. The
    /// effect identifies the one session that changed; the runner remains the
    /// sole owner that decides whether it is a failure or a cancellation.
    fn note_terminal_failures(&mut self, effects: &[AcquisitionEffect]) {
        for session in effects.iter().filter_map(|effect| match effect {
            AcquisitionEffect::CancelSession(session) => Some(*session),
            _ => None,
        }) {
            if self
                .runner
                .session(session)
                .is_some_and(|state| matches!(state.phase(), SessionPhase::Failed { .. }))
            {
                self.terminal_failures.insert(session.target_hash());
            }
        }
    }

    /// Publish a fresh immutable routing snapshot reflecting the runner's live
    /// (non-terminal) sessions. Gates are cached per session: rebuilding a gate
    /// while its session still has in-flight leases would double-admit those
    /// leases on the next lookup.
    fn refresh_routes(&mut self) {
        let live = self
            .runner
            .live_sessions()
            .map(|session| (session.target_hash(), session))
            .collect::<BTreeMap<_, _>>();
        let mut changed = false;
        self.routes.retain(|hash, entry| {
            let keep = live
                .get(hash)
                .is_some_and(|session| entry.session() == *session);
            if !keep {
                changed = true;
            }
            keep
        });
        for (hash, session) in live {
            if let std::collections::btree_map::Entry::Vacant(entry) = self.routes.entry(hash) {
                entry.insert(RouteEntry::new(
                    session,
                    Arc::new(AdmissionGate::new(AdmissionBudget::default(), session)),
                ));
                changed = true;
            }
        }
        if changed {
            self.publish_routing();
        }
    }

    fn publish_routing(&mut self) {
        self.routing_generation += 1;
        *self
            .ingress
            .routing_snapshot
            .write()
            .expect("coordinator routing snapshot write") = Arc::new(RoutingSnapshot::new(
            RoutingGeneration::new(self.routing_generation),
            self.routes.clone(),
        ));
    }

    fn dispatch(&mut self, effects: &[AcquisitionEffect]) {
        if effects.is_empty() {
            return;
        }
        let mut ports = CoordinatorPorts {
            requests: &mut self.requests,
            reads: &mut self.reads,
            writes: &mut self.writes,
            timers: &mut self.timers,
            handoffs: &mut self.handoffs,
            phase: &mut self.phase,
            cancellations: &mut self.cancellations,
        };
        for effect in effects {
            ports.dispatch(effect.clone());
        }
    }
}

/// Maps a wire `TmLedgerData.r#type` to the packet type. Values match the
/// protobuf `liBASE`/`liTX_NODE`/`liAS_NODE` constants.
fn map_wire_ledger_type(value: i32) -> Option<InboundLedgerDataType> {
    match value {
        0 => Some(InboundLedgerDataType::Base),
        1 => Some(InboundLedgerDataType::TransactionNode),
        2 => Some(InboundLedgerDataType::StateNode),
        _ => None,
    }
}

/// Decodes one wire ledger node with rippled-compatible legacy or
/// `LedgerNodeDepth` reference validation. Base packets reject all references.
fn decode_wire_ledger_node(
    node: &overlay::message::wire::TmLedgerNode,
    packet_type: InboundLedgerDataType,
    packet_index: usize,
) -> Option<InboundLedgerNodeData> {
    super::wire_ledger_node::decode_wire_ledger_node(node, packet_type, packet_index)
}

/// Frames a coordinator peer request exactly like the acquisition actor framed
/// it. Separated from the port so tests assert framing deterministically.
fn frame_ledger_request(
    sequences: &BTreeMap<SessionRef, u32>,
    request: &PeerRequest,
) -> Option<ProtocolMessage> {
    let session = request.session();
    match request.request() {
        LedgerDataRequest::GetLedger { sequence } => Some(make_get_ledger_with_node_ids(
            SHAMapHash::new(session.target_hash()),
            sequence.unwrap_or(0),
            LI_BASE,
            &[],
            0,
            None,
        )),
        LedgerDataRequest::GetLedgerNodes {
            kind,
            node_ids,
            sequence,
        } => Some(make_get_ledger_with_node_ids(
            SHAMapHash::new(session.target_hash()),
            *sequence,
            match kind {
                ledger::TreeKind::State => LI_AS_NODE,
                ledger::TreeKind::Transaction => LI_TX_NODE,
            },
            node_ids,
            0,
            None,
        )),
        LedgerDataRequest::GetNodes { nodes, sequence } => {
            let first = nodes.first()?;
            if nodes.iter().any(|node| node.kind() != first.kind()) {
                // TMGetObjectByHash carries one `type`; mixed tree kinds must
                // have been split by the runner and must never be filtered.
                return None;
            }
            let object_type = match first.kind() {
                ledger::TreeKind::State => InboundLedgerObjectType::StateNode,
                ledger::TreeKind::Transaction => InboundLedgerObjectType::TransactionNode,
            };
            let needed = nodes
                .iter()
                .map(|node| (object_type, node.hash()))
                .collect::<Vec<_>>();
            make_inbound_needed_by_hash_request(
                SHAMapHash::new(session.target_hash()),
                sequence
                    .or_else(|| sequences.get(&session).copied())
                    .unwrap_or(0),
                &needed,
            )
        }
    }
}

/// Overlay delivery of coordinator-produced peer requests. Frames the request
/// exactly like the actor and delivers to the targeted peer.
pub(crate) struct OverlayLedgerRequestPort {
    peers: SimplePeerSet,
    sequences: Mutex<BTreeMap<SessionRef, u32>>,
}

impl OverlayLedgerRequestPort {
    /// A port over a peer set. `SimplePeerSet` tracks availability and delivers
    /// to a specific peer by ID.
    pub(crate) fn new(peers: SimplePeerSet) -> Self {
        Self {
            peers,
            sequences: Mutex::new(BTreeMap::new()),
        }
    }

    /// Refresh the tracked peer list (overlay availability fact).
    pub(crate) fn refresh_peers(&self, peers: impl IntoIterator<Item = Arc<dyn Peer>>) {
        self.peers.refresh_peers(peers);
    }
}

impl LedgerRequestPort for OverlayLedgerRequestPort {
    fn send_ledger_request(&mut self, request: PeerRequest) {
        {
            let mut sequences = self.sequences.lock().expect("request port sequences lock");
            if let LedgerDataRequest::GetLedger {
                sequence: Some(sequence),
            } = request.request()
            {
                sequences.insert(request.session(), *sequence);
            }
        }
        let Some(message) = frame_ledger_request(
            &self.sequences.lock().expect("request port sequences lock"),
            &request,
        ) else {
            return;
        };
        let peer_id = request.peer_id().get() as OverlayPeerId;
        // Deliver to the exact peer the coordinator selected from its current
        // availability snapshot. Coordinator sessions do not call the legacy
        // `PeerSet::add_peers` lifecycle, so `find_peer` would incorrectly
        // require an empty legacy membership set and silently drop every
        // request. The selected peer remains current-overlay availability;
        // peer-loss facts cancel its session instead of re-broadcasting.
        if let Some(peer) = self.peers.find_available_peer(peer_id) {
            self.peers.send_request(&message, Some(&peer));
        }
    }
}

/// Shared broker ticket state between the read port and the cancellation
/// dispatcher. Holds only resource-local tickets; the coordinator owns session
/// lifecycle.
#[derive(Clone, Debug, Default)]
pub(crate) struct BrokerTicketState {
    tickets: Arc<Mutex<BTreeMap<SessionRef, Vec<ReadTicket>>>>,
}

/// Brokered NodeStore read submission backed by the shared [`NodeReadBroker`].
/// Maps the broker's typed `ReadReady` completions into coordinator
/// `ReadCompleted` events; a rejected admission reports a terminal read so a
/// plan never stalls on an in-flight read.
pub(crate) struct BrokerReadPort {
    broker: NodeReadBroker,
    tickets: BrokerTicketState,
    node_store: SHAMapStoreNodeStore,
    completions: RetainedControlEvents,
}

impl BrokerReadPort {
    /// Builds a read port over a broker, its physical NodeStore target, and
    /// the shared ticket state.
    pub(crate) fn new(
        broker: NodeReadBroker,
        tickets: BrokerTicketState,
        node_store: SHAMapStoreNodeStore,
        tx: EventSender,
    ) -> Self {
        Self {
            broker,
            tickets,
            node_store,
            completions: RetainedControlEvents::new(tx),
        }
    }
}

impl ReadPort for BrokerReadPort {
    fn submit_read(&mut self, request: ReadRequest) {
        let operation = request.operation();
        let session = operation.session();
        let key = ReadKey::new(
            *request.key().as_uint256(),
            request.ledger_sequence(),
            request.store_generation().get(),
        );
        let acquisition_id = session.session_id().get();
        let plan_id = session.plan_epoch().get();
        let sink_completions = self.completions.clone();
        let tickets = self.tickets.clone();
        let sink: ReadReadySink = Arc::new(move |ready: ReadReady| {
            if let Some(set) = tickets
                .tickets
                .lock()
                .expect("broker ticket state lock")
                .get_mut(&session)
            {
                set.retain(|ticket| *ticket != ready.ticket);
            }
            let outcome = match ready.outcome {
                BrokerReadOutcome::Found(object) => ReadOutcome::Settled {
                    node: Some(Bytes::from(object.get_data().clone())),
                },
                BrokerReadOutcome::Miss => ReadOutcome::Settled { node: None },
                BrokerReadOutcome::Cancelled => ReadOutcome::Cancelled,
                // A physical fault settles as a miss: the coordinator counts it
                // and its bounded local-read retry covers transient store faults.
                BrokerReadOutcome::Fault(_) => ReadOutcome::Settled { node: None },
            };
            sink_completions.push(AcquisitionEvent::ReadCompleted(ReadCompletion::new(
                operation, outcome,
            )));
        });
        match self.broker.request(key, acquisition_id, plan_id, sink) {
            ReadAdmission::Accepted(ticket)
            | ReadAdmission::Attached(ticket)
            | ReadAdmission::Deferred(ticket) => {
                {
                    self.tickets
                        .tickets
                        .lock()
                        .expect("broker ticket state lock")
                        .entry(session)
                        .or_default()
                        .push(ticket);
                }
                // `request` releases the broker lock before returning, and
                // the ticket-state lock above is scoped to end before physical
                // submission. The broker owns dispatch/coalescing; this port
                // only gives every ready physical read to NodeStore.
                self.broker.submit_ready_to_node_store(&self.node_store);
            }
            // The broker is stopped; report a terminal read so the pending
            // plan settles instead of stalling on an in-flight read.
            ReadAdmission::Rejected(_) => {
                self.completions
                    .push(AcquisitionEvent::ReadCompleted(ReadCompletion::new(
                        operation,
                        ReadOutcome::Stale,
                    )));
            }
        }
    }

    fn flush_completions(&mut self) {
        self.completions.flush();
    }
}

/// Cancellation dispatch for brokered reads. Shares [`BrokerTicketState`] with
/// the [`BrokerReadPort`] so a session cancellation releases exactly its own
/// tickets. The broker settles each cancelled ticket through its sink after the
/// broker lock is released; the coordinator ignores completions for a cancelled
/// session.
#[derive(Clone)]
pub(crate) struct BrokerCancellationDispatcher {
    broker: NodeReadBroker,
    tickets: BrokerTicketState,
}

impl BrokerCancellationDispatcher {
    /// Builds a cancellation dispatcher sharing the broker and ticket state.
    pub(crate) fn new(broker: NodeReadBroker, tickets: BrokerTicketState) -> Self {
        Self { broker, tickets }
    }
}

impl CancellationPort for BrokerCancellationDispatcher {
    fn session_cancelled(&mut self, session: SessionRef) {
        let tickets = self
            .tickets
            .tickets
            .lock()
            .expect("broker ticket state lock")
            .remove(&session)
            .unwrap_or_default()
            .into_iter()
            .collect::<Vec<_>>();
        for ticket in tickets {
            let _ = self.broker.cancel(ticket);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::ReadBrokerConfig;
    use acquisition::fake::{
        FakeCancellationPort, FakeHandoffPort, FakeLedgerRequestPort, FakePhasePort, FakeReadPort,
        FakeTimerPort, FakeWritePort,
    };
    use acquisition::{
        AcquireReason, IdCounter, LedgerTarget, OperationKind, OperationRef, PlanEpoch,
        ReadPriority, SessionId, StoreGeneration, SyncPhase, TimerKind,
    };
    use overlay::TmLedgerData;
    use overlay::message::wire::TmLedgerNode;
    use shamap::node_id::{SHAMapNodeId, deserialize_shamap_node_id};
    use shamap::tree_node::SHAMapTreeNode;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    const SEQ: u32 = 77;

    fn session(n: u64) -> SessionRef {
        SessionRef::new(
            RunEpoch::new(1),
            SessionId::new(n),
            Uint256::from(n),
            PlanEpoch::new(1),
            StoreGeneration::new(1),
        )
    }

    fn target(hash: u64, seq: u32) -> LedgerTarget {
        LedgerTarget::new(Uint256::from(hash), Some(seq))
    }

    fn fetch_pack() -> Arc<FetchPackCache> {
        Arc::new(FetchPackCache::new(
            1024,
            time::Duration::seconds(600),
            basics::tagged_cache::MonotonicClock::default(),
        ))
    }

    fn runner() -> CoordinatorRunner {
        CoordinatorRunner::new(RunEpoch::new(1))
    }

    type TestAdapter = CoordinatorAdapter<
        FakeLedgerRequestPort,
        FakeReadPort,
        FakeWritePort,
        FakeTimerPort,
        FakeHandoffPort,
        FakePhasePort,
        FakeCancellationPort,
    >;

    #[derive(Clone)]
    struct InlineReplyRequestPort {
        ingress: Arc<Mutex<Option<CoordinatorIngress>>>,
        dispositions: Arc<Mutex<Vec<LedgerDataIngressDisposition>>>,
    }

    impl LedgerRequestPort for InlineReplyRequestPort {
        fn send_ledger_request(&mut self, request: PeerRequest) {
            let reply = wire_ledger_data(
                request.session().target_hash(),
                LI_BASE,
                vec![(None, vec![1, 2, 3])],
            );
            let disposition = self
                .ingress
                .lock()
                .expect("inline ingress slot lock")
                .as_ref()
                .expect("ingress is published before dispatch")
                .route_ledger_data(request.peer_id().get() as OverlayPeerId, &reply);
            self.dispositions
                .lock()
                .expect("inline reply dispositions lock")
                .push(disposition);
        }
    }

    type InlineReplyAdapter = CoordinatorAdapter<
        InlineReplyRequestPort,
        FakeReadPort,
        FakeWritePort,
        FakeTimerPort,
        FakeHandoffPort,
        FakePhasePort,
        FakeCancellationPort,
    >;

    fn adapter() -> (TestAdapter, Arc<FetchPackCache>) {
        let cache = fetch_pack();
        let runner = runner();
        let (tx, rx) = std::sync::mpsc::sync_channel(CONTROL_EVENT_QUEUE_CAPACITY);
        let (packet_tx, packet_rx) = std::sync::mpsc::sync_channel(PACKET_INGRESS_QUEUE_CAPACITY);
        let adapter = TestAdapter::with_event_channel(
            runner,
            FakeLedgerRequestPort::new(),
            FakeReadPort::new(),
            FakeWritePort::new(),
            FakeTimerPort::new(),
            FakeHandoffPort::new(),
            FakePhasePort::new(),
            FakeCancellationPort::new(),
            Arc::clone(&cache),
            tx,
            rx,
            packet_tx,
            packet_rx,
        );
        (adapter, cache)
    }

    /// Connect and acquire a session, returning the session the coordinator
    /// minted.
    fn acquired(adapter: &mut TestAdapter) -> SessionRef {
        let effects = adapter.connectivity(&[1]);
        assert!(
            effects.contains(&AcquisitionEffect::SetServicePhase(SyncPhase::Connected)),
            "connectivity must promote to Connected"
        );
        let effects = adapter.acquire_requested(target(9, SEQ), AcquireReason::Consensus);
        effects
            .iter()
            .find_map(|effect| match effect {
                AcquisitionEffect::SendLedgerRequest(request) => Some(request.session()),
                _ => None,
            })
            .expect("a peer request must be emitted")
    }

    fn valid_inner_wire() -> Vec<u8> {
        let mut wire = vec![0; 16 * 32];
        wire.push(shamap::tree_node::WIRE_TYPE_INNER);
        wire
    }

    fn base_root_wire() -> Vec<u8> {
        let root = basics::memory::intrusive_pointer::make_shared_intrusive(
            shamap::tree_node::SHAMapTreeNode::new_inner(1),
        );
        root.set_child_hash(3, SHAMapHash::new(Uint256::from(0x73)));
        root.update_hash();
        root.serialize_for_wire().expect("root wire serializes")
    }

    fn wire_ledger_data(
        hash: Uint256,
        r#type: i32,
        nodes: Vec<(Option<Vec<u8>>, Vec<u8>)>,
    ) -> TmLedgerData {
        TmLedgerData {
            ledger_hash: hash.data().to_vec(),
            ledger_seq: SEQ,
            r#type,
            nodes: nodes
                .into_iter()
                .map(|(nodeid, nodedata)| TmLedgerNode {
                    nodedata,
                    nodeid,
                    ..TmLedgerNode::default()
                })
                .collect(),
            ..TmLedgerData::default()
        }
    }

    #[test]
    fn synchronous_reply_uses_immutable_ingress_while_request_dispatches() {
        let cache = fetch_pack();
        let (tx, rx) = std::sync::mpsc::sync_channel(CONTROL_EVENT_QUEUE_CAPACITY);
        let (packet_tx, packet_rx) = std::sync::mpsc::sync_channel(PACKET_INGRESS_QUEUE_CAPACITY);
        let ingress = Arc::new(Mutex::new(None));
        let dispositions = Arc::new(Mutex::new(Vec::new()));
        let requests = InlineReplyRequestPort {
            ingress: Arc::clone(&ingress),
            dispositions: Arc::clone(&dispositions),
        };
        let mut adapter = InlineReplyAdapter::with_event_channel(
            runner(),
            requests,
            FakeReadPort::new(),
            FakeWritePort::new(),
            FakeTimerPort::new(),
            FakeHandoffPort::new(),
            FakePhasePort::new(),
            FakeCancellationPort::new(),
            cache,
            tx,
            rx,
            packet_tx,
            packet_rx,
        );
        *ingress.lock().expect("inline ingress slot lock") = Some(adapter.ingress());

        adapter.connectivity(&[1]);
        let effects = adapter.acquire_requested(target(9, SEQ), AcquireReason::Consensus);
        assert!(effects.iter().any(|effect| matches!(
            effect,
            AcquisitionEffect::SendLedgerRequest(request) if request.session().target_hash() == Uint256::from(9)
        )));
        assert_eq!(
            dispositions
                .lock()
                .expect("inline reply dispositions lock")
                .as_slice(),
            &[LedgerDataIngressDisposition::Delivered],
            "an inline reply must enqueue through immutable ingress rather than re-enter the mutable owner"
        );
        assert_eq!(
            adapter.drain(),
            1,
            "the owner consumes the queued reply later"
        );
        assert_eq!(adapter.snapshot().packets_admitted(), 1);
    }

    #[test]
    fn acquire_dispatches_request_and_publishes_a_route() {
        let (mut adapter, _cache) = adapter();
        let session = acquired(&mut adapter);
        assert_eq!(
            adapter.snapshot().phase(),
            &SyncPhase::Syncing {
                target: target(9, SEQ)
            }
        );
        assert_eq!(
            adapter.snapshot().session_count(),
            1,
            "one live session must be tracked"
        );

        let snapshot = adapter.routing_snapshot();
        let route = snapshot
            .route(&Uint256::from(9))
            .expect("a route must exist for the acquired target");
        assert_eq!(route.session(), session);
    }

    #[test]
    fn preferred_lcl_divergence_demotes_tracking_without_a_session() {
        let (mut adapter, _cache) = adapter();
        let _session = acquired(&mut adapter);
        adapter.push(AcquisitionEvent::LclInstalled(
            acquisition::LedgerIdentity::new(Uint256::from(9), SEQ),
        ));
        assert_eq!(adapter.drain(), 1);
        assert_eq!(
            adapter.snapshot().phase(),
            &SyncPhase::Tracking {
                lcl: acquisition::LedgerIdentity::new(Uint256::from(9), SEQ)
            }
        );

        let effects = adapter.preferred_lcl_divergence(target(11, 11));
        assert_eq!(
            effects,
            vec![AcquisitionEffect::SetServicePhase(SyncPhase::Syncing {
                target: target(11, 11)
            })]
        );
        // A divergence fact is phase-only: it never mints a session. The
        // demand arrives as a separate acquire_requested fact.
        assert_eq!(adapter.snapshot().session_count(), 1);
        assert_eq!(
            adapter.snapshot().phase(),
            &SyncPhase::Syncing {
                target: target(11, 11)
            }
        );
    }

    #[test]
    fn preferred_lcl_divergence_then_acquire_mints_the_session() {
        let (mut adapter, _cache) = adapter();
        let _session = acquired(&mut adapter);
        adapter.push(AcquisitionEvent::LclInstalled(
            acquisition::LedgerIdentity::new(Uint256::from(9), SEQ),
        ));
        assert_eq!(adapter.drain(), 1);

        let effects = adapter.preferred_lcl_divergence(target(11, 11));
        assert_eq!(
            effects,
            vec![AcquisitionEffect::SetServicePhase(SyncPhase::Syncing {
                target: target(11, 11)
            })]
        );
        let effects = adapter.acquire_requested(target(11, 11), AcquireReason::Consensus);
        let session = effects
            .iter()
            .find_map(|effect| match effect {
                AcquisitionEffect::SendLedgerRequest(request) => Some(request.session()),
                _ => None,
            })
            .expect("the recovery demand must mint a session");
        assert_eq!(session.target_hash(), Uint256::from(11));
        assert_eq!(adapter.snapshot().session_count(), 2);
        assert_eq!(adapter.snapshot().rejected_events(), 0);
    }

    #[test]
    fn blocked_with_no_target_demotes_full_to_connected() {
        let (mut adapter, _cache) = adapter();
        let _session = acquired(&mut adapter);
        adapter.push(AcquisitionEvent::LclInstalled(
            acquisition::LedgerIdentity::new(Uint256::from(9), SEQ),
        ));
        adapter.push(AcquisitionEvent::PublicationCommitted {
            identity: acquisition::LedgerIdentity::new(Uint256::from(9), SEQ),
            fresh: true,
        });
        assert_eq!(adapter.drain(), 2);
        assert_eq!(
            adapter.snapshot().phase(),
            &SyncPhase::Full {
                lcl: acquisition::LedgerIdentity::new(Uint256::from(9), SEQ),
                published: acquisition::LedgerIdentity::new(Uint256::from(9), SEQ),
            }
        );

        let effects = adapter.blocked_with_no_target();
        assert_eq!(
            effects,
            vec![AcquisitionEffect::SetServicePhase(SyncPhase::Connected)]
        );
        assert_eq!(adapter.snapshot().phase(), &SyncPhase::Connected);
        assert_eq!(adapter.snapshot().session_count(), 1);
    }

    #[test]
    fn wire_ledger_data_routes_by_hash_and_preserves_node_ids() {
        let (mut adapter, _cache) = adapter();
        let session = acquired(&mut adapter);
        let snapshot = adapter.routing_snapshot();
        let gate = snapshot.route(&Uint256::from(9)).unwrap().gate().clone();

        let node_id = SHAMapNodeId::default().get_raw_string().to_vec();
        let message = wire_ledger_data(
            Uint256::from(9),
            2, // liAS_NODE
            vec![(Some(node_id.clone()), valid_inner_wire())],
        );
        let disposition = adapter.route_ledger_data(1, &message);
        assert_eq!(disposition, LedgerDataIngressDisposition::Delivered);
        assert_eq!(gate.current_packets(), 1, "admission reserves one packet");

        // The admitted packet preserves the wire node id verbatim through
        // ingress and into the owner queue.
        let mut pending = adapter.pending_events();
        assert_eq!(pending.len(), 1, "exactly one admitted packet is queued");
        let packet = match pending.remove(0) {
            AcquisitionEvent::PacketAdmitted(packet) => packet,
            other => panic!("expected an admitted packet, got {other:?}"),
        };
        assert_eq!(packet.peer_id(), PeerId::new(1));
        assert_eq!(packet.packet().nodes.len(), 1);
        assert_eq!(
            packet.packet().nodes[0].node_id.as_deref(),
            Some(node_id.as_slice()),
            "wire node ids survive into the admission packet"
        );

        // The owner consumes the admitted packet and returns the ingress
        // capacity; with the null plan seed the packet is retained by the
        // session mailbox.
        adapter.push(AcquisitionEvent::PacketAdmitted(packet));
        assert_eq!(adapter.drain(), 1);
        assert_eq!(adapter.snapshot().packets_admitted(), 1);
        assert_eq!(gate.current_packets(), 0, "the owner releases admission");
        assert_eq!(
            adapter
                .runner
                .session(session)
                .expect("live session")
                .packet_count(),
            1,
            "the packet is retained by the session mailbox (no engine seeded)"
        );
    }

    #[test]
    fn missing_node_id_is_invalid_for_state_packets() {
        let (mut adapter, _cache) = adapter();
        acquired(&mut adapter);
        let message = wire_ledger_data(
            Uint256::from(9),
            2, // liAS_NODE
            vec![(None, vec![1, 2, 3])],
        );
        assert_eq!(
            adapter.route_ledger_data(1, &message),
            LedgerDataIngressDisposition::Invalid
        );
    }

    #[test]
    fn base_packets_apply_without_node_ids() {
        let (mut adapter, _cache) = adapter();
        acquired(&mut adapter);
        let message = wire_ledger_data(
            Uint256::from(9),
            0, // liBASE
            vec![(None, vec![1, 2, 3])],
        );
        assert_eq!(
            adapter.route_ledger_data(1, &message),
            LedgerDataIngressDisposition::Delivered
        );
    }

    #[test]
    fn live_base_ingress_admits_rippled_network_wire_roots_without_rewriting() {
        let (mut adapter, _cache) = adapter();
        acquired(&mut adapter);
        let expected_wire = base_root_wire();
        let message = wire_ledger_data(
            Uint256::from(9),
            0, // liBASE
            vec![(None, vec![0xAB; 123]), (None, expected_wire.clone())],
        );

        assert_eq!(
            adapter.route_ledger_data(1, &message),
            LedgerDataIngressDisposition::Delivered
        );
        let mut pending = adapter.pending_events();
        let AcquisitionEvent::PacketAdmitted(packet) = pending.remove(0) else {
            panic!("Base reply must enter coordinator as an admitted packet");
        };
        assert_eq!(packet.packet().nodes[0].node_data, vec![0xAB; 123]);
        assert_eq!(packet.packet().nodes[1].node_data, expected_wire);
        assert!(
            SHAMapTreeNode::make_from_wire(&packet.packet().nodes[1].node_data)
                .expect("rippled root decodes as network wire")
                .is_some()
        );
        adapter.push(AcquisitionEvent::PacketAdmitted(packet));
        assert_eq!(adapter.drain(), 1);
        assert_eq!(adapter.snapshot().packets_admitted(), 1);
    }

    #[test]
    fn live_base_ingress_rejects_malformed_network_wire_roots() {
        let (mut adapter, _cache) = adapter();
        acquired(&mut adapter);
        let message = wire_ledger_data(
            Uint256::from(9),
            0, // liBASE
            vec![(None, vec![0xAB; 123]), (None, vec![0xFF])],
        );
        assert_eq!(
            adapter.route_ledger_data(1, &message),
            LedgerDataIngressDisposition::Invalid
        );
        assert!(adapter.pending_events().is_empty());
    }

    #[test]
    fn failed_terminal_session_is_reported_once_for_registry_cooldown() {
        let (mut adapter, _cache) = adapter();
        adapter.connectivity(&[1]);
        let effects = adapter.acquire_requested(target(9, SEQ), AcquireReason::Generic);
        let session = effects
            .iter()
            .find_map(|effect| match effect {
                AcquisitionEffect::SendLedgerRequest(request) => Some(request.session()),
                _ => None,
            })
            .expect("acquisition starts a session");
        let mut timeout = effects
            .iter()
            .find_map(|effect| match effect {
                AcquisitionEffect::ArmTimer(request)
                    if request.timer() == TimerKind::AcquireTimeout =>
                {
                    Some(request.clone().operation())
                }
                _ => None,
            })
            .expect("acquisition arms its timeout");

        for _ in 0..5 {
            let effects = adapter.handle_fact(AcquisitionEvent::TimerFired {
                operation: timeout,
                timer: TimerKind::AcquireTimeout,
            });
            timeout = effects
                .iter()
                .find_map(|effect| match effect {
                    AcquisitionEffect::ArmTimer(request)
                        if request.timer() == TimerKind::AcquireTimeout =>
                    {
                        Some(request.clone().operation())
                    }
                    _ => None,
                })
                .expect("pre-terminal timeout rearms with an exact new operation");
        }

        let effects = adapter.handle_fact(AcquisitionEvent::TimerFired {
            operation: timeout,
            timer: TimerKind::AcquireTimeout,
        });
        assert!(effects.contains(&AcquisitionEffect::CancelSession(session)));
        assert_eq!(
            adapter.take_terminal_failures(),
            vec![Uint256::from(9)],
            "the registry must receive the failed target once for cooldown admission"
        );
        assert!(
            adapter.take_terminal_failures().is_empty(),
            "draining terminal failures must not repeatedly extend the cooldown"
        );
    }

    #[test]
    fn unmatched_and_terminal_packets_are_never_admitted() {
        let (mut adapter, _cache) = adapter();
        acquired(&mut adapter);

        let message = wire_ledger_data(Uint256::from(99), 0, vec![(None, vec![1, 2, 3])]);
        assert_eq!(
            adapter.route_ledger_data(1, &message),
            LedgerDataIngressDisposition::Unmatched
        );

        // A replacement cancels the old session and drops its route: the same
        // target hash now routes to the new session's gate.
        let effects = adapter.acquire_requested(target(9, SEQ + 1), AcquireReason::Consensus);
        assert!(
            effects
                .iter()
                .any(|effect| matches!(effect, AcquisitionEffect::CancelSession(_)))
        );
        let snapshot = adapter.routing_snapshot();
        let route = snapshot
            .route(&Uint256::from(9))
            .expect("replacement route");
        assert_ne!(route.session(), session(1));
    }

    #[test]
    fn packet_ingress_queue_full_defers_and_releases_its_lease() {
        let (mut adapter, _cache) = adapter();
        acquired(&mut adapter);
        let gate = adapter
            .routing_snapshot()
            .route(&Uint256::from(9))
            .expect("live route")
            .gate()
            .clone();
        let message = wire_ledger_data(Uint256::from(9), 0, vec![(None, vec![1, 2, 3])]);

        for _ in 0..PACKET_INGRESS_QUEUE_CAPACITY {
            assert_eq!(
                adapter.route_ledger_data(1, &message),
                LedgerDataIngressDisposition::Delivered
            );
        }
        assert_eq!(gate.current_packets(), PACKET_INGRESS_QUEUE_CAPACITY as u64);

        // The next packet can reserve the per-session gate, but the bounded
        // global ingress queue rejects it. Its lease is settled immediately,
        // so it cannot become a mailbox/session side effect.
        assert_eq!(
            adapter.route_ledger_data(1, &message),
            LedgerDataIngressDisposition::Deferred
        );
        assert_eq!(gate.current_packets(), PACKET_INGRESS_QUEUE_CAPACITY as u64);
        assert_eq!(adapter.snapshot().packets_admitted(), 0);
    }

    #[test]
    fn drain_prioritizes_and_bounds_control_then_packet_work() {
        let (mut adapter, _cache) = adapter();
        acquired(&mut adapter);
        let message = wire_ledger_data(Uint256::from(9), 0, vec![(None, vec![1, 2, 3])]);
        for _ in 0..(PACKET_EVENTS_PER_DRAIN + 1) {
            assert_eq!(
                adapter.route_ledger_data(1, &message),
                LedgerDataIngressDisposition::Delivered
            );
        }
        adapter.push(AcquisitionEvent::LclInstalled(
            acquisition::LedgerIdentity::new(Uint256::from(9), SEQ),
        ));

        assert_eq!(adapter.drain(), 1 + PACKET_EVENTS_PER_DRAIN);
        assert_eq!(
            adapter.snapshot().phase(),
            &SyncPhase::Tracking {
                lcl: acquisition::LedgerIdentity::new(Uint256::from(9), SEQ)
            },
            "the queued control fact runs before the bounded packet turn"
        );
        assert_eq!(adapter.drain(), 1, "one packet remains for the next slice");
    }

    #[test]
    fn control_queue_is_bounded_and_recovers_after_a_drain_slice() {
        let (mut adapter, _cache) = adapter();
        let events = adapter.event_sender();
        for _ in 0..CONTROL_EVENT_QUEUE_CAPACITY {
            events
                .try_send(AcquisitionEvent::LclInstalled(
                    acquisition::LedgerIdentity::new(Uint256::from(9), SEQ),
                ))
                .expect("reserved control capacity");
        }
        assert!(matches!(
            events.try_send(AcquisitionEvent::LclInstalled(
                acquisition::LedgerIdentity::new(Uint256::from(9), SEQ),
            )),
            Err(mpsc::TrySendError::Full(_))
        ));

        assert_eq!(adapter.drain(), CONTROL_EVENTS_PER_DRAIN);
        events
            .try_send(AcquisitionEvent::LclInstalled(
                acquisition::LedgerIdentity::new(Uint256::from(9), SEQ),
            ))
            .expect("draining restores reserved lifecycle capacity");
    }

    #[test]
    fn deferred_packet_has_no_actor_side_effect() {
        let (mut adapter, _cache) = adapter();
        acquired(&mut adapter);
        let snapshot = adapter.routing_snapshot();
        let gate = snapshot.route(&Uint256::from(9)).unwrap().gate().clone();

        // Fill the gate's packet budget with live in-flight leases.
        let leases = (0..128)
            .map(|_| match gate.try_reserve(1, 1) {
                BackpressureOutcome::Admitted(lease) => lease,
                other => panic!("expected a reservation, got {other:?}"),
            })
            .collect::<Vec<_>>();
        assert_eq!(leases.len(), 128);
        let message = wire_ledger_data(Uint256::from(9), 0, vec![(None, vec![1, 2, 3])]);
        assert_eq!(
            adapter.route_ledger_data(1, &message),
            LedgerDataIngressDisposition::Deferred
        );
        assert_eq!(adapter.drain(), 0, "a deferred packet enqueues nothing");
    }

    #[test]
    fn fetch_pack_stash_does_not_enter_a_session() {
        let (mut adapter, cache) = adapter();
        acquired(&mut adapter);
        let data = Bytes::from_static(&[9, 9, 9]);
        let hash = protocol::sha512_half(&data);
        adapter.stash_fetch_pack(hash, data.clone());
        assert_eq!(adapter.drain(), 0, "a by-hash reply enqueues no event");
        assert_eq!(
            cache.get_fetch_pack(hash),
            Some(data.to_vec()),
            "the pack is retrievable by its content hash"
        );
    }

    #[test]
    fn get_ledger_requests_are_framed_with_li_base() {
        let mut counter = IdCounter::new();
        let session = session(1);
        let request = PeerRequest::new(
            session,
            OperationRef::new(
                session,
                OperationKind::PeerRequest,
                counter.next_id(),
                counter.next_id(),
            ),
            PeerId::new(7),
            LedgerDataRequest::GetLedger {
                sequence: Some(SEQ),
            },
        );
        let frame = frame_ledger_request(&BTreeMap::new(), &request).expect("GetLedger must frame");
        match frame.payload {
            overlay::ProtocolPayload::GetLedger(tm) => {
                assert_eq!(tm.itype, LI_BASE);
                assert_eq!(tm.ledger_seq, Some(SEQ));
                assert_eq!(
                    tm.ledger_hash.as_deref(),
                    Some(session.target_hash().data().as_slice())
                );
            }
            other => panic!("expected GetLedger, got {other:?}"),
        }
    }

    #[test]
    fn ledger_node_requests_preserve_node_ids_and_frame_as_state_ledger_data() {
        let mut counter = IdCounter::new();
        let session = session(1);
        let node_id = SHAMapNodeId::default();
        let request = PeerRequest::new(
            session,
            OperationRef::new(
                session,
                OperationKind::PeerRequest,
                counter.next_id(),
                counter.next_id(),
            ),
            PeerId::new(7),
            LedgerDataRequest::GetLedgerNodes {
                kind: ledger::TreeKind::State,
                node_ids: vec![node_id],
                sequence: SEQ,
            },
        );
        let frame =
            frame_ledger_request(&BTreeMap::new(), &request).expect("node request must frame");
        match frame.payload {
            overlay::ProtocolPayload::GetLedger(tm) => {
                assert_eq!(tm.itype, LI_AS_NODE);
                assert_eq!(tm.ledger_seq, Some(SEQ));
                assert_eq!(tm.node_i_ds, vec![node_id.get_raw_string()]);
            }
            other => panic!("expected GetLedger, got {other:?}"),
        }
    }

    #[test]
    fn get_nodes_requests_are_framed_with_the_promoted_sequence() {
        let mut counter = IdCounter::new();
        let session = session(1);
        let hashes = vec![Uint256::from(0xAB), Uint256::from(0xCD)];
        let nodes = vec![
            acquisition::LedgerNodeRequest::new(hashes[0], ledger::TreeKind::State),
            acquisition::LedgerNodeRequest::new(hashes[1], ledger::TreeKind::State),
        ];
        let request = PeerRequest::new(
            session,
            OperationRef::new(
                session,
                OperationKind::PeerRequest,
                counter.next_id(),
                counter.next_id(),
            ),
            PeerId::new(7),
            LedgerDataRequest::GetNodes {
                nodes,
                sequence: Some(SEQ),
            },
        );
        let frame = frame_ledger_request(&BTreeMap::new(), &request).expect("GetNodes must frame");
        match frame.payload {
            overlay::ProtocolPayload::GetObjects(tm) => {
                assert!(tm.query, "a GetNodes request is a query");
                assert_eq!(
                    tm.ledger_hash.as_deref(),
                    Some(session.target_hash().data().as_slice())
                );
                assert_eq!(tm.objects.len(), 2);
                assert_eq!(
                    tm.objects[0].hash.as_deref(),
                    Some(hashes[0].data().as_slice())
                );
                assert_eq!(
                    tm.objects[1].hash.as_deref(),
                    Some(hashes[1].data().as_slice())
                );
                assert_eq!(tm.objects[0].ledger_seq, Some(SEQ));
                assert_eq!(tm.r#type, 4, "state-node object-by-hash type");
            }
            other => panic!("expected GetObjects, got {other:?}"),
        }
    }

    #[test]
    fn mixed_tree_kind_request_is_rejected_before_wire_framing() {
        let mut counter = IdCounter::new();
        let session = session(1);
        let request = PeerRequest::new(
            session,
            OperationRef::new(
                session,
                OperationKind::PeerRequest,
                counter.next_id(),
                counter.next_id(),
            ),
            PeerId::new(7),
            LedgerDataRequest::GetNodes {
                nodes: vec![
                    acquisition::LedgerNodeRequest::new(Uint256::from(1), ledger::TreeKind::State),
                    acquisition::LedgerNodeRequest::new(
                        Uint256::from(2),
                        ledger::TreeKind::Transaction,
                    ),
                ],
                sequence: Some(SEQ),
            },
        );
        assert!(frame_ledger_request(&BTreeMap::new(), &request).is_none());
    }
    #[test]
    fn unknown_sequence_still_frames_a_base_request() {
        let mut counter = IdCounter::new();
        let session = session(1);
        let request = PeerRequest::new(
            session,
            OperationRef::new(
                session,
                OperationKind::PeerRequest,
                counter.next_id(),
                counter.next_id(),
            ),
            PeerId::new(7),
            LedgerDataRequest::GetLedger { sequence: None },
        );
        let frame = frame_ledger_request(&BTreeMap::new(), &request)
            .expect("unknown target must frame a Base request");
        match frame.payload {
            overlay::ProtocolPayload::GetLedger(tm) => {
                assert_eq!(tm.itype, LI_BASE);
                assert_eq!(tm.ledger_seq, None);
            }
            other => panic!("expected GetLedger Base, got {other:?}"),
        }
    }

    #[test]
    fn transaction_node_request_frames_a_transaction_object_type() {
        let mut counter = IdCounter::new();
        let session = session(1);
        let request = PeerRequest::new(
            session,
            OperationRef::new(
                session,
                OperationKind::PeerRequest,
                counter.next_id(),
                counter.next_id(),
            ),
            PeerId::new(7),
            LedgerDataRequest::GetNodes {
                nodes: vec![acquisition::LedgerNodeRequest::new(
                    Uint256::from(1),
                    ledger::TreeKind::Transaction,
                )],
                sequence: Some(SEQ),
            },
        );
        let frame =
            frame_ledger_request(&BTreeMap::new(), &request).expect("transaction node must frame");
        match frame.payload {
            overlay::ProtocolPayload::GetObjects(tm) => {
                assert_eq!(tm.r#type, 3, "transaction-node object-by-hash type");
            }
            other => panic!("expected GetObjects, got {other:?}"),
        }
    }

    #[test]
    fn coordinator_request_reaches_a_refreshed_peer_without_legacy_membership() {
        use std::net::{IpAddr, Ipv4Addr, SocketAddr};

        use protocol::{KeyType, SecretKey, derive_public_key};

        let peer = overlay::PeerImp::new(
            7,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 6007),
            derive_public_key(KeyType::Secp256k1, &SecretKey::from_bytes([7; 32]))
                .expect("peer public key"),
            "coordinator-available-peer",
        );
        let peers = SimplePeerSet::new(Vec::new());
        let mut port = OverlayLedgerRequestPort::new(peers);
        port.refresh_peers(vec![Arc::clone(&peer) as Arc<dyn Peer>]);

        let session = session(7);
        let mut counter = IdCounter::new();
        port.send_ledger_request(PeerRequest::new(
            session,
            OperationRef::new(
                session,
                OperationKind::PeerRequest,
                counter.next_id(),
                counter.next_id(),
            ),
            PeerId::new(7),
            LedgerDataRequest::GetLedger {
                sequence: Some(SEQ),
            },
        ));

        assert_eq!(
            peer.queued_messages().len(),
            1,
            "coordinator-targeted delivery must not depend on legacy add_peers membership"
        );
    }

    fn broker_test_node_store_with(
        path: &str,
        scheduler: Arc<dyn nodestore::Scheduler>,
    ) -> SHAMapStoreNodeStore {
        use basics::basic_config::Section;
        use nodestore::{Manager as _, ManagerImp, NullJournal};

        let manager = ManagerImp::new();
        let mut config = Section::new("node_db");
        config.set("type", "Memory");
        config.set("path", path);
        SHAMapStoreNodeStore::Single(
            manager
                .make_database(0, scheduler, 1, &config, Arc::new(NullJournal))
                .expect("memory node store"),
        )
    }

    fn broker_test_node_store(path: &str) -> SHAMapStoreNodeStore {
        broker_test_node_store_with(path, Arc::new(nodestore::DummyScheduler))
    }

    /// Retains scheduled NodeStore tasks so a test can prove read submission
    /// occurred without racing its typed completion against cancellation.
    #[derive(Default)]
    struct BrokerCaptureScheduler {
        queued: Mutex<VecDeque<Arc<dyn nodestore::Task>>>,
    }

    impl BrokerCaptureScheduler {
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

    impl nodestore::Scheduler for BrokerCaptureScheduler {
        fn schedule_task(&self, task: Arc<dyn nodestore::Task>) {
            self.queued
                .lock()
                .expect("captured task lock")
                .push_back(task);
        }

        fn on_fetch(&self, _report: nodestore::FetchReport) {}

        fn on_batch_write(&self, _report: nodestore::BatchWriteReport) {}
    }

    #[test]
    fn broker_read_maps_completions_and_cancellation_releases_tickets() {
        let broker = NodeReadBroker::new(ReadBrokerConfig::default()).expect("broker");
        let tickets = BrokerTicketState::default();
        let (tx, rx) = mpsc::sync_channel(CONTROL_EVENT_QUEUE_CAPACITY);
        let scheduler = Arc::new(BrokerCaptureScheduler::default());
        let mut port = BrokerReadPort::new(
            broker.clone(),
            tickets.clone(),
            broker_test_node_store_with("broker-read-cancel", scheduler.clone()),
            tx,
        );

        let session = session(2);
        let mut counter = IdCounter::new();
        let operation = OperationRef::new(
            session,
            OperationKind::Read,
            counter.next_id(),
            counter.next_id(),
        );
        let request = ReadRequest::new(
            operation,
            SHAMapHash::new(Uint256::from(0x11)),
            1,
            StoreGeneration::new(1),
            ReadPriority::Consensus,
        );

        let mut dispatcher = BrokerCancellationDispatcher::new(broker.clone(), tickets.clone());
        port.submit_read(request);
        assert_eq!(
            broker.snapshot().in_flight_keys,
            1,
            "accepted broker work is physically submitted after broker admission"
        );
        assert_eq!(
            tickets
                .tickets
                .lock()
                .unwrap()
                .get(&session)
                .map(|set| set.len()),
            Some(1),
            "an in-flight read retains exactly one ticket"
        );

        // Once broker admission has submitted physical I/O, cancellation races
        // a fast store miss. Either terminal outcome is valid for this exact
        // operation; the coordinator rejects it as stale after cancellation.
        dispatcher.session_cancelled(session);
        assert!(
            tickets.tickets.lock().unwrap().get(&session).is_none(),
            "cancellation releases every ticket for the session"
        );
        let event = rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("completion");
        match event {
            AcquisitionEvent::ReadCompleted(completion) => {
                assert_eq!(completion.operation(), operation);
                assert!(
                    matches!(
                        completion.outcome(),
                        ReadOutcome::Cancelled | ReadOutcome::Settled { node: None }
                    ),
                    "a submitted read may be cancelled or win the cancellation race as a miss"
                );
            }
            other => panic!("expected a read completion, got {other:?}"),
        }
    }

    #[test]
    fn broker_submitted_read_settles_as_an_exact_typed_completion() {
        let broker = NodeReadBroker::new(ReadBrokerConfig::default()).expect("broker");
        let tickets = BrokerTicketState::default();
        let (tx, rx) = mpsc::sync_channel(CONTROL_EVENT_QUEUE_CAPACITY);
        let scheduler = Arc::new(BrokerCaptureScheduler::default());
        let mut port = BrokerReadPort::new(
            broker.clone(),
            tickets,
            broker_test_node_store_with("broker-read-submit", scheduler.clone()),
            tx,
        );
        let session = session(4);
        let mut counter = IdCounter::new();
        let operation = OperationRef::new(
            session,
            OperationKind::Read,
            counter.next_id(),
            counter.next_id(),
        );

        port.submit_read(ReadRequest::new(
            operation,
            SHAMapHash::new(Uint256::from(0x44)),
            1,
            StoreGeneration::new(1),
            ReadPriority::Consensus,
        ));
        assert_eq!(
            broker.snapshot().in_flight_keys,
            1,
            "ready dispatch reaches the NodeStore asynchronous read queue"
        );
        match rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("typed NodeStore completion")
        {
            AcquisitionEvent::ReadCompleted(completion) => {
                assert_eq!(completion.operation(), operation);
                assert_eq!(completion.outcome(), &ReadOutcome::Settled { node: None });
            }
            other => panic!("expected a read completion, got {other:?}"),
        }
    }

    #[test]
    fn rejected_broker_admission_reports_a_terminal_read() {
        let broker = NodeReadBroker::new(ReadBrokerConfig::default()).expect("broker");
        broker.stop();
        let tickets = BrokerTicketState::default();
        let (tx, rx) = mpsc::sync_channel(CONTROL_EVENT_QUEUE_CAPACITY);
        let mut port = BrokerReadPort::new(
            broker,
            tickets,
            broker_test_node_store("broker-read-rejected"),
            tx,
        );

        let session = session(3);
        let mut counter = IdCounter::new();
        let operation = OperationRef::new(
            session,
            OperationKind::Read,
            counter.next_id(),
            counter.next_id(),
        );
        let request = ReadRequest::new(
            operation,
            SHAMapHash::new(Uint256::from(0x22)),
            1,
            StoreGeneration::new(1),
            ReadPriority::Consensus,
        );
        port.submit_read(request);
        let event = rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("terminal completion");
        match event {
            AcquisitionEvent::ReadCompleted(completion) => {
                assert_eq!(completion.operation(), operation);
                assert_eq!(completion.outcome(), &ReadOutcome::Stale);
            }
            other => panic!("expected a read completion, got {other:?}"),
        }
    }

    #[test]
    fn broker_read_key_matches_route_generation_scope() {
        // The read key isolates by (hash, ledger sequence, store generation), so
        // two sessions at different generations never coalesce.
        let session_a = SessionRef::new(
            RunEpoch::new(1),
            SessionId::new(1),
            Uint256::from(5),
            PlanEpoch::new(1),
            StoreGeneration::new(1),
        );
        let session_b = SessionRef::new(
            RunEpoch::new(1),
            SessionId::new(2),
            Uint256::from(5),
            PlanEpoch::new(1),
            StoreGeneration::new(2),
        );
        assert_ne!(session_a.store_generation(), session_b.store_generation());
        let key_a = ReadKey::new(
            *SHAMapHash::new(Uint256::from(0x33)).as_uint256(),
            1,
            session_a.store_generation().get(),
        );
        let key_b = ReadKey::new(
            *SHAMapHash::new(Uint256::from(0x33)).as_uint256(),
            1,
            session_b.store_generation().get(),
        );
        assert_ne!(key_a, key_b);
    }

    #[test]
    fn node_id_preservation_round_trips_the_wire() {
        // Root node ids survive wire -> InboundLedgerNodeData unchanged.
        let raw = SHAMapNodeId::default().get_raw_string().to_vec();
        assert_eq!(
            deserialize_shamap_node_id(&raw).expect("default root id deserializes"),
            SHAMapNodeId::default()
        );
        let node = decode_wire_ledger_node(
            &TmLedgerNode {
                nodedata: valid_inner_wire(),
                nodeid: Some(raw.clone()),
                ..TmLedgerNode::default()
            },
            InboundLedgerDataType::StateNode,
            0,
        )
        .expect("a node with an id decodes");
        assert_eq!(node.node_id.as_deref(), Some(raw.as_slice()));
    }
}
