//! The `SyncPhase` service lifecycle and its transition table.
//!
//! The transition rules below implement the required table from `AGENTS.md`
//! ("Service lifecycle authority"). The coordinator owns service phase: it is
//! the only production writer, and every transition carries the fact that
//! motivated it.

use crate::target::{LedgerIdentity, LedgerTarget};

/// The single service lifecycle owned by the coordinator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncPhase {
    /// No usable peer capability.
    Disconnected,
    /// Peer capability exists; no acquisition target yet.
    Connected,
    /// An acquisition target needs to be acquired.
    Syncing { target: LedgerTarget },
    /// The target is structurally complete, durable, accepted, and installed
    /// as the last closed ledger.
    Tracking { lcl: LedgerIdentity },
    /// The validated/published chain is contiguous and freshness policy passes.
    Full {
        lcl: LedgerIdentity,
        published: LedgerIdentity,
    },
    /// Shutdown requested; no further transitions.
    Stopping,
}

impl SyncPhase {
    /// A stable label for tracing and metrics.
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Disconnected => "disconnected",
            Self::Connected => "connected",
            Self::Syncing { .. } => "syncing",
            Self::Tracking { .. } => "tracking",
            Self::Full { .. } => "full",
            Self::Stopping => "stopping",
        }
    }

    /// True for every phase except the terminal `Stopping` phase.
    pub const fn is_active(&self) -> bool {
        !matches!(self, Self::Stopping)
    }

    /// True for the terminal `Stopping` phase.
    pub const fn is_terminal(&self) -> bool {
        matches!(self, Self::Stopping)
    }

    /// Applies a transition fact, rejecting illegal transitions.
    ///
    /// The error deliberately carries the exact `from` phase and motivating
    /// fact so illegal transitions are traceable; it is not on a hot path.
    #[allow(clippy::result_large_err)]
    pub fn apply(self, fact: TransitionFact) -> Result<SyncPhase, TransitionError> {
        phase_transition(&self, &fact).ok_or(TransitionError::Invalid { from: self, fact })
    }
}

/// A typed fact motivating a phase transition. External systems submit facts;
/// they never write operating mode directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionFact {
    /// Usable peer capability exists (`Connectivity` snapshot is non-empty).
    PeerCapabilityAvailable,
    /// No usable peers remain.
    PeerCapabilityLost,
    /// Consensus, validation, recovery, or startup needs a target acquired.
    TargetRequired { target: LedgerTarget },
    /// The syncing target is complete, durable, accepted, and installed as LCL.
    TargetInstalledAsLcl { lcl: LedgerIdentity },
    /// The validated/published chain is contiguous and freshness policy passes.
    ChainContiguous {
        lcl: LedgerIdentity,
        published: LedgerIdentity,
    },
    /// Preferred-LCL divergence with a concrete acquisition target.
    PreferredLclDivergence { target: LedgerTarget },
    /// Blocked state or loss of sufficient freshness with no concrete target.
    BlockedWithNoTarget,
    /// Shutdown requested.
    Shutdown,
}

/// The phase transition was not legal from the given source phase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionError {
    /// `fact` does not legalize a transition out of `from`.
    Invalid {
        from: SyncPhase,
        fact: TransitionFact,
    },
}

/// The legal `SyncPhase` transition set.
///
/// Implements the `AGENTS.md` "Required transition rules" table:
///
/// | From | Required fact/invariant | To |
/// |---|---|---|
/// | `Disconnected` | usable peer capability exists | `Connected` |
/// | `Connected` | consensus, validation, recovery, or startup target | `Syncing` |
/// | `Syncing` | target complete, durable, accepted, installed as LCL | `Tracking` |
/// | `Tracking` | validated/published chain contiguous and fresh | `Full` |
/// | `Full` | preferred-LCL divergence / stale validation / required target | `Syncing { target }` |
/// | `Full` | blocked state / freshness loss with no concrete target | `Connected` |
/// | any active phase | no usable peers | `Disconnected` |
/// | any nonterminal phase | shutdown | `Stopping` |
///
/// Documented clarifications, consistent with the "avoidance of doubt"
/// paragraph in `AGENTS.md`:
///
/// * `Connected -> Syncing` is also legal via `PreferredLclDivergence` when a
///   concrete target exists.
/// * `Connected -> Tracking` is legal via `TargetInstalledAsLcl` when a local
///   preferred LCL is installed without acquisition. This models rippled's
///   `switchLastClosedLedger` clearing `needNetworkLedger` when the preferred
///   LCL is already resident (no network ledger needed); the coordinator then
///   requires a fresh contiguous publication for `Tracking -> Full`.
/// * `Syncing -> Syncing` is legal via a retargeting `TargetRequired`; the
///   session owner is preserved while the target changes.
/// * `Tracking -> Syncing` is legal via `PreferredLclDivergence`; tracking is
///   retained while the LCL matches, but a divergent preferred LCL with a known
///   target resumes acquisition.
pub fn phase_transition(from: &SyncPhase, fact: &TransitionFact) -> Option<SyncPhase> {
    use SyncPhase::*;
    use TransitionFact::*;

    let to = match (from, fact) {
        (Disconnected, PeerCapabilityAvailable) => Connected,
        (Connected, TargetRequired { target }) => Syncing { target: *target },
        (Connected, PreferredLclDivergence { target }) => Syncing { target: *target },
        (Connected, TargetInstalledAsLcl { lcl }) => Tracking { lcl: *lcl },
        (Syncing { .. }, TargetRequired { target }) => Syncing { target: *target },
        (Syncing { .. }, TargetInstalledAsLcl { lcl }) => Tracking { lcl: *lcl },
        (Tracking { .. }, PreferredLclDivergence { target }) => Syncing { target: *target },
        (Tracking { .. }, ChainContiguous { lcl, published }) => Full {
            lcl: *lcl,
            published: *published,
        },
        (Full { .. }, TargetRequired { target }) => Syncing { target: *target },
        (Full { .. }, PreferredLclDivergence { target }) => Syncing { target: *target },
        (Full { .. }, BlockedWithNoTarget) => Connected,
        (Connected, PeerCapabilityLost) => Disconnected,
        (Syncing { .. }, PeerCapabilityLost) => Disconnected,
        (Tracking { .. }, PeerCapabilityLost) => Disconnected,
        (Full { .. }, PeerCapabilityLost) => Disconnected,
        (Disconnected, Shutdown) => Stopping,
        (Connected, Shutdown) => Stopping,
        (Syncing { .. }, Shutdown) => Stopping,
        (Tracking { .. }, Shutdown) => Stopping,
        (Full { .. }, Shutdown) => Stopping,
        _ => return None,
    };
    Some(to)
}

#[cfg(test)]
mod tests {
    use super::*;
    use basics::base_uint::Uint256;

    fn target(seq: u32) -> LedgerTarget {
        LedgerTarget::new(Uint256::from(u64::from(seq)), Some(seq))
    }

    fn identity(seq: u32) -> LedgerIdentity {
        LedgerIdentity::new(Uint256::from(u64::from(seq)), seq)
    }

    fn facts_for(from: SyncPhase) -> Vec<(TransitionFact, Option<SyncPhase>)> {
        let mut cases = Vec::new();

        // Every fact tried from this phase; legal pairs carry the expected
        // result, illegal pairs carry None.
        let facts = [
            TransitionFact::PeerCapabilityAvailable,
            TransitionFact::PeerCapabilityLost,
            TransitionFact::TargetRequired { target: target(10) },
            TransitionFact::TargetInstalledAsLcl { lcl: identity(10) },
            TransitionFact::ChainContiguous {
                lcl: identity(10),
                published: identity(10),
            },
            TransitionFact::PreferredLclDivergence { target: target(11) },
            TransitionFact::BlockedWithNoTarget,
            TransitionFact::Shutdown,
        ];
        for fact in facts {
            cases.push((fact, phase_transition(&from, &fact)));
        }
        cases
    }

    #[test]
    fn every_legal_transition_produces_the_documented_phase() {
        assert_eq!(
            phase_transition(
                &SyncPhase::Disconnected,
                &TransitionFact::PeerCapabilityAvailable
            ),
            Some(SyncPhase::Connected)
        );
        assert_eq!(
            phase_transition(
                &SyncPhase::Connected,
                &TransitionFact::TargetRequired { target: target(9) }
            ),
            Some(SyncPhase::Syncing { target: target(9) })
        );
        assert_eq!(
            phase_transition(
                &SyncPhase::Connected,
                &TransitionFact::PreferredLclDivergence { target: target(9) }
            ),
            Some(SyncPhase::Syncing { target: target(9) })
        );
        // A locally resident preferred LCL installed while `Connected` (no
        // acquisition needed) drives `Connected -> Tracking`.
        assert_eq!(
            phase_transition(
                &SyncPhase::Connected,
                &TransitionFact::TargetInstalledAsLcl { lcl: identity(9) }
            ),
            Some(SyncPhase::Tracking { lcl: identity(9) })
        );
        assert_eq!(
            phase_transition(
                &SyncPhase::Syncing { target: target(9) },
                &TransitionFact::TargetRequired { target: target(12) }
            ),
            Some(SyncPhase::Syncing { target: target(12) })
        );
        assert_eq!(
            phase_transition(
                &SyncPhase::Syncing { target: target(9) },
                &TransitionFact::TargetInstalledAsLcl { lcl: identity(9) }
            ),
            Some(SyncPhase::Tracking { lcl: identity(9) })
        );
        assert_eq!(
            phase_transition(
                &SyncPhase::Tracking { lcl: identity(9) },
                &TransitionFact::ChainContiguous {
                    lcl: identity(9),
                    published: identity(9)
                },
            ),
            Some(SyncPhase::Full {
                lcl: identity(9),
                published: identity(9)
            })
        );
        assert_eq!(
            phase_transition(
                &SyncPhase::Tracking { lcl: identity(9) },
                &TransitionFact::PreferredLclDivergence { target: target(12) },
            ),
            Some(SyncPhase::Syncing { target: target(12) })
        );
        assert_eq!(
            phase_transition(
                &SyncPhase::Full {
                    lcl: identity(9),
                    published: identity(9)
                },
                &TransitionFact::TargetRequired { target: target(12) }
            ),
            Some(SyncPhase::Syncing { target: target(12) })
        );
        assert_eq!(
            phase_transition(
                &SyncPhase::Full {
                    lcl: identity(9),
                    published: identity(9)
                },
                &TransitionFact::PreferredLclDivergence { target: target(12) }
            ),
            Some(SyncPhase::Syncing { target: target(12) })
        );
        assert_eq!(
            phase_transition(
                &SyncPhase::Full {
                    lcl: identity(9),
                    published: identity(9)
                },
                &TransitionFact::BlockedWithNoTarget
            ),
            Some(SyncPhase::Connected)
        );
        for phase in [
            SyncPhase::Connected,
            SyncPhase::Syncing { target: target(1) },
            SyncPhase::Tracking { lcl: identity(1) },
            SyncPhase::Full {
                lcl: identity(1),
                published: identity(1),
            },
        ] {
            assert_eq!(
                phase_transition(&phase, &TransitionFact::PeerCapabilityLost),
                Some(SyncPhase::Disconnected)
            );
        }
        for phase in [
            SyncPhase::Disconnected,
            SyncPhase::Connected,
            SyncPhase::Syncing { target: target(1) },
            SyncPhase::Tracking { lcl: identity(1) },
            SyncPhase::Full {
                lcl: identity(1),
                published: identity(1),
            },
        ] {
            assert_eq!(
                phase_transition(&phase, &TransitionFact::Shutdown),
                Some(SyncPhase::Stopping)
            );
        }
    }

    #[test]
    fn every_illegal_transition_is_rejected_without_changing_phase() {
        let phases = [
            SyncPhase::Disconnected,
            SyncPhase::Connected,
            SyncPhase::Syncing { target: target(1) },
            SyncPhase::Tracking { lcl: identity(1) },
            SyncPhase::Full {
                lcl: identity(1),
                published: identity(1),
            },
            SyncPhase::Stopping,
        ];
        for phase in phases {
            for (fact, expected) in facts_for(phase) {
                let observed = phase_transition(&phase, &fact);
                assert_eq!(
                    observed, expected,
                    "phase {:?} + fact {:?} must transition exactly as documented",
                    phase, fact
                );
                // Applying through the owned method preserves the from-phase on
                // an error, proving the snapshot is unchanged.
                match expected {
                    Some(to) => assert_eq!(phase.apply(fact), Ok(to)),
                    None => {
                        assert_eq!(
                            phase.apply(fact),
                            Err(TransitionError::Invalid { from: phase, fact })
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn peer_loss_demotes_then_recovery_requires_a_fresh_peer_fact() {
        // Full -> (no peers) -> Disconnected.
        let demoted = phase_transition(
            &SyncPhase::Full {
                lcl: identity(1),
                published: identity(1),
            },
            &TransitionFact::PeerCapabilityLost,
        );
        assert_eq!(demoted, Some(SyncPhase::Disconnected));

        // From Disconnected, no stale timer or target fact can recover.
        let stale = phase_transition(
            &SyncPhase::Disconnected,
            &TransitionFact::TargetRequired { target: target(2) },
        );
        assert_eq!(stale, None);
        let stale = phase_transition(
            &SyncPhase::Disconnected,
            &TransitionFact::PreferredLclDivergence { target: target(2) },
        );
        assert_eq!(stale, None);
        let stale = phase_transition(
            &SyncPhase::Disconnected,
            &TransitionFact::TargetInstalledAsLcl { lcl: identity(2) },
        );
        assert_eq!(stale, None);

        // Recovery requires a new usable-peer fact.
        assert_eq!(
            phase_transition(
                &SyncPhase::Disconnected,
                &TransitionFact::PeerCapabilityAvailable
            ),
            Some(SyncPhase::Connected)
        );
    }

    #[test]
    fn full_to_syncing_is_required_whenever_a_concrete_target_exists() {
        // "Full -> Syncing is required whenever a concrete acquisition target
        // exists; Full -> Connected is reserved for a demotion with no target."
        for fact in [
            TransitionFact::TargetRequired { target: target(20) },
            TransitionFact::PreferredLclDivergence { target: target(20) },
        ] {
            let result = phase_transition(
                &SyncPhase::Full {
                    lcl: identity(1),
                    published: identity(1),
                },
                &fact,
            );
            assert_eq!(result, Some(SyncPhase::Syncing { target: target(20) }));
        }

        // The next consensus target after a targetless demotion re-enters syncing.
        let demoted = SyncPhase::Full {
            lcl: identity(1),
            published: identity(1),
        }
        .apply(TransitionFact::BlockedWithNoTarget)
        .unwrap();
        assert_eq!(demoted, SyncPhase::Connected);
        let re_syncing = demoted
            .apply(TransitionFact::TargetRequired { target: target(20) })
            .unwrap();
        assert_eq!(re_syncing, SyncPhase::Syncing { target: target(20) });
    }

    #[test]
    fn stopping_is_terminal_and_active_phases_are_nonterminal() {
        assert!(SyncPhase::Stopping.is_terminal());
        assert!(!SyncPhase::Stopping.is_active());
        for phase in [
            SyncPhase::Disconnected,
            SyncPhase::Connected,
            SyncPhase::Syncing { target: target(1) },
            SyncPhase::Tracking { lcl: identity(1) },
            SyncPhase::Full {
                lcl: identity(1),
                published: identity(1),
            },
        ] {
            assert!(phase.is_active());
            assert!(!phase.is_terminal());
        }
    }
}
