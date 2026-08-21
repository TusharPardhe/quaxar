//! M4.2-C production handoff port: durable coordinator handoffs onto the
//! LedgerMaster/NetworkOps completed-ledger channel.
//!
//! The coordinator is the single producer of durable acquisitions. This port
//! adapts an [`acquisition::DurableLedger`] into the existing
//! [`CompletedInboundLedger`] delivery that LedgerMaster consumes, and is the
//! recipient-side deduplication point required by the durable handoff protocol:
//! a retried delivery of the same
//! [`DurableHandoffId`] cannot duplicate adoption or publication.
//!
//! Ownership: the port owns only delivery-local state — the set of successfully
//! published handoff ids and the session-origin map that converts a handoff into
//! a [`CompletedInboundLedger`]. It never mutates session lifecycle state and
//! never invokes coordinator logic.
//!
//! Delivery pressure: a `try_send` `Full`/`Disconnected` outcome leaves the
//! handoff unpublished and reports a typed [`AcquisitionEvent::DurableHandoffRejected`]
//! to the coordinator, which owns retry timing and re-publishes the exact same
//! handoff id. Only a successful enqueue records the id, making later retries
//! idempotent drops at this port.
//!
//! Parity note: rippled's non-validating `storeLedger` and history-only
//! `setFullLedger` paths are distinguished by the acquisition origin retained on
//! the completed ledger (`rippled/src/xrpld/app/ledger/detail/InboundLedger.cpp`).
//! The app records that origin per coordinator session through
//! [`CoordinatorHandoffPort::register_session`]; the port replays it onto the
//! delivered record exactly once per handoff.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{SyncSender, TrySendError};

use acquisition::{
    AcquisitionEvent, DurableHandoffId, DurableLedger, HandoffPort, HandoffRejectReason, SessionRef,
};

use super::registry::{AcquireReason, CompletedInboundLedger};

/// Delivery statistics for tracing. Mirrors the required durable-handoff
/// observability: delivered, deduplicated retries, stale sessions, and the
/// pressure the coordinator must retry against.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[allow(dead_code)] // production wiring lands in M4.2-C2/C3
pub struct HandoffPortStats {
    /// Handoffs successfully enqueued for LedgerMaster.
    pub delivered: u64,
    /// Retried deliveries of an already-published handoff id, dropped
    /// idempotently at this port.
    pub duplicated: u64,
    /// Handoffs for sessions the app never registered, dropped as stale.
    pub unknown_session: u64,
    /// `try_send` full: the handoff stays unpublished and retryable.
    pub channel_full: u64,
    /// `try_send` disconnected: LedgerMaster is gone; the handoff stays
    /// unpublished and retryable.
    pub disconnected: u64,
}

/// Exact identity of app-side provenance retained until the coordinator emits
/// its corresponding `SessionStarted`. A target hash alone never authorizes a
/// binding: callers must clear the same target, reason, and acquisition id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct DeferredDemandKey {
    target: basics::base_uint::Uint256,
    reason: AcquireReason,
    acquisition_id: u64,
    preferred_target: bool,
}

/// The coordinator retains at most two peerless demand classes: the latest
/// preferred target and one ordinary/cache target. Keep
/// the matching app-side origins in the same bounded shape so a lower-priority
/// demand for another target cannot overwrite the retained preferred origin.
const MAX_PENDING_SESSION_ORIGINS: usize = 2;

/// The production [`HandoffPort`]: delivers durable ledgers to the
/// LedgerMaster/NetworkOps completed-ledger channel with id-deduplication.
/// A successful enqueue only establishes delivery to the recipient queue; the
/// recipient bridge returns `DurableHandoffAcknowledged` after it has processed
/// this exact item.
#[allow(dead_code)] // production wiring lands in M4.2-C2/C3
pub struct CoordinatorHandoffPort {
    tx: SyncSender<CompletedInboundLedger>,
    events: SyncSender<AcquisitionEvent>,
    published: HashSet<DurableHandoffId>,
    /// Origin metadata is keyed by the complete session identity. A reused
    /// `SessionId` from another run, plan, target, or store generation must
    /// never authorize a stale durable handoff.
    sessions: HashMap<SessionRef, (AcquireReason, u64)>,
    /// Bounded app-origin bindings awaiting exact `SessionStarted` effects.
    /// The key includes target, reason, and acquisition id. This mirrors the
    /// coordinator's one preferred plus one peerless ordinary demand,
    /// and preserves preferred provenance across rejected lower-priority work
    /// even when that work targets a different ledger.
    pending_session_origins: HashMap<DeferredDemandKey, ()>,
    stats: HandoffPortStats,
}

impl CoordinatorHandoffPort {
    /// Builds a port over the completed-ledger channel LedgerMaster consumes.
    /// Rejected deliveries report through `events`; recipient acceptance is
    /// acknowledged separately by the registry bridge.
    #[allow(dead_code)] // production wiring lands in M4.2-C2/C3
    pub fn new(
        tx: SyncSender<CompletedInboundLedger>,
        events: SyncSender<AcquisitionEvent>,
    ) -> Self {
        Self {
            tx,
            events,
            published: HashSet::new(),
            sessions: HashMap::new(),
            pending_session_origins: HashMap::new(),
            stats: HandoffPortStats::default(),
        }
    }

    /// Records app-side origin before an acquire demand is submitted. The
    /// coordinator retains only its latest preferred demand and one ordinary
    /// peerless demand, so this map uses the same two-slot bound. Replacing an
    /// ordinary origin never touches preferred provenance, including across
    /// different targets.
    pub(crate) fn register_pending_session_origin(
        &mut self,
        target: basics::base_uint::Uint256,
        reason: AcquireReason,
        acquisition_id: u64,
        preferred_target: bool,
    ) {
        // An ordinary follow-up for the exact preferred target coalesces into
        // that preferred runner slot and must not replace its app provenance.
        if !preferred_target
            && self
                .pending_session_origins
                .keys()
                .any(|pending| pending.preferred_target && pending.target == target)
        {
            return;
        }
        let key = DeferredDemandKey {
            target,
            reason,
            acquisition_id,
            preferred_target,
        };
        if preferred_target {
            // Promotion of an exact ordinary target consumes that ordinary
            // class just as `retain_peerless_acquire` removes it before
            // inserting the preferred entry.
            self.pending_session_origins
                .retain(|pending, ()| !pending.preferred_target && pending.target != target);
        } else {
            self.pending_session_origins
                .retain(|pending, ()| pending.preferred_target);
        }
        self.pending_session_origins.insert(key, ());
        debug_assert!(self.pending_session_origins.len() <= MAX_PENDING_SESSION_ORIGINS);
    }

    /// Clears only the exact pending demand that did not create a session.
    /// A stale lower-priority caller cannot clear consensus provenance awaiting
    /// replay, even when it targets a different ledger.
    pub(crate) fn clear_pending_session_origin(
        &mut self,
        target: basics::base_uint::Uint256,
        reason: AcquireReason,
        acquisition_id: u64,
        preferred_target: bool,
    ) {
        self.pending_session_origins.remove(&DeferredDemandKey {
            target,
            reason,
            acquisition_id,
            preferred_target,
        });
    }

    /// Records the app-side origin of one live exact session so a later durable
    /// handoff can be converted into a [`CompletedInboundLedger`]. Re-registering
    /// the exact complete session identity replaces its origin; a stale handoff
    /// for any other session reference is dropped and counted.
    #[allow(dead_code)] // production wiring lands in M4.2-C2/C3
    pub fn register_session(
        &mut self,
        session: SessionRef,
        reason: AcquireReason,
        acquisition_id: u64,
    ) {
        self.sessions.insert(session, (reason, acquisition_id));
    }

    /// Reopens an enqueued handoff after the recipient could not accept it.
    /// The caller supplies the complete session identity; a stale rejection
    /// cannot remove deduplication for another run or session.
    pub(crate) fn reopen_after_recipient_rejection(
        &mut self,
        handoff: DurableHandoffId,
        session: SessionRef,
    ) -> bool {
        if !self.sessions.contains_key(&session) {
            self.stats.unknown_session += 1;
            return false;
        }
        self.published.remove(&handoff)
    }

    /// Current delivery statistics.
    #[allow(dead_code)] // production wiring lands in M4.2-C2/C3
    pub const fn stats(&self) -> HandoffPortStats {
        self.stats
    }
}

impl HandoffPort for CoordinatorHandoffPort {
    fn session_started(&mut self, session: SessionRef) {
        // Preferred policy wins an otherwise ambiguous same-target binding:
        // an ordinary follow-up coalesces into that exact retained runner slot.
        let pending = self
            .pending_session_origins
            .keys()
            .copied()
            .find(|pending| pending.target == session.target_hash() && pending.preferred_target)
            .or_else(|| {
                self.pending_session_origins
                    .keys()
                    .copied()
                    .find(|pending| pending.target == session.target_hash())
            });
        let Some(pending) = pending else {
            return;
        };
        self.pending_session_origins.remove(&pending);
        self.register_session(session, pending.reason, pending.acquisition_id);
    }

    fn publish_durable(&mut self, ledger: DurableLedger) {
        if self.published.contains(&ledger.handoff()) {
            self.stats.duplicated += 1;
            return;
        }
        let Some(&(reason, acquisition_id)) = self.sessions.get(&ledger.session()) else {
            self.stats.unknown_session += 1;
            return;
        };
        let item = CompletedInboundLedger {
            ledger: ledger.ledger().clone(),
            reason,
            acquisition_id,
            from_coordinator: true,
            durable_handoff: Some(ledger.handoff()),
            coordinator_session: Some(ledger.session()),
        };
        match self.tx.try_send(item) {
            Ok(()) => {
                // Enqueue is delivery, not recipient acceptance. Keep the id
                // deduplicated while the recipient processes it; a retry of
                // this exact handoff must not create a second adoption pass.
                self.published.insert(ledger.handoff());
                self.stats.delivered += 1;
            }
            Err(TrySendError::Full(_)) => {
                self.stats.channel_full += 1;
                self.reject(ledger, HandoffRejectReason::ChannelFull);
            }
            Err(TrySendError::Disconnected(_)) => {
                self.stats.disconnected += 1;
                self.reject(ledger, HandoffRejectReason::Disconnected);
            }
        }
    }
}

impl CoordinatorHandoffPort {
    /// Reports a rejected delivery to the coordinator. The coordinator keeps
    /// delivery ownership and re-publishes the exact pending handoff id; the
    /// rejection event is itself a typed event on the same channel the adapter
    /// drains, so delivery retry cadence is bounded by the strand turn, never a
    /// hot loop.
    fn reject(&self, ledger: DurableLedger, reason: HandoffRejectReason) {
        let rejected = AcquisitionEvent::DurableHandoffRejected {
            handoff: ledger.handoff(),
            session: ledger.session(),
            reason,
        };
        let _ = self.events.send(rejected);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::mpsc;

    use basics::base_uint::Uint256;
    use basics::sha_map_hash::SHAMapHash;
    use ledger::{LEDGER_DEFAULT_TIME_RESOLUTION, Ledger, LedgerHeader};

    use std::time::Duration;

    use acquisition::AcquireReason as CoordinatorAcquireReason;
    use acquisition::{
        AcquisitionEffect, AcquisitionEvent, AdmissionBudget, BudgetState, CoordinatorRunner,
        DurableLedger, IdCounter, LedgerTarget, PeerAvailabilitySnapshot, PeerId, RunEpoch,
        SessionRef, StoreGeneration,
    };

    use super::*;

    fn immutable_ledger(seq: u32, hash_seed: u8) -> Arc<Ledger> {
        let mut header = LedgerHeader {
            seq,
            close_time: 500 + seq,
            close_time_resolution: LEDGER_DEFAULT_TIME_RESOLUTION,
            ..LedgerHeader::default()
        };
        header.hash = SHAMapHash::new(Uint256::from_array([hash_seed; 32]));
        let mut ledger = Ledger::new(header, false);
        ledger.set_immutable(true);
        Arc::new(ledger)
    }

    fn session(counter: &mut IdCounter) -> SessionRef {
        SessionRef::new(
            counter.next_id(),
            counter.next_id(),
            Uint256::from(1),
            counter.next_id(),
            StoreGeneration::new(1),
        )
    }

    fn durable_handoff(
        counter: &mut IdCounter,
        session: SessionRef,
        ledger: Arc<Ledger>,
    ) -> DurableLedger {
        DurableLedger::new(counter.next_id(), session, ledger)
    }

    fn drain(rx: &mpsc::Receiver<CompletedInboundLedger>) -> Vec<CompletedInboundLedger> {
        rx.try_iter().collect()
    }

    #[test]
    fn pending_origin_binds_only_the_exact_started_session_before_delivery() {
        let (tx, rx) = mpsc::sync_channel(1);
        let (events, _) = mpsc::sync_channel(256);
        let mut port = CoordinatorHandoffPort::new(tx, events);
        let mut counter = IdCounter::new();
        let session = session(&mut counter);
        let other_session = SessionRef::new(
            counter.next_id(),
            counter.next_id(),
            Uint256::from(2),
            counter.next_id(),
            StoreGeneration::new(1),
        );
        let ledger = immutable_ledger(7, 1);

        port.register_pending_session_origin(
            session.target_hash(),
            AcquireReason::Consensus,
            99,
            true,
        );
        port.session_started(other_session);
        port.publish_durable(durable_handoff(
            &mut counter,
            other_session,
            Arc::clone(&ledger),
        ));
        assert!(drain(&rx).is_empty());
        assert_eq!(port.stats().unknown_session, 1);

        // SessionStarted is dispatched before the peer request; only this
        // complete identity consumes the pending origin and may hand off.
        port.session_started(session);
        port.publish_durable(durable_handoff(&mut counter, session, Arc::clone(&ledger)));
        let delivered = drain(&rx);
        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0].reason, AcquireReason::Consensus);
        assert_eq!(delivered[0].acquisition_id, 99);
    }

    #[test]
    fn deferred_consensus_origin_survives_same_hash_lower_priority_replay_and_clear() {
        let (tx, rx) = mpsc::sync_channel(1);
        let (events, _) = mpsc::sync_channel(256);
        let mut port = CoordinatorHandoffPort::new(tx, events);
        let mut counter = IdCounter::new();
        let session = session(&mut counter);

        port.register_pending_session_origin(
            session.target_hash(),
            AcquireReason::Consensus,
            41,
            true,
        );
        port.register_pending_session_origin(
            session.target_hash(),
            AcquireReason::Generic,
            42,
            false,
        );
        port.clear_pending_session_origin(session.target_hash(), AcquireReason::Generic, 42, false);
        port.session_started(session);
        port.publish_durable(durable_handoff(
            &mut counter,
            session,
            immutable_ledger(7, 1),
        ));

        let delivered = drain(&rx);
        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0].reason, AcquireReason::Consensus);
        assert_eq!(delivered[0].acquisition_id, 41);
    }

    #[test]
    fn peerless_preferred_origin_survives_cross_target_validation_reconnect() {
        let (tx, rx) = mpsc::sync_channel(2);
        let (events, _) = mpsc::sync_channel(256);
        let mut port = CoordinatorHandoffPort::new(tx, events);
        let mut runner = CoordinatorRunner::new(RunEpoch::new(1));
        let preferred = LedgerTarget::new(Uint256::from(1), Some(1));
        let validation = LedgerTarget::new(Uint256::from(2), Some(2));

        port.register_pending_session_origin(preferred.hash(), AcquireReason::Consensus, 41, true);
        assert!(
            runner
                .handle_event(AcquisitionEvent::ConsensusTarget(
                    acquisition::ConsensusTarget::new(
                        preferred,
                        CoordinatorAcquireReason::Consensus,
                    ),
                ))
                .is_empty()
        );

        // Validation work is Consensus-reasoned but belongs to the ordinary
        // peerless class. It must not evict preferred recovery provenance.
        port.register_pending_session_origin(
            validation.hash(),
            AcquireReason::Consensus,
            42,
            false,
        );
        assert!(
            runner
                .handle_event(AcquisitionEvent::ValidationTarget(validation))
                .is_empty()
        );
        assert_eq!(port.pending_session_origins.len(), 2);

        let replay = runner.handle_event(AcquisitionEvent::Connectivity(
            PeerAvailabilitySnapshot::new(vec![PeerId::new(1)]),
        ));
        let sessions = replay
            .iter()
            .filter_map(|effect| match effect {
                AcquisitionEffect::SessionStarted(session) => Some(*session),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(sessions.len(), 2);
        for session in &sessions {
            port.session_started(*session);
        }

        let mut counter = IdCounter::new();
        for session in sessions {
            let seq = if session.target_hash() == preferred.hash() {
                1
            } else {
                2
            };
            port.publish_durable(durable_handoff(
                &mut counter,
                session,
                immutable_ledger(seq, seq as u8),
            ));
        }
        let mut delivered = drain(&rx)
            .into_iter()
            .map(|item| item.acquisition_id)
            .collect::<Vec<_>>();
        delivered.sort_unstable();
        assert_eq!(delivered, vec![41, 42]);
        assert_eq!(port.stats().unknown_session, 0);
    }

    #[test]
    fn capacity_deferred_consensus_origin_survives_cross_target_rejection_and_replay() {
        let (tx, rx) = mpsc::sync_channel(1);
        let (events, _) = mpsc::sync_channel(256);
        let mut port = CoordinatorHandoffPort::new(tx, events);
        let budget = BudgetState::new(1, AdmissionBudget::new(4, 1024), Duration::from_secs(1));
        let mut runner = CoordinatorRunner::with_budget(RunEpoch::new(1), budget);
        let consensus_a = LedgerTarget::new(Uint256::from(1), Some(1));
        let consensus_b = LedgerTarget::new(Uint256::from(2), Some(2));
        let generic_a = LedgerTarget::new(Uint256::from(3), Some(3));

        let _ = runner.handle_event(AcquisitionEvent::Connectivity(
            PeerAvailabilitySnapshot::new(vec![PeerId::new(1)]),
        ));
        let _ = runner.handle_event(AcquisitionEvent::ConsensusTarget(
            acquisition::ConsensusTarget::new(consensus_a, CoordinatorAcquireReason::Consensus),
        ));

        // B is retained by the coordinator because A occupies its only slot.
        // Preserve B's exact app origin before submitting that deferred demand.
        port.register_pending_session_origin(
            consensus_b.hash(),
            AcquireReason::Consensus,
            41,
            true,
        );
        let deferred = runner.handle_event(AcquisitionEvent::ConsensusTarget(
            acquisition::ConsensusTarget::new(consensus_b, CoordinatorAcquireReason::Consensus),
        ));
        assert!(
            !deferred
                .iter()
                .any(|effect| matches!(effect, AcquisitionEffect::SessionStarted(_)))
        );
        assert!(runner.has_deferred_consensus_target(consensus_b));

        // A lower-priority request for a *different* target is rejected while
        // B reserves the next free slot. Its exact clear must not remove B.
        port.register_pending_session_origin(generic_a.hash(), AcquireReason::Generic, 42, false);
        let rejected = runner.handle_event(AcquisitionEvent::AcquireRequested {
            target: generic_a,
            reason: CoordinatorAcquireReason::Generic,
        });
        assert!(
            !rejected
                .iter()
                .any(|effect| matches!(effect, AcquisitionEffect::SessionStarted(_)))
        );
        port.clear_pending_session_origin(generic_a.hash(), AcquireReason::Generic, 42, false);
        assert_eq!(port.pending_session_origins.len(), 1);

        // Store rotation terminalizes the occupied session and replays the
        // coordinator-owned B demand. Its original consensus origin binds the
        // exact replayed session and recognizes its durable handoff.
        let replay = runner.handle_event(AcquisitionEvent::StoreRotated(StoreGeneration::new(2)));
        let replayed = replay
            .iter()
            .find_map(|effect| match effect {
                AcquisitionEffect::SessionStarted(session) => Some(*session),
                _ => None,
            })
            .expect("free capacity must replay the retained consensus target");
        assert_eq!(replayed.target_hash(), consensus_b.hash());
        port.session_started(replayed);

        let mut counter = IdCounter::new();
        port.publish_durable(durable_handoff(
            &mut counter,
            replayed,
            immutable_ledger(2, 2),
        ));
        let delivered = drain(&rx);
        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0].reason, AcquireReason::Consensus);
        assert_eq!(delivered[0].acquisition_id, 41);
        assert_eq!(port.stats().unknown_session, 0);
    }

    #[test]
    fn durable_handoff_delivers_one_completed_inbound_ledger_with_origin() {
        let (tx, rx) = mpsc::sync_channel(1);
        let (events, events_rx) = mpsc::sync_channel(256);
        let mut port = CoordinatorHandoffPort::new(tx, events);
        let mut counter = IdCounter::new();
        let session = session(&mut counter);
        let ledger = immutable_ledger(7, 1);
        port.register_session(session, AcquireReason::Consensus, 99);

        let handoff = durable_handoff(&mut counter, session, Arc::clone(&ledger));
        let handoff_id = handoff.handoff();
        port.publish_durable(handoff);

        let delivered = drain(&rx);
        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0].reason, AcquireReason::Consensus);
        assert_eq!(delivered[0].acquisition_id, 99);
        assert!(Arc::ptr_eq(&delivered[0].ledger, &ledger));
        assert_eq!(
            port.stats(),
            HandoffPortStats {
                delivered: 1,
                ..HandoffPortStats::default()
            }
        );
        assert_eq!(delivered[0].durable_handoff, Some(handoff_id));
        assert_eq!(delivered[0].coordinator_session, Some(session));
        assert!(
            events_rx.try_recv().is_err(),
            "queue delivery must wait for recipient acceptance before acknowledgement"
        );
    }

    #[test]
    fn retried_handoff_after_delivery_is_idempotent() {
        let (tx, rx) = mpsc::sync_channel(1);
        let (events, _) = mpsc::sync_channel(256);
        let mut port = CoordinatorHandoffPort::new(tx, events);
        let mut counter = IdCounter::new();
        let session = session(&mut counter);
        let ledger = immutable_ledger(8, 2);
        port.register_session(session, AcquireReason::Generic, 7);

        let handoff = durable_handoff(&mut counter, session, Arc::clone(&ledger));
        port.publish_durable(handoff.clone());
        port.publish_durable(handoff);

        assert_eq!(drain(&rx).len(), 1);
        assert_eq!(
            port.stats(),
            HandoffPortStats {
                delivered: 1,
                duplicated: 1,
                ..HandoffPortStats::default()
            }
        );
    }

    #[test]
    fn channel_full_handoff_stays_retryable_until_recipient_drains() {
        let (tx, rx) = mpsc::sync_channel(1);
        let (events, events_rx) = mpsc::sync_channel(256);
        let mut port = CoordinatorHandoffPort::new(tx, events);
        let mut counter = IdCounter::new();
        let session_a = session(&mut counter);
        let session_b = session(&mut counter);
        let ledger_a = immutable_ledger(9, 3);
        let ledger_b = immutable_ledger(10, 4);
        port.register_session(session_a, AcquireReason::History, 1);
        port.register_session(session_b, AcquireReason::Consensus, 2);

        let handoff_a = durable_handoff(&mut counter, session_a, ledger_a);
        let handoff_b = durable_handoff(&mut counter, session_b, ledger_b);
        port.publish_durable(handoff_a.clone());
        port.publish_durable(handoff_b.clone());

        // The buffer holds one item; the second handoff reports pressure and is
        // NOT recorded as published, so a retry of the same id re-attempts. The
        // rejection is a typed event the coordinator turns into a re-publish of
        // the exact pending id.
        assert_eq!(port.stats().delivered, 1);
        assert_eq!(port.stats().channel_full, 1);
        assert_eq!(drain(&rx).len(), 1);
        let rejections: Vec<_> = events_rx
            .try_iter()
            .filter_map(|event| match event {
                AcquisitionEvent::DurableHandoffRejected {
                    handoff,
                    session,
                    reason,
                } => Some((handoff, session, reason)),
                AcquisitionEvent::DurableHandoffAcknowledged(_) => None,
                other => panic!("expected a handoff event, got {other:?}"),
            })
            .collect();
        assert_eq!(rejections.len(), 1);
        assert_eq!(
            rejections[0].0,
            handoff_b.handoff(),
            "the rejected handoff id matches the unpublished delivery"
        );
        assert_eq!(rejections[0].1, handoff_b.session());
        assert_eq!(rejections[0].2, HandoffRejectReason::ChannelFull);

        port.publish_durable(handoff_b);
        assert_eq!(port.stats().delivered, 2);
        assert_eq!(port.stats().channel_full, 1);
        let delivered = drain(&rx);
        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0].acquisition_id, 2);
    }

    #[test]
    fn disconnected_handoff_reports_a_rejection_and_stays_retryable() {
        let (tx, rx) = mpsc::sync_channel(1);
        drop(rx);
        let (events, events_rx) = mpsc::sync_channel(256);
        let mut port = CoordinatorHandoffPort::new(tx, events);
        let mut counter = IdCounter::new();
        let session = session(&mut counter);
        let ledger = immutable_ledger(10, 5);
        port.register_session(session, AcquireReason::Consensus, 3);

        let handoff = durable_handoff(&mut counter, session, ledger);
        port.publish_durable(handoff.clone());

        assert_eq!(port.stats().delivered, 0);
        assert_eq!(port.stats().disconnected, 1);
        assert!(
            port.published.is_empty(),
            "a disconnected delivery is never recorded as published"
        );
        let rejections: Vec<_> = events_rx
            .try_iter()
            .map(|event| match event {
                AcquisitionEvent::DurableHandoffRejected {
                    handoff,
                    session,
                    reason,
                } => (handoff, session, reason),
                other => panic!("expected a handoff rejection, got {other:?}"),
            })
            .collect();
        assert_eq!(rejections.len(), 1);
        assert_eq!(rejections[0].0, handoff.handoff());
        assert_eq!(rejections[0].1, handoff.session());
        assert_eq!(rejections[0].2, HandoffRejectReason::Disconnected);
    }

    #[test]
    fn unknown_session_handoff_is_counted_and_never_delivered() {
        let (tx, rx) = mpsc::sync_channel(1);
        let (events, events_rx) = mpsc::sync_channel(256);
        let mut port = CoordinatorHandoffPort::new(tx, events);
        let mut counter = IdCounter::new();
        let session = session(&mut counter);
        let ledger = immutable_ledger(11, 5);

        port.publish_durable(durable_handoff(&mut counter, session, ledger));

        assert!(drain(&rx).is_empty());
        assert_eq!(port.stats().unknown_session, 1);
        assert_eq!(port.stats().delivered, 0);
        assert!(
            events_rx.try_recv().is_err(),
            "no ack for an unknown-session handoff"
        );
    }

    #[test]
    fn origins_are_keyed_by_the_complete_session_ref() {
        let (tx, rx) = mpsc::sync_channel(2);
        let (events, _) = mpsc::sync_channel(256);
        let mut port = CoordinatorHandoffPort::new(tx, events);
        let mut counter = IdCounter::new();
        let first = session(&mut counter);
        let replacement = SessionRef::new(
            counter.next_id(),
            first.session_id(),
            Uint256::from(2),
            counter.next_id(),
            StoreGeneration::new(2),
        );
        port.register_session(first, AcquireReason::Consensus, 10);
        port.register_session(replacement, AcquireReason::History, 20);

        port.publish_durable(durable_handoff(
            &mut counter,
            first,
            immutable_ledger(12, 6),
        ));
        port.publish_durable(durable_handoff(
            &mut counter,
            replacement,
            immutable_ledger(13, 7),
        ));

        let delivered = drain(&rx);
        assert_eq!(delivered.len(), 2);
        assert_eq!(delivered[0].reason, AcquireReason::Consensus);
        assert_eq!(delivered[0].acquisition_id, 10);
        assert_eq!(delivered[1].reason, AcquireReason::History);
        assert_eq!(delivered[1].acquisition_id, 20);
    }

    #[test]
    fn reregistered_session_origin_applies_to_later_handoffs() {
        let (tx, rx) = mpsc::sync_channel(1);
        let (events, _) = mpsc::sync_channel(256);
        let mut port = CoordinatorHandoffPort::new(tx, events);
        let mut counter = IdCounter::new();
        let session = session(&mut counter);
        port.register_session(session, AcquireReason::Consensus, 10);
        port.register_session(session, AcquireReason::Generic, 20);

        port.publish_durable(durable_handoff(
            &mut counter,
            session,
            immutable_ledger(12, 6),
        ));

        let delivered = drain(&rx);
        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0].reason, AcquireReason::Generic);
        assert_eq!(delivered[0].acquisition_id, 20);
    }
}
