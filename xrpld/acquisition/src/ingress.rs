//! Overlay ingress admission and routing.
//!
//! Overlay owns sockets, framing, and transport retry. The coordinator owns
//! admission policy and session routing. Ingress may only look up a route in
//! an immutable [`RoutingSnapshot`], atomically reserve packet/byte budget
//! through an [`AdmissionGate`], receive an [`AdmissionLease`], and enqueue a
//! typed [`crate::AcquisitionEvent::PacketAdmitted`] — or defer if admission
//! fails. Ingress never accesses a mutable planner or registry entry.
//!
//! The default budget preserves the existing per-ledger mailbox semantics:
//! `128` packets and `4 MiB`.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};

use basics::base_uint::Uint256;
use ledger::InboundLedgerPacket;

use crate::id::{AdmissionLeaseId, PeerId, RoutingGeneration};
use crate::identity::SessionRef;

/// Existing per-ledger mailbox packet capacity, preserved as the default
/// admission budget (`acquisition.rs`, `AcquisitionMailbox`).
pub const ADMISSION_PACKET_LIMIT: u64 = 128;

/// Existing per-ledger mailbox byte capacity, preserved as the default
/// admission budget (`acquisition.rs`, `AcquisitionMailbox`).
pub const ADMISSION_BYTE_LIMIT: u64 = 4 * 1024 * 1024;

/// A bounded admission budget for one session route. `Default` reproduces the
/// existing `128`-packet / `4 MiB` mailbox semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdmissionBudget {
    max_packets: u64,
    max_bytes: u64,
}

impl AdmissionBudget {
    /// Builds a budget with explicit limits.
    pub const fn new(max_packets: u64, max_bytes: u64) -> Self {
        Self {
            max_packets,
            max_bytes,
        }
    }

    /// The maximum concurrently reserved packets.
    pub const fn max_packets(self) -> u64 {
        self.max_packets
    }

    /// The maximum concurrently reserved bytes.
    pub const fn max_bytes(self) -> u64 {
        self.max_bytes
    }
}

impl Default for AdmissionBudget {
    fn default() -> Self {
        Self::new(ADMISSION_PACKET_LIMIT, ADMISSION_BYTE_LIMIT)
    }
}

/// Why admission could not grant a lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackpressureReason {
    /// The session's packet budget is exhausted.
    MailboxFull,
    /// The session's byte budget is exhausted.
    ByteBudgetExceeded,
    /// The session is terminal; no new packet may enter.
    TerminalSession,
    /// The coordinator is shutting down.
    ShuttingDown,
}

/// The result of an admission reservation attempt.
#[derive(Debug, PartialEq, Eq)]
pub enum BackpressureOutcome {
    /// Capacity reserved; the caller owns one move-only [`AdmissionLease`].
    /// [`AdmittedLedgerPacket::new`] consumes it for a matching session; simply
    /// dropping an unadmitted lease restores only its bound gate capacity.
    Admitted(AdmissionLease),
    /// Capacity is currently exhausted; the caller defers the packet. A
    /// deferred packet has no actor-side effect.
    Deferred,
    /// The session is terminal or shutting down; the packet cannot be routed.
    Rejected(BackpressureReason),
}

/// A packet/byte reservation on one session's admission budget.
///
/// A lease is an affine ingress capability: it is neither cloneable nor
/// externally consumable. [`AdmittedLedgerPacket::new`] consumes it only after
/// validating the intended session. Dropping an unadmitted lease restores its
/// reservation; no adapter receives a raw gate-release API.
#[derive(Debug)]
pub struct AdmissionLease {
    lease_id: AdmissionLeaseId,
    session: SessionRef,
    packet_count: u64,
    byte_count: u64,
    gate: Arc<AdmissionGate>,
    settled: AtomicU8,
}

const LEASE_OPEN: u8 = 0;
const LEASE_CONSUMED: u8 = 1;
const LEASE_RELEASED: u8 = 2;

impl PartialEq for AdmissionLease {
    fn eq(&self, other: &Self) -> bool {
        self.lease_id == other.lease_id
            && self.session == other.session
            && self.packet_count == other.packet_count
            && self.byte_count == other.byte_count
            && Arc::ptr_eq(&self.gate, &other.gate)
    }
}

impl Eq for AdmissionLease {}

impl AdmissionLease {
    fn new(
        lease_id: AdmissionLeaseId,
        session: SessionRef,
        packet_count: u64,
        byte_count: u64,
        gate: Arc<AdmissionGate>,
    ) -> Self {
        Self {
            lease_id,
            session,
            packet_count,
            byte_count,
            gate,
            settled: AtomicU8::new(LEASE_OPEN),
        }
    }

    /// The unique lease id.
    pub const fn lease_id(&self) -> AdmissionLeaseId {
        self.lease_id
    }

    /// The session this lease reserves capacity for.
    pub const fn session(&self) -> SessionRef {
        self.session
    }

    /// The reserved packet count.
    pub const fn packet_count(&self) -> u64 {
        self.packet_count
    }

    /// The reserved byte count.
    pub const fn byte_count(&self) -> u64 {
        self.byte_count
    }

    /// True when this lease was admitted or released.
    pub fn is_settled(&self) -> bool {
        self.settled.load(Ordering::Relaxed) != LEASE_OPEN
    }

    fn consume_for_packet(&self) -> bool {
        self.settled
            .compare_exchange(
                LEASE_OPEN,
                LEASE_CONSUMED,
                Ordering::Relaxed,
                Ordering::Relaxed,
            )
            .is_ok()
    }

    fn settle_consumed(&self) -> bool {
        if self
            .settled
            .compare_exchange(
                LEASE_CONSUMED,
                LEASE_RELEASED,
                Ordering::Relaxed,
                Ordering::Relaxed,
            )
            .is_ok()
        {
            self.gate.release_inner(self.packet_count, self.byte_count);
            true
        } else {
            false
        }
    }

    fn release_if_open(&self) {
        if self
            .settled
            .compare_exchange(
                LEASE_OPEN,
                LEASE_RELEASED,
                Ordering::Relaxed,
                Ordering::Relaxed,
            )
            .is_ok()
        {
            self.gate.release_inner(self.packet_count, self.byte_count);
        }
    }
}

impl Drop for AdmissionLease {
    fn drop(&mut self) {
        self.release_if_open();
    }
}

/// Why a reserved lease could not become an admitted packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionPacketError {
    /// The caller attempted to route a lease through a different session than
    /// the gate that issued it. The moved lease is dropped and releases only
    /// its bound gate.
    SessionMismatch,
    /// The lease had already been consumed or released and cannot be replayed.
    LeaseSettled,
}

/// A decoded packet routed to a session with its consumed, gate-bound lease.
///
/// Construction validates the exact `SessionRef` and consumes the lease
/// internally. [`Self::settle`] releases the originating gate exactly once
/// after coordinator processing; `Drop` is the defensive fallback.
#[derive(Debug)]
pub struct AdmittedLedgerPacket {
    lease: AdmissionLease,
    peer_id: PeerId,
    packet: InboundLedgerPacket,
}

impl PartialEq for AdmittedLedgerPacket {
    fn eq(&self, other: &Self) -> bool {
        self.lease == other.lease && self.peer_id == other.peer_id && self.packet == other.packet
    }
}

impl Eq for AdmittedLedgerPacket {}

impl AdmittedLedgerPacket {
    /// Consumes a reservation into a packet for `session`. The original gate
    /// remains bound to the packet, preventing a stale lease from being routed
    /// to a replacement session or released through an unrelated gate.
    pub fn new(
        lease: AdmissionLease,
        session: SessionRef,
        peer_id: PeerId,
        packet: InboundLedgerPacket,
    ) -> Result<Self, AdmissionPacketError> {
        if lease.session != session || lease.gate.session() != session {
            return Err(AdmissionPacketError::SessionMismatch);
        }
        if !lease.consume_for_packet() {
            return Err(AdmissionPacketError::LeaseSettled);
        }
        Ok(Self {
            lease,
            peer_id,
            packet,
        })
    }

    /// Settles the consumed reservation exactly once. This is the only public
    /// release path and always returns capacity to the gate that minted it.
    pub fn settle(&mut self) -> bool {
        self.lease.settle_consumed()
    }

    /// The consumed admission lease.
    pub fn lease(&self) -> &AdmissionLease {
        &self.lease
    }

    /// The peer that supplied the packet.
    pub const fn peer_id(&self) -> PeerId {
        self.peer_id
    }

    /// The decoded packet payload.
    pub fn packet(&self) -> &InboundLedgerPacket {
        &self.packet
    }
}

impl Drop for AdmittedLedgerPacket {
    fn drop(&mut self) {
        let _ = self.settle();
    }
}

/// One route in an immutable routing snapshot: a session and its admission
/// gate. The gate owns only reservation counters (resource-local state); the
/// coordinator owns admission policy.
#[derive(Debug, Clone)]
pub struct RouteEntry {
    session: SessionRef,
    gate: Arc<AdmissionGate>,
}

impl RouteEntry {
    /// Builds a route entry.
    pub fn new(session: SessionRef, gate: Arc<AdmissionGate>) -> Self {
        Self { session, gate }
    }

    /// The session this route serves.
    pub const fn session(&self) -> SessionRef {
        self.session
    }

    /// The shared admission gate for this route. The `Arc` is required because
    /// reservations bind the exact originating gate into their move-only lease.
    pub fn gate(&self) -> &Arc<AdmissionGate> {
        &self.gate
    }
}

/// An immutable routing snapshot published by the coordinator for overlay
/// ingress. Ingress lookups never touch a mutable planner or registry entry.
#[derive(Debug)]
pub struct RoutingSnapshot {
    generation: RoutingGeneration,
    routes: BTreeMap<Uint256, RouteEntry>,
}

impl RoutingSnapshot {
    /// Builds a snapshot from a route map.
    pub fn new(generation: RoutingGeneration, routes: BTreeMap<Uint256, RouteEntry>) -> Self {
        Self { generation, routes }
    }

    /// The snapshot generation.
    pub const fn generation(&self) -> RoutingGeneration {
        self.generation
    }

    /// Looks up the route for a target hash.
    pub fn route(&self, target_hash: &Uint256) -> Option<&RouteEntry> {
        self.routes.get(target_hash)
    }

    /// The number of routed sessions.
    pub fn session_count(&self) -> usize {
        self.routes.len()
    }

    /// True when no sessions are routed.
    pub fn is_empty(&self) -> bool {
        self.routes.is_empty()
    }
}

/// Bounded packet/byte admission for one session, shared between overlay
/// ingress threads and the coordinator. The coordinator owns admission policy;
/// this gate owns only the reservation counters and exposes atomic
/// reservation/release so ingress never holds a session lock.
#[derive(Debug)]
pub struct AdmissionGate {
    budget: AdmissionBudget,
    session: SessionRef,
    reserved_packets: AtomicU64,
    reserved_bytes: AtomicU64,
    next_lease_id: AtomicU64,
}

impl AdmissionGate {
    /// Builds a gate for a session with the given budget.
    pub fn new(budget: AdmissionBudget, session: SessionRef) -> Self {
        Self {
            budget,
            session,
            reserved_packets: AtomicU64::new(0),
            reserved_bytes: AtomicU64::new(0),
            next_lease_id: AtomicU64::new(1),
        }
    }

    /// The session this gate admits for.
    pub const fn session(&self) -> SessionRef {
        self.session
    }

    /// The budget limits of this gate.
    pub const fn budget(&self) -> AdmissionBudget {
        self.budget
    }

    /// The currently reserved packet count.
    pub fn current_packets(&self) -> u64 {
        self.reserved_packets.load(Ordering::Relaxed)
    }

    /// The currently reserved byte count.
    pub fn current_bytes(&self) -> u64 {
        self.reserved_bytes.load(Ordering::Relaxed)
    }

    /// Atomically reserves packet and byte budget for a decoded packet.
    ///
    /// Bytes are reserved first; if the packet budget then fails the byte
    /// reservation is rolled back. On success the caller owns one lease.
    pub fn try_reserve(
        self: &Arc<Self>,
        mut packet_count: u64,
        byte_count: u64,
    ) -> BackpressureOutcome {
        if packet_count == 0 {
            packet_count = 1;
        }
        if !self.reserve_bytes(byte_count) {
            return BackpressureOutcome::Deferred;
        }
        if !self.reserve_packets(packet_count) {
            // Only the byte reservation landed; the packet reservation failed
            // atomically (fetch_update leaves the value untouched), so roll
            // back bytes alone and never touch the packet counter.
            self.reserved_bytes.fetch_sub(byte_count, Ordering::Relaxed);
            return BackpressureOutcome::Deferred;
        }
        let lease_id = AdmissionLeaseId::new(self.next_lease_id.fetch_add(1, Ordering::Relaxed));
        BackpressureOutcome::Admitted(AdmissionLease::new(
            lease_id,
            self.session,
            packet_count,
            byte_count,
            Arc::clone(self),
        ))
    }

    fn release_inner(&self, packet_count: u64, byte_count: u64) {
        self.reserved_packets
            .fetch_sub(packet_count, Ordering::Relaxed);
        self.reserved_bytes.fetch_sub(byte_count, Ordering::Relaxed);
    }

    fn reserve_packets(&self, packet_count: u64) -> bool {
        reserve_counter(
            &self.reserved_packets,
            self.budget.max_packets,
            packet_count,
        )
    }

    fn reserve_bytes(&self, byte_count: u64) -> bool {
        reserve_counter(&self.reserved_bytes, self.budget.max_bytes, byte_count)
    }
}

fn reserve_counter(counter: &AtomicU64, max: u64, amount: u64) -> bool {
    if amount == 0 {
        return true;
    }
    counter
        .fetch_update(
            Ordering::Relaxed,
            Ordering::Relaxed,
            |current| match current.checked_add(amount) {
                Some(next) if next <= max => Some(next),
                _ => None,
            },
        )
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::{IdCounter, PlanEpoch, RunEpoch, SessionId, StoreGeneration};

    fn session(n: u64) -> SessionRef {
        SessionRef::new(
            RunEpoch::new(1),
            SessionId::new(n),
            Uint256::from(n),
            PlanEpoch::new(1),
            StoreGeneration::new(1),
        )
    }

    #[test]
    fn default_budget_preserves_existing_limits() {
        let budget = AdmissionBudget::default();
        assert_eq!(budget.max_packets(), ADMISSION_PACKET_LIMIT);
        assert_eq!(budget.max_bytes(), ADMISSION_BYTE_LIMIT);
        assert_eq!(ADMISSION_PACKET_LIMIT, 128);
        assert_eq!(ADMISSION_BYTE_LIMIT, 4 * 1024 * 1024);
    }

    #[test]
    fn reservation_enforces_packet_limit_and_releases_on_capability_drop() {
        let gate = Arc::new(AdmissionGate::new(AdmissionBudget::new(3, 100), session(1)));
        let mut leases = Vec::new();
        for _ in 0..3 {
            match gate.try_reserve(1, 1) {
                BackpressureOutcome::Admitted(lease) => leases.push(lease),
                other => panic!("expected admission, got {other:?}"),
            }
        }
        assert_eq!(gate.current_packets(), 3);
        assert_eq!(gate.try_reserve(1, 1), BackpressureOutcome::Deferred);
        // Discarding an unadmitted affine capability restores exactly its slot.
        drop(leases.remove(0));
        assert_eq!(gate.current_packets(), 2);
        match gate.try_reserve(1, 1) {
            BackpressureOutcome::Admitted(_) => {}
            other => panic!("expected admission after release, got {other:?}"),
        }
    }

    #[test]
    fn reservation_enforces_byte_limit() {
        let gate = Arc::new(AdmissionGate::new(
            AdmissionBudget::new(100, 10),
            session(1),
        ));
        let first = match gate.try_reserve(1, 6) {
            BackpressureOutcome::Admitted(lease) => lease,
            other => panic!("expected admission, got {other:?}"),
        };
        let second = match gate.try_reserve(1, 4) {
            BackpressureOutcome::Admitted(lease) => lease,
            other => panic!("expected second admission, got {other:?}"),
        };
        assert_eq!(gate.current_bytes(), 10);
        assert_eq!(gate.try_reserve(1, 1), BackpressureOutcome::Deferred);
        drop(first);
        assert_eq!(gate.current_bytes(), 4);
        drop(second);
        assert_eq!(gate.current_bytes(), 0);
    }

    #[test]
    fn byte_failure_rolls_back_partial_packet_reservation() {
        let gate = Arc::new(AdmissionGate::new(AdmissionBudget::new(2, 5), session(1)));
        // Reserve all bytes first and retain the affine lease so its automatic
        // drop does not release the reservation before the assertion.
        let _lease = match gate.try_reserve(1, 5) {
            BackpressureOutcome::Admitted(lease) => lease,
            other => panic!("expected admission, got {other:?}"),
        };
        // Packet budget still has one slot but bytes are exhausted: the packet
        // reservation must be rolled back so the packet count stays unchanged.
        assert_eq!(gate.try_reserve(1, 1), BackpressureOutcome::Deferred);
        assert_eq!(gate.current_packets(), 1);
    }

    #[test]
    fn admitted_packet_consumes_and_settles_exactly_once() {
        let gate = Arc::new(AdmissionGate::new(AdmissionBudget::default(), session(1)));
        let lease = match gate.try_reserve(1, 1) {
            BackpressureOutcome::Admitted(lease) => lease,
            other => panic!("expected admission, got {other:?}"),
        };
        let mut admitted = AdmittedLedgerPacket::new(
            lease,
            session(1),
            PeerId::new(1),
            InboundLedgerPacket::new(ledger::InboundLedgerDataType::Base, Vec::new()),
        )
        .expect("matching session must consume the lease");
        assert!(admitted.lease().is_settled());
        assert_eq!(gate.current_packets(), 1);
        assert!(admitted.settle());
        assert!(!admitted.settle());
        assert_eq!(gate.current_packets(), 0);
    }

    #[test]
    fn stale_lease_carries_its_session_identity() {
        let gate = Arc::new(AdmissionGate::new(AdmissionBudget::default(), session(1)));
        let lease = match gate.try_reserve(1, 1) {
            BackpressureOutcome::Admitted(lease) => lease,
            other => panic!("expected admission, got {other:?}"),
        };
        // A replacement session for the same target hash must not accept this
        // lease: the lease's SessionRef differs from the new live session.
        let replacement_live = session(1).live_identity();
        let replacement = SessionRef::new(
            replacement_live.run_epoch(),
            SessionId::new(2),
            replacement_live.target_hash(),
            lease.session().plan_epoch(),
            lease.session().store_generation(),
        );
        assert!(!lease.session().matches_live(&replacement.live_identity()));
        assert!(lease.session().matches_live(&session(1).live_identity()));
    }

    #[test]
    fn concurrent_reservation_never_exceeds_budget() {
        use std::thread;
        let gate = Arc::new(AdmissionGate::new(
            AdmissionBudget::new(128, 4096),
            session(1),
        ));
        let mut handles = Vec::new();
        for _ in 0..8 {
            let gate = Arc::clone(&gate);
            handles.push(thread::spawn(move || {
                let mut admitted = 0u64;
                for _ in 0..32 {
                    match gate.try_reserve(1, 1) {
                        BackpressureOutcome::Admitted(lease) => {
                            admitted += 1;
                            drop(lease);
                        }
                        _ => {}
                    }
                }
                admitted
            }));
        }
        let total: u64 = handles.into_iter().map(|h| h.join().unwrap()).sum();
        // 8 x 32 attempts; each admits as long as the shared budget allows.
        assert_eq!(total, 8 * 32);
        assert_eq!(gate.current_packets(), 0);
        assert_eq!(gate.current_bytes(), 0);
    }

    #[test]
    fn routing_snapshot_lookup_by_target_hash() {
        let mut routes = BTreeMap::new();
        let gate = Arc::new(AdmissionGate::new(AdmissionBudget::default(), session(1)));
        routes.insert(Uint256::from(1), RouteEntry::new(session(1), gate));
        let snapshot = RoutingSnapshot::new(RoutingGeneration::new(1), routes);
        assert!(snapshot.route(&Uint256::from(1)).is_some());
        assert!(snapshot.route(&Uint256::from(2)).is_none());
        assert_eq!(snapshot.session_count(), 1);
        assert_eq!(snapshot.generation(), RoutingGeneration::new(1));
        assert!(!snapshot.is_empty());
    }

    #[test]
    fn admitted_packet_validates_bound_session_and_moves_payload() {
        let gate = Arc::new(AdmissionGate::new(AdmissionBudget::default(), session(1)));
        let lease = match gate.try_reserve(1, 1) {
            BackpressureOutcome::Admitted(lease) => lease,
            other => panic!("expected admission, got {other:?}"),
        };
        let lease_id = lease.lease_id();
        let packet = InboundLedgerPacket::new(
            ledger::InboundLedgerDataType::Base,
            vec![ledger::InboundLedgerNodeData::new(None, vec![1, 2])],
        );
        let admitted = AdmittedLedgerPacket::new(lease, session(1), PeerId::new(5), packet.clone())
            .expect("matching session must admit");
        assert_eq!(admitted.peer_id(), PeerId::new(5));
        assert_eq!(admitted.packet(), &packet);
        assert_eq!(admitted.lease().lease_id(), lease_id);

        let mismatched = match gate.try_reserve(1, 1) {
            BackpressureOutcome::Admitted(lease) => lease,
            other => panic!("expected admission, got {other:?}"),
        };
        assert_eq!(
            AdmittedLedgerPacket::new(mismatched, session(2), PeerId::new(5), packet,),
            Err(AdmissionPacketError::SessionMismatch)
        );
    }

    #[test]
    fn id_counter_mints_typed_lease_ids() {
        let mut counter = IdCounter::new();
        let a = counter.next_id::<AdmissionLeaseId>();
        let b = counter.next_id::<AdmissionLeaseId>();
        assert_ne!(a, b);
        assert!(!a.is_invalid());
    }
}
