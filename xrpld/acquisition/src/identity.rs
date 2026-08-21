//! Session and operation identity, and the stale-event rules built on them.
//!
//! A [`SessionRef`] is the complete, comparable identity of one session at a
//! point in time. A [`LiveSessionIdentity`] is the invariant part of a live
//! session. An [`OperationRef`] additionally identifies one exact dispatched
//! operation so a late or rearmed same-kind completion cannot mutate a session.

use basics::base_uint::Uint256;

use crate::id::{
    OperationGeneration, OperationId, PlanEpoch, RunEpoch, SessionId, StoreGeneration,
};

/// Complete, comparable identity of one acquisition session.
///
/// `plan_epoch` and `store_generation` are part of the identity so a completion
/// that was dispatched against an old plan or an old database generation cannot
/// mutate the current session. Cancellation invalidates the session immediately;
/// the target hash alone never authorizes a result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionRef {
    run_epoch: RunEpoch,
    session_id: SessionId,
    target_hash: Uint256,
    plan_epoch: PlanEpoch,
    store_generation: StoreGeneration,
}

impl SessionRef {
    /// Builds a session reference from its complete identity.
    pub const fn new(
        run_epoch: RunEpoch,
        session_id: SessionId,
        target_hash: Uint256,
        plan_epoch: PlanEpoch,
        store_generation: StoreGeneration,
    ) -> Self {
        Self {
            run_epoch,
            session_id,
            target_hash,
            plan_epoch,
            store_generation,
        }
    }

    /// The run epoch of this session.
    pub const fn run_epoch(self) -> RunEpoch {
        self.run_epoch
    }

    /// The session id within its run epoch.
    pub const fn session_id(self) -> SessionId {
        self.session_id
    }

    /// The target ledger hash this session acquires.
    pub const fn target_hash(self) -> Uint256 {
        self.target_hash
    }

    /// The tree-plan generation this session reference was minted for.
    pub const fn plan_epoch(self) -> PlanEpoch {
        self.plan_epoch
    }

    /// The NodeStore generation this session reference was minted for.
    pub const fn store_generation(self) -> StoreGeneration {
        self.store_generation
    }

    /// The invariant identity of the live session.
    pub const fn live_identity(self) -> LiveSessionIdentity {
        LiveSessionIdentity {
            run_epoch: self.run_epoch,
            session_id: self.session_id,
            target_hash: self.target_hash,
        }
    }

    /// True when this reference belongs to the given live session. Compares the
    /// run epoch, session id, and target hash only; plan/store generation are
    /// validated separately for plan- and storage-scoped operations.
    pub fn matches_live(self, live: &LiveSessionIdentity) -> bool {
        self.run_epoch == live.run_epoch
            && self.session_id == live.session_id
            && self.target_hash == live.target_hash
    }
}

/// The invariant part of a live session identity: the run epoch, session id,
/// and target hash. Replacement sessions for the same target hash carry a new
/// session id, so every old `SessionRef` fails `matches_live`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LiveSessionIdentity {
    run_epoch: RunEpoch,
    session_id: SessionId,
    target_hash: Uint256,
}

impl LiveSessionIdentity {
    /// Builds a live-session identity.
    pub const fn new(run_epoch: RunEpoch, session_id: SessionId, target_hash: Uint256) -> Self {
        Self {
            run_epoch,
            session_id,
            target_hash,
        }
    }

    /// The run epoch of the live session.
    pub const fn run_epoch(self) -> RunEpoch {
        self.run_epoch
    }

    /// The session id of the live session.
    pub const fn session_id(self) -> SessionId {
        self.session_id
    }

    /// The target hash of the live session.
    pub const fn target_hash(self) -> Uint256 {
        self.target_hash
    }
}

/// The kind of an externally dispatched operation. Each kind is tracked with its
/// own operation id/generation so a late completion of one kind cannot be
/// confused with a different kind on the same session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OperationKind {
    /// A bounded tree-plan turn on the coordinator owner task.
    TreeTurn,
    /// A tree-plan turn offloaded to a CPU job.
    CpuTurn,
    /// A physical NodeStore read submitted through the broker.
    Read,
    /// An asynchronous NodeStore reprobe of one retained network frontier
    /// need. It is distinct from an ordinary traversal read so a late timeout
    /// completion cannot be mistaken for the original read it retried.
    RecoveryRead,
    /// A physical NodeStore write submitted through the write adapter.
    Write,
    /// A durability fence (final persistence barrier) completion.
    DurabilityFence,
    /// A peer request sent through the overlay adapter.
    PeerRequest,
    /// A timer armed through the timer port.
    Timer,
    /// A packet admission lease.
    AdmissionLease,
    /// A durable ledger handoff.
    DurableHandoff,
}

impl OperationKind {
    /// A stable label for observability.
    pub const fn label(self) -> &'static str {
        match self {
            Self::TreeTurn => "tree_turn",
            Self::CpuTurn => "cpu_turn",
            Self::Read => "read",
            Self::RecoveryRead => "recovery_read",
            Self::Write => "write",
            Self::DurabilityFence => "durability_fence",
            Self::PeerRequest => "peer_request",
            Self::Timer => "timer",
            Self::AdmissionLease => "admission_lease",
            Self::DurableHandoff => "durable_handoff",
        }
    }
}

/// Identity of one exact dispatched operation. Carried by every read, write,
/// durability fence, peer request, CPU turn, admission lease, timer arm, and
/// durable handoff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OperationRef {
    session: SessionRef,
    kind: OperationKind,
    operation_id: OperationId,
    generation: OperationGeneration,
}

impl OperationRef {
    /// Builds an operation reference for a session.
    pub const fn new(
        session: SessionRef,
        kind: OperationKind,
        operation_id: OperationId,
        generation: OperationGeneration,
    ) -> Self {
        Self {
            session,
            kind,
            operation_id,
            generation,
        }
    }

    /// The session this operation was dispatched against.
    pub const fn session(self) -> SessionRef {
        self.session
    }

    /// The kind of operation.
    pub const fn kind(self) -> OperationKind {
        self.kind
    }

    /// The per-session, per-kind operation id.
    pub const fn operation_id(self) -> OperationId {
        self.operation_id
    }

    /// The generation guarding against rearmed or late same-kind completions.
    pub const fn generation(self) -> OperationGeneration {
        self.generation
    }

    /// True when `self` is exactly the operation the coordinator expects to be
    /// in flight. Requires a full `SessionRef` match (including plan and store
    /// generation) plus the exact kind, operation id, and generation.
    pub fn is_expected_for(self, expected: &OperationRef) -> bool {
        self.session == expected.session
            && self.kind == expected.kind
            && self.operation_id == expected.operation_id
            && self.generation == expected.generation
    }

    /// True when `self` targets the same session as `other`, even if the
    /// operation identity differs.
    pub fn same_session(self, other: &OperationRef) -> bool {
        self.session == other.session
    }

    /// True when `self` and `other` share kind and generation (a rearm of the
    /// same kind bumps the generation, so this is false for rearmed ops).
    pub fn same_kind_and_generation(self, other: &OperationRef) -> bool {
        self.kind == other.kind && self.generation == other.generation
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::{IdCounter, PlanEpoch, StoreGeneration};

    fn sample_session(counter: &mut IdCounter) -> SessionRef {
        SessionRef::new(
            counter.next_id(),
            counter.next_id(),
            Uint256::from(7),
            counter.next_id(),
            counter.next_id(),
        )
    }

    #[test]
    fn session_ref_matches_live_identity() {
        let mut counter = IdCounter::new();
        let session = sample_session(&mut counter);
        let live = session.live_identity();
        assert!(session.matches_live(&live));

        // A replacement session for the same target hash fails to match.
        let replacement = SessionRef::new(
            live.run_epoch(),
            counter.next_id(),
            live.target_hash(),
            session.plan_epoch(),
            session.store_generation(),
        );
        assert!(!replacement.matches_live(&live));

        // A different run epoch fails to match even with the same session id.
        let other_epoch = SessionRef::new(
            counter.next_id(),
            live.session_id(),
            live.target_hash(),
            session.plan_epoch(),
            session.store_generation(),
        );
        assert!(!other_epoch.matches_live(&live));
    }

    #[test]
    fn plan_and_store_generation_are_part_of_session_identity() {
        let mut counter = IdCounter::new();
        let original = sample_session(&mut counter);
        let retargeted = SessionRef::new(
            original.run_epoch(),
            original.session_id(),
            original.target_hash(),
            PlanEpoch::new(original.plan_epoch().get() + 1),
            original.store_generation(),
        );
        assert_ne!(original, retargeted);

        let mut rotated = original;
        rotated.store_generation = StoreGeneration::new(original.store_generation().get() + 1);
        assert_ne!(original, rotated);
    }

    #[test]
    fn operation_ref_exact_matching_rejects_stale_identity() {
        let mut counter = IdCounter::new();
        let session = sample_session(&mut counter);
        let expected = OperationRef::new(
            session,
            OperationKind::Read,
            counter.next_id(),
            counter.next_id(),
        );

        assert!(expected.is_expected_for(&expected));

        // Same session, different operation id: stale.
        let wrong_id = OperationRef::new(
            session,
            OperationKind::Read,
            counter.next_id(),
            expected.generation(),
        );
        assert!(!wrong_id.is_expected_for(&expected));

        // Same session/kind/id, older generation (rearmed): stale.
        let older_generation = OperationRef::new(
            session,
            OperationKind::Read,
            expected.operation_id(),
            counter.next_id(),
        );
        assert!(!older_generation.is_expected_for(&expected));

        // Same session, different kind: not the expected operation.
        let wrong_kind = OperationRef::new(
            session,
            OperationKind::Timer,
            expected.operation_id(),
            expected.generation(),
        );
        assert!(!wrong_kind.is_expected_for(&expected));

        // Same target hash, replacement session: stale.
        let live = session.live_identity();
        let replacement_session = SessionRef::new(
            live.run_epoch(),
            counter.next_id(),
            live.target_hash(),
            session.plan_epoch(),
            session.store_generation(),
        );
        let replaced = OperationRef::new(
            replacement_session,
            OperationKind::Read,
            expected.operation_id(),
            expected.generation(),
        );
        assert!(!replaced.is_expected_for(&expected));
    }

    #[test]
    fn operation_ref_same_kind_generation_distinguishes_rearm() {
        let mut counter = IdCounter::new();
        let session = sample_session(&mut counter);
        let original = OperationRef::new(
            session,
            OperationKind::Timer,
            counter.next_id(),
            counter.next_id(),
        );
        let rearmed = OperationRef::new(
            session,
            OperationKind::Timer,
            counter.next_id(),
            original.generation(),
        );
        // A rearm keeps the same generation only if intentionally reusing it;
        // the coordinator bumps generation on rearm, so a same-generation timer
        // is considered a duplicate of the kind.
        assert!(original.same_kind_and_generation(&rearmed));
        assert!(!rearmed.is_expected_for(&original));
    }

    #[test]
    fn operation_kind_labels_are_stable() {
        assert_eq!(OperationKind::DurabilityFence.label(), "durability_fence");
        assert_eq!(OperationKind::DurableHandoff.label(), "durable_handoff");
    }
}
