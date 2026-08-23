//! Typed effects emitted by the coordinator.
//!
//! The coordinator changes session/lifecycle state first, then emits effects
//! after state mutation. Adapters execute effects outside coordinator
//! ownership and return typed completions. No peer send is emitted after a
//! terminal transition.

use crate::handoff::DurableLedger;
use crate::identity::SessionRef;
use crate::io::{ReadRequest, WriteBatch};
use crate::peer::PeerRequest;
use crate::phase::SyncPhase;
use crate::timer::TimerRequest;

/// A typed output effect for an adapter port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcquisitionEffect {
    /// Bind app-side delivery metadata to an exact newly live session. This is
    /// emitted before the session's first peer request, so even a synchronous
    /// loopback reply cannot complete before the adapter knows its identity.
    SessionStarted(SessionRef),

    /// Send a ledger-data request to a peer (overlay owns delivery).
    SendLedgerRequest(PeerRequest),

    /// Submit a brokered NodeStore read.
    SubmitRead(ReadRequest),

    /// Submit a NodeStore write batch.
    SubmitWrite(WriteBatch),

    /// Arm (or rearm) a timer.
    ArmTimer(TimerRequest),

    /// Publish a durable, fencing-complete ledger to the LedgerMaster/NetworkOps
    /// adapter.
    PublishDurable(DurableLedger),

    /// Publish the service phase to the phase port.
    SetServicePhase(SyncPhase),

    /// Notify adapters that a session was cancelled and its external
    /// operations are invalidated.
    CancelSession(SessionRef),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::{IdCounter, PlanEpoch, RunEpoch, SessionId, StoreGeneration};
    use crate::identity::OperationRef;
    use crate::peer::LedgerDataRequest;
    use basics::base_uint::Uint256;

    #[test]
    fn effects_are_typed_and_comparable() {
        let mut counter = IdCounter::new();
        let session = SessionRef::new(
            RunEpoch::new(1),
            SessionId::new(1),
            Uint256::from(1),
            PlanEpoch::new(1),
            StoreGeneration::new(1),
        );
        let phase = SyncPhase::Connected;
        let a = AcquisitionEffect::SetServicePhase(phase);
        assert_eq!(a, AcquisitionEffect::SetServicePhase(SyncPhase::Connected));
        assert_ne!(
            a,
            AcquisitionEffect::SetServicePhase(SyncPhase::Disconnected)
        );

        let request = PeerRequest::new(
            session,
            OperationRef::new(
                session,
                crate::identity::OperationKind::PeerRequest,
                counter.next_id(),
                counter.next_id(),
            ),
            crate::id::PeerId::new(1),
            LedgerDataRequest::GetLedger { sequence: Some(1) },
        );
        let b = AcquisitionEffect::SendLedgerRequest(request.clone());
        match &b {
            AcquisitionEffect::SendLedgerRequest(r) => assert_eq!(r, &request),
            other => panic!("unexpected effect {other:?}"),
        }
    }
}
