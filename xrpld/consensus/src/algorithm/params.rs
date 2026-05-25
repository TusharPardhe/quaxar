use std::collections::BTreeMap;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AvalancheState {
    Init,
    Mid,
    Late,
    Stuck,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AvalancheCutoff {
    pub consensus_time: i32,
    pub consensus_pct: usize,
    pub next: AvalancheState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsensusParms {
    pub validation_valid_wall: Duration,
    pub validation_valid_local: Duration,
    pub validation_valid_early: Duration,
    pub validation_set_expires: Duration,
    pub validation_freshness: Duration,
    pub propose_freshness: Duration,
    pub propose_interval: Duration,
    pub min_consensus_pct: usize,
    pub ledger_idle_interval: Duration,
    pub ledger_min_consensus: Duration,
    pub ledger_max_consensus: Duration,
    pub ledger_min_close: Duration,
    pub ledger_granularity: Duration,
    pub ledger_abandon_consensus_factor: usize,
    pub ledger_abandon_consensus: Duration,
    pub av_min_consensus_time: Duration,
    pub avalanche_cutoffs: BTreeMap<AvalancheState, AvalancheCutoff>,
    pub av_ct_consensus_pct: usize,
    pub av_min_rounds: usize,
    pub av_stalled_rounds: usize,
}

impl Default for ConsensusParms {
    fn default() -> Self {
        Self {
            validation_valid_wall: Duration::from_secs(5 * 60),
            validation_valid_local: Duration::from_secs(3 * 60),
            validation_valid_early: Duration::from_secs(3 * 60),
            validation_set_expires: Duration::from_secs(10 * 60),
            validation_freshness: Duration::from_secs(20),
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
        .expect("current avalanche state must exist");
    if current_cutoff.next != current_state && current_rounds >= minimum_rounds {
        let next_cutoff = parms
            .avalanche_cutoffs
            .get(&current_cutoff.next)
            .expect("next avalanche state must exist");
        if percent_time >= next_cutoff.consensus_time {
            return (next_cutoff.consensus_pct, Some(current_cutoff.next));
        }
    }
    (current_cutoff.consensus_pct, None)
}
