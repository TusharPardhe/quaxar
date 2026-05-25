use std::fmt;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::load_event::LoadEvent;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadMonitorStats {
    pub count: u64,
    pub latency_avg: Duration,
    pub latency_peak: Duration,
    pub is_overloaded: bool,
}

impl Default for LoadMonitorStats {
    fn default() -> Self {
        Self {
            count: 0,
            latency_avg: Duration::ZERO,
            latency_peak: Duration::ZERO,
            is_overloaded: false,
        }
    }
}

impl LoadMonitorStats {
    pub const fn new(
        count: u64,
        latency_avg: Duration,
        latency_peak: Duration,
        is_overloaded: bool,
    ) -> Self {
        Self {
            count,
            latency_avg,
            latency_peak,
            is_overloaded,
        }
    }
}

pub trait LoadMonitorJournal: Send + Sync + 'static {
    fn debug(&self, _message: &str) {}
    fn info(&self, _message: &str) {}
    fn warn(&self, _message: &str) {}
}

pub trait LoadMonitorJournalFactory: Send + Sync + 'static {
    fn make_load_monitor_journal(&self, name: &str) -> Arc<dyn LoadMonitorJournal>;
}

#[derive(Debug, Default)]
pub struct NullLoadMonitorJournal;

impl LoadMonitorJournal for NullLoadMonitorJournal {}

trait LoadMonitorClock: Send + Sync + 'static {
    fn now(&self) -> u64;
}

#[derive(Debug)]
struct SystemLoadMonitorClock {
    start: Instant,
}

impl Default for SystemLoadMonitorClock {
    fn default() -> Self {
        Self {
            start: Instant::now(),
        }
    }
}

impl LoadMonitorClock for SystemLoadMonitorClock {
    fn now(&self) -> u64 {
        Instant::now()
            .saturating_duration_since(self.start)
            .as_secs()
    }
}

struct LoadMonitorState {
    counts: u64,
    latency_events: u64,
    latency_avg_sum: Duration,
    latency_peak_sum: Duration,
    target_latency_avg: Duration,
    target_latency_peak: Duration,
    clock: Arc<dyn LoadMonitorClock>,
    last_update: u64,
    journal: Arc<dyn LoadMonitorJournal>,
}

impl LoadMonitorState {
    fn new(journal: Arc<dyn LoadMonitorJournal>) -> Self {
        Self::with_clock(journal, Arc::new(SystemLoadMonitorClock::default()))
    }

    fn with_clock(journal: Arc<dyn LoadMonitorJournal>, clock: Arc<dyn LoadMonitorClock>) -> Self {
        Self {
            counts: 0,
            latency_events: 0,
            latency_avg_sum: Duration::ZERO,
            latency_peak_sum: Duration::ZERO,
            target_latency_avg: Duration::ZERO,
            target_latency_peak: Duration::ZERO,
            clock,
            last_update: 0,
            journal,
        }
    }

    fn now(&self) -> u64 {
        self.clock.now()
    }

    fn update(&mut self) {
        let now = self.now();
        if now == self.last_update {
            return;
        }

        // Mirror the reference unsigned time-point comparison instead of letting
        // debug builds panic on the addition when the counter wraps.
        let stale_cutoff = self.last_update.wrapping_add(8);
        if now < self.last_update || now > stale_cutoff {
            self.counts = 0;
            self.latency_events = 0;
            self.latency_avg_sum = Duration::ZERO;
            self.latency_peak_sum = Duration::ZERO;
            self.last_update = now;
            return;
        }

        loop {
            self.last_update += 1;
            self.counts -= self.counts.div_ceil(4);
            self.latency_events -= self.latency_events.div_ceil(4);
            self.latency_avg_sum = self
                .latency_avg_sum
                .saturating_sub(self.latency_avg_sum / 4);
            self.latency_peak_sum = self
                .latency_peak_sum
                .saturating_sub(self.latency_peak_sum / 4);

            if self.last_update >= now {
                break;
            }
        }
    }

    fn over_target(&self, avg: Duration, peak: Duration) -> bool {
        (!self.target_latency_peak.is_zero() && peak > self.target_latency_peak)
            || (!self.target_latency_avg.is_zero() && avg > self.target_latency_avg)
    }

    fn snapshot(&self) -> LoadMonitorStats {
        if self.latency_events == 0 {
            return LoadMonitorStats::new(self.counts / 4, Duration::ZERO, Duration::ZERO, false);
        }

        let divisor = u32::try_from(self.latency_events.saturating_mul(4))
            .unwrap_or(u32::MAX)
            .max(1);
        let latency_avg = self.latency_avg_sum / divisor;
        let latency_peak = self.latency_peak_sum / divisor;

        LoadMonitorStats {
            count: self.counts / 4,
            latency_avg,
            latency_peak,
            is_overloaded: self.over_target(latency_avg, latency_peak),
        }
    }

    fn log_latency(&self, name: &str, run_time: Duration, wait_time: Duration, latency: Duration) {
        if latency <= Duration::from_millis(500) {
            return;
        }

        let message = format!(
            "Job: {name} run: {}ms wait: {}ms",
            round_to_millis(run_time).as_millis(),
            round_to_millis(wait_time).as_millis()
        );

        if latency > Duration::from_secs(1) {
            self.journal.warn(&message);
        } else {
            self.journal.info(&message);
        }
    }
}

pub struct LoadMonitor {
    state: Mutex<LoadMonitorState>,
}

impl fmt::Debug for LoadMonitor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.state.lock().expect("load monitor mutex poisoned");
        f.debug_struct("LoadMonitor")
            .field("counts", &state.counts)
            .field("latency_events", &state.latency_events)
            .field("target_latency_avg", &state.target_latency_avg)
            .field("target_latency_peak", &state.target_latency_peak)
            .finish()
    }
}

impl Default for LoadMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl LoadMonitor {
    pub fn new() -> Self {
        Self::with_journal(Arc::new(NullLoadMonitorJournal))
    }

    pub fn with_journal(journal: Arc<dyn LoadMonitorJournal>) -> Self {
        Self {
            state: Mutex::new(LoadMonitorState::new(journal)),
        }
    }

    #[cfg(test)]
    fn with_journal_and_clock(
        journal: Arc<dyn LoadMonitorJournal>,
        clock: Arc<dyn LoadMonitorClock>,
    ) -> Self {
        Self {
            state: Mutex::new(LoadMonitorState::with_clock(journal, clock)),
        }
    }

    pub fn add_load_sample(&self, sample: &LoadEvent) {
        let run_time = sample.run_time();
        let wait_time = sample.wait_time();
        let total = run_time + wait_time;
        let latency = if total < Duration::from_millis(2) {
            Duration::ZERO
        } else {
            round_to_millis(total)
        };
        let state = self.state.lock().expect("load monitor mutex poisoned");
        state.log_latency(sample.name(), run_time, wait_time, latency);
        drop(state);
        self.add_samples(1, latency);
    }

    pub fn add_samples(&self, count: i32, latency: Duration) {
        let Ok(count) = u64::try_from(count) else {
            return;
        };
        if count == 0 {
            return;
        }

        let mut state = self.state.lock().expect("load monitor mutex poisoned");
        state.update();
        // instead of saturating the counters at u64::MAX.
        state.counts = state.counts.wrapping_add(count);
        state.latency_events = state.latency_events.wrapping_add(count);
        state.latency_avg_sum = state.latency_avg_sum.saturating_add(latency);
        state.latency_peak_sum = state.latency_peak_sum.saturating_add(latency);

        let factor = state.latency_events.wrapping_mul(4) / count;
        let latency_peak = if let Ok(factor) = u32::try_from(factor) {
            latency.checked_mul(factor).unwrap_or(Duration::MAX)
        } else {
            Duration::MAX
        };

        if state.latency_peak_sum < latency_peak {
            state.latency_peak_sum = latency_peak;
        }
    }

    pub fn set_target_latency(&self, avg: Duration, peak: Duration) {
        let mut state = self.state.lock().expect("load monitor mutex poisoned");
        state.target_latency_avg = avg;
        state.target_latency_peak = peak;
    }

    pub fn is_over_target(&self, avg: Duration, peak: Duration) -> bool {
        let state = self.state.lock().expect("load monitor mutex poisoned");
        state.over_target(avg, peak)
    }

    pub fn is_over(&self) -> bool {
        self.get_stats().is_overloaded
    }

    pub fn get_stats(&self) -> LoadMonitorStats {
        let mut state = self.state.lock().expect("load monitor mutex poisoned");
        state.update();
        state.snapshot()
    }
}

fn round_to_millis(duration: Duration) -> Duration {
    let nanos = duration.as_nanos();
    let whole_millis = nanos / 1_000_000;
    let remainder_nanos = nanos % 1_000_000;
    let millis = if remainder_nanos < 500_000 {
        whole_millis
    } else if remainder_nanos > 500_000 {
        whole_millis.saturating_add(1)
    } else if whole_millis.is_multiple_of(2) {
        whole_millis
    } else {
        whole_millis.saturating_add(1)
    };
    Duration::from_millis(u64::try_from(millis).unwrap_or(u64::MAX))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::Mutex as StdMutex;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;
    use std::time::Duration;

    use super::{LoadMonitorClock, LoadMonitorJournal, LoadMonitorState};
    use crate::{LoadEvent, LoadMonitor};

    #[derive(Default)]
    struct RecordingJournal {
        entries: StdMutex<Vec<(String, String)>>,
    }

    impl LoadMonitorJournal for RecordingJournal {
        fn info(&self, message: &str) {
            self.entries
                .lock()
                .expect("recording journal poisoned")
                .push(("info".to_owned(), message.to_owned()));
        }

        fn warn(&self, message: &str) {
            self.entries
                .lock()
                .expect("recording journal poisoned")
                .push(("warn".to_owned(), message.to_owned()));
        }
    }

    #[derive(Default)]
    struct ManualClock {
        now: AtomicU64,
    }

    impl ManualClock {
        fn set(&self, now: u64) {
            self.now.store(now, Ordering::SeqCst);
        }
    }

    impl LoadMonitorClock for ManualClock {
        fn now(&self) -> u64 {
            self.now.load(Ordering::SeqCst)
        }
    }

    #[test]
    fn load_monitor_logs_over_threshold_messages_journal() {
        let journal = Arc::new(RecordingJournal::default());
        let state = LoadMonitorState::new(journal.clone());

        state.log_latency(
            "job-info",
            Duration::from_millis(260),
            Duration::from_millis(340),
            Duration::from_millis(600),
        );
        state.log_latency(
            "job-warn",
            Duration::from_millis(450),
            Duration::from_millis(650),
            Duration::from_millis(1_100),
        );

        let entries = journal.entries.lock().expect("recording journal poisoned");
        assert_eq!(
            entries
                .iter()
                .map(|(level, _)| level.as_str())
                .collect::<Vec<_>>(),
            vec!["info", "warn"]
        );
        assert!(entries[0].1.contains("Job: job-info"));
        assert!(entries[0].1.contains("run: 260ms"));
        assert!(entries[0].1.contains("wait: 340ms"));
        assert!(entries[1].1.contains("Job: job-warn"));
        assert!(entries[1].1.contains("run: 450ms"));
        assert!(entries[1].1.contains("wait: 650ms"));
    }

    #[test]
    fn load_monitor_decay_and_stale_reset_match_cpp_uptime_clock_rules() {
        let journal = Arc::new(RecordingJournal::default());
        let clock = Arc::new(ManualClock::default());
        let mut state = LoadMonitorState::with_clock(journal, clock.clone());
        state.counts = 10;
        state.latency_events = 10;
        state.latency_avg_sum = Duration::from_millis(400);
        state.latency_peak_sum = Duration::from_millis(800);

        clock.set(1);
        state.update();
        assert_eq!(state.counts, 7);
        assert_eq!(state.latency_events, 7);
        assert_eq!(state.latency_avg_sum, Duration::from_millis(300));
        assert_eq!(state.latency_peak_sum, Duration::from_millis(600));
        assert_eq!(state.last_update, 1);

        clock.set(10);
        state.update();
        assert_eq!(state.counts, 0);
        assert_eq!(state.latency_events, 0);
        assert_eq!(state.latency_avg_sum, Duration::ZERO);
        assert_eq!(state.latency_peak_sum, Duration::ZERO);
        assert_eq!(state.last_update, 10);
    }

    #[test]
    fn load_monitor_tracks_over_target_peak() {
        let monitor = LoadMonitor::new();
        monitor.set_target_latency(Duration::from_millis(250), Duration::from_millis(1_000));
        monitor.add_samples(1, Duration::from_millis(1_500));

        let stats = monitor.get_stats();
        assert_eq!(stats.latency_avg, Duration::from_millis(375));
        assert_eq!(stats.latency_peak, Duration::from_millis(1_500));
        assert!(stats.is_overloaded);
        assert!(monitor.is_over());
    }

    #[test]
    fn load_monitor_public_stats_follow_same_decay_shape() {
        let journal = Arc::new(RecordingJournal::default());
        let clock = Arc::new(ManualClock::default());
        let monitor = LoadMonitor::with_journal_and_clock(journal, clock.clone());
        monitor.add_samples(4, Duration::from_millis(64));

        let initial = monitor.get_stats();
        assert_eq!(initial.count, 1);
        assert_eq!(initial.latency_avg, Duration::from_millis(4));
        assert_eq!(initial.latency_peak, Duration::from_millis(16));

        clock.set(1);
        let decayed = monitor.get_stats();
        assert_eq!(decayed.count, 0);
        assert_eq!(decayed.latency_avg, Duration::from_millis(4));
        assert_eq!(decayed.latency_peak, Duration::from_millis(16));
        assert!(!decayed.is_overloaded);

        clock.set(10);
        assert_eq!(monitor.get_stats().count, 0);
        assert_eq!(monitor.get_stats().latency_avg, Duration::ZERO);
        assert_eq!(monitor.get_stats().latency_peak, Duration::ZERO);
    }

    #[test]
    fn load_monitor_wraps_unsigned_counter_state() {
        let journal = Arc::new(RecordingJournal::default());
        let clock = Arc::new(ManualClock::default());
        let monitor = LoadMonitor::with_journal_and_clock(journal, clock);
        {
            let mut state = monitor.state.lock().expect("load monitor mutex poisoned");
            state.counts = u64::MAX - 2;
            state.latency_events = u64::MAX - 2;
            state.latency_avg_sum = Duration::from_millis(4);
            state.latency_peak_sum = Duration::from_millis(8);
        }

        monitor.add_samples(5, Duration::from_millis(1));

        let state = monitor.state.lock().expect("load monitor mutex poisoned");
        assert_eq!(state.counts, 2);
        assert_eq!(state.latency_events, 2);
        assert_eq!(state.latency_avg_sum, Duration::from_millis(5));
        assert_eq!(state.latency_peak_sum, Duration::from_millis(9));
    }

    #[test]
    fn load_monitor_update_handles_wrapping_stale_cutoff() {
        let journal = Arc::new(RecordingJournal::default());
        let clock = Arc::new(ManualClock::default());
        let mut state = LoadMonitorState::with_clock(journal, clock.clone());
        state.counts = 8;
        state.latency_events = 8;
        state.last_update = u64::MAX - 4;

        clock.set(u64::MAX - 1);
        state.update();

        assert_eq!(state.last_update, u64::MAX - 1);
        assert_eq!(state.counts, 0);
        assert_eq!(state.latency_events, 0);
        assert_eq!(state.latency_avg_sum, Duration::ZERO);
        assert_eq!(state.latency_peak_sum, Duration::ZERO);
    }

    #[test]
    fn load_event_drop_reports_a_sample() {
        let monitor = Arc::new(LoadMonitor::new());
        monitor.set_target_latency(Duration::from_millis(1), Duration::from_millis(1));

        {
            let mut event = LoadEvent::new(Arc::clone(&monitor), "event", true);
            thread::sleep(Duration::from_millis(3));
            event.set_name("event-2");
        }

        let stats = monitor.get_stats();
        assert!(stats.latency_peak >= Duration::from_millis(1));
    }

    #[test]
    fn round_to_millis_chrono_round_ties_to_even() {
        assert_eq!(
            super::round_to_millis(Duration::from_nanos(500_000)),
            Duration::ZERO
        );
        assert_eq!(
            super::round_to_millis(Duration::from_nanos(1_500_000)),
            Duration::from_millis(2)
        );
        assert_eq!(
            super::round_to_millis(Duration::from_nanos(2_500_000)),
            Duration::from_millis(2)
        );
        assert_eq!(
            super::round_to_millis(Duration::from_nanos(3_500_000)),
            Duration::from_millis(4)
        );
    }
}
