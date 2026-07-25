//! Current `xrpld/core/TimeKeeper` runtime slice.
//!
//! The reference owner is a very small clock seam:
//! - `now()` returns network time (system clock shifted by the Ripple epoch),
//! - `closeTime()` adds the current close offset,
//! - `adjustCloseTime()` uses the same asymmetric quarter-step update rules.

use std::sync::atomic::{AtomicI64, Ordering};

use basics::chrono::{EPOCH_OFFSET_SECONDS, NetClockTimePoint};
use time::Duration;

pub trait TimeKeeperClock: Send + Sync + 'static {
    fn now_unix_seconds(&self) -> i64;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemTimeKeeperClock;

impl TimeKeeperClock for SystemTimeKeeperClock {
    fn now_unix_seconds(&self) -> i64 {
        time::OffsetDateTime::now_utc().unix_timestamp()
    }
}

pub struct TimeKeeper<C = SystemTimeKeeperClock> {
    clock: C,
    close_offset_seconds: AtomicI64,
    /// SNTP-reported offset in seconds.
    /// Positive = our clock is behind the server (we are slow).
    /// Applied by adding to the raw Unix timestamp in `now()`.
    sntp_offset_seconds: AtomicI64,
}

impl Default for TimeKeeper<SystemTimeKeeperClock> {
    fn default() -> Self {
        Self::new()
    }
}

impl TimeKeeper<SystemTimeKeeperClock> {
    pub fn new() -> Self {
        Self::with_clock(SystemTimeKeeperClock)
    }
}

impl<C> TimeKeeper<C>
where
    C: TimeKeeperClock,
{
    pub fn with_clock(clock: C) -> Self {
        Self {
            clock,
            close_offset_seconds: AtomicI64::new(0),
            sntp_offset_seconds: AtomicI64::new(0),
        }
    }

    pub fn now(&self) -> NetClockTimePoint {
        let sntp = self.sntp_offset_seconds.load(Ordering::Acquire);
        // Add the SNTP offset: positive means we are slow, so adding moves our
        // reported time forward to match the server clock.
        let unix = self.clock.now_unix_seconds().saturating_add(sntp);
        let net_seconds = unix.saturating_sub(EPOCH_OFFSET_SECONDS);
        NetClockTimePoint::new(u32::try_from(net_seconds).unwrap_or_default())
    }

    pub fn close_time(&self) -> NetClockTimePoint {
        self.now() + self.close_offset()
    }

    pub fn close_offset(&self) -> Duration {
        Duration::seconds(self.close_offset_seconds.load(Ordering::Acquire))
    }

    /// Update the SNTP-reported offset.  Called by the SNTP client when a
    /// new median offset is computed.  Positive = our clock is slow.
    pub fn set_sntp_offset(&self, offset_secs: i64) {
        self.sntp_offset_seconds.store(offset_secs, Ordering::Release);
    }

    /// Current SNTP offset in seconds.
    pub fn sntp_offset(&self) -> i64 {
        self.sntp_offset_seconds.load(Ordering::Acquire)
    }

    pub fn adjust_close_time(&self, by: Duration) -> Duration {
        let current = self.close_offset_seconds.load(Ordering::Acquire);
        if by == Duration::seconds(0) && current == 0 {
            return Duration::seconds(current);
        }

        let by_seconds = by.whole_seconds();
        let updated = if by_seconds > 1 {
            current + ((by_seconds + 3) / 4)
        } else if by_seconds < -1 {
            current + ((by_seconds - 3) / 4)
        } else {
            (current * 3) / 4
        };

        let _ = self.close_offset_seconds.compare_exchange(
            current,
            updated,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        Duration::seconds(self.close_offset_seconds.load(Ordering::Acquire))
    }
}

#[cfg(test)]
mod tests {
    use super::{TimeKeeper, TimeKeeperClock};
    use basics::chrono::{EPOCH_OFFSET_SECONDS, NetClockTimePoint};
    use std::sync::atomic::{AtomicI64, Ordering};
    use time::Duration;

    #[derive(Debug)]
    struct FixedClock {
        now: AtomicI64,
    }

    impl FixedClock {
        fn new(now: i64) -> Self {
            Self {
                now: AtomicI64::new(now),
            }
        }
    }

    impl TimeKeeperClock for FixedClock {
        fn now_unix_seconds(&self) -> i64 {
            self.now.load(Ordering::Relaxed)
        }
    }

    #[test]
    fn now_uses_ripple_epoch_adjustment() {
        let keeper = TimeKeeper::with_clock(FixedClock::new(EPOCH_OFFSET_SECONDS + 123));
        assert_eq!(keeper.now(), NetClockTimePoint::new(123));
    }

    #[test]
    fn sntp_offset_corrects_slow_clock() {
        // Positive offset means our clock is 10 s behind the server (slow).
        // now() must add the offset so the reported network time is corrected
        // forward, not pushed further behind.
        let keeper = TimeKeeper::with_clock(FixedClock::new(EPOCH_OFFSET_SECONDS + 100));
        keeper.set_sntp_offset(10); // we are 10 s slow
        assert_eq!(keeper.now(), NetClockTimePoint::new(110));
    }

    #[test]
    fn sntp_offset_corrects_fast_clock() {
        // Negative offset means our clock is 5 s ahead of the server (fast).
        // now() must subtract to correct backward.
        let keeper = TimeKeeper::with_clock(FixedClock::new(EPOCH_OFFSET_SECONDS + 100));
        keeper.set_sntp_offset(-5); // we are 5 s fast
        assert_eq!(keeper.now(), NetClockTimePoint::new(95));
    }

    #[test]
    fn close_time_adds_current_offset() {
        let keeper = TimeKeeper::with_clock(FixedClock::new(EPOCH_OFFSET_SECONDS + 1_000));
        keeper.adjust_close_time(Duration::seconds(8));

        assert_eq!(keeper.close_time(), NetClockTimePoint::new(1_002));
    }

    #[test]
    fn adjust_close_time_quarter_steps() {
        let keeper = TimeKeeper::with_clock(FixedClock::new(EPOCH_OFFSET_SECONDS));

        assert_eq!(
            keeper.adjust_close_time(Duration::seconds(8)),
            Duration::seconds(2)
        );
        assert_eq!(
            keeper.adjust_close_time(Duration::seconds(2)),
            Duration::seconds(3)
        );
        assert_eq!(
            keeper.adjust_close_time(Duration::seconds(-8)),
            Duration::seconds(1)
        );
        assert_eq!(
            keeper.adjust_close_time(Duration::seconds(0)),
            Duration::seconds(0)
        );
    }
}
