//! Supporting types for the [`crate::algorithm::consensus::Consensus`] state
//! machine. Ported from rippled's `ConsensusTypes.h`.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use basics::unordered_containers::{HashMap, HashSet};

use crate::model::{ConsensusProposal, DisputedTx};

/// How a node currently participates in consensus.
///
/// ```text
///   proposing               observing
///      \                       /
///       \---> wrong_ledger <---/
///                  ^
///                  |
///                  v
///             switched_ledger
/// ```
///
/// We enter the round proposing or observing. If we detect we are working
/// on the wrong prior ledger, we go to `WrongLedger` and attempt to acquire
/// the right one. Once acquired, we go to `SwitchedLedger`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsensusMode {
    /// Normal participant in consensus; we propose our position.
    Proposing,
    /// Observing peer positions, but not proposing our own.
    Observing,
    /// We have the wrong ledger and are attempting to acquire the right one.
    WrongLedger,
    /// We switched ledgers since starting this round but are now running on
    /// what we believe is the correct ledger.
    SwitchedLedger,
}

impl std::fmt::Display for ConsensusMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ConsensusMode::Proposing => "proposing",
            ConsensusMode::Observing => "observing",
            ConsensusMode::WrongLedger => "wrongLedger",
            ConsensusMode::SwitchedLedger => "switchedLedger",
        };
        f.write_str(s)
    }
}

/// Phases of consensus for a single ledger round.
///
/// ```text
///       "close"             "accept"
///  open ------- > establish ---------> accepted
///    ^               |                    |
///    |---------------|                    |
///    ^                     "start_round"   |
///    |------------------------------------|
/// ```
///
/// The typical transition goes open -> establish -> accepted, then a call
/// to `start_round` begins the process anew. If a wrong prior ledger is
/// detected during establish or accept, consensus internally goes back to
/// open (see `Consensus::handle_wrong_ledger`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsensusPhase {
    /// We haven't closed our ledger yet, but others might have.
    Open,
    /// Establishing consensus by exchanging proposals with peers.
    Establish,
    /// We have accepted a new last-closed ledger and are waiting on a call
    /// to `start_round` to begin the next round.
    Accepted,
}

impl std::fmt::Display for ConsensusPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ConsensusPhase::Open => "open",
            ConsensusPhase::Establish => "establish",
            ConsensusPhase::Accepted => "accepted",
        };
        f.write_str(s)
    }
}

/// Measures the duration of phases of consensus. Matches `ConsensusTimer`.
///
/// The reference has two overloads of `tick`: one that adds a fixed
/// duration (used by `simulate`), and one that measures elapsed time from
/// `start_` to a given `time_point` (used during normal operation). Rust
/// can't overload on parameter type alone in a way that reads as cleanly,
/// so these are split into `tick_fixed` and `tick_to`.
#[derive(Debug, Clone, Copy, Default)]
pub struct ConsensusTimer {
    start: Option<Instant>,
    dur: Duration,
}

impl ConsensusTimer {
    pub fn read(&self) -> Duration {
        self.dur
    }

    /// Add a fixed duration. Matches `tick(std::chrono::milliseconds)`.
    pub fn tick_fixed(&mut self, fixed: Duration) {
        self.dur += fixed;
    }

    /// Reset the timer to start counting from `tp`. Matches `reset()`.
    pub fn reset(&mut self, tp: Instant) {
        self.start = Some(tp);
        self.dur = Duration::ZERO;
    }

    /// Update the duration to the elapsed time since `reset`, as measured
    /// at `tp`. Matches `tick(time_point)`.
    pub fn tick_to(&mut self, tp: Instant) {
        if let Some(start) = self.start {
            self.dur = tp.saturating_duration_since(start);
        }
    }
}

/// Stores the set of initial close times reported by peers, for analysis
/// of clock drift. Matches `ConsensusCloseTimes`.
#[derive(Debug, Clone, Default)]
pub struct ConsensusCloseTimes {
    /// Close time estimates from peers, ordered for predictable traversal.
    pub peers: BTreeMap<basics::chrono::NetClockTimePoint, i32>,
    /// Our own close time estimate.
    pub self_: basics::chrono::NetClockTimePoint,
}

/// Whether we have, or don't have, consensus. Matches `ConsensusState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsensusState {
    /// We do not have consensus.
    No,
    /// The network has consensus without us.
    MovedOn,
    /// Consensus time limit has hard-expired.
    Expired,
    /// We have consensus along with the network.
    Yes,
}

/// Encapsulates the result of consensus for a single ledger round. Matches
/// `ConsensusResult<Traits>`.
///
/// The reference asserts `txns.id() == position.position()` in its
/// constructor. The Rust port keeps that as a `debug_assert!` in `new`.
pub struct ConsensusResult<TxSet, NodeId, LedgerId, TxSetId, Tx, TxId>
where
    TxSetId: Eq + std::hash::Hash + Clone,
    TxId: Eq + std::hash::Hash + Ord + Clone + ToString,
    NodeId: Eq + std::hash::Hash + Ord + Clone + ToString,
{
    /// The set of transactions consensus agrees go in the ledger.
    pub txns: TxSet,
    /// Our proposed position on transactions/close time.
    pub position: ConsensusProposal<NodeId, LedgerId, TxSetId>,
    /// Transactions under dispute with our peers.
    pub disputes: HashMap<TxId, DisputedTx<Tx, TxId, NodeId>>,
    /// Set of `TxSet` ids we have already compared/created disputes for.
    pub compares: HashSet<TxSetId>,
    /// Duration of the establish phase for this round.
    pub round_time: ConsensusTimer,
    /// State in which consensus ended. Once in the accept phase, will be
    /// `Yes`, `MovedOn`, or `Expired`.
    pub state: ConsensusState,
    /// The number of peers proposing during the round.
    pub proposers: usize,
}

impl<TxSet, NodeId, LedgerId, TxSetId, Tx, TxId>
    ConsensusResult<TxSet, NodeId, LedgerId, TxSetId, Tx, TxId>
where
    TxSetId: Eq + std::hash::Hash + Clone + PartialEq,
    TxId: Eq + std::hash::Hash + Ord + Clone + ToString,
    NodeId: Eq + std::hash::Hash + Ord + Clone + ToString,
{
    pub fn new(
        txns: TxSet,
        position: ConsensusProposal<NodeId, LedgerId, TxSetId>,
        txns_id: TxSetId,
    ) -> Self {
        debug_assert!(
            txns_id == position.position().clone(),
            "ConsensusResult::new: txns id must match position"
        );
        Self {
            txns,
            position,
            disputes: HashMap::default(),
            compares: HashSet::default(),
            round_time: ConsensusTimer::default(),
            state: ConsensusState::No,
            proposers: 0,
        }
    }
}

/// How many of `participants` must agree to reach `percent`.
///
/// The number may not precisely yield the requested percentage: with
/// `participants = 5` and `percent = 70`, this returns 3 (60%). There are
/// no security implications to this rounding. Matches `participantsNeeded`.
pub fn participants_needed(participants: i32, percent: i32) -> i32 {
    let result = ((participants * percent) + (percent / 2)) / 100;
    if result == 0 { 1 } else { result }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn participants_needed_matches_reference_examples() {
        // 5 participants at 70% -> 3 (not exactly 70%, matches reference note).
        assert_eq!(participants_needed(5, 70), 3);
        // Zero result is bumped to 1.
        assert_eq!(participants_needed(1, 10), 1);
        assert_eq!(participants_needed(10, 80), 8);
    }

    #[test]
    fn consensus_timer_tracks_elapsed_and_fixed_durations() {
        let mut timer = ConsensusTimer::default();
        assert_eq!(timer.read(), Duration::ZERO);

        let start = Instant::now();
        timer.reset(start);
        timer.tick_to(start + Duration::from_millis(500));
        assert_eq!(timer.read(), Duration::from_millis(500));

        timer.tick_fixed(Duration::from_millis(100));
        assert_eq!(timer.read(), Duration::from_millis(600));
    }

    #[test]
    fn mode_and_phase_display_match_reference_strings() {
        assert_eq!(ConsensusMode::Proposing.to_string(), "proposing");
        assert_eq!(ConsensusMode::Observing.to_string(), "observing");
        assert_eq!(ConsensusMode::WrongLedger.to_string(), "wrongLedger");
        assert_eq!(ConsensusMode::SwitchedLedger.to_string(), "switchedLedger");

        assert_eq!(ConsensusPhase::Open.to_string(), "open");
        assert_eq!(ConsensusPhase::Establish.to_string(), "establish");
        assert_eq!(ConsensusPhase::Accepted.to_string(), "accepted");
    }
}
