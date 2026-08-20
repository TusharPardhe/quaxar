//! Durable handoff protocol.
//!
//! A ledger must not be adoptable by validation, LCL installation, publication,
//! or the normal resolver before the durability fence passes. Once it passes,
//! the coordinator hands the durable ledger to the LedgerMaster/NetworkOps
//! adapter as a [`DurableLedger`] with a unique [`DurableHandoffId`]; the
//! adapter deduplicates by that id so retries cannot duplicate adoption or
//! publication, and returns a [`DurableHandoffAcknowledgement`] when durable
//! adoption is confirmed.

use std::sync::Arc;

use ledger::Ledger;

use crate::id::DurableHandoffId;
use crate::identity::SessionRef;

/// A durable, fencing-complete ledger handed from the coordinator to the
/// LedgerMaster/NetworkOps adapter. The handoff id is unique per session and
/// deduplicated by the recipient.
#[derive(Debug, Clone)]
pub struct DurableLedger {
    handoff: DurableHandoffId,
    session: SessionRef,
    ledger: Arc<Ledger>,
}

impl PartialEq for DurableLedger {
    fn eq(&self, other: &Self) -> bool {
        // Protocol equality is the handoff identity; the `Ledger` payload is
        // deliberately not compared (it has no `PartialEq`).
        self.handoff == other.handoff && self.session == other.session
    }
}

impl Eq for DurableLedger {}

impl DurableLedger {
    /// Builds a durable handoff.
    pub fn new(handoff: DurableHandoffId, session: SessionRef, ledger: Arc<Ledger>) -> Self {
        Self {
            handoff,
            session,
            ledger,
        }
    }

    /// The unique handoff id used for recipient-side deduplication.
    pub fn handoff(&self) -> DurableHandoffId {
        self.handoff
    }

    /// The session that produced this ledger.
    pub fn session(&self) -> SessionRef {
        self.session
    }

    /// The durable ledger.
    pub fn ledger(&self) -> &Arc<Ledger> {
        &self.ledger
    }
}

/// The recipient's confirmation that a durable handoff was adopted. The
/// coordinator owns retry timing and delivery state until it receives this;
/// a retried delivery carries the same handoff id and is idempotent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DurableHandoffAcknowledgement {
    handoff: DurableHandoffId,
    session: SessionRef,
}

impl DurableHandoffAcknowledgement {
    /// Builds an acknowledgement.
    pub const fn new(handoff: DurableHandoffId, session: SessionRef) -> Self {
        Self { handoff, session }
    }

    /// The acknowledged handoff id.
    pub const fn handoff(self) -> DurableHandoffId {
        self.handoff
    }

    /// The session that produced the handoff.
    pub const fn session(self) -> SessionRef {
        self.session
    }
}

/// Why the adapter could not deliver a durable handoff.
///
/// A rejected delivery leaves the handoff unpublished, so the coordinator
/// re-emits `PublishDurable` with the same id until the recipient acknowledges;
/// the recipient's id-deduplication makes every retry idempotent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoffRejectReason {
    /// The completed-ledger channel is full; retry when it drains.
    ChannelFull,
    /// The recipient is gone; retry is harmless until shutdown.
    Disconnected,
    /// The recipient could not accept/process an already enqueued handoff.
    /// The port reopens that exact handoff before the coordinator retries it.
    RecipientRejected,
}

impl HandoffRejectReason {
    /// A stable label for tracing.
    pub const fn label(self) -> &'static str {
        match self {
            Self::ChannelFull => "channel_full",
            Self::Disconnected => "disconnected",
            Self::RecipientRejected => "recipient_rejected",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::{IdCounter, StoreGeneration};
    use crate::phase::{SyncPhase, TransitionFact, phase_transition};
    use basics::base_uint::Uint256;

    #[test]
    fn handoff_identity_is_unique_per_session() {
        let mut counter = IdCounter::new();
        let session_a = SessionRef::new(
            counter.next_id(),
            counter.next_id(),
            Uint256::from(1),
            counter.next_id(),
            StoreGeneration::INVALID,
        );
        let session_b = SessionRef::new(
            counter.next_id(),
            counter.next_id(),
            Uint256::from(2),
            counter.next_id(),
            StoreGeneration::INVALID,
        );
        let a = DurableHandoffId::new(counter.next_id::<u64>());
        let b = DurableHandoffId::new(counter.next_id::<u64>());
        assert_ne!(a, b);
        let ack = DurableHandoffAcknowledgement::new(a, session_a);
        assert_eq!(ack.handoff(), a);
        assert_eq!(ack.session(), session_a);
        assert_ne!(ack.session(), session_b);
    }

    #[test]
    fn a_durable_completion_is_the_installed_lcl_fact() {
        // The durable path feeds the TargetInstalledAsLcl fact: a completed
        // acquisition becomes the tracking LCL.
        let target = crate::target::LedgerTarget::new(Uint256::from(1), Some(1));
        let lcl = crate::target::LedgerIdentity::new(Uint256::from(1), 1);
        let syncing = SyncPhase::Syncing { target };
        let next = phase_transition(&syncing, &TransitionFact::TargetInstalledAsLcl { lcl });
        assert_eq!(next, Some(SyncPhase::Tracking { lcl }));
    }

    #[test]
    fn reject_reason_labels_are_stable() {
        assert_eq!(HandoffRejectReason::ChannelFull.label(), "channel_full");
        assert_eq!(HandoffRejectReason::Disconnected.label(), "disconnected");
        assert_eq!(
            HandoffRejectReason::RecipientRejected.label(),
            "recipient_rejected"
        );
        assert_ne!(
            HandoffRejectReason::ChannelFull,
            HandoffRejectReason::Disconnected
        );
    }
}
