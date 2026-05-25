//! Rust port of `xrpl/basics/chrono.h`.
//!
//! The reference header does three important jobs:
//! - define convenience durations like `days` and `weeks`,
//! - define the Ripple network epoch and a branded network clock time point,
//! - provide string formatting helpers for UTC timestamps.
//!
//! Rust does not have the same `std::chrono::time_point<Clock>` type pattern,
//! so we model the branded network time as a small newtype instead.

use std::ops::{Add, Sub};
use std::sync::Mutex;
use std::time::{Duration as StdDuration, Instant};
use time::{Duration, Month, OffsetDateTime};

/// The number of Unix seconds between 1970-01-01 and 2000-01-01 UTC.
pub const EPOCH_OFFSET_SECONDS: i64 = 946_684_800;

/// The Ripple network epoch offset from the Unix epoch.
pub fn epoch_offset() -> Duration {
    Duration::seconds(EPOCH_OFFSET_SECONDS)
}

/// Convenience helper matching the role of `xrpl::days`.
pub fn days(count: i64) -> Duration {
    Duration::hours(count * 24)
}

/// Convenience helper matching the role of `xrpl::weeks`.
pub fn weeks(count: i64) -> Duration {
    days(count * 7)
}

/// Branded network time measured in whole seconds since 2000-01-01 UTC.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NetClockTimePoint {
    seconds_since_epoch: u32,
}

impl NetClockTimePoint {
    pub const fn new(seconds_since_epoch: u32) -> Self {
        Self {
            seconds_since_epoch,
        }
    }

    pub const fn as_seconds(self) -> u32 {
        self.seconds_since_epoch
    }

    pub fn time_since_epoch(self) -> Duration {
        Duration::seconds(i64::from(self.seconds_since_epoch))
    }

    pub fn to_datetime(self) -> OffsetDateTime {
        datetime_from_unix_seconds(i64::from(self.seconds_since_epoch) + EPOCH_OFFSET_SECONDS)
    }

    pub fn checked_add(self, duration: Duration) -> Option<Self> {
        let delta = whole_seconds(duration)?;
        let seconds = i64::from(self.seconds_since_epoch).checked_add(delta)?;
        u32::try_from(seconds).ok().map(Self::new)
    }

    pub fn checked_sub(self, duration: Duration) -> Option<Self> {
        let delta = whole_seconds(duration)?;
        let seconds = i64::from(self.seconds_since_epoch).checked_sub(delta)?;
        u32::try_from(seconds).ok().map(Self::new)
    }
}

impl From<u32> for NetClockTimePoint {
    fn from(value: u32) -> Self {
        Self::new(value)
    }
}

impl Add<Duration> for NetClockTimePoint {
    type Output = Self;

    fn add(self, rhs: Duration) -> Self::Output {
        self.checked_add(rhs)
            .expect("NetClockTimePoint addition must stay within u32 whole-second bounds")
    }
}

impl Sub<Duration> for NetClockTimePoint {
    type Output = Self;

    fn sub(self, rhs: Duration) -> Self::Output {
        self.checked_sub(rhs)
            .expect("NetClockTimePoint subtraction must stay within u32 whole-second bounds")
    }
}

impl Sub for NetClockTimePoint {
    type Output = Duration;

    fn sub(self, rhs: Self) -> Self::Output {
        Duration::seconds(i64::from(self.seconds_since_epoch) - i64::from(rhs.seconds_since_epoch))
    }
}

/// Shared formatting boundary so one function name can support both
/// `OffsetDateTime` and `NetClockTimePoint`, similar to reference overloads.
pub trait TimeFormat {
    fn to_xrpl_string(self) -> String;
    fn to_xrpl_iso_string(self) -> String;
}

pub fn to_string<T>(value: T) -> String
where
    T: TimeFormat,
{
    value.to_xrpl_string()
}

pub fn to_string_iso<T>(value: T) -> String
where
    T: TimeFormat,
{
    value.to_xrpl_iso_string()
}

impl TimeFormat for OffsetDateTime {
    fn to_xrpl_string(self) -> String {
        format_human_utc(self)
    }

    fn to_xrpl_iso_string(self) -> String {
        format_iso_utc(self)
    }
}

impl TimeFormat for NetClockTimePoint {
    fn to_xrpl_string(self) -> String {
        format_human_utc(self.to_datetime())
    }

    fn to_xrpl_iso_string(self) -> String {
        format_iso_utc(self.to_datetime())
    }
}

/// Wall-clock stopwatch boundary matching the role of `xrpl::Stopwatch`.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemStopwatch;

impl SystemStopwatch {
    pub fn now(&self) -> Instant {
        Instant::now()
    }
}

/// Manual stopwatch boundary for tests, matching the role of `xrpl::TestStopwatch`.
#[derive(Debug)]
pub struct ManualStopwatch {
    now: Mutex<Instant>,
}

impl ManualStopwatch {
    pub fn new(start: Instant) -> Self {
        Self {
            now: Mutex::new(start),
        }
    }

    pub fn now(&self) -> Instant {
        self.now
            .lock()
            .expect("manual stopwatch mutex poisoned")
            .to_owned()
    }

    pub fn set(&self, instant: Instant) {
        *self.now.lock().expect("manual stopwatch mutex poisoned") = instant;
    }

    pub fn advance(&self, duration: StdDuration) {
        let next = self.now() + duration;
        self.set(next);
    }
}

impl Default for ManualStopwatch {
    fn default() -> Self {
        Self::new(Instant::now())
    }
}

pub type Stopwatch = SystemStopwatch;
pub type TestStopwatch = ManualStopwatch;

static STOPWATCH: Stopwatch = SystemStopwatch;

pub fn stopwatch() -> &'static Stopwatch {
    &STOPWATCH
}

fn whole_seconds(duration: Duration) -> Option<i64> {
    let seconds = duration.whole_seconds();
    if duration == Duration::seconds(seconds) {
        Some(seconds)
    } else {
        None
    }
}

fn datetime_from_unix_seconds(unix_seconds: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(unix_seconds)
        .expect("NetClock time point should always map to a valid UTC timestamp")
}

fn format_human_utc(datetime: OffsetDateTime) -> String {
    format!(
        "{:04}-{}-{:02} {:02}:{:02}:{:02} UTC",
        datetime.year(),
        short_month(datetime.month()),
        datetime.day(),
        datetime.hour(),
        datetime.minute(),
        datetime.second()
    )
}

fn format_iso_utc(datetime: OffsetDateTime) -> String {
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        datetime.year(),
        u8::from(datetime.month()),
        datetime.day(),
        datetime.hour(),
        datetime.minute(),
        datetime.second()
    )
}

fn short_month(month: Month) -> &'static str {
    match month {
        Month::January => "Jan",
        Month::February => "Feb",
        Month::March => "Mar",
        Month::April => "Apr",
        Month::May => "May",
        Month::June => "Jun",
        Month::July => "Jul",
        Month::August => "Aug",
        Month::September => "Sep",
        Month::October => "Oct",
        Month::November => "Nov",
        Month::December => "Dec",
    }
}

#[cfg(test)]
mod tests {
    use super::{
        EPOCH_OFFSET_SECONDS, ManualStopwatch, NetClockTimePoint, TimeFormat, days, epoch_offset,
        stopwatch, to_string, to_string_iso, weeks,
    };
    use std::time::{Duration as StdDuration, Instant};
    use time::Duration;

    #[test]
    fn duration_helpers_match_cpp_values() {
        assert_eq!(days(1), Duration::hours(24));
        assert_eq!(weeks(1), Duration::hours(24 * 7));
        assert_eq!(epoch_offset(), Duration::seconds(EPOCH_OFFSET_SECONDS));
    }

    #[test]
    fn net_clock_time_point_uses_ripple_epoch() {
        let network_time = NetClockTimePoint::new(20);

        assert_eq!(network_time.as_seconds(), 20);
        assert_eq!(network_time.time_since_epoch(), Duration::seconds(20));
        assert_eq!(network_time.to_xrpl_string(), "2000-Jan-01 00:00:20 UTC");
        assert_eq!(network_time.to_xrpl_iso_string(), "2000-01-01T00:00:20Z");
    }

    #[test]
    fn net_clock_arithmetic_stays_in_whole_seconds() {
        let base = NetClockTimePoint::new(300);

        assert_eq!((base + Duration::seconds(20)).as_seconds(), 320);
        assert_eq!((base - Duration::seconds(50)).as_seconds(), 250);
        assert_eq!(base - NetClockTimePoint::new(240), Duration::seconds(60));
        assert_eq!(base.checked_add(Duration::milliseconds(1)), None);
    }

    #[test]
    fn formatting_supports_offset_date_time_and_net_clock() {
        let datetime = NetClockTimePoint::new(10).to_datetime();

        assert_eq!(to_string(datetime), "2000-Jan-01 00:00:10 UTC");
        assert_eq!(to_string_iso(datetime), "2000-01-01T00:00:10Z");
        assert_eq!(
            to_string(NetClockTimePoint::new(10)),
            "2000-Jan-01 00:00:10 UTC"
        );
        assert_eq!(
            to_string_iso(NetClockTimePoint::new(10)),
            "2000-01-01T00:00:10Z"
        );
    }

    #[test]
    fn manual_stopwatch_can_be_advanced_deterministically() {
        let start = Instant::now();
        let clock = ManualStopwatch::new(start);

        assert_eq!(clock.now(), start);

        clock.advance(StdDuration::from_secs(3));
        assert_eq!(clock.now().duration_since(start), StdDuration::from_secs(3));
    }

    #[test]
    fn global_stopwatch_exposes_wall_clock_role() {
        let first = stopwatch().now();
        let second = stopwatch().now();

        assert!(second >= first);
    }
}
