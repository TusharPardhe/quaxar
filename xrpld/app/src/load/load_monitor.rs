//! Port of rippled `LoadMonitor` (`libxrpl/core/detail/LoadMonitor.cpp`):
//! per-job-type event rate and latency, decayed by a quarter each second.
//! `JobQueue::get_json` reports it as `per_second`, `avg_time`, `peak_time`
//! and `over_target`.
//!
//! The owner serializes access (the job queue's state mutex), so this has no
//! lock of its own, unlike rippled's.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// rippled `UptimeClock`: whole seconds since the process started.
fn uptime_seconds() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs()
}

/// rippled `LoadMonitor::Stats`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LoadStats {
    /// Events per second.
    pub count: u64,
    pub latency_avg_ms: u64,
    pub latency_peak_ms: u64,
    pub is_overloaded: bool,
}

#[derive(Debug, Clone, Default)]
pub struct LoadMonitor {
    counts: u64,
    latency_events: u64,
    latency_ms_avg: u64,
    latency_ms_peak: u64,
    target_latency_avg_ms: u64,
    target_latency_peak_ms: u64,
    last_update: u64,
}

impl LoadMonitor {
    pub fn new(target_avg: Duration, target_peak: Duration) -> Self {
        Self {
            target_latency_avg_ms: millis(target_avg),
            target_latency_peak_ms: millis(target_peak),
            last_update: uptime_seconds(),
            ..Self::default()
        }
    }

    /// rippled `LoadMonitor::update`: decay a quarter per elapsed second,
    /// or reset after more than 8 s without an update.
    fn update(&mut self, now: u64) {
        if now == self.last_update {
            return;
        }
        if now < self.last_update || now > self.last_update + 8 {
            self.counts = 0;
            self.latency_events = 0;
            self.latency_ms_avg = 0;
            self.latency_ms_peak = 0;
            self.last_update = now;
            return;
        }
        while self.last_update < now {
            self.last_update += 1;
            self.counts -= (self.counts + 3) / 4;
            self.latency_events -= (self.latency_events + 3) / 4;
            self.latency_ms_avg -= self.latency_ms_avg / 4;
            self.latency_ms_peak -= self.latency_ms_peak / 4;
        }
    }

    /// rippled `addLoadSample`: one event whose wait plus run time is
    /// `total`; under 2 ms counts as 0 ("jitter").
    pub fn add_load_sample(&mut self, total: Duration) {
        let latency = if total < Duration::from_millis(2) {
            0
        } else {
            // round<milliseconds>
            u64::try_from((total.as_nanos() + 500_000) / 1_000_000).unwrap_or(u64::MAX)
        };
        self.add_samples_at(1, latency, uptime_seconds());
    }

    /// rippled `addSamples(count, latency)`.
    pub fn add_samples(&mut self, count: u64, latency: Duration) {
        self.add_samples_at(count, millis(latency), uptime_seconds());
    }

    fn add_samples_at(&mut self, count: u64, latency_ms: u64, now: u64) {
        if count == 0 {
            return;
        }
        self.update(now);
        self.counts += count;
        self.latency_events += count;
        self.latency_ms_avg += latency_ms;
        self.latency_ms_peak += latency_ms;
        let peak = self
            .latency_events
            .saturating_mul(latency_ms)
            .saturating_mul(4)
            / count;
        if self.latency_ms_peak < peak {
            self.latency_ms_peak = peak;
        }
    }

    fn is_over_target(&self, avg_ms: u64, peak_ms: u64) -> bool {
        (self.target_latency_peak_ms > 0 && peak_ms > self.target_latency_peak_ms)
            || (self.target_latency_avg_ms > 0 && avg_ms > self.target_latency_avg_ms)
    }

    /// rippled `getStats`.
    pub fn stats(&mut self) -> LoadStats {
        self.stats_at(uptime_seconds())
    }

    fn stats_at(&mut self, now: u64) -> LoadStats {
        self.update(now);
        let (avg, peak) = if self.latency_events == 0 {
            (0, 0)
        } else {
            let divisor = self.latency_events * 4;
            (
                self.latency_ms_avg / divisor,
                self.latency_ms_peak / divisor,
            )
        };
        LoadStats {
            count: self.counts / 4,
            latency_avg_ms: avg,
            latency_peak_ms: peak,
            is_overloaded: self.is_over_target(avg, peak),
        }
    }
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::LoadMonitor;
    use std::time::Duration;

    /// Ten events a second, 200 ms in total each second. The values are
    /// rippled's integer arithmetic: the count idles just under 40 (9 per
    /// second), and the average is a quarter of the 20 ms per event because
    /// rippled divides the decayed sum by four times the decayed count.
    #[test]
    fn steady_rate_matches_rippled_arithmetic() {
        let mut monitor = LoadMonitor::new(Duration::ZERO, Duration::ZERO);
        monitor.last_update = 100;
        for second in 100..140 {
            monitor.add_samples_at(10, 200, second);
        }
        let stats = monitor.stats_at(139);
        assert_eq!(stats.count, 9);
        assert_eq!(stats.latency_avg_ms, 5);
        assert_eq!(stats.latency_peak_ms, 20);
        assert!(!stats.is_overloaded, "no targets");
    }

    #[test]
    fn over_target_and_reset_after_eight_seconds() {
        let mut monitor = LoadMonitor::new(Duration::from_millis(10), Duration::from_millis(100));
        monitor.last_update = 100;
        monitor.add_samples_at(4, 4 * 50, 100);
        let stats = monitor.stats_at(100);
        assert_eq!(stats.count, 1);
        assert_eq!(stats.latency_avg_ms, 50 / 4);
        assert!(stats.is_overloaded, "avg over its 10 ms target");
        assert_eq!(monitor.stats_at(109).count, 0, "stale data is reset");
    }

    #[test]
    fn samples_under_two_ms_are_jitter() {
        let mut monitor = LoadMonitor::new(Duration::ZERO, Duration::ZERO);
        monitor.add_load_sample(Duration::from_micros(1_900));
        let stats = monitor.stats();
        assert_eq!(stats.latency_avg_ms, 0);
        assert_eq!(stats.latency_peak_ms, 0);
    }
}
