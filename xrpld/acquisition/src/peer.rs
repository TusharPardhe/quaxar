//! Peer availability and peer request surface.
//!
//! Overlay owns sockets and transport. The coordinator owns acquisition demand
//! and request policy; a peer send is a coordinator-produced effect carrying a
//! `SessionRef` and is never emitted after a terminal transition.

use basics::base_uint::Uint256;
use ledger::TreeKind;
use shamap::node_id::SHAMapNodeId;

use crate::id::PeerId;
use crate::identity::{OperationRef, SessionRef};

/// An immutable snapshot of currently usable peers, reported by overlay
/// connectivity. A non-empty snapshot is the `PeerCapabilityAvailable` fact; an
/// empty snapshot is `PeerCapabilityLost`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerAvailabilitySnapshot {
    peers: Vec<PeerId>,
}

impl PeerAvailabilitySnapshot {
    /// Builds a snapshot from a set of usable peers.
    pub fn new(peers: Vec<PeerId>) -> Self {
        Self { peers }
    }

    /// The usable peers, in overlay-report order.
    pub fn peers(&self) -> &[PeerId] {
        &self.peers
    }

    /// True when at least one usable peer exists.
    pub fn has_usable_peer_capability(&self) -> bool {
        !self.peers.is_empty()
    }
}

/// A coordinator-produced peer request. Overlay adapters own actual delivery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerRequest {
    session: SessionRef,
    operation: OperationRef,
    peer_id: PeerId,
    request: LedgerDataRequest,
}

impl PeerRequest {
    /// Builds a peer request for a session.
    pub const fn new(
        session: SessionRef,
        operation: OperationRef,
        peer_id: PeerId,
        request: LedgerDataRequest,
    ) -> Self {
        Self {
            session,
            operation,
            peer_id,
            request,
        }
    }

    /// The session this request serves.
    pub fn session(&self) -> SessionRef {
        self.session
    }

    /// The exact operation identity of this send.
    pub fn operation(&self) -> OperationRef {
        self.operation
    }

    /// The peer to send to.
    pub fn peer_id(&self) -> PeerId {
        self.peer_id
    }

    /// The request payload.
    pub fn request(&self) -> &LedgerDataRequest {
        &self.request
    }
}

/// One tree node requested from a peer. A hash alone is insufficient: state
/// and transaction SHAMap frontiers must remain distinct through the overlay
/// adapter so replies are routed to the matching tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LedgerNodeRequest {
    hash: Uint256,
    kind: TreeKind,
}

impl LedgerNodeRequest {
    /// Builds one kind-qualified node request.
    pub const fn new(hash: Uint256, kind: TreeKind) -> Self {
        Self { hash, kind }
    }

    /// The requested node hash.
    pub const fn hash(self) -> Uint256 {
        self.hash
    }

    /// The SHAMap tree this node belongs to.
    pub const fn kind(self) -> TreeKind {
        self.kind
    }
}

/// The outbound ledger-data request surface. Exact framing is owned by the
/// overlay adapter; this is the policy-level intent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LedgerDataRequest {
    /// Request the Base/header ledger packet. `None` is an unknown-sequence
    /// acquisition and must remain a Base request, never a `GetNodes` request
    /// for the target hash.
    GetLedger { sequence: Option<u32> },
    /// Request kind-qualified tree nodes through `TMGetLedger`. This is the
    /// normal rippled acquisition path; each node id preserves the SHAMap
    /// location required for a direct `TMLedgerData` reply.
    GetLedgerNodes {
        kind: TreeKind,
        node_ids: Vec<SHAMapNodeId>,
        sequence: u32,
    },
    /// Request kind-qualified tree nodes by hash after an aggressive
    /// no-progress retry. `sequence` is the Base-verified header sequence
    /// when available; hash-only sessions promote it after their header reply.
    GetNodes {
        nodes: Vec<LedgerNodeRequest>,
        sequence: Option<u32>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::{IdCounter, OperationGeneration, OperationId};
    use crate::identity::{OperationKind, SessionRef};

    #[test]
    fn peer_availability_capability_reflects_nonempty_snapshot() {
        assert!(!PeerAvailabilitySnapshot::new(vec![]).has_usable_peer_capability());
        assert!(PeerAvailabilitySnapshot::new(vec![PeerId::new(1)]).has_usable_peer_capability());
        assert_eq!(
            PeerAvailabilitySnapshot::new(vec![PeerId::new(1)]).peers(),
            &[PeerId::new(1)]
        );
    }

    #[test]
    fn peer_request_carries_full_identity() {
        let mut counter = IdCounter::new();
        let session = SessionRef::new(
            counter.next_id(),
            counter.next_id(),
            Uint256::from(1),
            counter.next_id(),
            counter.next_id(),
        );
        let operation = OperationRef::new(
            session,
            OperationKind::PeerRequest,
            counter.next_id(),
            counter.next_id(),
        );
        let request = PeerRequest::new(
            session,
            operation,
            PeerId::new(3),
            LedgerDataRequest::GetLedger { sequence: Some(4) },
        );
        assert_eq!(request.session(), session);
        assert_eq!(request.operation(), operation);
        assert_eq!(request.peer_id(), PeerId::new(3));
        assert_eq!(
            request.request(),
            &LedgerDataRequest::GetLedger { sequence: Some(4) }
        );
        assert_ne!(OperationId::INVALID, operation.operation_id());
        assert_ne!(OperationGeneration::INVALID, operation.generation());
    }

    #[test]
    fn node_requests_preserve_tree_kind_and_base_can_omit_sequence() {
        let node = LedgerNodeRequest::new(Uint256::from(9), TreeKind::Transaction);
        assert_eq!(node.hash(), Uint256::from(9));
        assert_eq!(node.kind(), TreeKind::Transaction);
        assert_eq!(
            LedgerDataRequest::GetLedger { sequence: None },
            LedgerDataRequest::GetLedger { sequence: None }
        );
    }
}
