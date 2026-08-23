//! Adapter port traits.
//!
//! Ports are the execution layer: each adapter owns its resource-local state
//! and executes effects outside coordinator ownership. Completions return as
//! typed events and never invoke a lifecycle closure while holding an adapter
//! lock.

use crate::effect::AcquisitionEffect;
use crate::handoff::DurableLedger;
use crate::identity::{OperationRef, SessionRef};
use crate::io::{ReadRequest, WriteBatch};
use crate::peer::{PeerRequest, PeerTargetCapability};
use crate::phase::SyncPhase;
use crate::target::LedgerTarget;
use crate::timer::TimerRequest;

/// Overlay-owned delivery of a coordinator-produced peer request.
pub trait LedgerRequestPort {
    /// Deliver a peer request. Never called after a terminal transition.
    fn send_ledger_request(&mut self, request: PeerRequest);

    /// Sample peers that currently advertise the exact ledger target. `None`
    /// preserves deterministic/legacy ports that cannot expose overlay
    /// capability; production returns `Some`, including an empty sample.
    fn peer_target_capabilities(&self, _target: LedgerTarget) -> Option<Vec<PeerTargetCapability>> {
        None
    }
}

/// Brokered NodeStore read submission. The broker owns read admission,
/// coalescing, dispatch, settlement, and cancellation.
pub trait ReadPort {
    /// Submit a physical read. The broker reports a typed `ReadCompletion`.
    fn submit_read(&mut self, request: ReadRequest);

    /// Upgrade retained reads when an existing Generic session becomes an
    /// exact consensus/recovery owner after admission. Fakes and ports without
    /// a retained priority queue have nothing to reclassify.
    fn promote_session_priority(&mut self, _session: SessionRef) -> usize {
        0
    }

    /// Retry retained exact read completions from the coordinator owner turn.
    /// Deterministic fakes have no cross-thread completion queue.
    fn flush_completions(&mut self) {}
}

/// NodeStore write submission. The adapter owns physical write submission only
/// and reports typed write/durability-fence completions.
pub trait WritePort {
    /// Submit a write batch.
    fn submit_write(&mut self, batch: WriteBatch);

    /// Retry bounded delivery of retained write and fence completions. The
    /// default is a no-op for deterministic fakes; production adapters use it
    /// from the coordinator owner turn so a full control queue never blocks a
    /// NodeStore worker or reorders an exact completion pair.
    fn flush_completions(&mut self) {}
}

/// Timer ownership. Timer threads never run session logic; wakeups return as
/// typed `TimerFired` events carrying the exact arming operation.
pub trait TimerPort {
    /// Arm (or rearm) a timer.
    fn arm(&mut self, request: TimerRequest);
    /// Disarm a pending timer for an operation.
    fn disarm(&mut self, operation: OperationRef);

    /// Retry retained exact timer wakeups from the coordinator owner turn.
    fn flush_completions(&mut self) {}
}

/// Durable handoff delivery. The recipient deduplicates by `DurableHandoffId`
/// and acknowledges via `DurableHandoffAcknowledged`.
pub trait HandoffPort {
    /// Bind adapter-local delivery metadata to a newly live exact session.
    /// The default is a no-op because generic handoff implementations may not
    /// need app-specific origin metadata.
    fn session_started(&mut self, _session: SessionRef) {}

    /// Deliver a durable, fencing-complete ledger to LedgerMaster/NetworkOps.
    fn publish_durable(&mut self, ledger: DurableLedger);
}

/// Service-phase publication. The coordinator is the only production writer.
pub trait PhasePort {
    /// Publish the current service phase.
    fn set_phase(&mut self, phase: SyncPhase);
}

/// Notification that a session was cancelled and its external operations are
/// invalidated (e.g. to release admission gates and drop leases).
pub trait CancellationPort {
    /// Report session cancellation to adapters.
    fn session_cancelled(&mut self, session: SessionRef);
}

/// A bundle of all ports. Adapters execute each effect outside coordinator
/// ownership; a mutex inside a narrow port implementation is acceptable only
/// for that port's resource-local state.
pub struct CoordinatorPorts<'a> {
    /// Peer request delivery.
    pub requests: &'a mut dyn LedgerRequestPort,
    /// Brokered reads.
    pub reads: &'a mut dyn ReadPort,
    /// NodeStore writes.
    pub writes: &'a mut dyn WritePort,
    /// Timers.
    pub timers: &'a mut dyn TimerPort,
    /// Durable handoffs.
    pub handoffs: &'a mut dyn HandoffPort,
    /// Service phase.
    pub phase: &'a mut dyn PhasePort,
    /// Cancellation notifications.
    pub cancellations: &'a mut dyn CancellationPort,
}

impl CoordinatorPorts<'_> {
    /// Dispatches one typed effect to the owning port. Keeps the effect enum
    /// the single typed output surface and the ports the execution layer.
    pub fn dispatch(&mut self, effect: AcquisitionEffect) {
        match effect {
            AcquisitionEffect::SessionStarted(session) => self.handoffs.session_started(session),
            AcquisitionEffect::SendLedgerRequest(request) => {
                self.requests.send_ledger_request(request)
            }
            AcquisitionEffect::SubmitRead(request) => self.reads.submit_read(request),
            AcquisitionEffect::SubmitWrite(batch) => self.writes.submit_write(batch),
            AcquisitionEffect::ArmTimer(request) => self.timers.arm(request),
            AcquisitionEffect::PublishDurable(ledger) => self.handoffs.publish_durable(ledger),
            AcquisitionEffect::SetServicePhase(phase) => self.phase.set_phase(phase),
            AcquisitionEffect::CancelSession(session) => {
                self.cancellations.session_cancelled(session)
            }
        }
    }
}
