//! Rust port of `xrpl/basics/UptimeClock.h`.
//!
//! The reference clock caches program uptime in seconds using a background thread so
//! hot paths can read an atomic value instead of querying the system clock on
//! every call.

use std::ops::{Add, Sub};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
use time::Duration as SignedDuration;

/// Cached seconds since first use of the uptime clock.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UptimeTimePoint {
    seconds_since_start: i32,
}

impl UptimeTimePoint {
    pub const fn new(seconds_since_start: i32) -> Self {
        Self {
            seconds_since_start,
        }
    }

    pub const fn as_seconds(self) -> i32 {
        self.seconds_since_start
    }

    pub fn time_since_epoch(self) -> Duration {
        Duration::from_secs(self.seconds_since_start as u64)
    }
}

impl Add<Duration> for UptimeTimePoint {
    type Output = Self;

    fn add(self, rhs: Duration) -> Self::Output {
        let delta =
            i32::try_from(rhs.as_secs()).expect("UptimeTimePoint addition must fit in i32 seconds");
        Self::new(
            self.seconds_since_start
                .checked_add(delta)
                .expect("UptimeTimePoint addition overflow"),
        )
    }
}

impl Sub<Duration> for UptimeTimePoint {
    type Output = Self;

    fn sub(self, rhs: Duration) -> Self::Output {
        let delta = i32::try_from(rhs.as_secs())
            .expect("UptimeTimePoint subtraction must fit in i32 seconds");
        Self::new(
            self.seconds_since_start
                .checked_sub(delta)
                .expect("UptimeTimePoint subtraction overflow"),
        )
    }
}

impl Sub for UptimeTimePoint {
    type Output = SignedDuration;

    fn sub(self, rhs: Self) -> Self::Output {
        SignedDuration::seconds(i64::from(
            self.seconds_since_start - rhs.seconds_since_start,
        ))
    }
}

/// Tracks program uptime to seconds precision.
#[derive(Clone, Copy, Debug, Default)]
pub struct UptimeClock;

impl UptimeClock {
    /// Returns seconds since first use of the uptime clock.
    pub fn now() -> UptimeTimePoint {
        let state = global_state();
        state.cached_now()
    }
}

struct UptimeState {
    start: Instant,
    now_secs: AtomicI32,
    stop: AtomicBool,
}

impl UptimeState {
    fn new(start: Instant) -> Self {
        Self {
            start,
            now_secs: AtomicI32::new(0),
            stop: AtomicBool::new(false),
        }
    }

    fn cached_now(&self) -> UptimeTimePoint {
        UptimeTimePoint::new(self.now_secs.load(Ordering::Relaxed))
    }

    fn refresh_from(&self, now: Instant) {
        let elapsed = now.saturating_duration_since(self.start).as_secs();
        let seconds = elapsed.min(i32::MAX as u64) as i32;
        self.now_secs.store(seconds, Ordering::Relaxed);
    }

    fn should_stop(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }
}

fn global_state() -> &'static Arc<UptimeState> {
    static STATE: OnceLock<Arc<UptimeState>> = OnceLock::new();

    STATE.get_or_init(|| {
        let state = Arc::new(UptimeState::new(Instant::now()));
        spawn_updater(Arc::clone(&state));
        state
    })
}

fn spawn_updater(state: Arc<UptimeState>) {
    thread::Builder::new()
        .name("xrpl-uptime-clock".to_owned())
        .spawn(move || {
            let mut next = Instant::now() + Duration::from_secs(1);
            while !state.should_stop() {
                let now = Instant::now();
                if now < next {
                    thread::sleep(next - now);
                    continue;
                }

                state.refresh_from(now);
                next += Duration::from_secs(1);
            }
        })
        .expect("UptimeClock updater thread should start");
}

#[cfg(test)]
mod tests {
    use super::{UptimeClock, UptimeState, UptimeTimePoint};
    use std::time::{Duration, Instant};
    use time::Duration as SignedDuration;

    #[test]
    fn uptime_time_point_supports_seconds_and_arithmetic() {
        let base = UptimeTimePoint::new(10);

        assert_eq!(base.as_seconds(), 10);
        assert_eq!(base.time_since_epoch(), Duration::from_secs(10));
        assert_eq!((base + Duration::from_secs(5)).as_seconds(), 15);
        assert_eq!((base - Duration::from_secs(3)).as_seconds(), 7);
        assert_eq!(base - UptimeTimePoint::new(4), SignedDuration::seconds(6));
        assert_eq!(UptimeTimePoint::new(4) - base, SignedDuration::seconds(-6));
    }

    #[test]
    fn state_refresh_uses_elapsed_whole_seconds() {
        let start = Instant::now();
        let state = UptimeState::new(start);

        state.refresh_from(start + Duration::from_millis(999));
        assert_eq!(state.cached_now().as_seconds(), 0);

        state.refresh_from(start + Duration::from_millis(1_500));
        assert_eq!(state.cached_now().as_seconds(), 1);

        state.refresh_from(start + Duration::from_millis(2_900));
        assert_eq!(state.cached_now().as_seconds(), 2);
    }

    #[test]
    fn global_clock_is_non_decreasing() {
        let first = UptimeClock::now();
        let second = UptimeClock::now();

        assert!(second >= first);
    }
}
