//! Typed events submitted to the coordinator.
//!
//! External code submits facts as typed events. The coordinator changes
//! session/lifecycle state; it never exposes mutable session references or
//! lifecycle callbacks. Every externally dispatched completion carries an
//! [`crate::OperationRef`] that the coordinator validates before mutation.

use crate::handoff::{DurableHandoffAcknowledgement, HandoffRejectReason};
use crate::id::StoreGeneration;
use crate::identity::OperationRef;
use crate::ingress::AdmittedLedgerPacket;
use crate::io::{DurabilityCompletion, ReadCompletion, WriteCompletion};
use crate::peer::PeerAvailabilitySnapshot;
use crate::phase::SyncPhase;
use crate::target::{AcquireReason, LedgerIdentity, LedgerTarget};
use crate::timer::TimerKind;

/// A typed input event for the coordinator.
#[derive(Debug, PartialEq, Eq)]
pub enum AcquisitionEvent {
    /// The startup operating-mode intent, derived from bootstrap configuration
    /// (networked start -> `Connected`, `start_valid` -> `Full`). Seeds the
    /// initial phase before any peer capability exists and re-publishes it
    /// idempotently through the phase port; it is not a transition and never
    /// touches a session. Quaxar preserves its legacy startup mode seed
    /// (`bootstrap.rs` startup write); rippled seeds `DISCONNECTED`/`FULL` in
    /// `NetworkOPs`' constructor (`rippled/src/xrpld/app/misc/NetworkOPs.cpp:318`).
    StartupMode { phase: SyncPhase },

    /// Overlay reports the current usable-peer snapshot. A non-empty snapshot
    /// motivates `Disconnected -> Connected`; an empty snapshot motivates any
    /// active phase `-> Disconnected`.
    Connectivity(PeerAvailabilitySnapshot),

    /// Overlay reports the current usable-peer snapshot for acquisition
    /// transport only. Sessions pause, resume, and retarget exactly as for
    /// [`AcquisitionEvent::Connectivity`], but the service phase is unchanged.
    /// This is used by rippled's `start_valid` zero-peer-threshold heartbeat,
    /// where peerless consensus remains operational.
    TransportConnectivity(PeerAvailabilitySnapshot),

    /// The active overlay peer count fell below NetworkOPs' configured
    /// consensus threshold. Connected peers remain usable for acquisition.
    ConsensusQuorumLost,

    /// The active overlay peer count again satisfies NetworkOPs' configured
    /// consensus threshold.
    ConsensusQuorumAvailable,

    /// LedgerMaster, validation, recovery, or startup requests acquisition of a
    /// target. Motivates `Connected/Syncing -> Syncing`.
    AcquireRequested {
        /// The target ledger.
        target: LedgerTarget,
        /// Why the target is needed.
        reason: AcquireReason,
    },

    /// The validations adaptor needs the newest trusted-validation ledger for
    /// preferred-ledger analysis (`GetConsL2`). This acquisition is consensus
    /// priority, but it is deliberately phase-neutral: learning ancestry for
    /// the validation trie is not evidence that the installed LCL diverged.
    ValidationTarget(LedgerTarget),

    /// The validations adaptor's accepted-boundary recovery candidate. `Some`
    /// latches one exact, phase-neutral acquisition owner; later candidates
    /// remain policy metadata until that owner completes, fails, or is
    /// cancelled. `None` clears only the future candidate and never cancels
    /// the live exact owner.
    ValidationRecoveryTarget(Option<LedgerTarget>),

    /// Overlay ingress admitted a decoded ledger packet with a consumed,
    /// gate-bound lease. The coordinator routes it by exact session identity
    /// and settles only its originating admission gate.
    PacketAdmitted(AdmittedLedgerPacket),

    /// The read broker settled a brokered NodeStore read.
    ReadCompleted(ReadCompletion),

    /// The NodeStore write adapter completed a write batch.
    WriteCompleted(WriteCompletion),

    /// The durability fence (final persistence barrier) completed.
    DurabilityFenced(DurabilityCompletion),

    /// The LedgerMaster/NetworkOps adapter acknowledged a durable handoff.
    DurableHandoffAcknowledged(DurableHandoffAcknowledgement),

    /// The adapter could not deliver a durable handoff (channel full or
    /// disconnected). The coordinator keeps the exact handoff pending and
    /// arms one `HandoffRetry` timer; only that exact timer may re-emit
    /// `PublishDurable`. Duplicate rejections while a retry is armed are stale
    /// and recipient-side id-deduplication keeps a later retry idempotent.
    DurableHandoffRejected {
        /// The handoff id the recipient rejected.
        handoff: crate::id::DurableHandoffId,
        /// The session that produced the handoff.
        session: crate::identity::SessionRef,
        /// Why delivery was rejected.
        reason: HandoffRejectReason,
    },

    /// A timer armed by the coordinator fired.
    TimerFired {
        /// The exact operation the timer was armed for.
        operation: OperationRef,
        /// The timer kind.
        timer: TimerKind,
    },

    /// Consensus reports its preferred-LCL target. Consensus remains the
    /// preferred-LCL policy owner; acquisition never installs an arbitrary LCL
    /// merely because a tree completed.
    ConsensusTarget(ConsensusTarget),

    /// Consensus observed that its preferred previous ledger differs from the
    /// current view (rippled `NetworkOPsImp::consensusViewChange` parity).
    /// This is a mode-only signal: it demotes `Tracking/Full -> Connected`
    /// without selecting or pinning an acquisition target.
    ConsensusViewChange,

    /// The serialized `checkLastClosedLedger` path reports a preferred-LCL
    /// divergence with a concrete acquisition target. Motivates
    /// `Connected/Tracking/Full -> Syncing { target }`, or retargets an
    /// existing Syncing phase to the latest preferred identity. It never mints
    /// a session: the acquisition demand arrives as a separate
    /// [`AcquisitionEvent::AcquireRequested`] fact, so a resident-and-compatible
    /// preferred LCL that is merely being switched to (no fetch needed) does not
    /// start a wasteful peer fetch.
    PreferredLclDivergence { target: LedgerTarget },

    /// NetworkOps completed an accepted-boundary preferred-LCL check with the
    /// current local LCL selected. This retires an obsolete syncing target and
    /// permits normal Tracking/Full promotion, matching rippled endConsensus.
    PreferredLclReconciled { lcl: LedgerIdentity },

    /// Consensus accepted a round with no usable peer positions, or NetworkOPs
    /// became amendment/UNL blocked, while the coordinator was `Full`.
    /// Motivates `Full -> Connected` and names no session or target, matching
    /// rippled's targetless operating-mode demotions.
    BlockedWithNoTarget,

    /// A ledger was installed as the last closed ledger.
    LclInstalled(LedgerIdentity),

    /// A ledger was committed to the published/validated chain. `fresh`
    /// reports the adapter's validated-chain freshness observation (the open
    /// ledger's parent-close-time freshness, rippled `endConsensus` parity); the
    /// coordinator owns the rule that `Tracking -> Full` requires both chain
    /// contiguity and a passing freshness policy.
    PublicationCommitted {
        /// The published/validated ledger identity.
        identity: LedgerIdentity,
        /// Whether the published chain is fresh per the app's freshness policy.
        fresh: bool,
    },

    /// The NodeStore database generation rotated. Isolates old and new
    /// `(hash, seq, generation)` reads.
    StoreRotated(StoreGeneration),

    /// A fetch-pack pass added by-hash node data to the shared fetch-pack
    /// cache. A fact, like [`AcquisitionEvent::Connectivity`]: it names no
    /// session. The coordinator re-advances each live session so the traversal
    /// can resolve newly resident by-hash nodes without waiting for a peer
    /// reply (rippled `gotFetchPack` parity).
    FetchPackAvailable,

    /// A periodic heartbeat fact (rippled `processHeartbeatTimer` parity).
    /// Names no session. The coordinator re-publishes its current phase so the
    /// phase port re-applies validated-ledger-age normalization on
    /// `Connected`/`Syncing`; it never changes a session or phase.
    Heartbeat,

    /// The application's configured inbound-ledger registry sweep fired.
    /// Sessions whose one-minute idle minimum elapsed are reaped only on this
    /// global cadence, matching `InboundLedgersImp::sweep`.
    RegistrySweep,

    /// The node is shutting down.
    Shutdown,
}

/// Consensus's preferred-LCL target and the reason acquisition should pursue it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConsensusTarget {
    target: LedgerTarget,
    reason: AcquireReason,
}

impl ConsensusTarget {
    /// Builds a consensus target report.
    pub const fn new(target: LedgerTarget, reason: AcquireReason) -> Self {
        Self { target, reason }
    }

    /// The preferred-LCL target.
    pub const fn target(self) -> LedgerTarget {
        self.target
    }

    /// The acquisition reason.
    pub const fn reason(self) -> AcquireReason {
        self.reason
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::{IdCounter, PeerId};
    use crate::identity::SessionRef;
    use crate::ingress::{AdmissionBudget, AdmissionGate};
    use crate::target::LedgerTarget;
    use basics::base_uint::Uint256;

    #[test]
    fn connectivity_event_carries_usable_peer_fact() {
        let snapshot = PeerAvailabilitySnapshot::new(vec![PeerId::new(1), PeerId::new(2)]);
        let event = AcquisitionEvent::Connectivity(snapshot.clone());
        match &event {
            AcquisitionEvent::Connectivity(s) => assert_eq!(s, &snapshot),
            other => panic!("unexpected event {other:?}"),
        }
    }

    #[test]
    fn consensus_target_event_preserves_reason() {
        let target = LedgerTarget::new(Uint256::from(1), Some(1));
        let ct = ConsensusTarget::new(target, AcquireReason::Consensus);
        assert_eq!(ct.target(), target);
        assert_eq!(ct.reason(), AcquireReason::Consensus);
    }

    #[test]
    fn packet_admitted_event_moves_lease() {
        let mut counter = IdCounter::new();
        let session = SessionRef::new(
            counter.next_id(),
            counter.next_id(),
            Uint256::from(1),
            counter.next_id(),
            counter.next_id(),
        );
        let gate = std::sync::Arc::new(AdmissionGate::new(AdmissionBudget::default(), session));
        let lease = match gate.try_reserve(1, 1) {
            crate::ingress::BackpressureOutcome::Admitted(lease) => lease,
            other => panic!("expected admission, got {other:?}"),
        };
        let packet = AdmittedLedgerPacket::new(
            lease,
            session,
            PeerId::new(1),
            ledger::InboundLedgerPacket::new(
                ledger::InboundLedgerDataType::Base,
                vec![ledger::InboundLedgerNodeData::new(None, vec![9])],
            ),
        )
        .expect("matching lease must admit");
        let event = AcquisitionEvent::PacketAdmitted(packet);
        match &event {
            AcquisitionEvent::PacketAdmitted(p) => {
                assert_eq!(p.peer_id(), PeerId::new(1));
                assert_eq!(p.lease().session(), session);
            }
            other => panic!("unexpected event {other:?}"),
        }
    }
}
