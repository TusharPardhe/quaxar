use basics::chrono::NetClockTimePoint;
use consensus::{
    ConsensusParms, ConsensusState, DECREASE_LEDGER_TIME_RESOLUTION_EVERY,
    INCREASE_LEDGER_TIME_RESOLUTION_EVERY, LEDGER_DEFAULT_TIME_RESOLUTION,
    LEDGER_GENESIS_TIME_RESOLUTION, LEDGER_POSSIBLE_TIME_RESOLUTIONS, check_consensus,
    effective_close_time, get_next_ledger_time_resolution, round_close_time, should_close_ledger,
};
use std::time::Duration as StdDuration;
use time::Duration as TimeDuration;

#[test]
fn close_time_resolution_increase_and_decrease_schedule() {
    assert_eq!(
        get_next_ledger_time_resolution(
            LEDGER_DEFAULT_TIME_RESOLUTION,
            true,
            INCREASE_LEDGER_TIME_RESOLUTION_EVERY
        ),
        LEDGER_POSSIBLE_TIME_RESOLUTIONS[1]
    );
    assert_eq!(
        get_next_ledger_time_resolution(
            LEDGER_DEFAULT_TIME_RESOLUTION,
            false,
            DECREASE_LEDGER_TIME_RESOLUTION_EVERY
        ),
        LEDGER_POSSIBLE_TIME_RESOLUTIONS[3]
    );
    assert_eq!(
        get_next_ledger_time_resolution(
            LEDGER_GENESIS_TIME_RESOLUTION,
            true,
            INCREASE_LEDGER_TIME_RESOLUTION_EVERY
        ),
        LEDGER_GENESIS_TIME_RESOLUTION
    );
    assert_eq!(
        get_next_ledger_time_resolution(
            *LEDGER_POSSIBLE_TIME_RESOLUTIONS.last().unwrap(),
            false,
            DECREASE_LEDGER_TIME_RESOLUTION_EVERY
        ),
        *LEDGER_POSSIBLE_TIME_RESOLUTIONS.last().unwrap()
    );
}

#[test]
fn round_close_time_midpoint_round_up_rule() {
    let close_time = NetClockTimePoint::new(95);
    assert_eq!(
        round_close_time(close_time, TimeDuration::seconds(30)),
        NetClockTimePoint::new(90)
    );

    let midpoint = NetClockTimePoint::new(75);
    assert_eq!(
        round_close_time(midpoint, TimeDuration::seconds(30)),
        NetClockTimePoint::new(90)
    );
}

#[test]
fn effective_close_time_prior_close_floor() {
    let raw = NetClockTimePoint::new(91);
    let prior = NetClockTimePoint::new(95);
    assert_eq!(
        effective_close_time(raw, TimeDuration::seconds(10), prior),
        NetClockTimePoint::new(96)
    );

    let later = NetClockTimePoint::new(125);
    assert_eq!(
        effective_close_time(later, TimeDuration::seconds(10), prior),
        NetClockTimePoint::new(130)
    );
}

#[test]
fn should_close_ledger_reference_vectors() {
    let parms = ConsensusParms::default();

    assert!(should_close_ledger(
        true,
        10,
        10,
        10,
        StdDuration::from_secs(10 * 60 + 1),
        StdDuration::from_secs(10),
        StdDuration::from_secs(1),
        StdDuration::from_secs(1),
        &parms,
    ));
    assert!(should_close_ledger(
        true,
        10,
        10,
        10,
        StdDuration::from_secs(10),
        StdDuration::from_secs(10 * 60 + 1),
        StdDuration::from_secs(1),
        StdDuration::from_secs(1),
        &parms,
    ));
    assert!(should_close_ledger(
        true,
        10,
        3,
        5,
        StdDuration::from_secs(10),
        StdDuration::from_secs(10),
        StdDuration::from_secs(10),
        StdDuration::from_secs(10),
        &parms,
    ));
    assert!(!should_close_ledger(
        false,
        10,
        0,
        0,
        StdDuration::from_secs(1),
        StdDuration::from_secs(1),
        StdDuration::from_secs(1),
        StdDuration::from_secs(10),
        &parms,
    ));
    assert!(should_close_ledger(
        false,
        10,
        0,
        0,
        StdDuration::from_secs(1),
        StdDuration::from_secs(10),
        StdDuration::from_secs(1),
        StdDuration::from_secs(10),
        &parms,
    ));
    assert!(!should_close_ledger(
        true,
        10,
        0,
        0,
        StdDuration::from_secs(10),
        StdDuration::from_secs(10),
        StdDuration::from_secs(1),
        StdDuration::from_secs(10),
        &parms,
    ));
    assert!(!should_close_ledger(
        true,
        10,
        0,
        0,
        StdDuration::from_secs(10),
        StdDuration::from_secs(10),
        StdDuration::from_secs(3),
        StdDuration::from_secs(10),
        &parms,
    ));
    assert!(should_close_ledger(
        true,
        10,
        0,
        0,
        StdDuration::from_secs(10),
        StdDuration::from_secs(10),
        StdDuration::from_secs(10),
        StdDuration::from_secs(10),
        &parms,
    ));
}

#[test]
fn check_consensus_reference_vectors() {
    let parms = ConsensusParms::default();

    assert_eq!(
        check_consensus(
            10,
            2,
            2,
            0,
            StdDuration::from_secs(3),
            StdDuration::from_secs(2),
            false,
            &parms,
            true,
        ),
        ConsensusState::No
    );
    assert_eq!(
        check_consensus(
            10,
            2,
            2,
            0,
            StdDuration::from_secs(3),
            StdDuration::from_secs(4),
            false,
            &parms,
            true,
        ),
        ConsensusState::No
    );
    assert_eq!(
        check_consensus(
            10,
            2,
            2,
            0,
            StdDuration::from_secs(3),
            StdDuration::from_secs(10),
            false,
            &parms,
            true,
        ),
        ConsensusState::Yes
    );
    assert_eq!(
        check_consensus(
            10,
            2,
            1,
            0,
            StdDuration::from_secs(3),
            StdDuration::from_secs(10),
            false,
            &parms,
            true,
        ),
        ConsensusState::No
    );
    assert_eq!(
        check_consensus(
            10,
            2,
            1,
            8,
            StdDuration::from_secs(3),
            StdDuration::from_secs(10),
            false,
            &parms,
            true,
        ),
        ConsensusState::MovedOn
    );
    assert_eq!(
        check_consensus(
            0,
            0,
            0,
            0,
            StdDuration::from_secs(3),
            StdDuration::from_secs(10),
            false,
            &parms,
            true,
        ),
        ConsensusState::No
    );
    assert_eq!(
        check_consensus(
            0,
            0,
            0,
            0,
            StdDuration::from_secs(3),
            StdDuration::from_secs(16),
            false,
            &parms,
            true,
        ),
        ConsensusState::Yes
    );
    assert_eq!(
        check_consensus(
            10,
            8,
            1,
            0,
            StdDuration::from_secs(1),
            StdDuration::from_secs(19),
            false,
            &parms,
            true,
        ),
        ConsensusState::Expired
    );

    assert_eq!(
        check_consensus(
            10,
            2,
            2,
            0,
            StdDuration::from_secs(3),
            StdDuration::from_secs(2),
            true,
            &parms,
            true,
        ),
        ConsensusState::No
    );
    assert_eq!(
        check_consensus(
            10,
            2,
            2,
            0,
            StdDuration::from_secs(3),
            StdDuration::from_secs(4),
            true,
            &parms,
            true,
        ),
        ConsensusState::No
    );
    assert_eq!(
        check_consensus(
            10,
            2,
            2,
            0,
            StdDuration::from_secs(3),
            StdDuration::from_secs(10),
            true,
            &parms,
            true,
        ),
        ConsensusState::Yes
    );
    assert_eq!(
        check_consensus(
            10,
            2,
            1,
            0,
            StdDuration::from_secs(3),
            StdDuration::from_secs(10),
            true,
            &parms,
            true,
        ),
        ConsensusState::Yes
    );
    assert_eq!(
        check_consensus(
            10,
            2,
            1,
            8,
            StdDuration::from_secs(3),
            StdDuration::from_secs(10),
            true,
            &parms,
            true,
        ),
        ConsensusState::Yes
    );
    assert_eq!(
        check_consensus(
            0,
            0,
            0,
            0,
            StdDuration::from_secs(3),
            StdDuration::from_secs(10),
            true,
            &parms,
            true,
        ),
        ConsensusState::No
    );
    assert_eq!(
        check_consensus(
            0,
            0,
            0,
            0,
            StdDuration::from_secs(3),
            StdDuration::from_secs(16),
            true,
            &parms,
            true,
        ),
        ConsensusState::Yes
    );
    assert_eq!(
        check_consensus(
            10,
            8,
            1,
            0,
            StdDuration::from_secs(1),
            StdDuration::from_secs(19),
            true,
            &parms,
            true,
        ),
        ConsensusState::Yes
    );
}

// === Adversarial / Fault-Injection: Clock Jump Backward ===

#[test]
fn clock_jump_backward_effective_close_time_clamps_to_parent() {
    // If system clock jumps backward, effective_close_time should never
    // produce a close time earlier than the parent's close time.
    let parent_close = NetClockTimePoint::new(1000);
    let resolution = TimeDuration::seconds(10);

    // Clock jumped backward — raw close time is before parent
    let raw_close = NetClockTimePoint::new(900);
    let effective = effective_close_time(raw_close, resolution, parent_close);

    // Must be >= parent close time (clamped)
    assert!(
        effective >= parent_close,
        "effective_close_time must not go backward: got {:?} < parent {:?}",
        effective,
        parent_close
    );
}

#[test]
fn clock_jump_backward_should_close_ledger_handles_zero_elapsed() {
    // If clock jumps backward, open_time could appear as zero.
    // should_close_ledger must not panic and should respect min_close time.
    let parms = ConsensusParms::default();

    // With transactions pending but zero open_time (clock jumped back),
    // the ledger should NOT close because open_time < ledger_min_close.
    let result = should_close_ledger(
        true,                       // any transactions
        5,                          // prev proposers
        0,                          // proposers closed
        0,                          // proposers validated
        StdDuration::from_secs(4),  // previous round time
        StdDuration::from_secs(0),  // time since prev close
        StdDuration::from_secs(0),  // open time (clock jumped back → appears zero)
        StdDuration::from_secs(15), // idle interval
        &parms,
    );
    // With zero open_time and transactions, should not close (below min_close)
    assert!(!result, "should not close ledger when open_time is zero due to clock jump");
}

#[test]
fn clock_jump_backward_round_close_time_never_before_epoch() {
    // Ensure round_close_time handles a time point near zero gracefully
    let close_time = NetClockTimePoint::new(5);
    let resolution = TimeDuration::seconds(30);

    let rounded = round_close_time(close_time, resolution);
    // Should not underflow or wrap
    assert!(rounded.as_seconds() < 1_000_000_000);
}
