//! Consensus algorithm parameters.
//!
//! Ported directly from rippled's `ConsensusParms.h`. Every field and value
//! here must match the reference source exactly — these parameters are not
//! meant to be tuned independently; drift here changes convergence and
//! recovery behavior in ways that are easy to get subtly wrong.

use std::collections::BTreeMap;
use std::time::Duration;

/// Avalanche voting state, matching `ConsensusParms::AvalancheState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AvalancheState {
    Init,
    Mid,
    Late,
    Stuck,
}

/// One entry in the avalanche cutoff table, matching
/// `ConsensusParms::AvalancheCutoff`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AvalancheCutoff {
    /// Percent of the previous round's duration that must have elapsed
    /// before this state's threshold applies.
    pub consensus_time: i32,
    /// Required yes-vote percentage while in this state.
    pub consensus_pct: usize,
    /// The next state to transition to once `consensus_time` is reached.
    pub next: AvalancheState,
}

/// Consensus algorithm parameters, matching `ConsensusParms` exactly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsensusParms {
    // -------------------------------------------------------------------
    // Validation and proposal durations are relative to NetClock times,
    // so they use second resolution (matching the reference comment).
    /// The duration a validation remains current after its ledger's close
    /// time. `validationValidWall` in the reference.
    pub validation_valid_wall: Duration,

    /// Duration a validation remains current after first observed.
    /// `validationValidLocal` in the reference.
    pub validation_valid_local: Duration,

    /// Duration pre-close in which validations are acceptable.
    /// `validationValidEarly` in the reference.
    pub validation_valid_early: Duration,

    /// How long we consider a proposal fresh. `proposeFRESHNESS`.
    pub propose_freshness: Duration,

    /// How often we force a new proposal to keep ours fresh.
    /// `proposeINTERVAL`.
    pub propose_interval: Duration,

    // -------------------------------------------------------------------
    // Consensus durations are relative to the internal Consensus clock
    // and use millisecond resolution (matching the reference comment).
    /// The percentage threshold above which we can declare consensus.
    /// `minConsensusPct`.
    pub min_consensus_pct: usize,

    /// The duration a ledger may remain idle before closing.
    /// `ledgerIdleInterval`.
    pub ledger_idle_interval: Duration,

    /// The minimum time we wait to ensure participation.
    /// `ledgerMinConsensus`.
    pub ledger_min_consensus: Duration,

    /// The maximum amount of time to spend pausing for laggards.
    /// `ledgerMaxConsensus`.
    pub ledger_max_consensus: Duration,

    /// Minimum time to wait to ensure others have computed the LCL.
    /// `ledgerMinClose`.
    pub ledger_min_close: Duration,

    /// How often we check state or change positions. `ledgerGRANULARITY`.
    pub ledger_granularity: Duration,

    /// How long to wait (as a multiple of the previous round's duration)
    /// before completely abandoning consensus. `ledgerAbandonConsensusFactor`.
    pub ledger_abandon_consensus_factor: usize,

    /// Maximum amount of time to give a consensus round.
    /// `ledgerAbandonConsensus`.
    pub ledger_abandon_consensus: Duration,

    /// The minimum amount of time to consider the previous round to have
    /// taken. `avMinConsensusTime`.
    pub av_min_consensus_time: Duration,

    /// Avalanche cutoff table, matching `avalancheCutoffs`.
    pub avalanche_cutoffs: BTreeMap<AvalancheState, AvalancheCutoff>,

    /// Percentage of nodes required to reach agreement on ledger close
    /// time. `avCtConsensusPct`.
    pub av_ct_consensus_pct: usize,

    /// Number of rounds before certain avalanche-state actions can happen.
    /// `avMinRounds`.
    pub av_min_rounds: usize,

    /// Number of rounds before a stuck vote is considered unlikely to
    /// change. `avStalledRounds`.
    pub av_stalled_rounds: usize,
}

impl Default for ConsensusParms {
    fn default() -> Self {
        Self {
            validation_valid_wall: Duration::from_secs(5 * 60),
            validation_valid_local: Duration::from_secs(3 * 60),
            validation_valid_early: Duration::from_secs(3 * 60),
            propose_freshness: Duration::from_secs(20),
            propose_interval: Duration::from_secs(12),
            min_consensus_pct: 80,
            ledger_idle_interval: Duration::from_secs(15),
            ledger_min_consensus: Duration::from_millis(1950),
            ledger_max_consensus: Duration::from_secs(15),
            ledger_min_close: Duration::from_secs(2),
            ledger_granularity: Duration::from_secs(1),
            ledger_abandon_consensus_factor: 10,
            ledger_abandon_consensus: Duration::from_secs(120),
            av_min_consensus_time: Duration::from_secs(5),
            avalanche_cutoffs: BTreeMap::from([
                (
                    AvalancheState::Init,
                    AvalancheCutoff {
                        consensus_time: 0,
                        consensus_pct: 50,
                        next: AvalancheState::Mid,
                    },
                ),
                (
                    AvalancheState::Mid,
                    AvalancheCutoff {
                        consensus_time: 50,
                        consensus_pct: 65,
                        next: AvalancheState::Late,
                    },
                ),
                (
                    AvalancheState::Late,
                    AvalancheCutoff {
                        consensus_time: 85,
                        consensus_pct: 70,
                        next: AvalancheState::Stuck,
                    },
                ),
                (
                    AvalancheState::Stuck,
                    AvalancheCutoff {
                        consensus_time: 200,
                        consensus_pct: 95,
                        next: AvalancheState::Stuck,
                    },
                ),
            ]),
            av_ct_consensus_pct: 75,
            av_min_rounds: 2,
            av_stalled_rounds: 4,
        }
    }
}

/// Determine the currently-required avalanche yes-vote weight, and whether
/// the state should transition to the next one.
///
/// Ported line-for-line from the reference `getNeededWeight` free function.
/// `current_rounds` is the number of times this dispute (or close-time vote)
/// has been evaluated in the current avalanche state; `minimum_rounds` is
/// `ConsensusParms::av_min_rounds`.
pub fn get_needed_weight(
    parms: &ConsensusParms,
    current_state: AvalancheState,
    percent_time: i32,
    current_rounds: usize,
    minimum_rounds: usize,
) -> (usize, Option<AvalancheState>) {
    let current_cutoff = parms
        .avalanche_cutoffs
        .get(&current_state)
        .expect("current avalanche state must exist in the cutoff table");

    if current_cutoff.next != current_state && current_rounds >= minimum_rounds {
        let next_cutoff = parms
            .avalanche_cutoffs
            .get(&current_cutoff.next)
            .expect("next avalanche state must exist in the cutoff table");
        debug_assert!(
            next_cutoff.consensus_time >= current_cutoff.consensus_time,
            "next avalanche state's cutoff time must not precede the current one"
        );
        if percent_time >= next_cutoff.consensus_time {
            return (next_cutoff.consensus_pct, Some(current_cutoff.next));
        }
    }
    (current_cutoff.consensus_pct, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every constant here must match `ConsensusParms.h` exactly. This is
    /// a direct transcription check, not a behavioral test — if this ever
    /// fails, the fix is almost always to update this test to match a
    /// deliberate, reviewed change to the reference source, not to "fix"
    /// the production default.
    #[test]
    fn defaults_match_reference_consensus_parms() {
        let p = ConsensusParms::default();

        assert_eq!(p.validation_valid_wall, Duration::from_secs(5 * 60));
        assert_eq!(p.validation_valid_local, Duration::from_secs(3 * 60));
        assert_eq!(p.validation_valid_early, Duration::from_secs(3 * 60));
        assert_eq!(p.propose_freshness, Duration::from_secs(20));
        assert_eq!(p.propose_interval, Duration::from_secs(12));

        assert_eq!(p.min_consensus_pct, 80);
        assert_eq!(p.ledger_idle_interval, Duration::from_secs(15));
        assert_eq!(p.ledger_min_consensus, Duration::from_millis(1950));
        assert_eq!(p.ledger_max_consensus, Duration::from_secs(15));
        assert_eq!(p.ledger_min_close, Duration::from_secs(2));
        assert_eq!(p.ledger_granularity, Duration::from_secs(1));
        assert_eq!(p.ledger_abandon_consensus_factor, 10);
        assert_eq!(p.ledger_abandon_consensus, Duration::from_secs(120));
        assert_eq!(p.av_min_consensus_time, Duration::from_secs(5));

        assert_eq!(p.av_ct_consensus_pct, 75);
        assert_eq!(p.av_min_rounds, 2);
        assert_eq!(p.av_stalled_rounds, 4);

        assert_eq!(p.avalanche_cutoffs.len(), 4);
        let init = p.avalanche_cutoffs[&AvalancheState::Init];
        assert_eq!(init.consensus_time, 0);
        assert_eq!(init.consensus_pct, 50);
        assert_eq!(init.next, AvalancheState::Mid);

        let mid = p.avalanche_cutoffs[&AvalancheState::Mid];
        assert_eq!(mid.consensus_time, 50);
        assert_eq!(mid.consensus_pct, 65);
        assert_eq!(mid.next, AvalancheState::Late);

        let late = p.avalanche_cutoffs[&AvalancheState::Late];
        assert_eq!(late.consensus_time, 85);
        assert_eq!(late.consensus_pct, 70);
        assert_eq!(late.next, AvalancheState::Stuck);

        let stuck = p.avalanche_cutoffs[&AvalancheState::Stuck];
        assert_eq!(stuck.consensus_time, 200);
        assert_eq!(stuck.consensus_pct, 95);
        assert_eq!(stuck.next, AvalancheState::Stuck);
    }

    /// Reference vectors for `getNeededWeight`, covering every state
    /// transition boundary. Values chosen to mirror rippled's own
    /// `Consensus_test.cpp` coverage of `getNeededWeight`.
    #[test]
    fn get_needed_weight_reference_vectors() {
        let p = ConsensusParms::default();

        // Below minimum_rounds: never transitions, even past the cutoff time.
        let (pct, next) = get_needed_weight(&p, AvalancheState::Init, 100, 0, 2);
        assert_eq!(pct, 50);
        assert_eq!(next, None);

        // At/above minimum_rounds, but before the next cutoff's time: stays.
        let (pct, next) = get_needed_weight(&p, AvalancheState::Init, 49, 2, 2);
        assert_eq!(pct, 50);
        assert_eq!(next, None);

        // At the exact cutoff boundary: transitions.
        let (pct, next) = get_needed_weight(&p, AvalancheState::Init, 50, 2, 2);
        assert_eq!(pct, 65);
        assert_eq!(next, Some(AvalancheState::Mid));

        let (pct, next) = get_needed_weight(&p, AvalancheState::Mid, 85, 2, 2);
        assert_eq!(pct, 70);
        assert_eq!(next, Some(AvalancheState::Late));

        let (pct, next) = get_needed_weight(&p, AvalancheState::Late, 200, 2, 2);
        assert_eq!(pct, 95);
        assert_eq!(next, Some(AvalancheState::Stuck));

        // Stuck loops back on itself: `next == current_state`, so the
        // transition guard (`current_cutoff.next != current_state`) blocks
        // any further transition regardless of time or rounds.
        let (pct, next) = get_needed_weight(&p, AvalancheState::Stuck, 1000, 100, 2);
        assert_eq!(pct, 95);
        assert_eq!(next, None);
    }
}
