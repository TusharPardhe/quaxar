//! Pure protocol surface for the single-owner acquisition migration.
//!
//! This crate defines the typed boundary between the acquisition domain and the
//! application adapters that serve it. The migration contract (see
//! `AGENTS.md`, milestone M1) requires a Rust-native, single-owner lifecycle:
//! one `AcquisitionCoordinator` owns every mutable acquisition-domain state on a
//! single serialized owner, receives typed [`AcquisitionEvent`]s, and emits
//! typed [`AcquisitionEffect`]s.
//!
//! Ownership rules enforced by this surface:
//!
//! * Every externally dispatched operation carries a complete [`SessionRef`]
//!   and an [`OperationRef`]. A raw target hash is never sufficient identity.
//! * A completion may mutate coordinator state only if its [`SessionRef`]
//!   matches the current live session and its operation id/generation matches
//!   the exact expected in-flight operation (see [`OperationRef::is_expected_for`]).
//! * Cancellation invalidates a session immediately; late network, read, write,
//!   timer, CPU, or handoff events are stale and ignored.
//! * The service phase transition table lives in [`SyncPhase`]; the coordinator
//!   is the only production writer of service phase.
//!
//! This crate must not depend on `xrpld/app`. Application-level services adapt
//! to it through the port traits in [`port`] and deterministic [`fake`] ports.

#![forbid(unsafe_code)]

mod effect;
mod error;
mod event;
pub mod fake;
mod handoff;
mod id;
mod identity;
mod ingress;
mod io;
mod peer;
mod phase;
mod plan;
mod port;
mod runner;
mod session;
mod shadow;
mod snapshot;
mod target;
mod timer;

pub use crate::effect::AcquisitionEffect;
pub use crate::error::AcquisitionError;
pub use crate::event::{AcquisitionEvent, ConsensusTarget};
pub use crate::handoff::{DurableHandoffAcknowledgement, DurableLedger, HandoffRejectReason};
pub use crate::id::{
    AdmissionLeaseId, DurableHandoffId, IdCounter, OperationGeneration, OperationId, PeerId,
    PlanEpoch, RoutingGeneration, RunEpoch, SessionId, StoreGeneration,
};
pub use crate::identity::{LiveSessionIdentity, OperationKind, OperationRef, SessionRef};
pub use crate::ingress::{
    ADMISSION_BYTE_LIMIT, ADMISSION_PACKET_LIMIT, AdmissionBudget, AdmissionGate, AdmissionLease,
    AdmissionPacketError, AdmittedLedgerPacket, BackpressureOutcome, BackpressureReason,
    RouteEntry, RoutingSnapshot,
};
pub use crate::io::{
    DurabilityCompletion, DurabilityOutcome, PersistNode, ReadCompletion, ReadOutcome,
    ReadPriority, ReadRequest, StoredObjectKind, WriteBatch, WriteCompletion, WriteOutcome,
};
pub use crate::peer::{
    LedgerDataRequest, LedgerNodeRequest, PeerAvailabilitySnapshot, PeerRequest,
    PeerTargetCapability,
};
pub use crate::phase::{SyncPhase, TransitionError, TransitionFact, phase_transition};
pub use crate::plan::{
    DEFAULT_MAX_ACQUIRE_TIMEOUTS, LedgerTreePlanEngine, MAX_NEW_READS_PER_PASS,
    MAX_PACKETS_FED_PER_TURN, MAX_PENDING_READS, MAX_RETAINED_NETWORK_FRONTIER,
    MAX_TIMEOUT_REPROBES, MAX_TURNS_PER_EVENT, NullPlanSeed, NullResident, PlanDurabilityOutcome,
    PlanNetworkApply, PlanNetworkNeed, PlanReadApply, PlanReadNeed, PlanReadOutcome, PlanSeed,
    PlanStepOutcome, PlanTimeout, PlanTurn, PlanWriteOutcome, ScriptedEngine, ScriptedStep,
    SessionMailbox, SessionPersistence, SessionPlan, TreeEngine, TurnContext,
};
pub use crate::port::{
    CancellationPort, CoordinatorPorts, HandoffPort, LedgerRequestPort, PhasePort, ReadPort,
    TimerPort, WritePort,
};
pub use crate::runner::{
    BudgetState, CoordinatorRunner, CoordinatorSession, CoordinatorState, RunnerSessionSnapshot,
    RunnerSnapshot, RunnerStats,
};
pub use crate::session::{
    CancelReason, FailureReason, SessionOutcome, SessionPhase, session_phase_transition,
};
pub use crate::shadow::{
    DisagreementKind, ReferenceDecision, ShadowConfig, ShadowEventTag, ShadowObservation,
    ShadowOutcome, ShadowRunner, ShadowSnapshot,
};
pub use crate::snapshot::CoordinatorSnapshot;
pub use crate::target::{AcquireReason, LedgerIdentity, LedgerTarget};
pub use crate::timer::{TimerKind, TimerRequest};
pub use ledger::TreePlanId;
