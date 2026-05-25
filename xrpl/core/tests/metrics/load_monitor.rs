use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use xrpl_core::load_monitor::LoadMonitorJournal;
use xrpl_core::{LoadEvent, LoadMonitor, LoadMonitorStats};

#[derive(Default)]
struct RecordingJournal {
    entries: Mutex<Vec<(String, String)>>,
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

#[test]
fn load_monitor_with_journal_emits_cpp_style_info_logging() {
    let journal = Arc::new(RecordingJournal::default());
    let monitor = Arc::new(LoadMonitor::with_journal(journal.clone()));

    {
        let _event = LoadEvent::new(Arc::clone(&monitor), "job", true);
        std::thread::sleep(Duration::from_millis(600));
    }

    let entries = journal.entries.lock().expect("recording journal poisoned");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].0, "info");
    assert!(entries[0].1.contains("Job: job"));
    assert!(entries[0].1.contains("wait:"));
    assert!(entries[0].1.contains("run:"));
}

#[test]
fn load_monitor_with_journal_emits_cpp_style_warn_logging_over_one_second() {
    let journal = Arc::new(RecordingJournal::default());
    let monitor = Arc::new(LoadMonitor::with_journal(journal.clone()));

    {
        let _event = LoadEvent::new(Arc::clone(&monitor), "job-warn", true);
        std::thread::sleep(Duration::from_millis(1_100));
    }

    let entries = journal.entries.lock().expect("recording journal poisoned");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].0, "warn");
    assert!(entries[0].1.contains("Job: job-warn"));
}

#[test]
fn load_monitor_stats_constructor_matches_public_shape() {
    let stats = LoadMonitorStats::new(
        4,
        Duration::from_millis(250),
        Duration::from_millis(1_000),
        true,
    );

    assert_eq!(stats.count, 4);
    assert_eq!(stats.latency_avg, Duration::from_millis(250));
    assert_eq!(stats.latency_peak, Duration::from_millis(1_000));
    assert!(stats.is_overloaded);
    assert_eq!(LoadMonitor::new().get_stats(), LoadMonitorStats::default());
}

#[test]
fn load_monitor_target_checks_match_public_cpp_style_overload_queries() {
    let monitor = LoadMonitor::new();
    monitor.set_target_latency(Duration::from_millis(250), Duration::from_millis(1_000));

    assert!(!monitor.is_over_target(Duration::from_millis(250), Duration::from_millis(1_000)));
    assert!(monitor.is_over_target(Duration::from_millis(251), Duration::from_millis(1_000)));
    assert!(monitor.is_over_target(Duration::from_millis(250), Duration::from_millis(1_001)));
}

#[test]
fn load_event_stop_does_not_double_report_when_dropped() {
    let monitor = Arc::new(LoadMonitor::new());
    let mut event = LoadEvent::new(Arc::clone(&monitor), "job", true);

    event.stop();
    let after_stop = format!("{monitor:?}");
    assert!(after_stop.contains("counts: 1"));
    assert!(after_stop.contains("latency_events: 1"));

    drop(event);
    let after_drop = format!("{monitor:?}");
    assert!(after_drop.contains("counts: 1"));
    assert!(after_drop.contains("latency_events: 1"));
}
