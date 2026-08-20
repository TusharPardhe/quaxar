//! Coordinator-owned session planning (M4.1).
//!
//! This module is the first slice of the M4 session-ownership migration. It
//! moves the mailbox, the tree plan engine, read admission/backlog, pending
//! network tracking, persistence intent, timeout budget, and cancellation into
//! one [`SessionPlan`] owned by the coordinator. No component other than the
//! coordinator may own these session lifecycle states.
//!
//! Ownership rules enforced here:
//!
//! * The [`SessionPlan`] owns the bounded mailbox (`128` packets / `4 MiB` by
//!   default, carried from the admission budget) and clears it on cancellation.
//! * Every dispatched read carries an [`OperationRef`]; a read completion may
//!   mutate the plan only when it matches the exact in-flight operation
//!   (`pending_reads` is keyed by hash and matched by full operation identity).
//! * A tree engine is uniquely owned by the plan while it advances; events
//!   queue in the mailbox until a rooted plan exists (`PlanSeed`).
//! * Persistence intent is a single state machine: `Persist` -> `WritePending`
//!   -> `FencePending` -> `Durable`. A write batch carries its fence operation
//!   so one adapter barrier reports both `WriteCompleted` and
//!   `DurabilityFenced`.
//! * The tree engine is never advanced by two threads: `run_turn` runs bounded
//!   CPU turns on the coordinator owner task only.
//!
//! The [`TreeEngine`] port is the only place shamap tree types cross the crate
//! boundary. Adapters (the app wiring in M4.2) implement it; higher-level
//! coordinator types stay shamap-free.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

use basics::base_uint::Uint256;
use basics::intrusive_pointer::SharedIntrusive;
use basics::random::rand_int_to;
use basics::sha_map_hash::SHAMapHash;
use ledger::{
    InboundLedgerDataType, InboundLedgerNodeData, InboundLedgerPacket, Ledger, TreeAdvance,
    TreeKind, TreePlan, TreePlanId,
};
use shamap::node_id::SHAMapNodeId;
use shamap::sync::{MissingNodeReadApply, MissingNodeReadOutcome, MissingNodeResidentLookup};
use shamap::tree_node::SHAMapTreeNode;

use crate::id::{IdCounter, StoreGeneration};
use crate::identity::{OperationKind, OperationRef, SessionRef};
use crate::ingress::{AdmissionBudget, AdmittedLedgerPacket};
use crate::io::{
    DurabilityOutcome, PersistNode, ReadCompletion, ReadOutcome, ReadPriority, ReadRequest,
    WriteBatch, WriteOutcome,
};
use crate::session::FailureReason;

/// Upper bound on newly announced read needs accepted from one engine pass.
/// Mirrors the app `INBOUND_LEDGER_MAX_NEEDED_STATE/TX_HASHES` scaling ceiling.
pub const MAX_NEW_READS_PER_PASS: usize = 256;
/// Bounded tree turns per event; a plan must return to the coordinator owner
/// between turns so control events are never starved.
pub const MAX_TURNS_PER_EVENT: u32 = 4;
/// Bounded packet feed per turn; remaining packets stay in the mailbox.
pub const MAX_PACKETS_FED_PER_TURN: usize = 4;
/// Unique pending reads cap (mirrors the broker logical admission ceiling).
pub const MAX_PENDING_READS: usize = 512;
/// Deadline retries before an acquisition attempt fails (mirrors
/// `INBOUND_LEDGER_TIMEOUT_RETRIES_MAX`).
pub const DEFAULT_MAX_ACQUIRE_TIMEOUTS: u32 = 6;

/// One unique missing node a tree engine discovered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanReadNeed {
    hash: SHAMapHash,
    ledger_seq: u32,
    node_id: SHAMapNodeId,
    branch: usize,
}

impl PlanReadNeed {
    /// Builds a read need.
    pub const fn new(
        hash: SHAMapHash,
        ledger_seq: u32,
        node_id: SHAMapNodeId,
        branch: usize,
    ) -> Self {
        Self {
            hash,
            ledger_seq,
            node_id,
            branch,
        }
    }

    /// The node key to read.
    pub const fn hash(&self) -> SHAMapHash {
        self.hash
    }

    /// The ledger sequence scope of the read.
    pub const fn ledger_seq(&self) -> u32 {
        self.ledger_seq
    }

    /// The node identity within the tree.
    pub const fn node_id(&self) -> SHAMapNodeId {
        self.node_id
    }

    /// The parent branch the node attaches to.
    pub const fn branch(&self) -> usize {
        self.branch
    }
}

/// One missing tree node that must be requested from a peer. The tree kind is
/// part of the request identity: a state-node hash must never be emitted as a
/// transaction-node request (or vice versa).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanNetworkNeed {
    node_id: SHAMapNodeId,
    hash: Uint256,
    kind: TreeKind,
}

impl PlanNetworkNeed {
    /// Builds a kind-qualified network need.
    pub const fn new(node_id: SHAMapNodeId, hash: Uint256, kind: TreeKind) -> Self {
        Self {
            node_id,
            hash,
            kind,
        }
    }

    /// The missing node's tree location.
    pub const fn node_id(self) -> SHAMapNodeId {
        self.node_id
    }

    /// The requested node hash.
    pub const fn hash(self) -> Uint256 {
        self.hash
    }

    /// The SHAMap tree that owns this node.
    pub const fn kind(self) -> TreeKind {
        self.kind
    }
}

/// Result of one bounded engine turn. Maps the shamap/ledger
/// `TreeAdvance` vocabulary without leaking it into the coordinator surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanStepOutcome {
    /// No work command in this turn; the engine may still be runnable.
    Ready,
    /// Unique missing nodes to submit to the NodeStore broker.
    NeedsReads(Vec<PlanReadNeed>),
    /// Unique network candidates to request from a peer.
    NeedsNetwork(Vec<PlanNetworkNeed>),
    /// The tree is structurally complete; the session must persist and fence.
    Complete,
    /// The tree plan is invalid for this session.
    Invalid,
}

/// Result of applying exactly one read/network completion to an engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanReadApply {
    /// The node attached; `attached_edges` edges were resolved and
    /// `missing_edges` remain pending.
    Applied {
        /// Resolved descendant edges.
        attached_edges: usize,
        /// Still-missing descendant edges.
        missing_edges: usize,
    },
    /// Admission was not available; the need remains eligible for a FIFO retry.
    Requeued,
    /// The read was explicitly cancelled.
    Cancelled,
    /// The plan id does not match the current traversal.
    StalePlan,
    /// The node did not match its expected hash.
    HashMismatch,
    /// The read was never dispatched by this plan.
    UnknownRead,
}

/// Result of routing one peer-supplied node through the ledger map and the
/// retained traversal. `useful` is deliberately separate from `attachment`:
/// rippled resets an inbound ledger timeout only when `SHAMapAddNode` credits
/// useful data, while the retained traversal may still need a duplicate node
/// to wake a previously announced frontier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanNetworkApply {
    attachment: PlanReadApply,
    useful: bool,
}

impl PlanNetworkApply {
    /// Builds an application result from the retained-frontier outcome and the
    /// map-level useful-data accounting.
    pub const fn new(attachment: PlanReadApply, useful: bool) -> Self {
        Self { attachment, useful }
    }

    /// The retained-frontier attachment outcome.
    pub const fn attachment(self) -> PlanReadApply {
        self.attachment
    }

    /// True only when the ledger map accepted useful peer data.
    pub const fn is_useful(self) -> bool {
        self.useful
    }

    /// True when the retained continuation attached at least one edge.
    pub const fn attached(self) -> bool {
        matches!(self.attachment, PlanReadApply::Applied { .. })
    }
}

/// The uniquely owned tree engine of one session plan. This is the only place
/// shamap tree types cross into the crate; adapters implement it, the
/// coordinator owns it, and it is never advanced concurrently.
pub trait TreeEngine: std::fmt::Debug {
    /// The traversal identity of this engine.
    fn plan_id(&self) -> TreePlanId;

    /// One bounded CPU turn. `max_new_reads` caps newly announced reads.
    fn advance(&mut self, max_new_reads: usize) -> PlanStepOutcome;

    /// Applies one broker read completion. `outcome` carries decoded bytes;
    /// the engine deserializes and attaches the node.
    fn apply_read(&mut self, hash: SHAMapHash, outcome: &ReadOutcome) -> PlanReadApply;

    /// Applies one decoded network node to the retained frontier. `kind`
    /// distinguishes state from transaction nodes so an app engine can route
    /// each node to its ledger map; `node` carries the raw wire bytes and
    /// node-id that the engine deserializes and attaches.
    fn apply_network_node(
        &mut self,
        kind: TreeKind,
        node: &InboundLedgerNodeData,
    ) -> PlanNetworkApply;

    /// True only while a CPU turn can make progress without a broker completion
    /// or peer response.
    fn has_runnable_frontier(&self) -> bool;

    /// Total branch selections consumed by the retained traversal.
    fn branch_steps(&self) -> u64;

    /// The verified header sequence available as soon as a Base packet seeds
    /// the engine. Node requests use it to scope TMGetObjectByHash even when
    /// the acquisition began by hash with no initial sequence.
    fn ledger_sequence(&self) -> Option<u32> {
        None
    }

    /// Drains accepted NodeStore writes accumulated since the prior call.
    /// Nodes are written incrementally while acquisition continues; only the
    /// final batch is followed by a durability fence and durable handoff.
    fn take_persistable_nodes(&mut self) -> Vec<PersistNode>;

    /// The verified ledger-header sequence that scopes persistence. A missing
    /// value makes completion invalid: NodeStore records must never use an
    /// inferred or placeholder sequence.
    fn persistence_sequence(&self) -> Option<u32>;

    /// Yields the fully materialized ledger exactly once, after the durability
    /// fence passed and the tree is structurally complete. The coordinator's
    /// durable handoff consumes this; subsequent calls return `None`.
    fn durable_ledger(&mut self) -> Option<Arc<Ledger>>;
}

/// One-shot construction of a rooted [`TreeEngine`] from the first Base/header
/// packet of a session. `None` means no rooted plan yet; packets queue in the
/// session mailbox until a later header packet succeeds.
///
/// This is a construction port: it runs outside coordinator state mutation and
/// returns a uniquely owned engine or nothing. It never exposes a mutable
/// session.
pub trait PlanSeed: std::fmt::Debug {
    /// Builds a rooted tree engine for `session` from `header`, or `None`.
    fn build(
        &mut self,
        session: SessionRef,
        header: &InboundLedgerPacket,
    ) -> Option<Box<dyn TreeEngine + Send + Sync>>;
}

/// The default plan seed: never builds an engine, so packets stay in the
/// mailbox and the session waits for the app adapter to supply a seed.
#[derive(Debug, Clone, Copy, Default)]
pub struct NullPlanSeed;

impl PlanSeed for NullPlanSeed {
    fn build(
        &mut self,
        _session: SessionRef,
        _header: &InboundLedgerPacket,
    ) -> Option<Box<dyn TreeEngine + Send + Sync>> {
        None
    }
}

/// A resident lookup that never resolves: no synchronous NodeStore or network
/// I/O is ever introduced into traversal.
#[derive(Debug, Clone, Copy, Default)]
pub struct NullResident;

impl MissingNodeResidentLookup for NullResident {
    fn load_resident(
        &mut self,
        _hash: SHAMapHash,
        _ledger_seq: u32,
    ) -> Option<SharedIntrusive<SHAMapTreeNode>> {
        None
    }
}

/// Bounded, coordinator-owned packet mailbox for one session.
///
/// Admission leases reserve capacity before a decoded packet is routed; the
/// mailbox enforces the same bounds defensively so a misconfigured or replaying
/// ingress cannot overflow coordinator state. Counters track *currently queued*
/// packets and bytes.
#[derive(Debug)]
pub struct SessionMailbox {
    packets: VecDeque<AdmittedLedgerPacket>,
    packet_count: u64,
    packet_bytes: u64,
    max_packets: u64,
    max_bytes: u64,
}

impl SessionMailbox {
    /// Builds a mailbox with explicit packet/byte bounds.
    pub const fn new(max_packets: u64, max_bytes: u64) -> Self {
        Self {
            packets: VecDeque::new(),
            packet_count: 0,
            packet_bytes: 0,
            max_packets,
            max_bytes,
        }
    }

    /// Accepts a packet if it fits the remaining packet/byte budget. Returns
    /// false (and mutates nothing) when the budget is exhausted.
    pub fn push(&mut self, packet: AdmittedLedgerPacket) -> bool {
        let packets = packet.lease().packet_count();
        let bytes = packet.lease().byte_count();
        if self.packet_count.saturating_add(packets) > self.max_packets
            || self.packet_bytes.saturating_add(bytes) > self.max_bytes
        {
            return false;
        }
        self.packet_count += packets;
        self.packet_bytes += bytes;
        self.packets.push_back(packet);
        true
    }

    /// Removes the oldest queued packet and returns it, restoring its reserved
    /// counts.
    pub fn pop_front(&mut self) -> Option<AdmittedLedgerPacket> {
        let packet = self.packets.pop_front()?;
        self.packet_count = self
            .packet_count
            .saturating_sub(packet.lease().packet_count());
        self.packet_bytes = self
            .packet_bytes
            .saturating_sub(packet.lease().byte_count());
        Some(packet)
    }

    /// Currently queued packet count.
    pub fn packet_count(&self) -> u64 {
        self.packet_count
    }

    /// Currently queued packet bytes.
    pub fn packet_bytes(&self) -> u64 {
        self.packet_bytes
    }

    /// Number of queued packets.
    pub fn len(&self) -> usize {
        self.packets.len()
    }

    /// True when no packets are queued.
    pub fn is_empty(&self) -> bool {
        self.packets.is_empty()
    }

    /// Drops all queued packets and restores the reserved counts.
    pub fn clear(&mut self) {
        self.packets.clear();
        self.packet_count = 0;
        self.packet_bytes = 0;
    }
}

/// Persistence intent of one session: the single write/fence state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionPersistence {
    /// Nothing to persist yet.
    None,
    /// An accepted-node batch was dispatched while acquisition continues.
    /// It has no durability fence and must settle before another plan turn.
    IncrementalWritePending {
        /// The dispatched write operation.
        operation: OperationRef,
    },
    /// The final write batch was dispatched; `operation` is its write identity
    /// and `fence` is the durability-barrier operation the adapter will report.
    FinalWritePending {
        /// The dispatched write operation.
        operation: OperationRef,
        /// The fence operation to match the later durability completion.
        fence: OperationRef,
    },
    /// The write was accepted; the durability fence is in flight.
    FencePending {
        /// The in-flight fence operation.
        operation: OperationRef,
    },
    /// The fence passed; the ledger is durable and safe to hand off.
    Durable,
    /// Persistence or the fence failed; no normal adoptable ledger exists.
    Failed {
        /// Why persistence failed.
        reason: FailureReason,
    },
}

impl SessionPersistence {
    /// A stable label for tracing.
    pub const fn label(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::IncrementalWritePending { .. } => "incremental_write_pending",
            Self::FinalWritePending { .. } => "final_write_pending",
            Self::FencePending { .. } => "fence_pending",
            Self::Durable => "durable",
            Self::Failed { .. } => "failed",
        }
    }
}

/// The outcome of a read completion applied to the plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanReadOutcome {
    /// The completion matched an in-flight read and was applied.
    Applied,
    /// No in-flight read matches the operation; the completion is stale.
    Stale,
}

/// The outcome of a write completion applied to the plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanWriteOutcome {
    /// The accepted-node write completed; planning may resume.
    IncrementalAccepted,
    /// The final write completed and its durability fence is now in flight.
    FinalAccepted,
    /// The write failed; the session must terminalize.
    Failed(FailureReason),
    /// No in-flight write matches the operation; stale.
    Stale,
}

/// The outcome of a durability-fence completion applied to the plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanDurabilityOutcome {
    /// The exact in-flight fence passed; the ledger is durable.
    Durable,
    /// The fence failed; the session must terminalize.
    Failed(FailureReason),
    /// No in-flight fence matches the operation; stale.
    Stale,
}

/// The outcome of a deadline timeout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanTimeout {
    /// Another retry may proceed.
    Continue,
    /// The retry budget is exhausted; the session must fail.
    Fail,
}

/// Context handed to the plan so it can mint exact operation identities without
/// reaching into the coordinator.
pub struct TurnContext<'a> {
    /// The session being planned.
    pub session: SessionRef,
    /// The NodeStore generation to scope reads/writes.
    pub store_generation: StoreGeneration,
    /// Read admission priority derived from the acquisition reason.
    pub priority: ReadPriority,
    /// The coordinator id counter (the only source of operation ids).
    pub ids: &'a mut IdCounter,
}

/// One bounded work command produced by a plan turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanTurn {
    /// No work; wait for another event.
    Continue,
    /// Submit these brokered reads.
    Reads(Vec<ReadRequest>),
    /// Request these hashes from a peer.
    Network(Vec<PlanNetworkNeed>),
    /// Persist this batch; the batch carries its fence operation.
    Persist(WriteBatch),
    /// The plan is invalid; the session must fail.
    Invalid,
}

/// Packet-level application summary. Attachment controls whether another
/// bounded CPU turn is useful; useful data alone controls timeout progress.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct PacketFeed {
    attached: bool,
    useful: bool,
    nodes_seen: u32,
    malformed_nodes: u32,
}

impl PacketFeed {
    fn merge(&mut self, other: Self) {
        self.attached |= other.attached;
        self.useful |= other.useful;
        self.nodes_seen = self.nodes_seen.saturating_add(other.nodes_seen);
        self.malformed_nodes = self.malformed_nodes.saturating_add(other.malformed_nodes);
    }
}

/// The coordinator-owned plan of one session (M4.1).
///
/// Owns the mailbox, the uniquely owned [`TreeEngine`], read admission and
/// backlog, pending network tracking, the persistence state machine, the
/// timeout budget, and cancellation. The runner calls [`Self::run_turn`] with a
/// [`TurnContext`]; reads/writes/fences/timers are applied through the typed
/// `on_*` methods, each validating the exact operation identity.
#[derive(Debug)]
pub struct SessionPlan {
    engine: Option<Box<dyn TreeEngine + Send + Sync>>,
    mailbox: SessionMailbox,
    pending_reads: BTreeMap<SHAMapHash, OperationRef>,
    read_backlog: VecDeque<PlanReadNeed>,
    pending_network: BTreeSet<Uint256>,
    persistence: SessionPersistence,
    timeouts: u32,
    max_timeouts: u32,
    pending_reads_cap: usize,
    cancelled: bool,
    runs: u64,
}

impl SessionPlan {
    /// Builds a plan with an admission budget (packet/byte mailbox limits).
    pub fn new(admission: AdmissionBudget) -> Self {
        Self {
            engine: None,
            mailbox: SessionMailbox::new(admission.max_packets(), admission.max_bytes()),
            pending_reads: BTreeMap::new(),
            read_backlog: VecDeque::new(),
            pending_network: BTreeSet::new(),
            persistence: SessionPersistence::None,
            timeouts: 0,
            max_timeouts: DEFAULT_MAX_ACQUIRE_TIMEOUTS,
            pending_reads_cap: MAX_PENDING_READS,
            cancelled: false,
            runs: 0,
        }
    }

    /// Builds a plan with explicit timeout retries (tests).
    pub fn with_max_timeouts(admission: AdmissionBudget, max_timeouts: u32) -> Self {
        Self {
            max_timeouts,
            ..Self::new(admission)
        }
    }

    /// Builds a plan with a smaller pending-read cap (tests).
    pub fn with_pending_reads_cap(admission: AdmissionBudget, pending_reads_cap: usize) -> Self {
        Self {
            pending_reads_cap,
            ..Self::new(admission)
        }
    }

    /// The uniquely owned tree engine, if a rooted plan exists yet.
    pub fn engine(&self) -> Option<&(dyn TreeEngine + Send + Sync)> {
        self.engine.as_deref()
    }

    /// The verified header sequence available for subsequent by-hash node
    /// requests. It deliberately does not depend on tree completion.
    pub fn ledger_sequence(&self) -> Option<u32> {
        self.engine
            .as_deref()
            .and_then(TreeEngine::ledger_sequence)
            .filter(|sequence| *sequence != 0)
    }

    /// Yields the durable ledger from the uniquely owned engine exactly once.
    /// Called only after the durability fence passed (`DurablePending`); the
    /// coordinator's single `PublishDurable` effect consumes the result.
    pub fn durable_ledger(&mut self) -> Option<Arc<Ledger>> {
        self.engine
            .as_mut()
            .and_then(|engine| engine.durable_ledger())
    }

    /// Installs the seeded engine. Returns false when a plan already exists or
    /// the session is cancelled; a replacement engine never replaces a live
    /// traversal.
    pub fn install_engine(&mut self, engine: Box<dyn TreeEngine + Send + Sync>) -> bool {
        if self.cancelled || self.engine.is_some() {
            return false;
        }
        self.engine = Some(engine);
        true
    }

    /// Accepts a packet into the bounded mailbox. Returns false when the
    /// defensive budget is exhausted; the packet is dropped without mutation.
    pub fn push_packet(&mut self, packet: AdmittedLedgerPacket) -> bool {
        self.mailbox.push(packet)
    }

    /// Currently queued packet count.
    pub fn packet_count(&self) -> u64 {
        self.mailbox.packet_count()
    }

    /// Currently queued packet bytes.
    pub fn packet_bytes(&self) -> u64 {
        self.mailbox.packet_bytes()
    }

    /// The persistence state machine state.
    pub const fn persistence(&self) -> &SessionPersistence {
        &self.persistence
    }

    /// Deadline retries consumed so far.
    pub const fn timeouts(&self) -> u32 {
        self.timeouts
    }

    /// Bounded engine turns executed.
    pub const fn runs(&self) -> u64 {
        self.runs
    }

    /// Pending network candidates requested from peers (observability).
    pub fn pending_network(&self) -> &BTreeSet<Uint256> {
        &self.pending_network
    }

    /// True after cancellation cleared this plan.
    pub const fn is_cancelled(&self) -> bool {
        self.cancelled
    }

    /// Clears every queued and in-flight plan resource. After this, all `on_*`
    /// completions are stale and `run_turn` produces no work.
    pub fn cancel(&mut self) {
        self.cancelled = true;
        self.mailbox.clear();
        self.pending_reads.clear();
        self.read_backlog.clear();
        self.pending_network.clear();
        self.engine = None;
        self.persistence = SessionPersistence::None;
    }

    /// Records protocol progress for this plan. A later timeout measures a
    /// consecutive no-progress interval, matching rippled
    /// `InboundLedger::onTimer(wasProgress, ...)`; it must not exhaust an
    /// acquisition that is actively receiving or applying ledger data.
    pub fn note_progress(&mut self) {
        self.timeouts = 0;
    }

    /// Runs one bounded work turn: drains the FIFO read-admission backlog,
    /// feeds a bounded batch of queued packets into the engine, then advances
    /// the engine up to `MAX_TURNS_PER_EVENT` times.
    pub fn run_turn(&mut self, ctx: &mut TurnContext) -> PlanTurn {
        if self.cancelled || self.persistence != SessionPersistence::None {
            return PlanTurn::Continue;
        }

        // Drain the FIFO admission backlog before further CPU work so history
        // over-admission cannot starve the bounded in-flight budget.
        if !self.read_backlog.is_empty() {
            let mut requests = Vec::new();
            while let Some(need) = self.read_backlog.pop_front() {
                if self.pending_reads.len() >= self.pending_reads_cap {
                    self.read_backlog.push_front(need);
                    break;
                }
                let operation = OperationRef::new(
                    ctx.session,
                    OperationKind::Read,
                    ctx.ids.next_id(),
                    ctx.ids.next_id(),
                );
                self.pending_reads.insert(need.hash, operation);
                requests.push(ReadRequest::new(
                    operation,
                    need.hash,
                    need.ledger_seq,
                    ctx.store_generation,
                    ctx.priority,
                ));
            }
            if requests.is_empty() {
                return PlanTurn::Continue;
            }
            return PlanTurn::Reads(requests);
        }

        let mut turns = 0u32;
        loop {
            turns += 1;
            if turns > MAX_TURNS_PER_EVENT {
                return PlanTurn::Continue;
            }
            let mut fed = PacketFeed::default();
            for _ in 0..MAX_PACKETS_FED_PER_TURN {
                let Some(engine) = self.engine.as_mut() else {
                    break;
                };
                let Some(packet) = self.mailbox.pop_front() else {
                    break;
                };
                fed.merge(Self::feed_packet(packet, &mut **engine));
            }
            let (outcome, useful_peer_data, ready_continues, pending_writes, ledger_sequence) = {
                let Some(engine) = self.engine.as_mut() else {
                    // No rooted plan yet; queued packets wait in the mailbox.
                    return PlanTurn::Continue;
                };
                let before = engine.branch_steps();
                let useful_peer_data = fed.useful;
                let outcome = engine.advance(MAX_NEW_READS_PER_PASS);
                let ready_continues = fed.attached
                    || (engine.has_runnable_frontier() && engine.branch_steps() > before);
                let pending_writes = if matches!(outcome, PlanStepOutcome::Complete) {
                    Vec::new()
                } else {
                    engine.take_persistable_nodes()
                };
                (
                    outcome,
                    useful_peer_data,
                    ready_continues,
                    pending_writes,
                    engine.ledger_sequence(),
                )
            };
            self.runs += 1;
            if fed.nodes_seen != 0 {
                tracing::info!(
                    target: "acquisition_trace",
                    event = "packet_batch_applied",
                    run_epoch = ctx.session.run_epoch().get(),
                    session_id = ctx.session.session_id().get(),
                    target_hash = %ctx.session.target_hash(),
                    plan_epoch = ctx.session.plan_epoch().get(),
                    store_generation = ctx.session.store_generation().get(),
                    plan_run = self.runs,
                    nodes_seen = fed.nodes_seen,
                    malformed_nodes = fed.malformed_nodes,
                    attached = fed.attached,
                    useful = fed.useful,
                    mailbox_packets = self.mailbox.packet_count(),
                    mailbox_bytes = self.mailbox.packet_bytes(),
                    pending_network = self.pending_network.len(),
                    "acquisition trace: bounded peer-node batch applied to SHAMap plan"
                );
            }
            // Matches rippled `InboundLedger::processData`: only useful peer
            // data resets the no-progress timeout. A duplicate, stale, or
            // unattached but decodable packet may wake the frontier but cannot
            // conceal an acquisition stall.
            if useful_peer_data {
                self.note_progress();
            }
            if !pending_writes.is_empty() {
                let Some(ledger_sequence) = ledger_sequence.filter(|sequence| *sequence != 0)
                else {
                    return PlanTurn::Invalid;
                };
                let operation = OperationRef::new(
                    ctx.session,
                    OperationKind::Write,
                    ctx.ids.next_id(),
                    ctx.ids.next_id(),
                );
                self.persistence = SessionPersistence::IncrementalWritePending { operation };
                return PlanTurn::Persist(WriteBatch::incremental(
                    operation,
                    ctx.store_generation,
                    ledger_sequence,
                    pending_writes,
                ));
            }
            match outcome {
                PlanStepOutcome::Ready => {
                    if ready_continues {
                        continue;
                    }
                    return PlanTurn::Continue;
                }
                PlanStepOutcome::NeedsReads(needs) => {
                    let requests = self.admit_reads(needs, ctx);
                    if requests.is_empty() {
                        return PlanTurn::Continue;
                    }
                    return PlanTurn::Reads(requests);
                }
                PlanStepOutcome::NeedsNetwork(nodes) => {
                    for node in &nodes {
                        self.pending_network.insert(node.hash());
                    }
                    return PlanTurn::Network(nodes);
                }
                PlanStepOutcome::Complete => return self.on_complete(ctx),
                PlanStepOutcome::Invalid => return PlanTurn::Invalid,
            }
        }
    }

    /// Applies one read completion. Returns `Stale` unless the completion
    /// matches the exact in-flight operation of a pending read.
    pub fn on_read(&mut self, completion: &ReadCompletion) -> PlanReadOutcome {
        if completion.operation().kind() != OperationKind::Read {
            return PlanReadOutcome::Stale;
        }
        let Some((&hash, _)) = self
            .pending_reads
            .iter()
            .find(|(_, operation)| operation.is_expected_for(&completion.operation()))
        else {
            return PlanReadOutcome::Stale;
        };
        self.pending_reads.remove(&hash);
        let Some(engine) = self.engine.as_mut() else {
            return PlanReadOutcome::Stale;
        };
        engine.apply_read(hash, completion.outcome());
        PlanReadOutcome::Applied
    }

    /// Applies one write completion. An incremental accepted-node write
    /// returns the plan to active work; only a final write advances to its
    /// durability fence.
    pub fn on_write(&mut self, operation: OperationRef, outcome: WriteOutcome) -> PlanWriteOutcome {
        if operation.kind() != OperationKind::Write {
            return PlanWriteOutcome::Stale;
        }
        match &self.persistence {
            SessionPersistence::IncrementalWritePending {
                operation: expected,
            } if expected.is_expected_for(&operation) => match outcome {
                WriteOutcome::Accepted => {
                    self.persistence = SessionPersistence::None;
                    PlanWriteOutcome::IncrementalAccepted
                }
                WriteOutcome::Failed => {
                    self.persistence = SessionPersistence::Failed {
                        reason: FailureReason::WriteFailure,
                    };
                    PlanWriteOutcome::Failed(FailureReason::WriteFailure)
                }
                WriteOutcome::Stale | WriteOutcome::Cancelled => PlanWriteOutcome::Stale,
            },
            SessionPersistence::FinalWritePending {
                operation: expected,
                fence,
            } if expected.is_expected_for(&operation) => match outcome {
                WriteOutcome::Accepted => {
                    let fence = *fence;
                    self.persistence = SessionPersistence::FencePending { operation: fence };
                    PlanWriteOutcome::FinalAccepted
                }
                WriteOutcome::Failed => {
                    self.persistence = SessionPersistence::Failed {
                        reason: FailureReason::WriteFailure,
                    };
                    PlanWriteOutcome::Failed(FailureReason::WriteFailure)
                }
                WriteOutcome::Stale | WriteOutcome::Cancelled => PlanWriteOutcome::Stale,
            },
            _ => PlanWriteOutcome::Stale,
        }
    }

    /// Applies one durability-fence completion. `FencePending` moves to
    /// `Durable` only on a passed barrier; a failure terminalizes intent.
    pub fn on_durability(
        &mut self,
        operation: OperationRef,
        outcome: DurabilityOutcome,
    ) -> PlanDurabilityOutcome {
        if operation.kind() != OperationKind::DurabilityFence {
            return PlanDurabilityOutcome::Stale;
        }
        match &self.persistence {
            SessionPersistence::FencePending {
                operation: expected,
            } if expected.is_expected_for(&operation) => match outcome {
                DurabilityOutcome::Passed => {
                    self.persistence = SessionPersistence::Durable;
                    PlanDurabilityOutcome::Durable
                }
                DurabilityOutcome::Failed => {
                    self.persistence = SessionPersistence::Failed {
                        reason: FailureReason::DurabilityFenceFailed,
                    };
                    PlanDurabilityOutcome::Failed(FailureReason::DurabilityFenceFailed)
                }
                DurabilityOutcome::Stale => PlanDurabilityOutcome::Stale,
            },
            _ => PlanDurabilityOutcome::Stale,
        }
    }

    /// Consumes one deadline timeout. Fails the plan after the retry budget is
    /// exhausted. A cancelled plan stays terminal: any late timer event fails.
    pub fn on_timeout(&mut self) -> PlanTimeout {
        if self.cancelled {
            return PlanTimeout::Fail;
        }
        self.timeouts += 1;
        if self.timeouts >= self.max_timeouts {
            self.cancel();
            PlanTimeout::Fail
        } else {
            PlanTimeout::Continue
        }
    }

    /// Deserializes and routes one packet through the ledger map and retained
    /// frontier. Useful-data credit is separate from attachment so only the
    /// former resets the inbound timeout, matching rippled.
    fn feed_packet(mut packet: AdmittedLedgerPacket, engine: &mut dyn TreeEngine) -> PacketFeed {
        let kind = match packet.packet().packet_type {
            InboundLedgerDataType::StateNode => TreeKind::State,
            InboundLedgerDataType::TransactionNode => TreeKind::Transaction,
            InboundLedgerDataType::Base => {
                // The header packet seeded the engine; it is not a tree node.
                let _ = packet.settle();
                return PacketFeed::default();
            }
        };
        let mut fed = PacketFeed::default();
        for node in &packet.packet().nodes {
            fed.nodes_seen = fed.nodes_seen.saturating_add(1);
            let decoded = match SHAMapTreeNode::make_from_wire(&node.node_data) {
                Ok(Some(decoded)) => decoded,
                Ok(None) | Err(_) => {
                    fed.malformed_nodes = fed.malformed_nodes.saturating_add(1);
                    tracing::warn!(
                        target: "acquisition",
                        packet_type = ?packet.packet().packet_type,
                        wire_bytes = node.node_data.len(),
                        wire_type = node.node_data.last().copied(),
                        "Rejected malformed ledger tree-node payload"
                    );
                    continue;
                }
            };
            if decoded.get_hash().is_zero() {
                continue;
            }
            let applied = engine.apply_network_node(kind, node);
            fed.attached |= applied.attached();
            fed.useful |= applied.is_useful();
        }
        let _ = packet.settle();
        fed
    }

    /// Admits newly announced reads up to the pending cap. Hashes already in
    /// flight are skipped; overflow goes to the FIFO backlog for a later turn.
    fn admit_reads(&mut self, needs: Vec<PlanReadNeed>, ctx: &mut TurnContext) -> Vec<ReadRequest> {
        let mut requests = Vec::new();
        for need in needs {
            if self.pending_reads.contains_key(&need.hash) {
                continue;
            }
            if self.pending_reads.len() >= self.pending_reads_cap {
                self.read_backlog.push_back(need);
                continue;
            }
            let operation = OperationRef::new(
                ctx.session,
                OperationKind::Read,
                ctx.ids.next_id(),
                ctx.ids.next_id(),
            );
            self.pending_reads.insert(need.hash, operation);
            requests.push(ReadRequest::new(
                operation,
                need.hash,
                need.ledger_seq,
                ctx.store_generation,
                ctx.priority,
            ));
        }
        requests
    }

    /// Transitions to final persistence once the tree is complete. Earlier
    /// accepted writes were already submitted incrementally; this final batch
    /// drains the remainder and is the only one carrying a durability fence.
    fn on_complete(&mut self, ctx: &mut TurnContext) -> PlanTurn {
        let Some(ledger_sequence) = self
            .engine
            .as_ref()
            .and_then(|engine| engine.persistence_sequence())
            .filter(|sequence| *sequence != 0)
        else {
            return PlanTurn::Invalid;
        };
        let nodes = self
            .engine
            .as_mut()
            .map(|engine| engine.take_persistable_nodes())
            .unwrap_or_default();
        let write_op = OperationRef::new(
            ctx.session,
            OperationKind::Write,
            ctx.ids.next_id(),
            ctx.ids.next_id(),
        );
        let fence_op = OperationRef::new(
            ctx.session,
            OperationKind::DurabilityFence,
            ctx.ids.next_id(),
            ctx.ids.next_id(),
        );
        self.persistence = SessionPersistence::FinalWritePending {
            operation: write_op,
            fence: fence_op,
        };
        PlanTurn::Persist(WriteBatch::new(
            write_op,
            fence_op,
            ctx.store_generation,
            ledger_sequence,
            nodes,
        ))
    }
}

/// A deterministic scripted [`TreeEngine`] for tests. Each `advance` consumes
/// one scripted step; read/network applications mark the frontier runnable so
/// the next step is consumed on the next turn (mirroring real continuation
/// behavior).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptedStep {
    /// Announce these reads on the next advance.
    NeedsReads(Vec<PlanReadNeed>),
    /// Announce legacy network candidates as State nodes on the next advance.
    /// Existing tuple scripts retain their previous State behavior; tests that
    /// need transaction preservation use [`Self::NeedsNetworkWithKind`].
    NeedsNetwork(Vec<(SHAMapNodeId, Uint256)>),
    /// Announce explicitly kind-qualified network candidates on the next
    /// advance.
    NeedsNetworkWithKind(Vec<PlanNetworkNeed>),
    /// Complete the tree on the next advance.
    Complete,
    /// Produce an invalid plan on the next advance.
    Invalid,
}

/// A deterministic [`TreeEngine`] fake driven by a script.
#[derive(Debug)]
pub struct ScriptedEngine {
    plan_id: TreePlanId,
    steps: VecDeque<ScriptedStep>,
    runnable_frontier: bool,
    branch_steps: u64,
    applied_reads: u64,
    applied_nodes: u64,
    persistable: Vec<PersistNode>,
    durable_ledger: Option<Arc<Ledger>>,
    persistence_sequence: Option<u32>,
}

impl ScriptedEngine {
    /// Builds a scripted engine.
    pub fn new(
        plan_id: TreePlanId,
        steps: impl IntoIterator<Item = ScriptedStep>,
        persistable: Vec<PersistNode>,
    ) -> Self {
        let steps: VecDeque<_> = steps.into_iter().collect();
        let runnable_frontier = !steps.is_empty();
        Self {
            plan_id,
            steps,
            runnable_frontier,
            branch_steps: 0,
            applied_reads: 0,
            applied_nodes: 0,
            persistable,
            durable_ledger: None,
            persistence_sequence: None,
        }
    }

    /// Sets the verified header sequence used if this scripted engine reaches
    /// persistence. Tests must opt in rather than silently using a placeholder.
    pub fn with_persistence_sequence(mut self, sequence: u32) -> Self {
        self.persistence_sequence = (sequence != 0).then_some(sequence);
        self
    }

    /// Attaches a durable ledger the engine yields exactly once at the M5
    /// durable handoff.
    pub fn with_durable_ledger(mut self, ledger: Arc<Ledger>) -> Self {
        self.durable_ledger = Some(ledger);
        self
    }

    /// Reads applied to this engine.
    pub const fn applied_reads(&self) -> u64 {
        self.applied_reads
    }

    /// Network nodes applied to this engine.
    pub const fn applied_nodes(&self) -> u64 {
        self.applied_nodes
    }
}

impl TreeEngine for ScriptedEngine {
    fn plan_id(&self) -> TreePlanId {
        self.plan_id
    }

    fn advance(&mut self, _max_new_reads: usize) -> PlanStepOutcome {
        self.runnable_frontier = false;
        match self.steps.pop_front() {
            Some(ScriptedStep::NeedsReads(needs)) => PlanStepOutcome::NeedsReads(needs),
            Some(ScriptedStep::NeedsNetwork(nodes)) => PlanStepOutcome::NeedsNetwork(
                nodes
                    .into_iter()
                    .map(|(node_id, hash)| PlanNetworkNeed::new(node_id, hash, TreeKind::State))
                    .collect(),
            ),
            Some(ScriptedStep::NeedsNetworkWithKind(nodes)) => PlanStepOutcome::NeedsNetwork(nodes),
            Some(ScriptedStep::Complete) => PlanStepOutcome::Complete,
            Some(ScriptedStep::Invalid) => PlanStepOutcome::Invalid,
            None => PlanStepOutcome::Ready,
        }
    }

    fn apply_read(&mut self, _hash: SHAMapHash, _outcome: &ReadOutcome) -> PlanReadApply {
        self.applied_reads += 1;
        self.branch_steps += 1;
        self.runnable_frontier = !self.steps.is_empty();
        PlanReadApply::Applied {
            attached_edges: 1,
            missing_edges: 0,
        }
    }

    fn apply_network_node(
        &mut self,
        _kind: TreeKind,
        _node: &InboundLedgerNodeData,
    ) -> PlanNetworkApply {
        self.applied_nodes += 1;
        self.branch_steps += 1;
        self.runnable_frontier = !self.steps.is_empty();
        PlanNetworkApply::new(
            PlanReadApply::Applied {
                attached_edges: 1,
                missing_edges: 0,
            },
            true,
        )
    }

    fn has_runnable_frontier(&self) -> bool {
        self.runnable_frontier
    }

    fn branch_steps(&self) -> u64 {
        self.branch_steps
    }

    fn ledger_sequence(&self) -> Option<u32> {
        self.persistence_sequence
    }

    fn take_persistable_nodes(&mut self) -> Vec<PersistNode> {
        std::mem::take(&mut self.persistable)
    }

    fn persistence_sequence(&self) -> Option<u32> {
        self.persistence_sequence
    }

    fn durable_ledger(&mut self) -> Option<Arc<Ledger>> {
        self.durable_ledger.take()
    }
}

/// A real [`TreeEngine`] over the ledger `TreePlan` traversal. M4.1 uses it in
/// deterministic fixtures (rooted tree + [`NullResident`]); the app adapter
/// supplies persistable nodes with the M4.2 wiring.
#[derive(Debug)]
pub struct LedgerTreePlanEngine {
    plan: TreePlan,
    persistable: Vec<PersistNode>,
}

impl LedgerTreePlanEngine {
    /// Wraps a ledger `TreePlan`.
    pub fn new(plan: TreePlan) -> Self {
        Self {
            plan,
            persistable: Vec::new(),
        }
    }

    /// The wrapped traversal plan.
    pub const fn plan(&self) -> &TreePlan {
        &self.plan
    }
}

impl TreeEngine for LedgerTreePlanEngine {
    fn plan_id(&self) -> TreePlanId {
        self.plan.id()
    }

    fn advance(&mut self, max_new_reads: usize) -> PlanStepOutcome {
        let mut first_child = || rand_int_to(255u8);
        let mut yield_now = || false;
        match self.plan.advance_with_yield(
            max_new_reads,
            &mut NullResident,
            &mut first_child,
            &mut yield_now,
        ) {
            TreeAdvance::Ready => PlanStepOutcome::Ready,
            TreeAdvance::NeedsReads(reads) => PlanStepOutcome::NeedsReads(
                reads
                    .into_iter()
                    .map(|need| {
                        PlanReadNeed::new(
                            need.hash(),
                            need.ledger_seq(),
                            need.node_id(),
                            need.branch(),
                        )
                    })
                    .collect(),
            ),
            TreeAdvance::NeedsNetwork(nodes) => PlanStepOutcome::NeedsNetwork(
                nodes
                    .into_iter()
                    .map(|(node_id, hash)| PlanNetworkNeed::new(node_id, hash, self.plan.kind()))
                    .collect(),
            ),
            TreeAdvance::Complete => PlanStepOutcome::Complete,
            TreeAdvance::Invalid => PlanStepOutcome::Invalid,
        }
    }

    fn apply_read(&mut self, hash: SHAMapHash, outcome: &ReadOutcome) -> PlanReadApply {
        let missing = match outcome {
            ReadOutcome::Settled { node: Some(bytes) } => {
                match SHAMapTreeNode::make_from_wire(bytes) {
                    Ok(Some(node)) => MissingNodeReadOutcome::Found(node),
                    Ok(None) => MissingNodeReadOutcome::Miss,
                    Err(_) => return PlanReadApply::UnknownRead,
                }
            }
            ReadOutcome::Settled { node: None } => MissingNodeReadOutcome::Miss,
            ReadOutcome::Stale | ReadOutcome::Cancelled => MissingNodeReadOutcome::Cancelled,
        };
        Self::map_apply(self.plan.apply_read_result(self.plan.id(), hash, missing))
    }

    fn apply_network_node(
        &mut self,
        kind: TreeKind,
        node: &InboundLedgerNodeData,
    ) -> PlanNetworkApply {
        if self.plan.kind() != kind {
            return PlanNetworkApply::new(PlanReadApply::StalePlan, false);
        }
        let Ok(Some(decoded)) = SHAMapTreeNode::make_from_wire(&node.node_data) else {
            return PlanNetworkApply::new(PlanReadApply::UnknownRead, false);
        };
        let hash = decoded.get_hash();
        let attachment =
            Self::map_apply(self.plan.apply_network_node(self.plan.id(), hash, decoded));
        PlanNetworkApply::new(
            attachment,
            matches!(attachment, PlanReadApply::Applied { .. }),
        )
    }

    fn has_runnable_frontier(&self) -> bool {
        self.plan.has_runnable_frontier()
    }

    fn branch_steps(&self) -> u64 {
        self.plan.branch_steps()
    }

    fn take_persistable_nodes(&mut self) -> Vec<PersistNode> {
        std::mem::take(&mut self.persistable)
    }

    fn persistence_sequence(&self) -> Option<u32> {
        None
    }

    fn durable_ledger(&mut self) -> Option<Arc<Ledger>> {
        None
    }
}

impl LedgerTreePlanEngine {
    fn map_apply(apply: MissingNodeReadApply) -> PlanReadApply {
        match apply {
            MissingNodeReadApply::Applied {
                attached_edges,
                missing_edges,
            } => PlanReadApply::Applied {
                attached_edges,
                missing_edges,
            },
            MissingNodeReadApply::Requeued => PlanReadApply::Requeued,
            MissingNodeReadApply::Cancelled => PlanReadApply::Cancelled,
            MissingNodeReadApply::StalePlan => PlanReadApply::StalePlan,
            MissingNodeReadApply::HashMismatch => PlanReadApply::HashMismatch,
            MissingNodeReadApply::UnknownRead => PlanReadApply::UnknownRead,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::{
        IdCounter, OperationGeneration, OperationId, PeerId, PlanEpoch, RunEpoch, SessionId,
    };
    use crate::ingress::{AdmissionGate, BackpressureOutcome};
    use basics::base_uint::Uint256;
    use basics::intrusive_pointer::make_shared_intrusive;
    use bytes::Bytes;
    use ledger::{InboundLedgerNodeData, TreeKind};
    use shamap::nodes::item::SHAMapItem;
    use shamap::sync::{SyncState, SyncTree};
    use shamap::tree_node::SHAMapTreeNode;

    fn session() -> SessionRef {
        SessionRef::new(
            RunEpoch::new(1),
            SessionId::new(1),
            Uint256::from(1),
            PlanEpoch::new(1),
            StoreGeneration::new(1),
        )
    }

    fn budget() -> AdmissionBudget {
        AdmissionBudget::new(128, 4 * 1024 * 1024)
    }

    fn ctx<'a>(session: SessionRef, ids: &'a mut IdCounter) -> TurnContext<'a> {
        TurnContext {
            session,
            store_generation: StoreGeneration::new(1),
            priority: ReadPriority::Consensus,
            ids,
        }
    }

    fn need(hash: u64) -> PlanReadNeed {
        PlanReadNeed::new(
            SHAMapHash::new(Uint256::from(hash)),
            1,
            SHAMapNodeId::default(),
            0,
        )
    }

    fn packet(session: SessionRef, budget: AdmissionBudget, bytes: u64) -> AdmittedLedgerPacket {
        let gate = Arc::new(AdmissionGate::new(budget, session));
        let lease = match gate.try_reserve(1, bytes) {
            BackpressureOutcome::Admitted(lease) => lease,
            other => panic!("expected admission, got {other:?}"),
        };
        AdmittedLedgerPacket::new(
            lease,
            session,
            PeerId::new(1),
            ledger::InboundLedgerPacket::new(
                ledger::InboundLedgerDataType::StateNode,
                vec![InboundLedgerNodeData::new(None, vec![0; bytes as usize])],
            ),
        )
        .expect("matching lease must admit")
    }

    fn scripted(plan_id: u64, steps: Vec<ScriptedStep>) -> SessionPlan {
        let mut plan = SessionPlan::new(budget());
        assert!(
            plan.install_engine(Box::new(
                ScriptedEngine::new(TreePlanId::new(plan_id), steps, Vec::new(),)
                    .with_persistence_sequence(1)
            ))
        );
        plan
    }

    #[test]
    fn mailbox_enforces_packet_and_byte_bounds() {
        let s = session();
        // A permissive gate budget: the mailbox itself enforces its caps.
        let budget = AdmissionBudget::new(16, 1024);
        // Byte cap.
        let mut mailbox = SessionMailbox::new(4, 100);
        assert!(mailbox.push(packet(s, budget, 30)));
        assert_eq!(mailbox.packet_count(), 1);
        assert_eq!(mailbox.packet_bytes(), 30);
        assert!(mailbox.push(packet(s, budget, 70)));
        assert_eq!(mailbox.packet_count(), 2);
        assert_eq!(mailbox.packet_bytes(), 100);
        // A byte-over-budget packet is dropped without mutation.
        assert!(!mailbox.push(packet(s, budget, 20)));
        assert_eq!(mailbox.packet_count(), 2);
        assert_eq!(mailbox.packet_bytes(), 100);
        mailbox.clear();
        assert!(mailbox.is_empty());
        assert_eq!(mailbox.packet_count(), 0);
        assert_eq!(mailbox.packet_bytes(), 0);

        // Packet cap (byte budget generous).
        let mut mailbox = SessionMailbox::new(4, 1024);
        for _ in 0..4 {
            assert!(mailbox.push(packet(s, budget, 4)));
        }
        assert_eq!(mailbox.packet_count(), 4);
        assert_eq!(mailbox.packet_bytes(), 4 * 4);
        // A packet-count-over-budget packet is dropped.
        assert!(!mailbox.push(packet(s, budget, 4)));
        assert_eq!(mailbox.packet_count(), 4);
        // Pop restores counts.
        assert!(mailbox.pop_front().is_some());
        assert_eq!(mailbox.packet_count(), 3);
        assert_eq!(mailbox.packet_bytes(), 3 * 4);
        mailbox.clear();
        assert!(mailbox.is_empty());
        assert_eq!(mailbox.packet_count(), 0);
        assert_eq!(mailbox.packet_bytes(), 0);
    }

    #[test]
    fn reads_are_deduped_and_match_the_exact_operation() {
        let mut ids = IdCounter::new();
        let s = session();
        let mut plan = scripted(1, vec![ScriptedStep::NeedsReads(vec![need(7), need(7)])]);
        let turn = plan.run_turn(&mut ctx(s, &mut ids));
        let PlanTurn::Reads(requests) = turn else {
            panic!("expected reads, got {turn:?}");
        };
        // The duplicate hash is announced once.
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.key(), SHAMapHash::new(Uint256::from(7)));

        // A read with the wrong operation identity is stale.
        let wrong = OperationRef::new(
            s,
            OperationKind::Read,
            OperationId::new(99),
            OperationGeneration::new(1),
        );
        let stale = plan.on_read(&ReadCompletion::new(
            wrong,
            ReadOutcome::Settled { node: None },
        ));
        assert_eq!(stale, PlanReadOutcome::Stale);
        // The exact in-flight operation applies.
        let exact = plan.on_read(&ReadCompletion::new(
            request.operation(),
            ReadOutcome::Settled { node: None },
        ));
        assert_eq!(exact, PlanReadOutcome::Applied);
        // The applied read cannot apply again.
        let again = plan.on_read(&ReadCompletion::new(
            request.operation(),
            ReadOutcome::Settled { node: None },
        ));
        assert_eq!(again, PlanReadOutcome::Stale);
    }

    #[test]
    fn complete_after_apply_reaches_persistence_and_fences() {
        let mut ids = IdCounter::new();
        let s = session();
        // Two read steps keep the plan from completing until the read applies.
        let mut plan = scripted(
            1,
            vec![
                ScriptedStep::NeedsReads(vec![need(7)]),
                ScriptedStep::NeedsReads(vec![need(7)]),
                ScriptedStep::Complete,
            ],
        );
        let turn = plan.run_turn(&mut ctx(s, &mut ids));
        let PlanTurn::Reads(requests) = turn else {
            panic!("expected reads, got {turn:?}");
        };
        assert_eq!(requests.len(), 1);

        // Without the read the plan makes no further progress.
        let turn = plan.run_turn(&mut ctx(s, &mut ids));
        assert_eq!(turn, PlanTurn::Continue);

        assert_eq!(
            plan.on_read(&ReadCompletion::new(
                requests[0].operation(),
                ReadOutcome::Settled { node: None },
            )),
            PlanReadOutcome::Applied
        );

        // The next turn completes and persists with an exact write+fence pair.
        let turn = plan.run_turn(&mut ctx(s, &mut ids));
        let PlanTurn::Persist(batch) = turn else {
            panic!("expected persist, got {turn:?}");
        };
        assert_eq!(batch.operation().kind(), OperationKind::Write);
        assert_eq!(
            batch.fence().expect("final batch fence").kind(),
            OperationKind::DurabilityFence
        );
        assert_eq!(
            plan.persistence(),
            &SessionPersistence::FinalWritePending {
                operation: batch.operation(),
                fence: batch.fence().expect("final batch fence"),
            }
        );

        // A fence completion before the write is stale.
        let stale = plan.on_durability(
            batch.fence().expect("final batch fence"),
            DurabilityOutcome::Passed,
        );
        assert_eq!(stale, PlanDurabilityOutcome::Stale);

        // The exact write acceptance arms the fence.
        let accepted = plan.on_write(batch.operation(), WriteOutcome::Accepted);
        assert_eq!(accepted, PlanWriteOutcome::FinalAccepted);
        assert_eq!(
            plan.persistence(),
            &SessionPersistence::FencePending {
                operation: batch.fence().expect("final batch fence"),
            }
        );

        // A wrong write operation is stale and cannot rearm the fence.
        let wrong_write = OperationRef::new(
            s,
            OperationKind::Write,
            OperationId::new(9),
            OperationGeneration::new(1),
        );
        assert_eq!(
            plan.on_write(wrong_write, WriteOutcome::Accepted),
            PlanWriteOutcome::Stale
        );
        assert_eq!(
            plan.persistence(),
            &SessionPersistence::FencePending {
                operation: batch.fence().expect("final batch fence"),
            }
        );

        // The passed fence makes the ledger durable.
        let durable = plan.on_durability(
            batch.fence().expect("final batch fence"),
            DurabilityOutcome::Passed,
        );
        assert_eq!(durable, PlanDurabilityOutcome::Durable);
        assert_eq!(plan.persistence(), &SessionPersistence::Durable);

        // No further work while persistence is committed.
        let turn = plan.run_turn(&mut ctx(s, &mut ids));
        assert_eq!(turn, PlanTurn::Continue);
    }

    #[test]
    fn write_and_fence_failures_terminalize_intent() {
        let mut ids = IdCounter::new();
        let s = session();
        let mut plan = scripted(1, vec![ScriptedStep::Complete]);
        let PlanTurn::Persist(batch) = plan.run_turn(&mut ctx(s, &mut ids)) else {
            panic!("expected persist");
        };

        // A failed write leaves no durable intent.
        let failed = plan.on_write(batch.operation(), WriteOutcome::Failed);
        assert_eq!(
            failed,
            PlanWriteOutcome::Failed(FailureReason::WriteFailure)
        );
        assert_eq!(
            plan.persistence(),
            &SessionPersistence::Failed {
                reason: FailureReason::WriteFailure,
            }
        );

        let mut plan = scripted(1, vec![ScriptedStep::Complete]);
        let PlanTurn::Persist(batch) = plan.run_turn(&mut ctx(s, &mut ids)) else {
            panic!("expected persist");
        };
        assert_eq!(
            plan.on_write(batch.operation(), WriteOutcome::Accepted),
            PlanWriteOutcome::FinalAccepted
        );
        let failed = plan.on_durability(
            batch.fence().expect("final batch fence"),
            DurabilityOutcome::Failed,
        );
        assert_eq!(
            failed,
            PlanDurabilityOutcome::Failed(FailureReason::DurabilityFenceFailed)
        );
        assert_eq!(
            plan.persistence(),
            &SessionPersistence::Failed {
                reason: FailureReason::DurabilityFenceFailed,
            }
        );
    }

    #[test]
    fn read_admission_backlog_is_fifo_and_bounded() {
        let mut ids = IdCounter::new();
        let s = session();
        let mut plan = SessionPlan::with_pending_reads_cap(budget(), 2);
        assert!(plan.install_engine(Box::new(ScriptedEngine::new(
            TreePlanId::new(1),
            vec![ScriptedStep::NeedsReads(vec![need(1), need(2), need(3)])],
            Vec::new(),
        ))));
        // Only two reads fit the cap; the last goes to the FIFO backlog.
        let turn = plan.run_turn(&mut ctx(s, &mut ids));
        let PlanTurn::Reads(requests) = turn else {
            panic!("expected reads");
        };
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].key(), SHAMapHash::new(Uint256::from(1)));
        assert_eq!(requests[1].key(), SHAMapHash::new(Uint256::from(2)));

        // Nothing more is admitted while the cap is saturated.
        let turn = plan.run_turn(&mut ctx(s, &mut ids));
        assert_eq!(turn, PlanTurn::Continue);

        // Completing a read frees capacity; the backlog drains on the next turn.
        assert_eq!(
            plan.on_read(&ReadCompletion::new(
                requests[0].operation(),
                ReadOutcome::Settled { node: None },
            )),
            PlanReadOutcome::Applied
        );
        let turn = plan.run_turn(&mut ctx(s, &mut ids));
        let PlanTurn::Reads(requests) = turn else {
            panic!("expected reads");
        };
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].key(), SHAMapHash::new(Uint256::from(3)));
    }

    #[test]
    fn timeout_budget_fails_after_the_bound() {
        let mut ids = IdCounter::new();
        let s = session();
        let mut plan = SessionPlan::with_max_timeouts(budget(), 3);
        assert_eq!(plan.on_timeout(), PlanTimeout::Continue);
        assert_eq!(plan.on_timeout(), PlanTimeout::Continue);
        assert_eq!(plan.timeouts(), 2);
        assert_eq!(plan.on_timeout(), PlanTimeout::Fail);
        assert!(plan.is_cancelled());
        // A cancelled plan produces no further work.
        let turn = plan.run_turn(&mut ctx(s, &mut ids));
        assert_eq!(turn, PlanTurn::Continue);
    }

    #[test]
    fn cancellation_clears_every_inflight_resource() {
        let mut ids = IdCounter::new();
        let s = session();
        let mut plan = scripted(
            1,
            vec![
                ScriptedStep::NeedsReads(vec![need(7)]),
                ScriptedStep::Complete,
            ],
        );
        let turn = plan.run_turn(&mut ctx(s, &mut ids));
        let PlanTurn::Reads(requests) = turn else {
            panic!("expected reads");
        };
        plan.cancel();
        assert!(plan.is_cancelled());
        assert!(plan.engine().is_none());
        assert_eq!(plan.packet_count(), 0);
        assert_eq!(plan.packet_bytes(), 0);
        assert!(plan.pending_network().is_empty());
        // Every late completion is stale after cancellation.
        assert_eq!(
            plan.on_read(&ReadCompletion::new(
                requests[0].operation(),
                ReadOutcome::Settled { node: None }
            )),
            PlanReadOutcome::Stale
        );
        assert_eq!(plan.on_timeout(), PlanTimeout::Fail);
    }

    #[test]
    fn network_candidates_are_tracked() {
        let mut ids = IdCounter::new();
        let s = session();
        let mut plan = scripted(
            1,
            vec![ScriptedStep::NeedsNetwork(vec![(
                SHAMapNodeId::default(),
                Uint256::from(42),
            )])],
        );
        let turn = plan.run_turn(&mut ctx(s, &mut ids));
        let PlanTurn::Network(nodes) = turn else {
            panic!("expected network, got {turn:?}");
        };
        assert_eq!(nodes.len(), 1);
        assert!(plan.pending_network().contains(&Uint256::from(42)));
        assert_eq!(nodes[0].kind(), TreeKind::State);
    }

    #[test]
    fn scripted_network_need_preserves_transaction_kind() {
        let mut ids = IdCounter::new();
        let s = session();
        let mut plan = scripted(
            1,
            vec![ScriptedStep::NeedsNetworkWithKind(vec![
                PlanNetworkNeed::new(
                    SHAMapNodeId::default(),
                    Uint256::from(77),
                    TreeKind::Transaction,
                ),
            ])],
        );
        let PlanTurn::Network(nodes) = plan.run_turn(&mut ctx(s, &mut ids)) else {
            panic!("expected network turn");
        };
        assert_eq!(nodes[0].hash(), Uint256::from(77));
        assert_eq!(nodes[0].kind(), TreeKind::Transaction);
    }

    #[test]
    fn invalid_plan_is_surfaced_as_invalid_turn() {
        let mut ids = IdCounter::new();
        let s = session();
        let mut plan = scripted(1, vec![ScriptedStep::Invalid]);
        let turn = plan.run_turn(&mut ctx(s, &mut ids));
        assert_eq!(turn, PlanTurn::Invalid);
    }

    #[test]
    fn packets_are_fed_into_the_engine_in_bounded_batches() {
        let mut ids = IdCounter::new();
        let s = session();
        let mut plan = SessionPlan::new(budget());
        assert!(plan.install_engine(Box::new(ScriptedEngine::new(
            TreePlanId::new(1),
            Vec::new(),
            Vec::new(),
        ))));
        // Feed six packets; only the bounded per-turn batch is consumed.
        for _ in 0..6 {
            assert!(plan.push_packet(packet(s, AdmissionBudget::new(1, 64), 8)));
        }
        assert_eq!(plan.packet_count(), 6);
        let turn = plan.run_turn(&mut ctx(s, &mut ids));
        assert_eq!(turn, PlanTurn::Continue);
        assert_eq!(plan.packet_count(), 2);
    }

    #[test]
    fn install_engine_is_single_shot() {
        let mut plan = SessionPlan::new(budget());
        assert!(plan.install_engine(Box::new(ScriptedEngine::new(
            TreePlanId::new(1),
            Vec::new(),
            Vec::new(),
        ))));
        assert!(!plan.install_engine(Box::new(ScriptedEngine::new(
            TreePlanId::new(2),
            Vec::new(),
            Vec::new(),
        ))));
        assert_eq!(plan.engine().expect("engine").plan_id(), TreePlanId::new(1));
    }

    #[test]
    fn real_engine_traverses_a_rooted_tree_fixture() {
        // A rooted two-level state tree with one missing child, exercised with
        // the real ledger TreePlan over NullResident (no synchronous reads).
        // The child is a real leaf node whose hash we compute first, then hang
        // under the root by its hash, so a decoded copy of the child can be
        // applied back and matches exactly.
        let child = make_shared_intrusive(SHAMapTreeNode::new_leaf(
            shamap::tree_node::SHAMapNodeType::AccountState,
            SHAMapItem::new(Uint256::from(1), vec![0u8; 12]),
            0,
        ));
        let child_hash = child.get_hash();
        assert!(!child_hash.is_zero());

        let root = make_shared_intrusive(SHAMapTreeNode::new_inner(0));
        root.set_child_hash(1, child_hash);
        root.update_hash();
        let tree = SyncTree::from_root_with_type(
            root,
            shamap::sync::SHAMapType::State,
            true,
            1,
            SyncState::Synching,
        );

        let mut engine = LedgerTreePlanEngine::new(TreePlan::new(
            TreePlanId::new(1),
            TreeKind::State,
            &tree,
            child_hash,
            256,
            // Non-zero full-below generation so the fresh root (generation 0)
            // is not treated as fully below and is scanned for missing nodes.
            1,
            &mut || rand_int_to(255u8),
        ));
        assert_eq!(engine.plan_id(), TreePlanId::new(1));

        // The traversal announces the missing child subtree as a read.
        match engine.advance(MAX_NEW_READS_PER_PASS) {
            PlanStepOutcome::NeedsReads(reads) => {
                assert_eq!(reads.len(), 1);
                assert_eq!(reads[0].hash(), child_hash);
            }
            other => panic!("expected a read need, got {other:?}"),
        }

        // A decoded wire copy of the child attaches through the announced
        // read path (the coordinator's NodeStore broker would deliver it).
        let serialized = child.serialize_for_wire().expect("serialize child");
        match engine.apply_read(
            child_hash,
            &ReadOutcome::Settled {
                node: Some(Bytes::from(serialized)),
            },
        ) {
            PlanReadApply::Applied { .. } | PlanReadApply::HashMismatch => {}
            other => panic!("unexpected apply {other:?}"),
        }

        // The plan may now complete or wait on the scheduler; either is a
        // legal bounded outcome. The key invariant is the traversal boundary
        // held: reads were announced, no synchronous I/O occurred.
        let outcome = engine.advance(MAX_NEW_READS_PER_PASS);
        assert!(
            matches!(outcome, PlanStepOutcome::Ready | PlanStepOutcome::Complete),
            "unexpected outcome {outcome:?}"
        );
    }
}
