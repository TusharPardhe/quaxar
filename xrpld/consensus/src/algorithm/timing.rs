//! Ledger close-time resolution binning. Ported from `LedgerTiming.h`.
//!
//! The XRPL protocol uses binning to represent time intervals using only
//! one timestamp, letting servers derive a common close time for the next
//! ledger without requiring perfectly synchronized clocks. The bin size
//! (resolution) is adjusted dynamically based on whether the previous
//! round reached close-time agreement, to try to avoid disagreements.

use basics::chrono::NetClockTimePoint;
use time::Duration;

/// Possible ledger close-time resolutions. Values must not be duplicated
/// and must be in strictly increasing order (both invariants the
/// reference documents on `kLedgerPossibleTimeResolutions`).
pub const LEDGER_POSSIBLE_TIME_RESOLUTIONS: [Duration; 6] =
    [Duration::seconds(10), Duration::seconds(20), Duration::seconds(30), Duration::seconds(60), Duration::seconds(90), Duration::seconds(120)];

/// Initial resolution of ledger close time. Matches `kLedgerDefaultTimeResolution`.
pub const LEDGER_DEFAULT_TIME_RESOLUTION: Duration = LEDGER_POSSIBLE_TIME_RESOLUTIONS[2];

/// Close-time resolution in the genesis ledger. Matches `kLedgerGenesisTimeResolution`.
pub const LEDGER_GENESIS_TIME_RESOLUTION: Duration = LEDGER_POSSIBLE_TIME_RESOLUTIONS[0];

/// How often we increase the close-time resolution (in number of
/// ledgers). Matches `kIncreaseLedgerTimeResolutionEvery`.
pub const INCREASE_LEDGER_TIME_RESOLUTION_EVERY: u32 = 8;

/// How often we decrease the close-time resolution (in number of
/// ledgers). Matches `kDecreaseLedgerTimeResolutionEvery`.
pub const DECREASE_LEDGER_TIME_RESOLUTION_EVERY: u32 = 1;

/// Calculates the close-time resolution for the specified ledger. Matches
/// `getNextLedgerTimeResolution`.
///
/// If the previous round did *not* reach close-time agreement, this tries
/// to move to a coarser (larger) resolution bin, to improve the chance of
/// agreement now. If the previous round *did* agree, this tries to move
/// to a finer (smaller) resolution bin, to see if agreement still holds
/// at higher precision. Both adjustments only happen on their respective
/// cadence (`DECREASE_LEDGER_TIME_RESOLUTION_EVERY`/
/// `INCREASE_LEDGER_TIME_RESOLUTION_EVERY` ledgers), and never move past
/// the ends of [`LEDGER_POSSIBLE_TIME_RESOLUTIONS`].
///
/// `previous_resolution` must be one of [`LEDGER_POSSIBLE_TIME_RESOLUTIONS`];
/// if it is not (should never happen in practice), this returns it
/// unchanged, matching the reference's own defensive fallback.
pub fn get_next_ledger_time_resolution(previous_resolution: Duration, previous_agree: bool, ledger_seq: u32) -> Duration {
    debug_assert_ne!(ledger_seq, 0, "get_next_ledger_time_resolution: valid ledger sequence");

    let Some(index) = LEDGER_POSSIBLE_TIME_RESOLUTIONS.iter().position(|&r| r == previous_resolution) else {
        return previous_resolution;
    };

    // If we did not previously agree, try to decrease the resolution
    // (move to a coarser/larger bin) to improve the chance we agree now.
    if !previous_agree && ledger_seq.is_multiple_of(DECREASE_LEDGER_TIME_RESOLUTION_EVERY) {
        if let Some(&coarser) = LEDGER_POSSIBLE_TIME_RESOLUTIONS.get(index + 1) {
            return coarser;
        }
        return previous_resolution;
    }

    // If we previously agreed, try to increase the resolution (move to a
    // finer/smaller bin) to determine if we can continue to agree.
    if previous_agree && ledger_seq.is_multiple_of(INCREASE_LEDGER_TIME_RESOLUTION_EVERY) {
        if index > 0 {
            return LEDGER_POSSIBLE_TIME_RESOLUTIONS[index - 1];
        }
        return previous_resolution;
    }

    previous_resolution
}

/// Calculates the close time for a ledger, given a close-time resolution.
/// Rounds up if `close_time` is exactly midway between multiples of
/// `resolution`. Matches `roundCloseTime`.
///
/// A zero `close_time` (the default/epoch value) is returned unchanged,
/// matching the reference's special-case for an unset close time.
pub fn round_close_time(close_time: NetClockTimePoint, resolution: Duration) -> NetClockTimePoint {
    if close_time == NetClockTimePoint::default() {
        return close_time;
    }

    let resolution_secs = resolution.whole_seconds().max(1);
    let raw_secs = i64::from(close_time.as_seconds());
    let shifted = raw_secs + resolution_secs / 2;
    let rounded = shifted - shifted.rem_euclid(resolution_secs);
    NetClockTimePoint::new(rounded.max(0) as u32)
}

/// Calculates the effective ledger close time: after rounding
/// `close_time` to `resolution`, also ensures it is strictly later than
/// `prior_close_time` (by at least one second), so ledgers can never
/// close with a timestamp at or before their parent's. Matches
/// `effCloseTime`.
///
/// A zero `close_time` is returned unchanged, matching `roundCloseTime`'s
/// own special case (which this function delegates to first).
pub fn effective_close_time(close_time: NetClockTimePoint, resolution: Duration, prior_close_time: NetClockTimePoint) -> NetClockTimePoint {
    if close_time == NetClockTimePoint::default() {
        return close_time;
    }

    let rounded = round_close_time(close_time, resolution);
    let min_allowed = NetClockTimePoint::new(prior_close_time.as_seconds().saturating_add(1));
    rounded.max(min_allowed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolutions_are_six_and_strictly_increasing() {
        assert_eq!(LEDGER_POSSIBLE_TIME_RESOLUTIONS.len(), 6);
        for i in 0..LEDGER_POSSIBLE_TIME_RESOLUTIONS.len() - 1 {
            assert!(LEDGER_POSSIBLE_TIME_RESOLUTIONS[i] < LEDGER_POSSIBLE_TIME_RESOLUTIONS[i + 1]);
        }
    }

    #[test]
    fn default_and_genesis_resolutions_match_reference() {
        assert_eq!(LEDGER_DEFAULT_TIME_RESOLUTION.whole_seconds(), 30);
        assert_eq!(LEDGER_GENESIS_TIME_RESOLUTION.whole_seconds(), 10);
    }

    #[test]
    fn next_resolution_decreases_on_disagreement_every_ledger() {
        assert_eq!(get_next_ledger_time_resolution(Duration::seconds(30), false, 1).whole_seconds(), 60);
        assert_eq!(get_next_ledger_time_resolution(Duration::seconds(30), false, 2).whole_seconds(), 60);
    }

    #[test]
    fn next_resolution_increases_on_agreement_every_eighth_ledger_only() {
        assert_eq!(get_next_ledger_time_resolution(Duration::seconds(30), true, 8).whole_seconds(), 20);
        assert_eq!(get_next_ledger_time_resolution(Duration::seconds(30), true, 1).whole_seconds(), 30);
        assert_eq!(get_next_ledger_time_resolution(Duration::seconds(30), true, 16).whole_seconds(), 20);
    }

    #[test]
    fn next_resolution_clamps_at_the_ends_of_the_ladder() {
        assert_eq!(get_next_ledger_time_resolution(Duration::seconds(10), true, 8).whole_seconds(), 10);
        assert_eq!(get_next_ledger_time_resolution(Duration::seconds(120), false, 1).whole_seconds(), 120);
    }

    #[test]
    fn round_close_time_zero_stays_zero() {
        assert_eq!(round_close_time(NetClockTimePoint::default(), Duration::seconds(30)), NetClockTimePoint::default());
    }

    #[test]
    fn round_close_time_rounds_up_on_ties_and_to_nearest_otherwise() {
        assert_eq!(round_close_time(NetClockTimePoint::new(45), Duration::seconds(30)).time_since_epoch().whole_seconds(), 60);
        assert_eq!(round_close_time(NetClockTimePoint::new(60), Duration::seconds(30)).time_since_epoch().whole_seconds(), 60);
        assert_eq!(round_close_time(NetClockTimePoint::new(15), Duration::seconds(10)).time_since_epoch().whole_seconds(), 20);
    }

    #[test]
    fn effective_close_time_enforces_strictly_later_than_prior() {
        let t = NetClockTimePoint::new(10);
        let prior = NetClockTimePoint::new(100);
        let effective = effective_close_time(t, Duration::seconds(30), prior);
        assert!(effective.time_since_epoch().whole_seconds() >= 101);
    }

    #[test]
    fn effective_close_time_zero_stays_zero() {
        assert_eq!(effective_close_time(NetClockTimePoint::default(), Duration::seconds(30), NetClockTimePoint::new(100)), NetClockTimePoint::default());
    }
}
