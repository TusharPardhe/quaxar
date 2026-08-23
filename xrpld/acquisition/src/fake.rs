//! Deterministic fake ports for unit tests and shadow-mode diagnostics.
//!
//! Every port records its effect stream in plain `Vec`s so tests can assert on
//! exact coordinator outputs without timers, threads, or real storage.

use crate::handoff::DurableLedger;
use crate::identity::{OperationRef, SessionRef};
use crate::io::{ReadRequest, WriteBatch};
use crate::peer::PeerRequest;
use crate::phase::SyncPhase;
use crate::port::{
    CancellationPort, HandoffPort, LedgerRequestPort, PhasePort, ReadPort, TimerPort, WritePort,
};
use crate::timer::TimerRequest;

/// Records every peer request the coordinator produced.
#[derive(Debug, Default)]
pub struct FakeLedgerRequestPort {
    /// Requests in emission order.
    pub sent: Vec<PeerRequest>,
}

impl FakeLedgerRequestPort {
    /// A fresh, empty fake port.
    pub fn new() -> Self {
        Self::default()
    }
}

impl LedgerRequestPort for FakeLedgerRequestPort {
    fn send_ledger_request(&mut self, request: PeerRequest) {
        self.sent.push(request);
    }
}

/// Records every read the coordinator submitted.
#[derive(Debug, Default)]
pub struct FakeReadPort {
    /// Reads in submission order.
    pub submitted: Vec<ReadRequest>,
    /// Sessions whose retained reads were upgraded after exact-owner binding.
    pub promoted_sessions: Vec<SessionRef>,
}

impl FakeReadPort {
    /// A fresh, empty fake port.
    pub fn new() -> Self {
        Self::default()
    }
}

impl ReadPort for FakeReadPort {
    fn submit_read(&mut self, request: ReadRequest) {
        self.submitted.push(request);
    }

    fn promote_session_priority(&mut self, session: SessionRef) -> usize {
        self.promoted_sessions.push(session);
        0
    }
}

/// Records every write batch the coordinator submitted.
#[derive(Debug, Default)]
pub struct FakeWritePort {
    /// Batches in submission order.
    pub submitted: Vec<WriteBatch>,
}

impl FakeWritePort {
    /// A fresh, empty fake port.
    pub fn new() -> Self {
        Self::default()
    }
}

impl WritePort for FakeWritePort {
    fn submit_write(&mut self, batch: WriteBatch) {
        self.submitted.push(batch);
    }
}

/// Records every timer arm and disarm.
#[derive(Debug, Default)]
pub struct FakeTimerPort {
    /// Armed timers in arming order.
    pub armed: Vec<TimerRequest>,
    /// Disarmed operations in disarming order.
    pub disarmed: Vec<OperationRef>,
}

impl FakeTimerPort {
    /// A fresh, empty fake port.
    pub fn new() -> Self {
        Self::default()
    }
}

impl TimerPort for FakeTimerPort {
    fn arm(&mut self, request: TimerRequest) {
        self.armed.push(request);
    }

    fn disarm(&mut self, operation: OperationRef) {
        self.disarmed.push(operation);
    }
}

/// Records every durable ledger published.
#[derive(Debug, Default)]
pub struct FakeHandoffPort {
    /// Published durable ledgers in handoff order.
    pub published: Vec<DurableLedger>,
}

impl FakeHandoffPort {
    /// A fresh, empty fake port.
    pub fn new() -> Self {
        Self::default()
    }
}

impl HandoffPort for FakeHandoffPort {
    fn publish_durable(&mut self, ledger: DurableLedger) {
        self.published.push(ledger);
    }
}

/// Records every service-phase publication.
#[derive(Debug, Default)]
pub struct FakePhasePort {
    /// Phases in publication order.
    pub phases: Vec<SyncPhase>,
}

impl FakePhasePort {
    /// A fresh, empty fake port.
    pub fn new() -> Self {
        Self::default()
    }
}

impl PhasePort for FakePhasePort {
    fn set_phase(&mut self, phase: SyncPhase) {
        self.phases.push(phase);
    }
}

/// Records every session cancellation notification.
#[derive(Debug, Default)]
pub struct FakeCancellationPort {
    /// Cancelled sessions in notification order.
    pub cancelled: Vec<SessionRef>,
}

impl FakeCancellationPort {
    /// A fresh, empty fake port.
    pub fn new() -> Self {
        Self::default()
    }
}

impl CancellationPort for FakeCancellationPort {
    fn session_cancelled(&mut self, session: SessionRef) {
        self.cancelled.push(session);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::{PlanEpoch, RunEpoch, SessionId, StoreGeneration};
    use crate::identity::SessionRef;
    use crate::port::CoordinatorPorts;

    #[test]
    fn dispatch_routes_each_effect_to_its_port() {
        let mut requests = FakeLedgerRequestPort::new();
        let mut reads = FakeReadPort::new();
        let mut writes = FakeWritePort::new();
        let mut timers = FakeTimerPort::new();
        let mut handoffs = FakeHandoffPort::new();
        let mut phase = FakePhasePort::new();
        let mut cancellations = FakeCancellationPort::new();

        {
            let mut ports = CoordinatorPorts {
                requests: &mut requests,
                reads: &mut reads,
                writes: &mut writes,
                timers: &mut timers,
                handoffs: &mut handoffs,
                phase: &mut phase,
                cancellations: &mut cancellations,
            };

            ports.dispatch(crate::AcquisitionEffect::SetServicePhase(
                SyncPhase::Connected,
            ));
            ports.dispatch(crate::AcquisitionEffect::CancelSession(SessionRef::new(
                RunEpoch::new(1),
                SessionId::new(1),
                basics::base_uint::Uint256::from(1),
                PlanEpoch::new(1),
                StoreGeneration::new(1),
            )));
        }

        assert_eq!(phase.phases, vec![SyncPhase::Connected]);
        assert_eq!(cancellations.cancelled.len(), 1);
        assert!(reads.submitted.is_empty());
        assert!(writes.submitted.is_empty());
        assert!(timers.armed.is_empty());
        assert!(handoffs.published.is_empty());
        assert!(requests.sent.is_empty());
    }
}
