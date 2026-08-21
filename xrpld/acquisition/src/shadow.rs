//! Coordinator shadow mode and observability.
//!
//! The [`ShadowRunner`] consumes the same typed [`AcquisitionEvent`] stream the
//! production coordinator consumes (M4+), but it is strictly read-only: it
//! derives what the coordinator rules *would* decide and compares those derived
//! decisions against reference facts observed from the current production
//! implementation (the pre-migration registry/actor state). It never issues
//! production effects, never touches a port, and cannot mutate a live session:
//! its only outputs are immutable [`ShadowObservation`]s and a bounded
//! [`ShadowSnapshot`] for tracing, CLI, and metrics.
//!
//! Ownership rules:
//!
//! * The runner is driven serially by its caller (one strand/thread), the same
//!   way the coordinator will be driven. It holds only its own mirror state;
//!   [`SessionRef`]s are comparison keys, never handles into production.
//! * When [`ShadowConfig::enabled`] is false the runner is a no-op, so baseline
//!   behavior is unchanged with shadow mode disabled.
//! * Shadow mode is never a second mutating authority: this module has no port
//!   types and no method returns an [`crate::AcquisitionEffect`].

use std::collections::{BTreeMap, VecDeque};

use basics::base_uint::Uint256;

use crate::effect::AcquisitionEffect;
use crate::event::{AcquisitionEvent, ConsensusTarget};
use crate::id::{IdCounter, PlanEpoch, RunEpoch, StoreGeneration};
use crate::identity::{OperationKind, OperationRef, SessionRef};
use crate::ingress::AdmittedLedgerPacket;
use crate::io::{DurabilityOutcome, ReadOutcome, WriteOutcome};
use crate::peer::PeerAvailabilitySnapshot;
use crate::phase::{SyncPhase, TransitionFact, phase_transition};
use crate::session::{CancelReason, FailureReason, SessionOutcome, SessionPhase};
use crate::target::{AcquireReason, LedgerIdentity, LedgerTarget};
use crate::timer::TimerKind;

/// Shadow-mode configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShadowConfig {
    /// When false, [`ShadowRunner`] records and compares nothing.
    pub enabled: bool,
    /// Maximum mirror sessions retained for comparison. The oldest tracked
    /// session is evicted first.
    pub max_mirror_sessions: usize,
    /// Maximum buffered observations. The oldest observation is evicted first;
    /// counters still account for every observation.
    pub max_buffered_observations: usize,
}

impl Default for ShadowConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_mirror_sessions: 256,
            max_buffered_observations: 1024,
        }
    }
}

impl ShadowConfig {
    /// A configuration that performs no recording or comparison.
    pub const fn disabled() -> Self {
        Self {
            enabled: false,
            max_mirror_sessions: 0,
            max_buffered_observations: 0,
        }
    }
}

/// A lightweight terminal-outcome label for shadow comparison. The full
/// [`SessionOutcome`] carries a `Ledger` payload that tracing does not need.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadowOutcome {
    /// The ledger is durable and handed off.
    Durable,
    /// The session failed; no normal adoptable ledger was produced.
    Failed { reason: FailureReason },
    /// The session was cancelled.
    Cancelled { reason: CancelReason },
}

impl ShadowOutcome {
    /// A stable label for tracing.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Durable => "durable",
            Self::Failed { reason } => match reason {
                FailureReason::InvalidTreePlan => "failed_invalid_tree_plan",
                FailureReason::InvalidPacketData => "failed_invalid_packet_data",
                FailureReason::ReadFailure => "failed_read_failure",
                FailureReason::WriteFailure => "failed_write_failure",
                FailureReason::DurabilityFenceFailed => "failed_durability_fence",
                FailureReason::NoUsablePeers => "failed_no_usable_peers",
                FailureReason::AcquisitionTimeout => "failed_acquisition_timeout",
                FailureReason::NodeStoreFull => "failed_node_store_full",
            },
            Self::Cancelled { reason } => match reason {
                CancelReason::Replaced => "cancelled_replaced",
                CancelReason::StoreRotated => "cancelled_store_rotated",
                CancelReason::Shutdown => "cancelled_shutdown",
                CancelReason::LclInstalled => "cancelled_lcl_installed",
                CancelReason::Explicit => "cancelled_explicit",
                CancelReason::IdleExpired => "cancelled_idle_expired",
            },
        }
    }
}

impl From<&SessionOutcome> for ShadowOutcome {
    fn from(outcome: &SessionOutcome) -> Self {
        match outcome {
            SessionOutcome::Durable(_) => Self::Durable,
            SessionOutcome::Failed { reason, .. } => Self::Failed { reason: *reason },
            SessionOutcome::Cancelled { reason, .. } => Self::Cancelled { reason: *reason },
        }
    }
}

/// How one shadow comparison resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisagreementKind {
    /// Derived and reference decisions agree.
    Match,
    /// The service phase differs between the coordinator rules and the
    /// production reference.
    PhaseMismatch,
    /// The terminal outcome differs.
    OutcomeMismatch,
    /// The tracked target hash differs.
    TargetMismatch,
    /// The reference reported a session the mirror does not track (or the
    /// mirror tracks a session the reference never reports).
    SessionPresenceMismatch,
    /// The coordinator rules reject an event or fact the production system
    /// accepted. This is a target-architecture divergence to categorize and
    /// adjudicate, not to paper over.
    DerivedRejected,
    /// A late event for a terminal or unknown session; harmless and counted.
    StaleEvent,
}

impl DisagreementKind {
    /// True when this category is not a clean match.
    pub const fn is_disagreement(self) -> bool {
        !matches!(self, Self::Match)
    }

    /// A stable label for tracing.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Match => "match",
            Self::PhaseMismatch => "phase_mismatch",
            Self::OutcomeMismatch => "outcome_mismatch",
            Self::TargetMismatch => "target_mismatch",
            Self::SessionPresenceMismatch => "session_presence_mismatch",
            Self::DerivedRejected => "derived_rejected",
            Self::StaleEvent => "stale_event",
        }
    }
}

/// A stable tag identifying which event type was shadowed, for tracing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadowEventTag {
    StartupMode,
    Connectivity,
    AcquireRequested,
    ValidationTarget,
    PacketAdmitted,
    ReadCompleted,
    WriteCompleted,
    DurabilityFenced,
    DurableHandoffAcknowledged,
    DurableHandoffRejected,
    TimerFired,
    ConsensusTarget,
    PreferredLclDivergence,
    PreferredLclReconciled,
    BlockedWithNoTarget,
    LclInstalled,
    PublicationCommitted,
    StoreRotated,
    FetchPackAvailable,
    Heartbeat,
    RegistrySweep,
    Shutdown,
}

impl ShadowEventTag {
    /// A stable label for tracing.
    pub const fn label(self) -> &'static str {
        match self {
            Self::StartupMode => "startup_mode",
            Self::Connectivity => "connectivity",
            Self::AcquireRequested => "acquire_requested",
            Self::ValidationTarget => "validation_target",
            Self::PacketAdmitted => "packet_admitted",
            Self::ReadCompleted => "read_completed",
            Self::WriteCompleted => "write_completed",
            Self::DurabilityFenced => "durability_fenced",
            Self::DurableHandoffAcknowledged => "durable_handoff_acknowledged",
            Self::DurableHandoffRejected => "durable_handoff_rejected",
            Self::TimerFired => "timer_fired",
            Self::ConsensusTarget => "consensus_target",
            Self::PreferredLclDivergence => "preferred_lcl_divergence",
            Self::PreferredLclReconciled => "preferred_lcl_reconciled",
            Self::BlockedWithNoTarget => "blocked_with_no_target",
            Self::LclInstalled => "lcl_installed",
            Self::PublicationCommitted => "publication_committed",
            Self::StoreRotated => "store_rotated",
            Self::FetchPackAvailable => "fetch_pack_available",
            Self::Heartbeat => "heartbeat",
            Self::RegistrySweep => "registry_sweep",
            Self::Shutdown => "shutdown",
        }
    }
}

impl From<&AcquisitionEvent> for ShadowEventTag {
    fn from(event: &AcquisitionEvent) -> Self {
        match event {
            AcquisitionEvent::StartupMode { .. } => Self::StartupMode,
            AcquisitionEvent::Connectivity(_) => Self::Connectivity,
            AcquisitionEvent::AcquireRequested { .. } => Self::AcquireRequested,
            AcquisitionEvent::ValidationTarget(_) => Self::ValidationTarget,
            AcquisitionEvent::PacketAdmitted(_) => Self::PacketAdmitted,
            AcquisitionEvent::ReadCompleted(_) => Self::ReadCompleted,
            AcquisitionEvent::WriteCompleted(_) => Self::WriteCompleted,
            AcquisitionEvent::DurabilityFenced(_) => Self::DurabilityFenced,
            AcquisitionEvent::DurableHandoffAcknowledged(_) => Self::DurableHandoffAcknowledged,
            AcquisitionEvent::DurableHandoffRejected { .. } => Self::DurableHandoffRejected,
            AcquisitionEvent::TimerFired { .. } => Self::TimerFired,
            AcquisitionEvent::ConsensusTarget(_) => Self::ConsensusTarget,
            AcquisitionEvent::PreferredLclDivergence { .. } => Self::PreferredLclDivergence,
            AcquisitionEvent::PreferredLclReconciled { .. } => Self::PreferredLclReconciled,
            AcquisitionEvent::BlockedWithNoTarget => Self::BlockedWithNoTarget,
            AcquisitionEvent::LclInstalled(_) => Self::LclInstalled,
            AcquisitionEvent::PublicationCommitted { .. } => Self::PublicationCommitted,
            AcquisitionEvent::StoreRotated(_) => Self::StoreRotated,
            AcquisitionEvent::FetchPackAvailable => Self::FetchPackAvailable,
            AcquisitionEvent::Heartbeat => Self::Heartbeat,
            AcquisitionEvent::RegistrySweep => Self::RegistrySweep,
            AcquisitionEvent::Shutdown => Self::Shutdown,
        }
    }
}

/// A production-observed decision fed to the shadow for comparison.
///
/// The app-side shadow probe reads the current registry/actor state and reports
/// it here; the runner compares it with what the coordinator rules derive.
/// `phase` and `target_hash` are optional so a probe may report only what it can
/// observe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReferenceDecision {
    session: SessionRef,
    phase: Option<SyncPhase>,
    outcome: Option<ShadowOutcome>,
    target_hash: Option<Uint256>,
    queue_depth: usize,
}

impl ReferenceDecision {
    /// Builds a reference decision.
    pub const fn new(
        session: SessionRef,
        phase: Option<SyncPhase>,
        outcome: Option<ShadowOutcome>,
        target_hash: Option<Uint256>,
        queue_depth: usize,
    ) -> Self {
        Self {
            session,
            phase,
            outcome,
            target_hash,
            queue_depth,
        }
    }

    /// The session the production system is reporting on.
    pub const fn session(self) -> SessionRef {
        self.session
    }

    /// The production-observed service phase, if observable.
    pub const fn phase(self) -> Option<SyncPhase> {
        self.phase
    }

    /// The production-observed terminal outcome, if any.
    pub const fn outcome(self) -> Option<ShadowOutcome> {
        self.outcome
    }

    /// The production-observed target hash, if observable.
    pub const fn target_hash(self) -> Option<Uint256> {
        self.target_hash
    }

    /// The production-observed queue depth.
    pub const fn queue_depth(self) -> usize {
        self.queue_depth
    }
}

/// One categorized observation from shadow derivation or reference comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShadowObservation {
    event: ShadowEventTag,
    session: Option<SessionRef>,
    kind: DisagreementKind,
    derived_phase: Option<SyncPhase>,
    derived_outcome: Option<ShadowOutcome>,
    reference_phase: Option<SyncPhase>,
    reference_outcome: Option<ShadowOutcome>,
    reason: Option<TransitionFact>,
    queue_depth: usize,
}

impl ShadowObservation {
    /// The event type that produced this observation.
    pub const fn event(&self) -> ShadowEventTag {
        self.event
    }

    /// The session the observation concerns, when applicable.
    pub const fn session(&self) -> Option<SessionRef> {
        self.session
    }

    /// The categorized outcome of the comparison.
    pub const fn kind(&self) -> DisagreementKind {
        self.kind
    }

    /// The service phase the coordinator rules derive.
    pub const fn derived_phase(&self) -> Option<&SyncPhase> {
        self.derived_phase.as_ref()
    }

    /// The terminal outcome the coordinator rules derive.
    pub const fn derived_outcome(&self) -> Option<ShadowOutcome> {
        self.derived_outcome
    }

    /// The production-observed phase, when this is a reference comparison.
    pub const fn reference_phase(&self) -> Option<&SyncPhase> {
        self.reference_phase.as_ref()
    }

    /// The production-observed outcome, when this is a reference comparison.
    pub const fn reference_outcome(&self) -> Option<ShadowOutcome> {
        self.reference_outcome
    }

    /// The fact that motivated the derived phase transition, when one fired.
    pub const fn reason(&self) -> Option<TransitionFact> {
        self.reason
    }

    /// The observed queue depth at the time of the observation.
    pub const fn queue_depth(&self) -> usize {
        self.queue_depth
    }
}

/// An immutable, bounded snapshot of the shadow mirror for RPC/CLI/metrics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShadowSnapshot {
    enabled: bool,
    run_epoch: RunEpoch,
    phase: SyncPhase,
    mirror_sessions: usize,
    active_by_reason: BTreeMap<AcquireReason, usize>,
    queue_depth: usize,
    matches: u64,
    disagreements: u64,
    stale_events: u64,
    observations_buffered: usize,
}

impl ShadowSnapshot {
    /// The shadow phase, as derived by the coordinator rules.
    pub const fn phase(&self) -> &SyncPhase {
        &self.phase
    }

    /// The number of sessions in the shadow mirror.
    pub const fn mirror_sessions(&self) -> usize {
        self.mirror_sessions
    }

    /// Active (non-terminal) mirror sessions grouped by acquisition reason.
    pub fn active_by_reason(&self) -> &BTreeMap<AcquireReason, usize> {
        &self.active_by_reason
    }

    /// Total categorized matches.
    pub const fn matches(&self) -> u64 {
        self.matches
    }

    /// Total categorized disagreements (excluding stale events).
    pub const fn disagreements(&self) -> u64 {
        self.disagreements
    }

    /// Total stale events observed.
    pub const fn stale_events(&self) -> u64 {
        self.stale_events
    }

    /// True when shadow mode is enabled for this run.
    pub const fn enabled(&self) -> bool {
        self.enabled
    }
}

/// A serialized, read-only mirror of what the coordinator rules would decide.
///
/// The caller drives this on a single strand/thread. All state is owned here;
/// the only outputs are observations and snapshots. No method on this type
/// returns an effect or touches a port.
#[derive(Debug)]
pub struct ShadowRunner {
    config: ShadowConfig,
    phase: SyncPhase,
    run_epoch: RunEpoch,
    session_counter: IdCounter,
    store_generation: StoreGeneration,
    has_usable_peers: bool,
    latest_consensus_target: Option<LedgerTarget>,
    mirror: BTreeMap<SessionRef, MirrorSession>,
    queue_depth: usize,
    observations: VecDeque<ShadowObservation>,
    matches: u64,
    disagreements: u64,
    stale_events: u64,
}

/// The mirror state for one session.
#[derive(Debug)]
struct MirrorSession {
    session: SessionRef,
    target: LedgerTarget,
    phase: SessionPhase,
    reason: AcquireReason,
    queue_depth: usize,
    terminal: Option<ShadowOutcome>,
    expiry_sweep_eligible: bool,
    /// Exact expiry operation most recently observed from the production
    /// coordinator's `ArmTimer` effect. A touch replaces this identity, so a
    /// late earlier wakeup cannot make the mirror sweep-eligible.
    pending_expiry_timer: Option<OperationRef>,
    /// Conservative mirror of a queued/running JtLedgerData-equivalent local
    /// traversal. The event stream does not expose individual plan turns, so a
    /// packet/read retains this ownership until a persistence or terminal
    /// boundary proves it has ended.
    local_scan_in_flight: bool,
}

impl ShadowRunner {
    /// Builds a shadow runner for a run epoch.
    pub const fn new(config: ShadowConfig, run_epoch: RunEpoch) -> Self {
        Self {
            config,
            phase: SyncPhase::Disconnected,
            run_epoch,
            session_counter: IdCounter::new(),
            store_generation: StoreGeneration::new(1),
            has_usable_peers: false,
            latest_consensus_target: None,
            mirror: BTreeMap::new(),
            queue_depth: 0,
            observations: VecDeque::new(),
            matches: 0,
            disagreements: 0,
            stale_events: 0,
        }
    }

    /// True when shadow mode records and compares.
    pub const fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Observe coordinator effects that carry identities required for exact
    /// shadow matching. This remains read-only with respect to production: a
    /// `SessionStarted` effect replaces the mirror's provisional identity with
    /// the exact production identity, and an expiry arm records only its latest
    /// operation in that private mirror.
    pub fn observe_effects(&mut self, effects: &[AcquisitionEffect]) {
        if !self.config.enabled {
            return;
        }
        for effect in effects {
            match effect {
                AcquisitionEffect::SessionStarted(session) => {
                    self.adopt_production_session(*session);
                }
                AcquisitionEffect::ArmTimer(request)
                    if request.timer() == TimerKind::SessionExpiry =>
                {
                    let operation = request.operation();
                    if let Some(mirror) = self.mirror.get_mut(&operation.session())
                        && matches!(mirror.phase, SessionPhase::Active | SessionPhase::Dormant)
                    {
                        mirror.pending_expiry_timer = Some(operation);
                        mirror.expiry_sweep_eligible = false;
                    }
                }
                _ => {}
            }
        }
    }

    fn adopt_production_session(&mut self, production: SessionRef) {
        if self.mirror.contains_key(&production) {
            return;
        }
        let provisional = self
            .mirror
            .iter()
            .rev()
            .find(|(_, mirror)| {
                !mirror.phase.is_terminal() && mirror.target.hash() == production.target_hash()
            })
            .map(|(session, _)| *session);
        let Some(provisional) = provisional else {
            return;
        };
        let Some(mut mirror) = self.mirror.remove(&provisional) else {
            return;
        };
        mirror.session = production;
        self.mirror.insert(production, mirror);
    }

    /// Derives what the coordinator rules would do for one event.
    ///
    /// Strictly read-only with respect to production: never issues effects.
    /// Returns the observations the caller should trace.
    pub fn record(&mut self, event: &AcquisitionEvent) -> Vec<ShadowObservation> {
        if !self.config.enabled {
            return Vec::new();
        }
        let tag = ShadowEventTag::from(event);
        let mut out = Vec::new();
        match event {
            AcquisitionEvent::StartupMode { phase } => {
                self.derive_startup_mode(tag, *phase, &mut out);
            }
            AcquisitionEvent::Connectivity(snapshot) => {
                self.derive_connectivity(tag, snapshot, &mut out);
            }
            AcquisitionEvent::AcquireRequested { target, reason } => {
                self.derive_acquire(tag, *target, *reason, false, false, &mut out);
            }
            AcquisitionEvent::ValidationTarget(target) => {
                self.derive_acquire(
                    tag,
                    *target,
                    AcquireReason::Consensus,
                    false,
                    true,
                    &mut out,
                );
            }
            AcquisitionEvent::PacketAdmitted(packet) => {
                self.derive_packet(tag, packet, &mut out);
            }
            AcquisitionEvent::ReadCompleted(completion) => {
                self.derive_read(
                    tag,
                    completion.operation().session(),
                    completion.operation().kind(),
                    completion.outcome(),
                    &mut out,
                );
            }
            AcquisitionEvent::WriteCompleted(completion) => {
                self.derive_write(
                    tag,
                    completion.operation().session(),
                    completion.outcome(),
                    &mut out,
                );
            }
            AcquisitionEvent::DurabilityFenced(completion) => {
                self.derive_durability(
                    tag,
                    completion.operation().session(),
                    completion.outcome(),
                    &mut out,
                );
            }
            AcquisitionEvent::DurableHandoffAcknowledged(acknowledgement) => {
                self.derive_ack(tag, acknowledgement.session(), &mut out);
            }
            AcquisitionEvent::DurableHandoffRejected {
                handoff: _,
                session,
                reason: _,
            } => self.derive_handoff_rejected(tag, *session, &mut out),
            AcquisitionEvent::TimerFired { operation, timer } => {
                self.derive_timer(tag, *operation, *timer, &mut out);
            }
            AcquisitionEvent::ConsensusTarget(target) => {
                self.derive_consensus(tag, *target, &mut out);
            }
            AcquisitionEvent::PreferredLclDivergence { target } => {
                self.derive_preferred_lcl_divergence(tag, *target, &mut out);
            }
            AcquisitionEvent::PreferredLclReconciled { lcl } => {
                self.latest_consensus_target = None;
                let fact = TransitionFact::PreferredLclReconciled { lcl: *lcl };
                let (derived, reason, kind) = self.apply_fact(Some(fact));
                self.push(
                    tag, None, kind, derived, None, None, None, reason, None, &mut out,
                );
            }
            AcquisitionEvent::BlockedWithNoTarget => {
                self.derive_blocked_with_no_target(tag, &mut out)
            }
            AcquisitionEvent::LclInstalled(identity) => {
                self.derive_lcl_installed(tag, *identity, &mut out);
            }
            AcquisitionEvent::PublicationCommitted { identity, fresh } => {
                self.derive_publication(tag, *identity, *fresh, &mut out);
            }
            AcquisitionEvent::StoreRotated(generation) => {
                self.derive_store_rotation(tag, *generation, &mut out);
            }
            AcquisitionEvent::FetchPackAvailable => self.derive_fetch_pack(tag, &mut out),
            AcquisitionEvent::Heartbeat => self.derive_heartbeat(tag, &mut out),
            AcquisitionEvent::RegistrySweep => self.derive_registry_sweep(tag, &mut out),
            AcquisitionEvent::Shutdown => self.derive_shutdown(tag, &mut out),
        }
        if self.promote_latest_viable_syncing_anchor() {
            self.push(
                tag,
                None,
                DisagreementKind::Match,
                Some(self.phase),
                None,
                None,
                None,
                None,
                None,
                &mut out,
            );
        }
        out
    }

    /// Compares a production-observed decision against the shadow mirror.
    pub fn compare_reference(&mut self, reference: &ReferenceDecision) -> Vec<ShadowObservation> {
        if !self.config.enabled {
            return Vec::new();
        }
        let mut out = Vec::new();
        self.compare_reference_inner(reference, &mut out);
        out
    }

    /// Drains buffered observations for tracing/metrics.
    pub fn drain_observations(&mut self) -> Vec<ShadowObservation> {
        self.observations.drain(..).collect()
    }

    /// An immutable bounded snapshot of the mirror.
    pub fn snapshot(&self) -> ShadowSnapshot {
        let mut active_by_reason: BTreeMap<AcquireReason, usize> = BTreeMap::new();
        for mirror in self.mirror.values() {
            if !mirror.phase.is_terminal() && mirror.phase != SessionPhase::Dormant {
                *active_by_reason.entry(mirror.reason).or_insert(0) += 1;
            }
        }
        ShadowSnapshot {
            enabled: self.config.enabled,
            run_epoch: self.run_epoch,
            phase: self.phase,
            mirror_sessions: self.mirror.len(),
            active_by_reason,
            queue_depth: self.queue_depth,
            matches: self.matches,
            disagreements: self.disagreements,
            stale_events: self.stale_events,
            observations_buffered: self.observations.len(),
        }
    }

    fn derive_connectivity(
        &mut self,
        tag: ShadowEventTag,
        snapshot: &PeerAvailabilitySnapshot,
        out: &mut Vec<ShadowObservation>,
    ) {
        let has_usable_peers = snapshot.has_usable_peer_capability();
        let fact = if has_usable_peers {
            (self.phase == SyncPhase::Disconnected)
                .then_some(TransitionFact::PeerCapabilityAvailable)
        } else {
            (self.phase != SyncPhase::Disconnected).then_some(TransitionFact::PeerCapabilityLost)
        };
        let (derived, reason, kind) = self.apply_fact(fact);
        self.has_usable_peers = has_usable_peers;
        self.push(
            tag, None, kind, derived, None, None, None, reason, None, out,
        );
    }

    fn derive_acquire(
        &mut self,
        tag: ShadowEventTag,
        target: LedgerTarget,
        reason: AcquireReason,
        preferred_target: bool,
        phase_neutral: bool,
        out: &mut Vec<ShadowObservation>,
    ) {
        if !self.has_usable_peers {
            self.push(
                tag,
                None,
                DisagreementKind::DerivedRejected,
                Some(self.phase),
                None,
                None,
                None,
                Some(TransitionFact::TargetRequired { target }),
                Some(self.queue_depth),
                out,
            );
            return;
        }
        let preserve_installed_lcl = matches!(
            self.phase,
            SyncPhase::Tracking { .. } | SyncPhase::Full { .. }
        );
        if let Some((session, promotion)) = self
            .mirror
            .iter()
            .find(|(_, mirror)| {
                !mirror.phase.is_terminal()
                    && mirror.target.hash() == target.hash()
                    && (mirror.target == target
                        || (mirror.target.sequence().is_none()
                            && target.sequence().is_some()
                            && mirror.queue_depth == 0))
            })
            .map(|(session, mirror)| (*session, mirror.target != target))
        {
            if promotion && let Some(mirror) = self.mirror.get_mut(&session) {
                mirror.target = target;
            }
            if preferred_target {
                self.activate_latest_consensus(session);
            }
            let (derived, transition_reason, kind) = if phase_neutral || preserve_installed_lcl {
                (Some(self.phase), None, DisagreementKind::Match)
            } else {
                self.apply_fact(Some(TransitionFact::TargetRequired { target }))
            };
            self.push(
                tag,
                Some(session),
                kind,
                derived,
                None,
                None,
                None,
                transition_reason,
                Some(0),
                out,
            );
            return;
        }
        let session = self.mint_session(target.hash());
        self.mirror.insert(
            session,
            MirrorSession {
                session,
                target,
                phase: SessionPhase::Active,
                reason,
                queue_depth: 0,
                terminal: None,
                expiry_sweep_eligible: false,
                pending_expiry_timer: None,
                // Every newly admitted production session starts with an
                // asynchronous local header probe.
                local_scan_in_flight: true,
            },
        );
        if preferred_target {
            self.activate_latest_consensus(session);
        }
        let (derived, reason, kind) = if phase_neutral || preserve_installed_lcl {
            (Some(self.phase), None, DisagreementKind::Match)
        } else {
            self.apply_fact(Some(TransitionFact::TargetRequired { target }))
        };
        self.push(
            tag,
            Some(session),
            kind,
            derived,
            None,
            None,
            None,
            reason,
            Some(0),
            out,
        );
    }

    fn derive_packet(
        &mut self,
        tag: ShadowEventTag,
        packet: &AdmittedLedgerPacket,
        out: &mut Vec<ShadowObservation>,
    ) {
        let session = packet.lease().session();
        let queue_depth = self.queue_depth;
        let Some(mirror) = self.mirror.get_mut(&session) else {
            // A packet for a session the coordinator rules do not track is
            // stale or foreign; harmless and counted.
            self.push(
                tag,
                Some(session),
                DisagreementKind::StaleEvent,
                None,
                None,
                None,
                None,
                None,
                Some(queue_depth),
                out,
            );
            return;
        };
        if mirror.phase != SessionPhase::Active {
            self.push(
                tag,
                Some(session),
                DisagreementKind::StaleEvent,
                None,
                None,
                None,
                None,
                None,
                Some(queue_depth),
                out,
            );
            return;
        }
        mirror.queue_depth += 1;
        mirror.local_scan_in_flight = true;
        self.queue_depth += 1;
        let session_queue_depth = mirror.queue_depth;
        let derived_phase = Some(self.phase);
        self.push(
            tag,
            Some(session),
            DisagreementKind::Match,
            derived_phase,
            None,
            None,
            None,
            None,
            Some(session_queue_depth),
            out,
        );
    }

    fn derive_read(
        &mut self,
        tag: ShadowEventTag,
        session: SessionRef,
        operation_kind: OperationKind,
        outcome: &ReadOutcome,
        out: &mut Vec<ShadowObservation>,
    ) {
        let Some(mirror) = self.mirror.get_mut(&session) else {
            self.push(
                tag,
                Some(session),
                DisagreementKind::StaleEvent,
                None,
                None,
                None,
                None,
                None,
                Some(self.queue_depth),
                out,
            );
            return;
        };
        if mirror.phase != SessionPhase::Active
            || matches!(outcome, ReadOutcome::Stale | ReadOutcome::Cancelled)
        {
            self.push(
                tag,
                Some(session),
                DisagreementKind::StaleEvent,
                None,
                None,
                None,
                None,
                None,
                Some(self.queue_depth),
                out,
            );
            return;
        }
        // A header miss returns the session to peer acquisition. Ordinary
        // traversal reads conservatively retain local-scan ownership because
        // their completion may synchronously schedule the next 512-read batch.
        if operation_kind == OperationKind::HeaderRead {
            mirror.local_scan_in_flight = matches!(outcome, ReadOutcome::Settled { node: Some(_) });
        } else {
            mirror.local_scan_in_flight = true;
        }
        let queue_depth = mirror.queue_depth;
        self.push(
            tag,
            Some(session),
            DisagreementKind::Match,
            Some(self.phase),
            None,
            None,
            None,
            None,
            Some(queue_depth),
            out,
        );
    }

    fn derive_write(
        &mut self,
        tag: ShadowEventTag,
        session: SessionRef,
        outcome: WriteOutcome,
        out: &mut Vec<ShadowObservation>,
    ) {
        let Some(mirror) = self.mirror.get_mut(&session) else {
            self.push(
                tag,
                Some(session),
                DisagreementKind::StaleEvent,
                None,
                None,
                None,
                None,
                None,
                Some(self.queue_depth),
                out,
            );
            return;
        };
        if mirror.phase.is_terminal()
            || mirror.phase == SessionPhase::Dormant
            || matches!(outcome, WriteOutcome::Stale | WriteOutcome::Cancelled)
        {
            self.push(
                tag,
                Some(session),
                DisagreementKind::StaleEvent,
                None,
                None,
                None,
                None,
                None,
                Some(self.queue_depth),
                out,
            );
            return;
        }
        let terminal = match outcome {
            WriteOutcome::Accepted if mirror.phase == SessionPhase::Active => {
                mirror.local_scan_in_flight = false;
                mirror.phase = SessionPhase::Persisting;
                None
            }
            WriteOutcome::Failed if !mirror.phase.is_terminal() => {
                let terminal = Some(ShadowOutcome::Failed {
                    reason: FailureReason::WriteFailure,
                });
                mirror.phase = SessionPhase::Failed {
                    reason: FailureReason::WriteFailure,
                };
                mirror.terminal = terminal;
                mirror.local_scan_in_flight = false;
                terminal
            }
            _ => {
                self.push(
                    tag,
                    Some(session),
                    DisagreementKind::DerivedRejected,
                    Some(self.phase),
                    None,
                    None,
                    None,
                    None,
                    Some(self.queue_depth),
                    out,
                );
                return;
            }
        };
        let queue_depth = mirror.queue_depth;
        self.push(
            tag,
            Some(session),
            DisagreementKind::Match,
            Some(self.phase),
            terminal,
            None,
            None,
            None,
            Some(queue_depth),
            out,
        );
    }

    fn derive_durability(
        &mut self,
        tag: ShadowEventTag,
        session: SessionRef,
        outcome: DurabilityOutcome,
        out: &mut Vec<ShadowObservation>,
    ) {
        let Some(mirror) = self.mirror.get_mut(&session) else {
            self.push(
                tag,
                Some(session),
                DisagreementKind::StaleEvent,
                None,
                None,
                None,
                None,
                None,
                Some(self.queue_depth),
                out,
            );
            return;
        };
        if mirror.phase.is_terminal()
            || mirror.phase == SessionPhase::Dormant
            || matches!(outcome, DurabilityOutcome::Stale)
        {
            self.push(
                tag,
                Some(session),
                DisagreementKind::StaleEvent,
                None,
                None,
                None,
                None,
                None,
                Some(self.queue_depth),
                out,
            );
            return;
        }
        let terminal = match outcome {
            DurabilityOutcome::Passed if mirror.phase == SessionPhase::Persisting => {
                mirror.phase = SessionPhase::DurablePending;
                None
            }
            DurabilityOutcome::Failed if !mirror.phase.is_terminal() => {
                let terminal = Some(ShadowOutcome::Failed {
                    reason: FailureReason::DurabilityFenceFailed,
                });
                mirror.phase = SessionPhase::Failed {
                    reason: FailureReason::DurabilityFenceFailed,
                };
                mirror.terminal = terminal;
                terminal
            }
            _ => {
                self.push(
                    tag,
                    Some(session),
                    DisagreementKind::DerivedRejected,
                    Some(self.phase),
                    None,
                    None,
                    None,
                    None,
                    Some(self.queue_depth),
                    out,
                );
                return;
            }
        };
        let queue_depth = mirror.queue_depth;
        self.push(
            tag,
            Some(session),
            DisagreementKind::Match,
            Some(self.phase),
            terminal,
            None,
            None,
            None,
            Some(queue_depth),
            out,
        );
    }

    fn derive_ack(
        &mut self,
        tag: ShadowEventTag,
        session: SessionRef,
        out: &mut Vec<ShadowObservation>,
    ) {
        let Some(mirror) = self.mirror.get_mut(&session) else {
            self.push(
                tag,
                Some(session),
                DisagreementKind::StaleEvent,
                None,
                None,
                None,
                None,
                None,
                Some(self.queue_depth),
                out,
            );
            return;
        };
        let (kind, terminal) = if mirror.phase == SessionPhase::DurablePending {
            let terminal = Some(ShadowOutcome::Durable);
            mirror.phase = SessionPhase::Complete;
            mirror.terminal = terminal;
            (DisagreementKind::Match, terminal)
        } else if mirror.phase.is_terminal() {
            (DisagreementKind::StaleEvent, None)
        } else {
            (DisagreementKind::DerivedRejected, None)
        };
        let queue_depth = mirror.queue_depth;
        let derived_phase = Some(self.phase);
        self.push(
            tag,
            Some(session),
            kind,
            derived_phase,
            terminal,
            None,
            None,
            None,
            Some(queue_depth),
            out,
        );
    }

    /// A rejected durable delivery leaves the session in `DurablePending`.
    /// The production coordinator arms one exact retry timer; shadow records
    /// the scheduling fact without mutating its mirror or predicting a send.
    fn derive_handoff_rejected(
        &mut self,
        tag: ShadowEventTag,
        session: SessionRef,
        out: &mut Vec<ShadowObservation>,
    ) {
        let kind = match self.mirror.get(&session) {
            Some(mirror) if mirror.phase == SessionPhase::DurablePending => DisagreementKind::Match,
            Some(_) => DisagreementKind::StaleEvent,
            None => DisagreementKind::StaleEvent,
        };
        let derived_phase = Some(self.phase);
        self.push(
            tag,
            Some(session),
            kind,
            derived_phase,
            None,
            None,
            None,
            None,
            None,
            out,
        );
    }

    fn derive_timer(
        &mut self,
        tag: ShadowEventTag,
        operation: OperationRef,
        timer: TimerKind,
        out: &mut Vec<ShadowObservation>,
    ) {
        let session = operation.session();
        let Some(mirror) = self.mirror.get(&session) else {
            self.push(
                tag,
                Some(session),
                DisagreementKind::StaleEvent,
                None,
                None,
                None,
                None,
                None,
                Some(self.queue_depth),
                out,
            );
            return;
        };
        if timer == TimerKind::SessionExpiry {
            let exact = mirror
                .pending_expiry_timer
                .is_some_and(|expected| expected.is_expected_for(&operation));
            if !matches!(mirror.phase, SessionPhase::Active | SessionPhase::Dormant) || !exact {
                self.push(
                    tag,
                    Some(session),
                    DisagreementKind::StaleEvent,
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some(self.queue_depth),
                    out,
                );
                return;
            }
            let mirror = self
                .mirror
                .get_mut(&session)
                .expect("validated live shadow session");
            mirror.pending_expiry_timer = None;
            mirror.expiry_sweep_eligible = true;
            let queue_depth = mirror.queue_depth;
            self.push(
                tag,
                Some(session),
                DisagreementKind::Match,
                Some(self.phase),
                None,
                None,
                None,
                None,
                Some(queue_depth),
                out,
            );
            return;
        }
        if mirror.phase != SessionPhase::Active && mirror.phase != SessionPhase::DurablePending {
            self.push(
                tag,
                Some(session),
                DisagreementKind::StaleEvent,
                None,
                None,
                None,
                None,
                None,
                Some(self.queue_depth),
                out,
            );
        } else {
            let queue_depth = {
                let mirror = self
                    .mirror
                    .get_mut(&session)
                    .expect("validated live shadow session");
                mirror.queue_depth
            };
            self.push(
                tag,
                Some(session),
                DisagreementKind::Match,
                Some(self.phase),
                None,
                None,
                None,
                None,
                Some(queue_depth),
                out,
            );
        }
    }

    fn derive_registry_sweep(&mut self, tag: ShadowEventTag, out: &mut Vec<ShadowObservation>) {
        let expired = self
            .mirror
            .iter()
            .filter(|(_, mirror)| {
                mirror.expiry_sweep_eligible
                    && matches!(mirror.phase, SessionPhase::Active | SessionPhase::Dormant)
                    && !mirror.local_scan_in_flight
            })
            .map(|(session, _)| *session)
            .collect::<Vec<_>>();
        for session in expired {
            let mirror = self
                .mirror
                .get_mut(&session)
                .expect("selected mirror exists");
            let terminal = Some(ShadowOutcome::Cancelled {
                reason: CancelReason::IdleExpired,
            });
            mirror.phase = SessionPhase::Cancelled {
                reason: CancelReason::IdleExpired,
            };
            mirror.terminal = terminal;
            self.push(
                tag,
                Some(session),
                DisagreementKind::Match,
                Some(self.phase),
                terminal,
                None,
                None,
                None,
                None,
                out,
            );
        }
    }

    fn promote_latest_viable_syncing_anchor(&mut self) -> bool {
        let SyncPhase::Syncing { target: anchor } = self.phase else {
            return false;
        };
        let viable = |target: LedgerTarget, mirror: &BTreeMap<SessionRef, MirrorSession>| {
            mirror.values().any(|session| {
                !matches!(
                    session.phase,
                    SessionPhase::Failed { .. } | SessionPhase::Cancelled { .. }
                ) && session.target.hash() == target.hash()
            })
        };
        if viable(anchor, &self.mirror) {
            return false;
        }
        let Some(latest) = self.latest_consensus_target else {
            return false;
        };
        if latest.hash() == anchor.hash() || !viable(latest, &self.mirror) {
            return false;
        }
        self.phase = SyncPhase::Syncing { target: latest };
        true
    }

    fn derive_consensus(
        &mut self,
        tag: ShadowEventTag,
        target: ConsensusTarget,
        out: &mut Vec<ShadowObservation>,
    ) {
        self.latest_consensus_target = Some(target.target());
        self.derive_acquire(tag, target.target(), target.reason(), true, false, out);
    }

    fn derive_preferred_lcl_divergence(
        &mut self,
        tag: ShadowEventTag,
        target: LedgerTarget,
        out: &mut Vec<ShadowObservation>,
    ) {
        // Rippled consensusViewChange parity: a preferred-LCL divergence with a
        // concrete target demotes Connected/Tracking/Full -> Syncing without
        // minting a session. Acquisition demand arrives as a separate
        // AcquireRequested fact, so a resident-and-compatible switch (no fetch
        // needed) does not start a wasteful peer fetch.
        let fact = TransitionFact::PreferredLclDivergence { target };
        let (derived, reason, kind) = self.apply_fact(Some(fact));
        self.push(
            tag, None, kind, derived, None, None, None, reason, None, out,
        );
    }

    fn derive_blocked_with_no_target(
        &mut self,
        tag: ShadowEventTag,
        out: &mut Vec<ShadowObservation>,
    ) {
        // Quaxar-specific `no_consensus_positions` demotion: `Full -> Connected`
        // with no concrete target.
        let fact = TransitionFact::BlockedWithNoTarget;
        let (derived, reason, kind) = self.apply_fact(Some(fact));
        self.push(
            tag, None, kind, derived, None, None, None, reason, None, out,
        );
    }

    fn derive_lcl_installed(
        &mut self,
        tag: ShadowEventTag,
        identity: LedgerIdentity,
        out: &mut Vec<ShadowObservation>,
    ) {
        if self
            .latest_consensus_target
            .is_some_and(|target| target.hash() == identity.hash())
        {
            self.latest_consensus_target = None;
        }
        match self.phase {
            SyncPhase::Syncing { target } if target.hash() == identity.hash() => {
                let fact = TransitionFact::TargetInstalledAsLcl { lcl: identity };
                let (derived, reason, kind) = self.apply_fact(Some(fact));
                self.push(
                    tag, None, kind, derived, None, None, None, reason, None, out,
                );
            }
            // A locally resident preferred LCL installed while `Connected`
            // (no acquisition needed) drives `Connected -> Tracking`, matching
            // rippled switchLastClosedLedger clearing needNetworkLedger.
            SyncPhase::Connected => {
                let fact = TransitionFact::TargetInstalledAsLcl { lcl: identity };
                let (derived, reason, kind) = self.apply_fact(Some(fact));
                self.push(
                    tag, None, kind, derived, None, None, None, reason, None, out,
                );
            }
            SyncPhase::Tracking { lcl } if identity.sequence() > lcl.sequence() => {
                self.phase = SyncPhase::Tracking { lcl: identity };
                self.push(
                    tag,
                    None,
                    DisagreementKind::Match,
                    Some(self.phase),
                    None,
                    None,
                    None,
                    None,
                    None,
                    out,
                );
            }
            SyncPhase::Full { lcl, published } if identity.sequence() > lcl.sequence() => {
                self.phase = SyncPhase::Full {
                    lcl: identity,
                    published,
                };
                self.push(
                    tag,
                    None,
                    DisagreementKind::Match,
                    Some(self.phase),
                    None,
                    None,
                    None,
                    None,
                    None,
                    out,
                );
            }
            SyncPhase::Tracking { .. } | SyncPhase::Full { .. } => {
                self.push(
                    tag,
                    None,
                    DisagreementKind::Match,
                    Some(self.phase),
                    None,
                    None,
                    None,
                    None,
                    None,
                    out,
                );
            }
            // Installing an LCL that is not the syncing target violates the
            // rules: the production system accepted a fact the coordinator
            // would reject.
            _ => {
                let derived_phase = Some(self.phase);
                self.push(
                    tag,
                    None,
                    DisagreementKind::DerivedRejected,
                    derived_phase,
                    None,
                    None,
                    None,
                    None,
                    None,
                    out,
                );
            }
        }
        let cancelled = self
            .mirror
            .iter()
            .filter(|(_, mirror)| {
                mirror.target.hash() == identity.hash()
                    && matches!(
                        mirror.phase,
                        SessionPhase::Active | SessionPhase::Dormant | SessionPhase::Persisting
                    )
            })
            .map(|(session, _)| *session)
            .collect::<Vec<_>>();
        for session in cancelled {
            let mirror = self
                .mirror
                .get_mut(&session)
                .expect("selected mirror exists");
            let terminal = Some(ShadowOutcome::Cancelled {
                reason: CancelReason::LclInstalled,
            });
            mirror.phase = SessionPhase::Cancelled {
                reason: CancelReason::LclInstalled,
            };
            mirror.terminal = terminal;
            self.push(
                tag,
                Some(session),
                DisagreementKind::Match,
                Some(self.phase),
                terminal,
                None,
                None,
                None,
                None,
                out,
            );
        }
    }

    fn derive_publication(
        &mut self,
        tag: ShadowEventTag,
        identity: LedgerIdentity,
        fresh: bool,
        out: &mut Vec<ShadowObservation>,
    ) {
        match self.phase {
            // Production NetworkOps emits a publication fact only after it
            // proved this is the tracked LCL or its contiguous published
            // ancestor. Mirror the runner's defensive sequence guard.
            SyncPhase::Tracking { lcl } if fresh => {
                let fact = TransitionFact::ChainContiguous {
                    lcl,
                    published: identity,
                };
                let (derived, reason, kind) = self.apply_fact(Some(fact));
                self.push(
                    tag, None, kind, derived, None, None, None, reason, None, out,
                );
            }
            SyncPhase::Full { lcl, published } if identity.sequence() > published.sequence() => {
                self.phase = SyncPhase::Full {
                    lcl,
                    published: identity,
                };
                self.push(
                    tag,
                    None,
                    DisagreementKind::Match,
                    Some(self.phase),
                    None,
                    None,
                    None,
                    None,
                    None,
                    out,
                );
            }
            _ => {
                let derived_phase = Some(self.phase);
                self.push(
                    tag,
                    None,
                    DisagreementKind::DerivedRejected,
                    derived_phase,
                    None,
                    None,
                    None,
                    None,
                    None,
                    out,
                );
            }
        }
    }

    fn derive_store_rotation(
        &mut self,
        tag: ShadowEventTag,
        generation: StoreGeneration,
        out: &mut Vec<ShadowObservation>,
    ) {
        self.store_generation = generation;
        let old = self
            .mirror
            .iter()
            .filter(|(session, mirror)| {
                !mirror.phase.is_terminal() && session.store_generation() < generation
            })
            .map(|(session, _)| *session)
            .collect::<Vec<_>>();
        let had_old = !old.is_empty();
        for session in old {
            let Some(mirror) = self.mirror.get_mut(&session) else {
                continue;
            };
            let terminal = Some(ShadowOutcome::Cancelled {
                reason: CancelReason::StoreRotated,
            });
            mirror.phase = SessionPhase::Cancelled {
                reason: CancelReason::StoreRotated,
            };
            mirror.terminal = terminal;
            let queue_depth = mirror.queue_depth;
            let derived_phase = Some(self.phase);
            self.push(
                tag,
                Some(session),
                DisagreementKind::Match,
                derived_phase,
                terminal,
                None,
                None,
                None,
                Some(queue_depth),
                out,
            );
        }
        if !had_old {
            let derived_phase = Some(self.phase);
            self.push(
                tag,
                None,
                DisagreementKind::Match,
                derived_phase,
                None,
                None,
                None,
                None,
                None,
                out,
            );
        }
    }

    /// A fetch-pack availability fact. It names no session and never changes
    /// the service phase; record it against every live mirror session so shadow
    /// comparisons can correlate re-advance decisions.
    fn derive_fetch_pack(&mut self, tag: ShadowEventTag, out: &mut Vec<ShadowObservation>) {
        let sessions = self
            .mirror
            .iter()
            .filter(|(_, mirror)| mirror.phase == SessionPhase::Active)
            .map(|(session, _)| *session)
            .collect::<Vec<_>>();
        if sessions.is_empty() {
            let derived_phase = Some(self.phase);
            self.push(
                tag,
                None,
                DisagreementKind::Match,
                derived_phase,
                None,
                None,
                None,
                None,
                None,
                out,
            );
            return;
        }
        for session in sessions {
            let Some(mirror) = self.mirror.get(&session) else {
                continue;
            };
            let queue_depth = mirror.queue_depth;
            let derived_phase = Some(self.phase);
            self.push(
                tag,
                Some(session),
                DisagreementKind::Match,
                derived_phase,
                None,
                None,
                None,
                None,
                Some(queue_depth),
                out,
            );
        }
    }

    /// A heartbeat fact. It names no session and never changes the service
    /// phase or any session (the coordinator only re-publishes the current
    /// phase so the phase port can re-normalize). Record a match against the
    /// current phase so shadow comparisons stay correlated.
    /// A startup-mode seed is copied wholesale into the mirror: it is an
    /// initial phase, not a transition, so no transition-table derivation
    /// applies and no rejection is possible.
    fn derive_startup_mode(
        &mut self,
        tag: ShadowEventTag,
        phase: SyncPhase,
        out: &mut Vec<ShadowObservation>,
    ) {
        self.phase = phase;
        self.push(
            tag,
            None,
            DisagreementKind::Match,
            Some(phase),
            None,
            None,
            None,
            None,
            None,
            out,
        );
    }

    fn derive_heartbeat(&mut self, tag: ShadowEventTag, out: &mut Vec<ShadowObservation>) {
        let derived_phase = Some(self.phase);
        self.push(
            tag,
            None,
            DisagreementKind::Match,
            derived_phase,
            None,
            None,
            None,
            None,
            None,
            out,
        );
    }

    fn derive_shutdown(&mut self, tag: ShadowEventTag, out: &mut Vec<ShadowObservation>) {
        let (_, reason, kind) = self.apply_fact(Some(TransitionFact::Shutdown));
        let sessions = self
            .mirror
            .iter()
            .filter(|(_, mirror)| !mirror.phase.is_terminal())
            .map(|(session, _)| *session)
            .collect::<Vec<_>>();
        let had_active = !sessions.is_empty();
        for session in sessions {
            let Some(mirror) = self.mirror.get_mut(&session) else {
                continue;
            };
            let terminal = Some(ShadowOutcome::Cancelled {
                reason: CancelReason::Shutdown,
            });
            mirror.phase = SessionPhase::Cancelled {
                reason: CancelReason::Shutdown,
            };
            mirror.terminal = terminal;
            let queue_depth = mirror.queue_depth;
            let derived_phase = Some(self.phase);
            self.push(
                tag,
                Some(session),
                DisagreementKind::Match,
                derived_phase,
                terminal,
                None,
                None,
                reason,
                Some(queue_depth),
                out,
            );
        }
        if !had_active {
            let derived_phase = Some(self.phase);
            self.push(
                tag,
                None,
                kind,
                derived_phase,
                None,
                None,
                None,
                reason,
                None,
                out,
            );
        }
    }

    fn compare_reference_inner(
        &mut self,
        reference: &ReferenceDecision,
        out: &mut Vec<ShadowObservation>,
    ) {
        let Some(mirror) = self.mirror.get(&reference.session()) else {
            self.push(
                ShadowEventTag::Connectivity,
                Some(reference.session()),
                DisagreementKind::SessionPresenceMismatch,
                Some(self.phase),
                None,
                reference.phase(),
                reference.outcome(),
                None,
                Some(reference.queue_depth()),
                out,
            );
            return;
        };
        let derived_outcome = mirror.terminal;
        let kind = if let Some(reference_hash) = reference.target_hash()
            && reference_hash != mirror.session.target_hash()
        {
            DisagreementKind::TargetMismatch
        } else if derived_outcome != reference.outcome()
            && (derived_outcome.is_some() || reference.outcome().is_some())
        {
            DisagreementKind::OutcomeMismatch
        } else if let Some(reference_phase) = reference.phase()
            && reference_phase != self.phase
        {
            DisagreementKind::PhaseMismatch
        } else {
            DisagreementKind::Match
        };
        self.push(
            ShadowEventTag::Connectivity,
            Some(reference.session()),
            kind,
            Some(self.phase),
            derived_outcome,
            reference.phase(),
            reference.outcome(),
            None,
            Some(reference.queue_depth()),
            out,
        );
    }

    /// Applies a transition fact to the mirror phase, returning the derived
    /// phase, the fact, and the categorized result. A rejected fact records
    /// `DerivedRejected` (the production system accepted something the
    /// coordinator rules forbid).
    fn apply_fact(
        &mut self,
        fact: Option<TransitionFact>,
    ) -> (Option<SyncPhase>, Option<TransitionFact>, DisagreementKind) {
        let Some(fact) = fact else {
            return (Some(self.phase), None, DisagreementKind::Match);
        };
        match phase_transition(&self.phase, &fact) {
            Some(next) => {
                self.phase = next;
                (Some(next), Some(fact), DisagreementKind::Match)
            }
            None => (
                Some(self.phase),
                Some(fact),
                DisagreementKind::DerivedRejected,
            ),
        }
    }

    fn activate_latest_consensus(&mut self, session: SessionRef) {
        // Preferred-target policy ranks work but does not suspend independent
        // per-hash acquisitions. rippled retains multiple InboundLedger
        // instances concurrently under its single registry owner.
        if let Some(mirror) = self.mirror.get_mut(&session)
            && mirror.reason == AcquireReason::Consensus
            && mirror.phase == SessionPhase::Dormant
        {
            mirror.phase = SessionPhase::Active;
        }
    }

    fn mint_session(&mut self, target_hash: Uint256) -> SessionRef {
        let session = SessionRef::new(
            self.run_epoch,
            self.session_counter.next_id(),
            target_hash,
            PlanEpoch::new(1),
            self.store_generation,
        );
        while self.mirror.len() >= self.config.max_mirror_sessions {
            let (oldest, _) = self
                .mirror
                .pop_first()
                .expect("mirror length is bounded by the eviction loop");
            self.mirror.remove(&oldest);
        }
        session
    }

    #[allow(clippy::too_many_arguments)]
    fn push(
        &mut self,
        event: ShadowEventTag,
        session: Option<SessionRef>,
        kind: DisagreementKind,
        derived_phase: Option<SyncPhase>,
        derived_outcome: Option<ShadowOutcome>,
        reference_phase: Option<SyncPhase>,
        reference_outcome: Option<ShadowOutcome>,
        reason: Option<TransitionFact>,
        queue_depth: Option<usize>,
        out: &mut Vec<ShadowObservation>,
    ) {
        let observation = ShadowObservation {
            event,
            session,
            kind,
            derived_phase,
            derived_outcome,
            reference_phase,
            reference_outcome,
            reason,
            queue_depth: queue_depth.unwrap_or(self.queue_depth),
        };
        match kind {
            DisagreementKind::Match => self.matches += 1,
            DisagreementKind::StaleEvent => self.stale_events += 1,
            _ => self.disagreements += 1,
        }
        out.push(observation.clone());
        self.observations.push_back(observation);
        while self.observations.len() > self.config.max_buffered_observations {
            self.observations.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handoff::DurableHandoffAcknowledgement;
    use crate::id::{DurableHandoffId, OperationGeneration, OperationId, SessionId};
    use crate::identity::{OperationKind, OperationRef};
    use crate::ingress::{AdmissionBudget, AdmissionGate, BackpressureOutcome};
    use crate::io::{DurabilityCompletion, ReadCompletion, WriteCompletion};

    fn runner() -> ShadowRunner {
        ShadowRunner::new(ShadowConfig::default(), RunEpoch::new(1))
    }

    fn target(seq: u32) -> LedgerTarget {
        LedgerTarget::new(Uint256::from(u64::from(seq)), Some(seq))
    }

    fn identity(seq: u32) -> LedgerIdentity {
        LedgerIdentity::new(Uint256::from(u64::from(seq)), seq)
    }

    fn session() -> SessionRef {
        SessionRef::new(
            RunEpoch::new(1),
            SessionId::new(1),
            Uint256::from(1),
            PlanEpoch::new(1),
            StoreGeneration::new(1),
        )
    }

    fn acquire(runner: &mut ShadowRunner, seq: u32) -> SessionRef {
        let target = target(seq);
        runner.record(&AcquisitionEvent::AcquireRequested {
            target,
            reason: AcquireReason::Consensus,
        });
        SessionRef::new(
            RunEpoch::new(1),
            SessionId::new(1),
            target.hash(),
            PlanEpoch::new(1),
            StoreGeneration::new(1),
        )
    }

    fn timer_operation(session: SessionRef, id: u64) -> OperationRef {
        OperationRef::new(
            session,
            OperationKind::Timer,
            OperationId::new(id),
            OperationGeneration::new(id),
        )
    }

    fn observe_expiry_arm(shadow: &mut ShadowRunner, session: SessionRef, id: u64) -> OperationRef {
        let operation = timer_operation(session, id);
        shadow.observe_effects(&[AcquisitionEffect::ArmTimer(
            crate::timer::TimerRequest::new(
                operation,
                TimerKind::SessionExpiry,
                std::time::Duration::from_secs(60),
            ),
        )]);
        operation
    }

    fn admitted_packet(session: SessionRef) -> AdmittedLedgerPacket {
        let gate = std::sync::Arc::new(AdmissionGate::new(AdmissionBudget::default(), session));
        match gate.try_reserve(1, 1) {
            BackpressureOutcome::Admitted(lease) => AdmittedLedgerPacket::new(
                lease,
                session,
                crate::id::PeerId::new(1),
                ledger::InboundLedgerPacket::new(
                    ledger::InboundLedgerDataType::Base,
                    vec![ledger::InboundLedgerNodeData::new(None, vec![1])],
                ),
            )
            .expect("matching lease must admit"),
            other => panic!("expected admission, got {other:?}"),
        }
    }

    fn read_completion(session: SessionRef) -> ReadCompletion {
        let operation = OperationRef::new(
            session,
            crate::identity::OperationKind::Read,
            crate::id::OperationId::new(1),
            crate::id::OperationGeneration::new(1),
        );
        ReadCompletion::new(operation, ReadOutcome::Settled { node: None })
    }

    fn header_read_completion(session: SessionRef) -> ReadCompletion {
        let operation = OperationRef::new(
            session,
            OperationKind::HeaderRead,
            OperationId::new(2),
            OperationGeneration::new(2),
        );
        ReadCompletion::new(operation, ReadOutcome::Settled { node: None })
    }

    fn write_completion(session: SessionRef, outcome: WriteOutcome) -> WriteCompletion {
        let operation = OperationRef::new(
            session,
            crate::identity::OperationKind::Write,
            crate::id::OperationId::new(1),
            crate::id::OperationGeneration::new(1),
        );
        WriteCompletion::new(operation, outcome)
    }

    fn durability_completion(
        session: SessionRef,
        outcome: DurabilityOutcome,
    ) -> DurabilityCompletion {
        let operation = OperationRef::new(
            session,
            crate::identity::OperationKind::DurabilityFence,
            crate::id::OperationId::new(1),
            crate::id::OperationGeneration::new(1),
        );
        DurabilityCompletion::new(operation, outcome)
    }

    #[test]
    fn shadow_mode_never_dispatches_effects() {
        // The runner's only outputs are observations and snapshots: no effect
        // type is produced by any path, and no port is reachable from this
        // module. Feed a full lifecycle and assert everything is derivable.
        let mut shadow = runner();
        shadow.record(&AcquisitionEvent::Connectivity(
            PeerAvailabilitySnapshot::new(vec![crate::id::PeerId::new(1)]),
        ));
        let session = acquire(&mut shadow, 1);
        shadow.record(&AcquisitionEvent::PacketAdmitted(admitted_packet(session)));
        shadow.record(&AcquisitionEvent::ReadCompleted(read_completion(session)));
        shadow.record(&AcquisitionEvent::WriteCompleted(write_completion(
            session,
            WriteOutcome::Accepted,
        )));
        shadow.record(&AcquisitionEvent::DurabilityFenced(durability_completion(
            session,
            DurabilityOutcome::Passed,
        )));
        shadow.record(&AcquisitionEvent::DurableHandoffAcknowledged(
            DurableHandoffAcknowledgement::new(DurableHandoffId::new(1), session),
        ));
        shadow.record(&AcquisitionEvent::LclInstalled(identity(1)));

        let snapshot = shadow.snapshot();
        assert_eq!(snapshot.phase(), &SyncPhase::Tracking { lcl: identity(1) });
        assert_eq!(snapshot.matches(), 8);
        assert_eq!(snapshot.disagreements(), 0);
        assert_eq!(snapshot.mirror_sessions(), 1);
        // The session is terminal by now, so no active session remains.
        assert!(snapshot.active_by_reason().is_empty());
        // Everything the runner produced is drainable as observations.
        let drained = shadow.drain_observations();
        assert_eq!(drained.len(), snapshot.observations_buffered);
    }

    #[test]
    fn shadow_startup_mode_seeds_the_initial_phase() {
        let mut shadow = runner();
        let observations = shadow.record(&AcquisitionEvent::StartupMode {
            phase: SyncPhase::Connected,
        });
        assert_eq!(shadow.snapshot().phase(), &SyncPhase::Connected);
        assert_eq!(observations[0].event(), ShadowEventTag::StartupMode);
        assert_eq!(observations[0].derived_phase(), Some(&SyncPhase::Connected));
        // A startup seed is not a transition: it never mints a session and
        // cannot be rejected.
        assert_eq!(shadow.snapshot().mirror_sessions(), 0);
        assert_eq!(shadow.snapshot().disagreements(), 0);
    }

    #[test]
    fn shadow_preferred_lcl_divergence_demotes_tracking_to_syncing() {
        let mut shadow = runner();
        shadow.record(&AcquisitionEvent::Connectivity(
            PeerAvailabilitySnapshot::new(vec![crate::id::PeerId::new(1)]),
        ));
        let _session = acquire(&mut shadow, 1);
        shadow.record(&AcquisitionEvent::LclInstalled(identity(1)));
        assert_eq!(
            shadow.snapshot().phase(),
            &SyncPhase::Tracking { lcl: identity(1) }
        );

        let observations =
            shadow.record(&AcquisitionEvent::PreferredLclDivergence { target: target(9) });
        assert_eq!(
            shadow.snapshot().phase(),
            &SyncPhase::Syncing { target: target(9) }
        );
        // A divergence fact is phase-only: it never mints a mirror session.
        assert_eq!(shadow.snapshot().mirror_sessions(), 1);
        assert_eq!(shadow.snapshot().disagreements(), 0);
        assert_eq!(
            observations[0].event(),
            ShadowEventTag::PreferredLclDivergence
        );
        assert_eq!(
            observations[0].derived_phase(),
            Some(&SyncPhase::Syncing { target: target(9) })
        );
    }

    #[test]
    fn shadow_preferred_lcl_divergence_preserves_recovery_anchor() {
        let mut shadow = runner();
        shadow.record(&AcquisitionEvent::Connectivity(
            PeerAvailabilitySnapshot::new(vec![crate::id::PeerId::new(1)]),
        ));
        let _ = acquire(&mut shadow, 1);
        assert_eq!(
            shadow.snapshot().phase(),
            &SyncPhase::Syncing { target: target(1) }
        );

        let observations =
            shadow.record(&AcquisitionEvent::PreferredLclDivergence { target: target(9) });
        assert_eq!(
            shadow.snapshot().phase(),
            &SyncPhase::Syncing { target: target(1) }
        );
        assert_eq!(observations[0].kind(), DisagreementKind::Match);
        assert_eq!(shadow.snapshot().disagreements(), 0);
    }

    fn shadow_with_anchor_and_latest() -> (ShadowRunner, SessionRef, LedgerTarget) {
        let mut shadow = runner();
        shadow.record(&AcquisitionEvent::Connectivity(
            PeerAvailabilitySnapshot::new(vec![crate::id::PeerId::new(1)]),
        ));
        let anchor = target(10);
        let anchor_session = shadow
            .record(&AcquisitionEvent::AcquireRequested {
                target: anchor,
                reason: AcquireReason::Generic,
            })
            .into_iter()
            .find_map(|observation| observation.session())
            .expect("anchor mirror session");
        // The local header probe missed, leaving this session idle at its
        // network boundary and therefore eligible for registry expiry.
        shadow.record(&AcquisitionEvent::ReadCompleted(header_read_completion(
            anchor_session,
        )));
        let latest = target(20);
        shadow.record(&AcquisitionEvent::ConsensusTarget(ConsensusTarget::new(
            latest,
            AcquireReason::Consensus,
        )));
        assert_eq!(
            shadow.snapshot().phase(),
            &SyncPhase::Syncing { target: anchor }
        );
        (shadow, anchor_session, latest)
    }

    #[test]
    fn shadow_terminal_failure_promotes_latest_viable_anchor() {
        let (mut shadow, anchor_session, latest) = shadow_with_anchor_and_latest();
        let observations = shadow.record(&AcquisitionEvent::WriteCompleted(write_completion(
            anchor_session,
            WriteOutcome::Failed,
        )));

        assert_eq!(
            shadow.snapshot().phase(),
            &SyncPhase::Syncing { target: latest }
        );
        assert_eq!(
            observations
                .last()
                .and_then(ShadowObservation::derived_phase),
            Some(&SyncPhase::Syncing { target: latest })
        );
    }

    #[test]
    fn shadow_idle_expiry_promotes_latest_viable_anchor() {
        let (mut shadow, anchor_session, latest) = shadow_with_anchor_and_latest();
        let operation = observe_expiry_arm(&mut shadow, anchor_session, 100);
        shadow.record(&AcquisitionEvent::TimerFired {
            operation,
            timer: TimerKind::SessionExpiry,
        });
        let observations = shadow.record(&AcquisitionEvent::RegistrySweep);

        assert_eq!(
            shadow.snapshot().phase(),
            &SyncPhase::Syncing { target: latest }
        );
        assert_eq!(
            observations
                .last()
                .and_then(ShadowObservation::derived_phase),
            Some(&SyncPhase::Syncing { target: latest })
        );
    }

    #[test]
    fn dormant_shadow_session_accepts_exact_expiry_and_global_sweep() {
        let mut shadow = runner();
        shadow.record(&AcquisitionEvent::Connectivity(
            PeerAvailabilitySnapshot::new(vec![crate::id::PeerId::new(1)]),
        ));
        let session = acquire(&mut shadow, 1);
        shadow.record(&AcquisitionEvent::ReadCompleted(header_read_completion(
            session,
        )));
        shadow
            .mirror
            .get_mut(&session)
            .expect("mirrored session")
            .phase = SessionPhase::Dormant;
        let operation = observe_expiry_arm(&mut shadow, session, 100);

        let observations = shadow.record(&AcquisitionEvent::TimerFired {
            operation,
            timer: TimerKind::SessionExpiry,
        });
        assert_eq!(observations[0].kind(), DisagreementKind::Match);

        shadow.record(&AcquisitionEvent::RegistrySweep);
        assert_eq!(
            shadow.mirror.get(&session).expect("swept mirror").phase,
            SessionPhase::Cancelled {
                reason: CancelReason::IdleExpired,
            }
        );
    }

    #[test]
    fn shadow_registry_sweep_preserves_anchor_during_local_scan() {
        let (mut shadow, anchor_session, _latest) = shadow_with_anchor_and_latest();
        shadow.record(&AcquisitionEvent::PacketAdmitted(admitted_packet(
            anchor_session,
        )));
        let operation = observe_expiry_arm(&mut shadow, anchor_session, 100);
        shadow.record(&AcquisitionEvent::TimerFired {
            operation,
            timer: TimerKind::SessionExpiry,
        });
        let observations = shadow.record(&AcquisitionEvent::RegistrySweep);

        assert_eq!(
            shadow.snapshot().phase(),
            &SyncPhase::Syncing { target: target(10) }
        );
        assert_eq!(
            shadow
                .mirror
                .get(&anchor_session)
                .expect("scan owner remains mirrored")
                .phase,
            SessionPhase::Active
        );
        assert!(observations.iter().all(|observation| {
            observation.derived_outcome()
                != Some(ShadowOutcome::Cancelled {
                    reason: CancelReason::IdleExpired,
                })
        }));
    }

    #[test]
    fn shadow_keeps_moving_consensus_hashes_concurrently_active() {
        let mut shadow = runner();
        shadow.record(&AcquisitionEvent::Connectivity(
            PeerAvailabilitySnapshot::new(vec![crate::id::PeerId::new(1)]),
        ));
        let first = shadow
            .record(&AcquisitionEvent::ConsensusTarget(ConsensusTarget::new(
                target(10),
                AcquireReason::Consensus,
            )))
            .into_iter()
            .find_map(|observation| observation.session())
            .expect("first consensus session");
        let second = shadow
            .record(&AcquisitionEvent::ConsensusTarget(ConsensusTarget::new(
                target(20),
                AcquireReason::Consensus,
            )))
            .into_iter()
            .find_map(|observation| observation.session())
            .expect("second consensus session");

        assert_eq!(
            shadow.mirror.get(&first).expect("first").phase,
            SessionPhase::Active
        );
        assert_eq!(
            shadow.mirror.get(&second).expect("second").phase,
            SessionPhase::Active
        );
        assert_eq!(
            shadow
                .snapshot()
                .active_by_reason()
                .get(&AcquireReason::Consensus),
            Some(&2)
        );
        let progress = shadow.record(&AcquisitionEvent::PacketAdmitted(admitted_packet(first)));
        assert_eq!(progress[0].kind(), DisagreementKind::Match);

        shadow.record(&AcquisitionEvent::WriteCompleted(write_completion(
            first,
            WriteOutcome::Failed,
        )));
        assert_eq!(
            shadow.snapshot().phase(),
            &SyncPhase::Syncing { target: target(20) }
        );
    }

    #[test]
    fn shadow_store_rotation_cancels_all_without_transient_anchor_promotion() {
        let (mut shadow, _anchor_session, _latest) = shadow_with_anchor_and_latest();
        let anchor = target(10);
        shadow.record(&AcquisitionEvent::StoreRotated(StoreGeneration::new(2)));

        assert_eq!(
            shadow.snapshot().phase(),
            &SyncPhase::Syncing { target: anchor }
        );
        assert_eq!(
            shadow.snapshot().active_by_reason().values().sum::<usize>(),
            0
        );
    }

    #[test]
    fn shadow_full_lcl_and_publication_advance_independently() {
        let mut shadow = runner();
        shadow.record(&AcquisitionEvent::StartupMode {
            phase: SyncPhase::Full {
                lcl: identity(9),
                published: identity(9),
            },
        });

        let observations = shadow.record(&AcquisitionEvent::LclInstalled(identity(10)));
        assert_eq!(observations[0].kind(), DisagreementKind::Match);
        assert_eq!(
            shadow.snapshot().phase(),
            &SyncPhase::Full {
                lcl: identity(10),
                published: identity(9),
            }
        );

        let observations = shadow.record(&AcquisitionEvent::PublicationCommitted {
            identity: identity(11),
            fresh: true,
        });
        assert_eq!(observations[0].kind(), DisagreementKind::Match);
        assert_eq!(
            shadow.snapshot().phase(),
            &SyncPhase::Full {
                lcl: identity(10),
                published: identity(11),
            }
        );

        let observations = shadow.record(&AcquisitionEvent::PublicationCommitted {
            identity: identity(12),
            fresh: false,
        });
        assert_eq!(observations[0].kind(), DisagreementKind::Match);
        assert_eq!(
            shadow.snapshot().phase(),
            &SyncPhase::Full {
                lcl: identity(10),
                published: identity(12),
            }
        );
    }

    #[test]
    fn shadow_blocked_with_no_target_demotes_full_to_connected() {
        let mut shadow = runner();
        shadow.record(&AcquisitionEvent::Connectivity(
            PeerAvailabilitySnapshot::new(vec![crate::id::PeerId::new(1)]),
        ));
        let _session = acquire(&mut shadow, 1);
        shadow.record(&AcquisitionEvent::LclInstalled(identity(1)));
        shadow.record(&AcquisitionEvent::PublicationCommitted {
            identity: identity(1),
            fresh: true,
        });
        assert_eq!(
            shadow.snapshot().phase(),
            &SyncPhase::Full {
                lcl: identity(1),
                published: identity(1),
            }
        );

        let observations = shadow.record(&AcquisitionEvent::BlockedWithNoTarget);
        assert_eq!(shadow.snapshot().phase(), &SyncPhase::Connected);
        assert_eq!(observations[0].event(), ShadowEventTag::BlockedWithNoTarget);
        assert_eq!(observations[0].derived_phase(), Some(&SyncPhase::Connected));
        assert_eq!(shadow.snapshot().disagreements(), 0);
    }

    #[test]
    fn disabled_shadow_records_and_compares_nothing() {
        let mut shadow = ShadowRunner::new(ShadowConfig::disabled(), RunEpoch::new(1));
        assert!(!shadow.is_enabled());
        let observations = shadow.record(&AcquisitionEvent::AcquireRequested {
            target: target(1),
            reason: AcquireReason::Consensus,
        });
        assert!(observations.is_empty());
        let reference = ReferenceDecision::new(session(), None, None, None, 0);
        assert!(shadow.compare_reference(&reference).is_empty());
        let snapshot = shadow.snapshot();
        assert!(!snapshot.enabled());
        assert_eq!(snapshot.matches(), 0);
        assert_eq!(snapshot.disagreements(), 0);
    }

    #[test]
    fn session_started_effect_adopts_the_exact_production_identity() {
        let mut shadow = runner();
        shadow.record(&AcquisitionEvent::Connectivity(
            PeerAvailabilitySnapshot::new(vec![crate::id::PeerId::new(1)]),
        ));
        let provisional = acquire(&mut shadow, 1);
        let production = SessionRef::new(
            provisional.run_epoch(),
            provisional.session_id(),
            provisional.target_hash(),
            PlanEpoch::new(2),
            provisional.store_generation(),
        );
        shadow.observe_effects(&[AcquisitionEffect::SessionStarted(production)]);

        assert!(!shadow.mirror.contains_key(&provisional));
        assert!(shadow.mirror.contains_key(&production));

        let expiry = timer_operation(production, 7);
        shadow.observe_effects(&[AcquisitionEffect::ArmTimer(
            crate::timer::TimerRequest::new(
                expiry,
                TimerKind::SessionExpiry,
                std::time::Duration::from_secs(60),
            ),
        )]);
        let observations = shadow.record(&AcquisitionEvent::TimerFired {
            operation: expiry,
            timer: TimerKind::SessionExpiry,
        });
        assert_eq!(observations[0].kind(), DisagreementKind::Match);
        assert_eq!(shadow.snapshot().stale_events(), 0);
    }

    #[test]
    fn acquisition_from_disconnected_is_derived_rejected() {
        let mut shadow = runner();
        // No usable-peer fact yet; the rules forbid starting acquisition.
        let observations = shadow.record(&AcquisitionEvent::AcquireRequested {
            target: target(1),
            reason: AcquireReason::Consensus,
        });
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].kind(), DisagreementKind::DerivedRejected);
        assert_eq!(shadow.snapshot().phase(), &SyncPhase::Disconnected);
        assert_eq!(shadow.snapshot().mirror_sessions(), 0);
    }

    #[test]
    fn ordinary_consensus_target_is_phase_neutral_after_lcl_install() {
        let full = SyncPhase::Full {
            lcl: identity(10),
            published: identity(10),
        };
        let mut shadow = runner();
        shadow.record(&AcquisitionEvent::StartupMode { phase: full });
        shadow.record(&AcquisitionEvent::Connectivity(
            PeerAvailabilitySnapshot::new(vec![crate::id::PeerId::new(1)]),
        ));

        let observations = shadow.record(&AcquisitionEvent::ConsensusTarget(ConsensusTarget::new(
            target(11),
            AcquireReason::Consensus,
        )));

        assert_eq!(shadow.snapshot().phase(), &full);
        assert_eq!(shadow.snapshot().mirror_sessions(), 1);
        assert!(observations.iter().all(|observation| {
            observation.kind() == DisagreementKind::Match
                && observation.derived_phase() == Some(&full)
        }));
    }

    #[test]
    fn rearmed_expiry_rejects_the_stale_operation_before_global_sweep() {
        let mut shadow = runner();
        shadow.record(&AcquisitionEvent::Connectivity(
            PeerAvailabilitySnapshot::new(vec![crate::id::PeerId::new(1)]),
        ));
        let session = acquire(&mut shadow, 1);
        // A completed header miss proves the local scan reached its network
        // boundary, so an exact current expiry may be swept.
        shadow.record(&AcquisitionEvent::ReadCompleted(header_read_completion(
            session,
        )));

        let stale = timer_operation(session, 10);
        let current = timer_operation(session, 11);
        shadow.observe_effects(&[AcquisitionEffect::ArmTimer(
            crate::timer::TimerRequest::new(
                stale,
                TimerKind::SessionExpiry,
                std::time::Duration::from_secs(60),
            ),
        )]);
        shadow.observe_effects(&[AcquisitionEffect::ArmTimer(
            crate::timer::TimerRequest::new(
                current,
                TimerKind::SessionExpiry,
                std::time::Duration::from_secs(60),
            ),
        )]);

        let observations = shadow.record(&AcquisitionEvent::TimerFired {
            operation: stale,
            timer: TimerKind::SessionExpiry,
        });
        assert_eq!(observations[0].kind(), DisagreementKind::StaleEvent);
        shadow.record(&AcquisitionEvent::RegistrySweep);
        assert_eq!(
            shadow.mirror.get(&session).expect("live mirror").phase,
            SessionPhase::Active
        );

        let observations = shadow.record(&AcquisitionEvent::TimerFired {
            operation: current,
            timer: TimerKind::SessionExpiry,
        });
        assert_eq!(observations[0].kind(), DisagreementKind::Match);
        shadow.record(&AcquisitionEvent::RegistrySweep);
        assert_eq!(
            shadow.mirror.get(&session).expect("expired mirror").phase,
            SessionPhase::Cancelled {
                reason: CancelReason::IdleExpired
            }
        );
    }

    #[test]
    fn phase_disagreement_is_categorized() {
        let mut shadow = runner();
        shadow.record(&AcquisitionEvent::Connectivity(
            PeerAvailabilitySnapshot::new(vec![crate::id::PeerId::new(1)]),
        ));
        assert_eq!(shadow.snapshot().phase(), &SyncPhase::Connected);

        let reference = ReferenceDecision::new(
            session(),
            Some(SyncPhase::Full {
                lcl: identity(1),
                published: identity(1),
            }),
            None,
            None,
            0,
        );
        let observations = shadow.compare_reference(&reference);
        assert_eq!(observations.len(), 1);
        assert_eq!(
            observations[0].kind(),
            DisagreementKind::SessionPresenceMismatch
        );

        // Compare an actually tracked session; the derived phase must match the
        // reference that corresponds to the same event history.
        let session = acquire(&mut shadow, 1);
        let reference = ReferenceDecision::new(
            session,
            Some(SyncPhase::Syncing { target: target(1) }),
            None,
            Some(target(1).hash()),
            0,
        );
        let observations = shadow.compare_reference(&reference);
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].kind(), DisagreementKind::Match);

        // A reference reporting the wrong phase is a phase mismatch.
        let reference = ReferenceDecision::new(
            session,
            Some(SyncPhase::Full {
                lcl: identity(1),
                published: identity(1),
            }),
            None,
            Some(target(1).hash()),
            0,
        );
        let observations = shadow.compare_reference(&reference);
        assert_eq!(observations[0].kind(), DisagreementKind::PhaseMismatch);
    }

    #[test]
    fn outcome_and_target_disagreements_are_categorized() {
        let mut shadow = runner();
        shadow.record(&AcquisitionEvent::Connectivity(
            PeerAvailabilitySnapshot::new(vec![crate::id::PeerId::new(1)]),
        ));
        let session = acquire(&mut shadow, 1);
        shadow.record(&AcquisitionEvent::WriteCompleted(write_completion(
            session,
            WriteOutcome::Failed,
        )));

        // The rules derive a write-failure outcome.
        let matching = ReferenceDecision::new(
            session,
            None,
            Some(ShadowOutcome::Failed {
                reason: FailureReason::WriteFailure,
            }),
            Some(target(1).hash()),
            0,
        );
        assert_eq!(
            shadow.compare_reference(&matching)[0].kind(),
            DisagreementKind::Match
        );

        // Production claims a durable outcome for a session the rules failed.
        let divergent = ReferenceDecision::new(
            session,
            None,
            Some(ShadowOutcome::Durable),
            Some(target(1).hash()),
            0,
        );
        assert_eq!(
            shadow.compare_reference(&divergent)[0].kind(),
            DisagreementKind::OutcomeMismatch
        );

        // Production claims a different target hash.
        let wrong_target = ReferenceDecision::new(
            session,
            None,
            Some(ShadowOutcome::Failed {
                reason: FailureReason::WriteFailure,
            }),
            Some(target(9).hash()),
            0,
        );
        assert_eq!(
            shadow.compare_reference(&wrong_target)[0].kind(),
            DisagreementKind::TargetMismatch
        );
    }

    #[test]
    fn stale_events_are_categorized_and_counted() {
        let mut shadow = runner();
        shadow.record(&AcquisitionEvent::Connectivity(
            PeerAvailabilitySnapshot::new(vec![crate::id::PeerId::new(1)]),
        ));
        let session = acquire(&mut shadow, 1);
        // Store rotation cancels the old-generation session.
        shadow.record(&AcquisitionEvent::StoreRotated(StoreGeneration::new(2)));
        // A late packet for the cancelled session is stale.
        let observations =
            shadow.record(&AcquisitionEvent::PacketAdmitted(admitted_packet(session)));
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].kind(), DisagreementKind::StaleEvent);
        assert_eq!(observations[0].session(), Some(session));
        let snapshot = shadow.snapshot();
        assert_eq!(snapshot.stale_events(), 1);
    }

    #[test]
    fn store_rotation_cancels_old_generation_sessions() {
        let mut shadow = runner();
        shadow.record(&AcquisitionEvent::Connectivity(
            PeerAvailabilitySnapshot::new(vec![crate::id::PeerId::new(1)]),
        ));
        let session = acquire(&mut shadow, 1);
        shadow.record(&AcquisitionEvent::StoreRotated(StoreGeneration::new(2)));

        let observations = shadow.compare_reference(&ReferenceDecision::new(
            session,
            None,
            Some(ShadowOutcome::Cancelled {
                reason: CancelReason::StoreRotated,
            }),
            Some(target(1).hash()),
            0,
        ));
        assert_eq!(observations[0].kind(), DisagreementKind::Match);
    }

    #[test]
    fn shutdown_terminates_all_sessions_and_phase() {
        let mut shadow = runner();
        shadow.record(&AcquisitionEvent::Connectivity(
            PeerAvailabilitySnapshot::new(vec![crate::id::PeerId::new(1)]),
        ));
        let session = acquire(&mut shadow, 1);
        shadow.record(&AcquisitionEvent::Shutdown);

        let snapshot = shadow.snapshot();
        assert_eq!(snapshot.phase(), &SyncPhase::Stopping);
        let observations = shadow.compare_reference(&ReferenceDecision::new(
            session,
            Some(SyncPhase::Stopping),
            Some(ShadowOutcome::Cancelled {
                reason: CancelReason::Shutdown,
            }),
            Some(target(1).hash()),
            0,
        ));
        assert_eq!(observations[0].kind(), DisagreementKind::Match);
    }

    #[test]
    fn heartbeat_is_a_noop_fact_that_stays_correlated() {
        let mut shadow = runner();
        shadow.record(&AcquisitionEvent::Connectivity(
            PeerAvailabilitySnapshot::new(vec![crate::id::PeerId::new(1)]),
        ));
        let _session = acquire(&mut shadow, 1);

        let observations = shadow.record(&AcquisitionEvent::Heartbeat);
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].kind(), DisagreementKind::Match);
        assert_eq!(observations[0].session(), None);
        assert_eq!(
            shadow.snapshot().phase(),
            &SyncPhase::Syncing { target: target(1) }
        );
        // The heartbeat never mutates a session or changes the phase.
        assert_eq!(shadow.snapshot().mirror_sessions(), 1);
    }

    #[test]
    fn full_durable_path_derives_durable_outcome_and_tracking() {
        let mut shadow = runner();
        shadow.record(&AcquisitionEvent::Connectivity(
            PeerAvailabilitySnapshot::new(vec![crate::id::PeerId::new(1)]),
        ));
        let session = acquire(&mut shadow, 1);
        shadow.record(&AcquisitionEvent::WriteCompleted(write_completion(
            session,
            WriteOutcome::Accepted,
        )));
        shadow.record(&AcquisitionEvent::DurabilityFenced(durability_completion(
            session,
            DurabilityOutcome::Passed,
        )));
        shadow.record(&AcquisitionEvent::DurableHandoffAcknowledged(
            DurableHandoffAcknowledgement::new(DurableHandoffId::new(1), session),
        ));
        shadow.record(&AcquisitionEvent::LclInstalled(identity(1)));
        shadow.record(&AcquisitionEvent::PublicationCommitted {
            identity: identity(1),
            fresh: true,
        });

        let snapshot = shadow.snapshot();
        assert_eq!(
            snapshot.phase(),
            &SyncPhase::Full {
                lcl: identity(1),
                published: identity(1)
            }
        );
        let observations = shadow.compare_reference(&ReferenceDecision::new(
            session,
            None,
            Some(ShadowOutcome::Durable),
            Some(target(1).hash()),
            0,
        ));
        assert_eq!(observations[0].kind(), DisagreementKind::Match);
    }

    #[test]
    fn illegal_lcl_install_is_derived_rejected() {
        let mut shadow = runner();
        shadow.record(&AcquisitionEvent::Connectivity(
            PeerAvailabilitySnapshot::new(vec![crate::id::PeerId::new(1)]),
        ));
        let _ = acquire(&mut shadow, 1);
        // Installing an LCL that is not the syncing target is rejected.
        let observations = shadow.record(&AcquisitionEvent::LclInstalled(identity(2)));
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].kind(), DisagreementKind::DerivedRejected);
        assert_eq!(
            shadow.snapshot().phase(),
            &SyncPhase::Syncing { target: target(1) }
        );
    }

    #[test]
    fn stale_publication_is_derived_rejected_until_fresh() {
        let mut shadow = runner();
        shadow.record(&AcquisitionEvent::Connectivity(
            PeerAvailabilitySnapshot::new(vec![crate::id::PeerId::new(1)]),
        ));
        let _ = acquire(&mut shadow, 1);
        shadow.record(&AcquisitionEvent::LclInstalled(identity(1)));
        assert_eq!(
            shadow.snapshot().phase(),
            &SyncPhase::Tracking { lcl: identity(1) }
        );

        // A non-fresh publication of the tracked LCL cannot reach Full.
        let observations = shadow.record(&AcquisitionEvent::PublicationCommitted {
            identity: identity(1),
            fresh: false,
        });
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].kind(), DisagreementKind::DerivedRejected);
        assert_eq!(
            shadow.snapshot().phase(),
            &SyncPhase::Tracking { lcl: identity(1) }
        );

        // The matching fresh publication drives Tracking -> Full.
        let observations = shadow.record(&AcquisitionEvent::PublicationCommitted {
            identity: identity(1),
            fresh: true,
        });
        assert_eq!(observations[0].kind(), DisagreementKind::Match);
        assert_eq!(
            shadow.snapshot().phase(),
            &SyncPhase::Full {
                lcl: identity(1),
                published: identity(1)
            }
        );
    }

    #[test]
    fn active_sessions_are_grouped_by_reason() {
        let mut shadow = runner();
        shadow.record(&AcquisitionEvent::Connectivity(
            PeerAvailabilitySnapshot::new(vec![crate::id::PeerId::new(1)]),
        ));
        shadow.record(&AcquisitionEvent::AcquireRequested {
            target: target(1),
            reason: AcquireReason::Consensus,
        });
        shadow.record(&AcquisitionEvent::AcquireRequested {
            target: target(2),
            reason: AcquireReason::History,
        });
        shadow.record(&AcquisitionEvent::AcquireRequested {
            target: target(3),
            reason: AcquireReason::History,
        });
        let snapshot = shadow.snapshot();
        assert_eq!(
            snapshot.active_by_reason().get(&AcquireReason::Consensus),
            Some(&1)
        );
        assert_eq!(
            snapshot.active_by_reason().get(&AcquireReason::History),
            Some(&2)
        );
    }

    #[test]
    fn observation_buffer_is_bounded() {
        let mut shadow = ShadowRunner::new(
            ShadowConfig {
                enabled: true,
                max_mirror_sessions: 4,
                max_buffered_observations: 3,
            },
            RunEpoch::new(1),
        );
        for seq in 1..=6u32 {
            shadow.record(&AcquisitionEvent::Connectivity(
                PeerAvailabilitySnapshot::new(vec![crate::id::PeerId::new(1)]),
            ));
            shadow.record(&AcquisitionEvent::AcquireRequested {
                target: target(seq),
                reason: AcquireReason::History,
            });
        }
        let snapshot = shadow.snapshot();
        assert!(snapshot.observations_buffered <= 3);
        assert!(snapshot.mirror_sessions <= 4);
    }
}
