//! Free functions governing ledger-close and consensus-reached decisions.
//! Ported from rippled's `Consensus.cpp`.

use std::time::Duration;

use tracing::{debug, trace, warn};

use crate::algorithm::params::ConsensusParms;
use crate::algorithm::types::ConsensusState;

/// Determines whether the current ledger should close at this time.
///
/// Call when a ledger is open and there is no close in progress, or when a
/// transaction is received and no close is in progress.
///
/// - `any_transactions`: whether any transactions have been received.
/// - `prev_proposers`: proposers in the last closing.
/// - `proposers_closed`: proposers who have currently closed this ledger.
/// - `proposers_validated`: proposers who have validated the last closed
///   ledger.
/// - `prev_round_time`: time for the previous ledger to reach consensus.
/// - `time_since_prev_close`: time since the previous ledger's (possibly
///   rounded) close time.
/// - `open_time`: duration this ledger has been open.
/// - `idle_interval`: the network's desired idle interval.
#[allow(clippy::too_many_arguments)]
pub fn should_close_ledger(
    any_transactions: bool,
    prev_proposers: usize,
    proposers_closed: usize,
    proposers_validated: usize,
    prev_round_time: Duration,
    time_since_prev_close: Duration,
    open_time: Duration,
    idle_interval: Duration,
    parms: &ConsensusParms,
) -> bool {
    // The reference guards against a negative prevRoundTime (represented in
    // Rust as a saturating Duration, which cannot go negative) and against
    // unexpectedly large values for either duration; those are unexpected
    // cases where we just close the ledger.
    if prev_round_time > Duration::from_secs(600) || time_since_prev_close > Duration::from_secs(600) {
        warn!(
            any_transactions,
            prev_proposers,
            proposers_closed,
            time_since_prev_close_secs = time_since_prev_close.as_secs(),
            prev_round_time_secs = prev_round_time.as_secs(),
            "shouldCloseLedger: unexpected timing, closing ledger"
        );
        return true;
    }

    if (proposers_closed + proposers_validated) > (prev_proposers / 2) {
        // More than half the network has closed; we close too.
        trace!("shouldCloseLedger: others have closed");
        return true;
    }

    if !any_transactions {
        // Only close at the end of the idle interval.
        return time_since_prev_close >= idle_interval;
    }

    // Preserve minimum ledger open time.
    if open_time < parms.ledger_min_close {
        debug!("shouldCloseLedger: must wait minimum time before closing");
        return false;
    }

    // Don't let this ledger close more than twice as fast as the previous
    // ledger reached consensus, so slower validators can slow down the
    // network.
    if open_time < prev_round_time / 2 {
        debug!("shouldCloseLedger: ledger has not been open long enough");
        return false;
    }

    true
}

/// Whether we have reached the required agreement threshold.
///
/// - `agreeing`: number of participants agreeing with the position under
///   test.
/// - `total`: total number of participants considered.
/// - `count_self`: whether to count ourselves as an additional agreeing
///   participant.
/// - `min_consensus_pct`: required percentage for consensus.
/// - `reached_max`: whether the maximum consensus duration has elapsed.
/// - `stalled`: whether the network appears stalled on disputed
///   transactions.
fn check_consensus_reached(
    mut agreeing: usize,
    mut total: usize,
    count_self: bool,
    min_consensus_pct: usize,
    reached_max: bool,
    stalled: bool,
) -> bool {
    // If we are alone for too long, we have consensus. Delaying consensus
    // like this avoids a peer racing ahead of proposers because it hasn't
    // received any. The reachedMax check gives proposals time to arrive.
    if total == 0 {
        return reached_max;
    }

    // We only get stalled when disputed transactions unequivocally have
    // >= min_consensus_pct agreement either for or against. This prevents
    // manipulation by a minority of byzantine peers.
    if stalled {
        return true;
    }

    if count_self {
        agreeing += 1;
        total += 1;
    }

    let current_percentage = (agreeing * 100) / total;
    current_percentage >= min_consensus_pct
}

/// Determine whether the network reached consensus and whether we joined.
///
/// - `prev_proposers`: proposers in the last closing (not including us).
/// - `current_proposers`: proposers in this closing so far (not including
///   us).
/// - `current_agree`: proposers who agree with us.
/// - `current_finished`: proposers who have validated a ledger after this
///   one.
/// - `previous_agree_time`: how long it took to agree on the last ledger.
/// - `current_agree_time`: how long we've been trying to agree.
/// - `stalled`: the network appears to be stalled -- neither we nor our
///   peers have changed a disputed-transaction vote in a while.
/// - `proposing`: whether we should count ourselves.
#[allow(clippy::too_many_arguments)]
pub fn check_consensus(
    prev_proposers: usize,
    current_proposers: usize,
    current_agree: usize,
    current_finished: usize,
    previous_agree_time: Duration,
    current_agree_time: Duration,
    stalled: bool,
    parms: &ConsensusParms,
    proposing: bool,
) -> ConsensusState {
    if current_agree_time <= parms.ledger_min_consensus {
        return ConsensusState::No;
    }

    if current_proposers < (prev_proposers * 3 / 4) {
        // Less than 3/4 of the last ledger's proposers are present; don't
        // rush, we may need more time.
        if current_agree_time < (previous_agree_time + parms.ledger_min_consensus) {
            trace!("checkConsensus: too fast, not enough proposers");
            return ConsensusState::No;
        }
    }

    let reached_max = current_agree_time > parms.ledger_max_consensus;

    // Have we, together with the nodes on our UNL, reached the threshold to
    // declare consensus?
    if check_consensus_reached(
        current_agree,
        current_proposers,
        proposing,
        parms.min_consensus_pct,
        reached_max,
        stalled,
    ) {
        debug!(stalled, "checkConsensus: normal consensus");
        return ConsensusState::Yes;
    }

    // Have sufficient nodes on our UNL moved on and reached the threshold?
    if check_consensus_reached(current_finished, current_proposers, false, parms.min_consensus_pct, reached_max, false) {
        warn!("checkConsensus: we see no consensus, but enough nodes have moved on");
        return ConsensusState::MovedOn;
    }

    let max_agree_time = previous_agree_time.saturating_mul(parms.ledger_abandon_consensus_factor as u32);
    let clamped_max = max_agree_time.clamp(parms.ledger_max_consensus, parms.ledger_abandon_consensus);
    if current_agree_time > clamped_max {
        warn!("checkConsensus: consensus taken too long");
        // Note: the Expired result may be overridden by the caller.
        return ConsensusState::Expired;
    }

    trace!("checkConsensus: no consensus yet");
    ConsensusState::No
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parms() -> ConsensusParms {
        ConsensusParms::default()
    }

    #[test]
    fn should_close_ledger_closes_on_unexpected_timing() {
        let p = parms();
        assert!(should_close_ledger(
            true,
            10,
            0,
            0,
            Duration::from_secs(700),
            Duration::from_secs(1),
            Duration::from_secs(1),
            p.ledger_idle_interval,
            &p,
        ));
    }

    #[test]
    fn should_close_ledger_closes_when_majority_already_closed() {
        let p = parms();
        // prev_proposers=10, half=5; closed(3)+validated(3)=6 > 5.
        assert!(should_close_ledger(
            true,
            10,
            3,
            3,
            Duration::from_secs(1),
            Duration::from_millis(500),
            Duration::from_millis(500),
            p.ledger_idle_interval,
            &p,
        ));
    }

    #[test]
    fn should_close_ledger_waits_for_idle_interval_with_no_transactions() {
        let p = parms();
        assert!(!should_close_ledger(
            false,
            10,
            0,
            0,
            Duration::from_secs(1),
            Duration::from_secs(1),
            Duration::from_secs(1),
            p.ledger_idle_interval,
            &p,
        ));
        assert!(should_close_ledger(
            false,
            10,
            0,
            0,
            Duration::from_secs(1),
            p.ledger_idle_interval,
            Duration::from_secs(1),
            p.ledger_idle_interval,
            &p,
        ));
    }

    #[test]
    fn should_close_ledger_respects_min_close_and_half_prev_round_time() {
        let p = parms();
        // Under ledger_min_close: don't close.
        assert!(!should_close_ledger(
            true,
            10,
            0,
            0,
            Duration::from_secs(4),
            Duration::from_secs(0),
            Duration::from_millis(500),
            p.ledger_idle_interval,
            &p,
        ));
        // Past min close but under half of prev_round_time: don't close.
        assert!(!should_close_ledger(
            true,
            10,
            0,
            0,
            Duration::from_secs(10),
            Duration::from_secs(0),
            p.ledger_min_close + Duration::from_millis(1),
            p.ledger_idle_interval,
            &p,
        ));
        // Past both thresholds: close.
        assert!(should_close_ledger(
            true,
            10,
            0,
            0,
            Duration::from_secs(2),
            Duration::from_secs(0),
            Duration::from_secs(2),
            p.ledger_idle_interval,
            &p,
        ));
    }

    #[test]
    fn check_consensus_no_before_min_consensus_time() {
        let p = parms();
        let state = check_consensus(10, 10, 10, 0, Duration::from_secs(2), p.ledger_min_consensus, false, &p, true);
        assert_eq!(state, ConsensusState::No);
    }

    #[test]
    fn check_consensus_yes_when_threshold_reached() {
        let p = parms();
        // 10 proposers, all agreeing, well past min consensus time.
        let state = check_consensus(
            10,
            10,
            10,
            0,
            Duration::from_secs(2),
            p.ledger_min_consensus + Duration::from_secs(1),
            false,
            &p,
            true,
        );
        assert_eq!(state, ConsensusState::Yes);
    }

    #[test]
    fn check_consensus_moved_on_when_others_finished_without_us() {
        let p = parms();
        // No agreement with our position at all, but plenty have finished.
        let state = check_consensus(
            10,
            10,
            0,
            9,
            Duration::from_secs(2),
            p.ledger_min_consensus + Duration::from_secs(1),
            false,
            &p,
            true,
        );
        assert_eq!(state, ConsensusState::MovedOn);
    }

    #[test]
    fn check_consensus_expired_after_abandon_threshold() {
        let p = parms();
        // Nobody agrees, nobody finished, and we've well exceeded max consensus.
        let state = check_consensus(10, 10, 0, 0, Duration::from_secs(2), p.ledger_abandon_consensus + Duration::from_secs(1), false, &p, true);
        assert_eq!(state, ConsensusState::Expired);
    }

    #[test]
    fn check_consensus_stalled_forces_yes() {
        let p = parms();
        let state = check_consensus(
            10,
            10,
            0,
            0,
            Duration::from_secs(2),
            p.ledger_min_consensus + Duration::from_secs(1),
            true,
            &p,
            true,
        );
        assert_eq!(state, ConsensusState::Yes);
    }
}
