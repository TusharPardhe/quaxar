use app::{
    JobQueue, JobType, LoadFeeControl, LoadManager, LoadManagerEvents, LoadManagerJournal,
    LoadManagerTiming, SharedLoadFeeTrack,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Default)]
struct RecordingFees {
    raised: AtomicUsize,
    lowered: AtomicUsize,
}

impl LoadFeeControl for RecordingFees {
    fn raise_local_fee(&self) -> bool {
        self.raised.fetch_add(1, Ordering::Relaxed);
        true
    }

    fn lower_local_fee(&self) -> bool {
        self.lowered.fetch_add(1, Ordering::Relaxed);
        true
    }
}

#[derive(Default)]
struct RecordingEvents {
    reports: AtomicUsize,
}

impl LoadManagerEvents for RecordingEvents {
    fn report_fee_change(&self) {
        self.reports.fetch_add(1, Ordering::Relaxed);
    }
}

#[derive(Default)]
struct RecordingJournal {
    entries: Mutex<Vec<String>>,
}

impl LoadManagerJournal for RecordingJournal {
    fn debug(&self, message: &str) {
        self.entries
            .lock()
            .expect("journal mutex")
            .push(message.to_owned());
    }

    fn info(&self, message: &str) {
        self.entries
            .lock()
            .expect("journal mutex")
            .push(message.to_owned());
    }

    fn warn(&self, message: &str) {
        self.entries
            .lock()
            .expect("journal mutex")
            .push(message.to_owned());
    }

    fn fatal(&self, message: &str) {
        self.entries
            .lock()
            .expect("journal mutex")
            .push(message.to_owned());
    }
}

#[test]
fn load_manager_warns_on_stall_and_raises_fee_on_stop_when_queue_is_overloaded() {
    let queue = JobQueue::new();
    assert!(queue.add_job(JobType::Pack, "first", || {}));
    assert!(queue.add_job(JobType::Pack, "second", || {}));

    let fees = Arc::new(RecordingFees::default());
    let events = Arc::new(RecordingEvents::default());
    let journal = Arc::new(RecordingJournal::default());
    let manager = LoadManager::with_timing(
        queue,
        fees.clone(),
        events.clone(),
        journal.clone(),
        LoadManagerTiming {
            tick_interval: Duration::from_millis(5),
            reporting_interval: Duration::from_millis(10),
            stall_fatal_log_limit: Duration::from_millis(30),
            stall_logic_error_limit: Duration::from_millis(60),
        },
    );

    manager.start();
    manager.activate_stall_detector();
    std::thread::sleep(Duration::from_millis(18));
    manager.stop();

    let entries = journal.entries.lock().expect("journal mutex").clone();
    assert!(
        entries
            .iter()
            .any(|entry| entry.contains("Server stalled for"))
    );
    assert!(
        entries
            .iter()
            .any(|entry| entry.contains("Raising local fee"))
    );
    assert_eq!(fees.raised.load(Ordering::Relaxed), 1);
    assert_eq!(fees.lowered.load(Ordering::Relaxed), 0);
    assert_eq!(events.reports.load(Ordering::Relaxed), 1);
}

#[test]
fn load_manager_can_raise_real_load_fee_track_on_stop() {
    let queue = JobQueue::new();
    assert!(queue.add_job(JobType::Pack, "first", || {}));
    assert!(queue.add_job(JobType::Pack, "second", || {}));

    let fees = Arc::new(SharedLoadFeeTrack::new());
    let events = Arc::new(RecordingEvents::default());
    let journal = Arc::new(RecordingJournal::default());
    let manager = LoadManager::with_timing(
        queue,
        fees.clone(),
        events,
        journal,
        LoadManagerTiming {
            tick_interval: Duration::from_millis(5),
            reporting_interval: Duration::from_millis(10),
            stall_fatal_log_limit: Duration::from_millis(30),
            stall_logic_error_limit: Duration::from_millis(60),
        },
    );

    manager.start();
    manager.activate_stall_detector();
    std::thread::sleep(Duration::from_millis(18));
    manager.stop();

    let snapshot = fees.snapshot();
    assert_eq!(snapshot.raise_count, 1);
    assert_eq!(snapshot.local_fee, fees.load_base());
    assert!(fees.is_loaded_local());
}
