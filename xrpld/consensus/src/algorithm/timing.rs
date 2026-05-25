use basics::chrono::NetClockTimePoint;
use time::Duration;

pub const LEDGER_POSSIBLE_TIME_RESOLUTIONS: [Duration; 6] = [
    Duration::seconds(10),
    Duration::seconds(20),
    Duration::seconds(30),
    Duration::seconds(60),
    Duration::seconds(90),
    Duration::seconds(120),
];
pub const LEDGER_DEFAULT_TIME_RESOLUTION: Duration = LEDGER_POSSIBLE_TIME_RESOLUTIONS[2];
pub const LEDGER_GENESIS_TIME_RESOLUTION: Duration = LEDGER_POSSIBLE_TIME_RESOLUTIONS[0];
pub const INCREASE_LEDGER_TIME_RESOLUTION_EVERY: u32 = 8;
pub const DECREASE_LEDGER_TIME_RESOLUTION_EVERY: u32 = 1;

pub fn get_next_ledger_time_resolution(
    previous_resolution: Duration,
    previous_agree: bool,
    ledger_seq: u32,
) -> Duration {
    assert_ne!(ledger_seq, 0, "valid ledger sequence");
    let Some(index) = LEDGER_POSSIBLE_TIME_RESOLUTIONS
        .iter()
        .position(|resolution| *resolution == previous_resolution)
    else {
        return previous_resolution;
    };

    if !previous_agree
        && ledger_seq.is_multiple_of(DECREASE_LEDGER_TIME_RESOLUTION_EVERY)
        && let Some(next) = LEDGER_POSSIBLE_TIME_RESOLUTIONS.get(index + 1)
    {
        return *next;
    }

    if previous_agree
        && ledger_seq.is_multiple_of(INCREASE_LEDGER_TIME_RESOLUTION_EVERY)
        && index > 0
    {
        return LEDGER_POSSIBLE_TIME_RESOLUTIONS[index - 1];
    }

    previous_resolution
}

pub fn round_close_time(
    close_time: NetClockTimePoint,
    close_resolution: Duration,
) -> NetClockTimePoint {
    if close_time == NetClockTimePoint::default() {
        return close_time;
    }
    let adjusted = close_time + close_resolution / 2;
    let rounded = adjusted.time_since_epoch().whole_seconds()
        - (adjusted.time_since_epoch().whole_seconds() % close_resolution.whole_seconds());
    NetClockTimePoint::new(
        u32::try_from(rounded).expect("rounded close time must fit in u32 network seconds"),
    )
}

pub fn effective_close_time(
    close_time: NetClockTimePoint,
    resolution: Duration,
    prior_close_time: NetClockTimePoint,
) -> NetClockTimePoint {
    if close_time == NetClockTimePoint::default() {
        return close_time;
    }
    let rounded = round_close_time(close_time, resolution);
    std::cmp::max(rounded, prior_close_time + Duration::seconds(1))
}
