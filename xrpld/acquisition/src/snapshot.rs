//! Immutable observer snapshots.
//!
//! RPC/CLI/metrics consumers read immutable, bounded snapshots. A live mutable
//! session object is never exposed to observers.

use std::collections::BTreeMap;

use crate::id::RunEpoch;
use crate::phase::SyncPhase;
use crate::target::AcquireReason;

/// An immutable snapshot of coordinator-observable state for RPC/CLI/metrics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoordinatorSnapshot {
    run_epoch: RunEpoch,
    phase: SyncPhase,
    session_count: usize,
    active_by_reason: BTreeMap<AcquireReason, usize>,
    admission_reserved_packets: u64,
    admission_reserved_bytes: u64,
    cancelled_sessions: u64,
    stale_events: u64,
}

impl CoordinatorSnapshot {
    /// Builds an observer snapshot.
    pub fn new(
        run_epoch: RunEpoch,
        phase: SyncPhase,
        session_count: usize,
        active_by_reason: BTreeMap<AcquireReason, usize>,
        admission_reserved_packets: u64,
        admission_reserved_bytes: u64,
        cancelled_sessions: u64,
        stale_events: u64,
    ) -> Self {
        Self {
            run_epoch,
            phase,
            session_count,
            active_by_reason,
            admission_reserved_packets,
            admission_reserved_bytes,
            cancelled_sessions,
            stale_events,
        }
    }

    /// The coordinator run epoch.
    pub const fn run_epoch(&self) -> RunEpoch {
        self.run_epoch
    }

    /// The current service phase.
    pub const fn phase(&self) -> &SyncPhase {
        &self.phase
    }

    /// The number of tracked sessions.
    pub const fn session_count(&self) -> usize {
        self.session_count
    }

    /// Active sessions grouped by acquisition reason.
    pub fn active_by_reason(&self) -> &BTreeMap<AcquireReason, usize> {
        &self.active_by_reason
    }

    /// Currently reserved admission packets across all sessions.
    pub const fn admission_reserved_packets(&self) -> u64 {
        self.admission_reserved_packets
    }

    /// Currently reserved admission bytes across all sessions.
    pub const fn admission_reserved_bytes(&self) -> u64 {
        self.admission_reserved_bytes
    }

    /// Total cancelled sessions.
    pub const fn cancelled_sessions(&self) -> u64 {
        self.cancelled_sessions
    }

    /// Total stale events observed (late completions after cancellation or
    /// replacement).
    pub const fn stale_events(&self) -> u64 {
        self.stale_events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::RunEpoch;
    use crate::target::LedgerTarget;
    use basics::base_uint::Uint256;

    #[test]
    fn snapshot_exposes_bounded_observable_state() {
        let mut active = BTreeMap::new();
        active.insert(AcquireReason::Consensus, 1);
        let snapshot = CoordinatorSnapshot::new(
            RunEpoch::new(1),
            SyncPhase::Syncing {
                target: LedgerTarget::new(Uint256::from(1), Some(1)),
            },
            1,
            active,
            4,
            64,
            2,
            7,
        );
        assert_eq!(snapshot.run_epoch(), RunEpoch::new(1));
        assert_eq!(snapshot.session_count(), 1);
        assert_eq!(
            snapshot.active_by_reason().get(&AcquireReason::Consensus),
            Some(&1)
        );
        assert_eq!(snapshot.admission_reserved_packets(), 4);
        assert_eq!(snapshot.cancelled_sessions(), 2);
        assert_eq!(snapshot.stale_events(), 7);
    }
}
