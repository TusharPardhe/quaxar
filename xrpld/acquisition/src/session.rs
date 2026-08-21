//! Per-session lifecycle: terminal outcomes, cancellation, and the durable-only
//! completion invariant.
//!
//! The full session planner (mailbox, tree plan, persistence intent) is
//! coordinator-owned from M4 onward. This module defines the lifecycle contract
//! the coordinator enforces: at most one terminal outcome, cancellation
//! invalidates the session immediately, and a normal successful result is
//! reachable only after the durability fence passes (`DurablePending`).

use crate::handoff::DurableLedger;
use crate::identity::SessionRef;

/// The mutable lifecycle phase of one acquisition session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionPhase {
    /// The session owns the current consensus network permit and may advance
    /// its plan, route packets, issue reads, and request peers.
    Active,
    /// A retained consensus session whose plan and `SessionRef` remain owned,
    /// but which has no network permit. It accepts no packets, reads, timers,
    /// plan turns, frontier work, or retries until the exact target becomes
    /// current again. `Persisting` and `DurablePending` never enter Dormant.
    Dormant,
    /// All required nodes are present and the tree is complete; the session is
    /// persisting all required nodes/metadata.
    Persisting,
    /// The durability fence passed; the durable result is being delivered and
    /// waits for handoff acknowledgement.
    DurablePending,
    /// Terminal success: the durable handoff was acknowledged.
    Complete,
    /// Terminal failure; no normal adoptable ledger was produced.
    Failed { reason: FailureReason },
    /// Terminal: the session was cancelled and its external operations are
    /// invalidated. Late completions are stale.
    Cancelled { reason: CancelReason },
}

impl SessionPhase {
    /// True for the terminal phases: `Complete`, `Failed`, `Cancelled`.
    pub const fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Complete | Self::Failed { .. } | Self::Cancelled { .. }
        )
    }

    /// True only for a durable, acknowledged success.
    pub const fn is_success(&self) -> bool {
        matches!(self, Self::Complete)
    }

    /// A stable label for tracing.
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Dormant => "dormant",
            Self::Persisting => "persisting",
            Self::DurablePending => "durable_pending",
            Self::Complete => "complete",
            Self::Failed { .. } => "failed",
            Self::Cancelled { .. } => "cancelled",
        }
    }
}

/// Why a session failed. A failure must leave no normal resolver-visible
/// completed ledger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FailureReason {
    /// The tree plan produced an invalid/unsupported plan.
    InvalidTreePlan,
    /// A decoded packet was invalid for this session.
    InvalidPacketData,
    /// A brokered read failed.
    ReadFailure,
    /// A write batch failed.
    WriteFailure,
    /// The durability fence failed.
    DurabilityFenceFailed,
    /// No usable peers were available to complete the acquisition.
    NoUsablePeers,
    /// The acquisition attempt deadline elapsed.
    AcquisitionTimeout,
    /// The NodeStore reported full.
    NodeStoreFull,
}

impl FailureReason {
    /// A stable label for tracing and metrics.
    pub const fn label(self) -> &'static str {
        match self {
            Self::InvalidTreePlan => "invalid_tree_plan",
            Self::InvalidPacketData => "invalid_packet_data",
            Self::ReadFailure => "read_failure",
            Self::WriteFailure => "write_failure",
            Self::DurabilityFenceFailed => "durability_fence_failed",
            Self::NoUsablePeers => "no_usable_peers",
            Self::AcquisitionTimeout => "acquisition_timeout",
            Self::NodeStoreFull => "node_store_full",
        }
    }
}

/// Why a session was cancelled. Cancellation invalidates the session
/// immediately; every late network, read, write, timer, CPU, or handoff event
/// is stale and must not mutate the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CancelReason {
    /// A new session for the same or a newer target replaced this one.
    Replaced,
    /// The NodeStore generation rotated under this session.
    StoreRotated,
    /// The node is shutting down.
    Shutdown,
    /// The exact target was installed locally as the LCL before this session
    /// reached its own durability handoff.
    LclInstalled,
    /// An explicit cancellation request.
    Explicit,
    /// The per-hash acquisition received no repeated demand for one minute.
    IdleExpired,
}

impl CancelReason {
    /// A stable label for tracing.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Replaced => "replaced",
            Self::StoreRotated => "store_rotated",
            Self::Shutdown => "shutdown",
            Self::LclInstalled => "lcl_installed",
            Self::Explicit => "explicit",
            Self::IdleExpired => "idle_expired",
        }
    }
}

/// The single terminal result of a session. A successful session produces at
/// most one `Durable`, carrying a unique handoff id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionOutcome {
    /// The ledger is durable and handed off; terminal success.
    Durable(DurableLedger),
    /// The session failed; no normal adoptable ledger was produced.
    Failed {
        session: SessionRef,
        reason: FailureReason,
    },
    /// The session was cancelled; its results are invalidated.
    Cancelled {
        session: SessionRef,
        reason: CancelReason,
    },
}

impl SessionOutcome {
    /// True when the outcome is terminal.
    pub const fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Durable(_) | Self::Failed { .. } | Self::Cancelled { .. }
        )
    }
}

/// The legal per-session phase transitions.
///
/// Invariants enforced:
///
/// * `Complete` is reachable only from `DurablePending` — a passed durability
///   fence is required before a normal adoptable result exists.
/// * A terminal phase is final: no transition out of `Complete`, `Failed`, or
///   `Cancelled`.
/// * `Active` and `Dormant` are the only reversible pair. Dormancy retains
///   the exact plan/session identity but removes its network permit.
/// * `Failed` is reachable from `Active`/`Persisting` (never from
///   `DurablePending`, where the result is already committed to delivery).
/// * `Cancelled` is reachable from `Active`/`Dormant`/`Persisting` (never from
///   `DurablePending`, so cancellation cannot revoke a committed durable
///   result).
pub fn session_phase_transition(from: &SessionPhase, to: &SessionPhase) -> bool {
    use SessionPhase::*;
    match (from, to) {
        (from, to) if from == to => true,
        (Active, Dormant) | (Dormant, Active) => true,
        (Active, Persisting) => true,
        (Persisting, DurablePending) => true,
        (DurablePending, Complete) => true,
        (Active, Failed { .. }) | (Persisting, Failed { .. }) => true,
        (Active, Cancelled { .. })
        | (Dormant, Cancelled { .. })
        | (Persisting, Cancelled { .. }) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::{PlanEpoch, RunEpoch, SessionId, StoreGeneration};
    use basics::base_uint::Uint256;

    fn session() -> SessionRef {
        SessionRef::new(
            RunEpoch::new(1),
            SessionId::new(1),
            Uint256::from(1),
            PlanEpoch::new(1),
            StoreGeneration::new(1),
        )
    }

    #[test]
    fn durable_only_invariant_complete_requires_fence() {
        use SessionPhase::*;
        // Completing without the durability fence is illegal from every
        // non-fenced phase.
        assert!(!session_phase_transition(&Active, &Complete));
        assert!(!session_phase_transition(&Persisting, &Complete));
        assert!(session_phase_transition(&DurablePending, &Complete));
        assert!(SessionPhase::Complete.is_terminal());
        assert!(SessionPhase::Complete.is_success());
    }

    #[test]
    fn at_most_one_terminal_outcome() {
        use SessionPhase::*;
        let terminals = [
            Complete,
            Failed {
                reason: FailureReason::WriteFailure,
            },
            Cancelled {
                reason: CancelReason::Replaced,
            },
        ];
        for terminal in &terminals {
            assert!(terminal.is_terminal());
            // A different phase may never be reached from a terminal phase.
            for other in [
                Active,
                Dormant,
                Persisting,
                DurablePending,
                Complete,
                Failed {
                    reason: FailureReason::ReadFailure,
                },
                Cancelled {
                    reason: CancelReason::Shutdown,
                },
            ] {
                if &other != terminal {
                    assert!(
                        !session_phase_transition(terminal, &other),
                        "terminal {:?} must not transition to {:?}",
                        terminal,
                        other
                    );
                }
            }
            // The same-phase no-op is idempotent and harmless.
            assert!(session_phase_transition(terminal, terminal));
        }
    }

    #[test]
    fn dormant_is_reversible_only_before_persistence() {
        use SessionPhase::*;
        assert!(session_phase_transition(&Active, &Dormant));
        assert!(session_phase_transition(&Dormant, &Active));
        assert!(!session_phase_transition(&Persisting, &Dormant));
        assert!(!session_phase_transition(&DurablePending, &Dormant));
        assert_eq!(Dormant.label(), "dormant");
    }

    #[test]
    fn cancellation_is_legal_before_the_fence_but_not_after() {
        use SessionPhase::*;
        assert!(session_phase_transition(
            &Active,
            &Cancelled {
                reason: CancelReason::Replaced
            }
        ));
        assert!(session_phase_transition(
            &Dormant,
            &Cancelled {
                reason: CancelReason::Replaced
            }
        ));
        assert!(session_phase_transition(
            &Persisting,
            &Cancelled {
                reason: CancelReason::Shutdown
            }
        ));
        // Once the fence passed the result is committed to delivery.
        assert!(!session_phase_transition(
            &DurablePending,
            &Cancelled {
                reason: CancelReason::Replaced
            }
        ));
    }

    #[test]
    fn failure_is_legal_before_the_fence_but_not_after() {
        use SessionPhase::*;
        assert!(session_phase_transition(
            &Active,
            &Failed {
                reason: FailureReason::ReadFailure
            }
        ));
        assert!(session_phase_transition(
            &Persisting,
            &Failed {
                reason: FailureReason::DurabilityFenceFailed
            }
        ));
        assert!(!session_phase_transition(
            &DurablePending,
            &Failed {
                reason: FailureReason::WriteFailure
            }
        ));
    }

    #[test]
    fn durable_success_path_is_linear() {
        use SessionPhase::*;
        let path = [Active, Persisting, DurablePending, Complete];
        for window in path.windows(2) {
            assert!(
                session_phase_transition(&window[0], &window[1]),
                "{} -> {} must be legal",
                window[0].label(),
                window[1].label()
            );
        }
    }

    #[test]
    fn session_outcome_terminality() {
        let outcome = SessionOutcome::Failed {
            session: session(),
            reason: FailureReason::InvalidTreePlan,
        };
        assert!(outcome.is_terminal());
    }

    #[test]
    fn failure_reason_labels_are_stable() {
        assert_eq!(
            FailureReason::DurabilityFenceFailed.label(),
            "durability_fence_failed"
        );
        assert_eq!(CancelReason::Replaced.label(), "replaced");
        assert_eq!(SessionPhase::DurablePending.label(), "durable_pending");
    }
}
