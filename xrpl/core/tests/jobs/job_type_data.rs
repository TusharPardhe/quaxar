use std::sync::{Arc, Mutex};
use std::time::Duration;

use xrpl_core::{
    INVALID_JOB_TYPE_INFO, JobType, JobTypeData, JobTypeDataCollector, JobTypeDataEvent,
    JobTypeInfo, LoadMonitorJournal, LoadMonitorJournalFactory, LoadMonitorStats,
};

#[derive(Default)]
struct RecordingCollector {
    created: Mutex<Vec<String>>,
    notifications: Arc<Mutex<Vec<(String, Duration)>>>,
}

struct RecordingEvent {
    name: String,
    notifications: Arc<Mutex<Vec<(String, Duration)>>>,
}

#[derive(Default)]
struct RecordingJournalFactory {
    created: Mutex<Vec<String>>,
    entries: Arc<Mutex<Vec<(String, String)>>>,
}

#[derive(Clone)]
struct RecordingJournal {
    entries: Arc<Mutex<Vec<(String, String)>>>,
}

impl LoadMonitorJournal for RecordingJournal {
    fn info(&self, message: &str) {
        self.entries
            .lock()
            .expect("journal entries mutex poisoned")
            .push(("info".to_owned(), message.to_owned()));
    }

    fn warn(&self, message: &str) {
        self.entries
            .lock()
            .expect("journal entries mutex poisoned")
            .push(("warn".to_owned(), message.to_owned()));
    }
}

impl LoadMonitorJournalFactory for RecordingJournalFactory {
    fn make_load_monitor_journal(&self, name: &str) -> Arc<dyn LoadMonitorJournal> {
        self.created
            .lock()
            .expect("created journals mutex poisoned")
            .push(name.to_owned());
        Arc::new(RecordingJournal {
            entries: Arc::clone(&self.entries),
        })
    }
}

impl JobTypeDataEvent for RecordingEvent {
    fn notify(&self, duration: Duration) {
        self.notifications
            .lock()
            .expect("notifications mutex poisoned")
            .push((self.name.clone(), duration));
    }
}

impl JobTypeDataCollector for RecordingCollector {
    fn make_event(&self, name: &str) -> Arc<dyn JobTypeDataEvent> {
        self.created
            .lock()
            .expect("created mutex poisoned")
            .push(name.to_owned());
        Arc::new(RecordingEvent {
            name: name.to_owned(),
            notifications: Arc::clone(&self.notifications),
        })
    }
}

#[test]
fn job_type_data_exposes_runtime_load_and_metadata() {
    let info = JobTypeInfo::new(
        JobType::Transaction,
        "transaction",
        4,
        Duration::from_millis(250),
        Duration::from_secs(1),
    );
    let data = JobTypeData::new(info);

    assert_eq!(data.name(), "transaction");
    assert_eq!(data.job_type(), JobType::Transaction);
    assert_eq!(data.type_(), JobType::Transaction);
    assert_eq!(data.waiting, 0);
    assert_eq!(data.running, 0);
    assert_eq!(data.deferred, 0);
    assert_eq!(data.dequeue_event_name(), Some("transaction_q"));
    assert_eq!(data.execute_event_name(), Some("transaction"));
    assert_eq!(data.events().dequeue.as_deref(), Some("transaction_q"));
    assert_eq!(data.events().execute.as_deref(), Some("transaction"));

    let stats = data.stats();
    assert_eq!(stats.average_latency, Duration::from_millis(250));
    assert_eq!(stats.peak_latency, Duration::from_secs(1));
    assert_eq!(stats.load, LoadMonitorStats::default());
    assert_eq!(data.load_stats(), LoadMonitorStats::default());

    data.load().add_samples(4, Duration::from_millis(64));

    let live = data.load_stats();
    assert_eq!(live.count, 1);
    assert_eq!(live.latency_avg, Duration::from_millis(4));
    assert_eq!(live.latency_peak, Duration::from_millis(16));
    assert!(!live.is_overloaded);
    assert_eq!(data.stats().load, live);
}

#[test]
fn job_type_data_skips_events_for_special_types() {
    let data = JobTypeData::new(INVALID_JOB_TYPE_INFO);

    assert!(data.dequeue_event_name().is_none());
    assert!(data.execute_event_name().is_none());
    assert_eq!(data.events().dequeue, None);
    assert_eq!(data.events().execute, None);
    assert_eq!(data.stats().average_latency, Duration::ZERO);
    assert_eq!(data.stats().peak_latency, Duration::ZERO);
    assert_eq!(data.stats().load, LoadMonitorStats::default());
}

#[test]
fn job_type_data_special_types_do_not_create_or_notify_collector_events() {
    let collector = Arc::new(RecordingCollector::default());
    let data = JobTypeData::new_with_collector(INVALID_JOB_TYPE_INFO, Some(Arc::clone(&collector)));

    assert!(
        collector
            .created
            .lock()
            .expect("created mutex poisoned")
            .is_empty()
    );

    data.notify_dequeue(Duration::from_millis(12));
    data.notify_execute(Duration::from_millis(18));

    assert!(
        collector
            .notifications
            .lock()
            .expect("notifications mutex poisoned")
            .is_empty()
    );
}

#[test]
fn job_type_data_owns_real_event_sinks_when_collector_is_present() {
    let collector = Arc::new(RecordingCollector::default());
    let info = JobTypeInfo::new(
        JobType::Transaction,
        "transaction",
        4,
        Duration::from_millis(250),
        Duration::from_secs(1),
    );
    let data = JobTypeData::new_with_collector(info, Some(Arc::clone(&collector)));

    assert_eq!(
        *collector.created.lock().expect("created mutex poisoned"),
        vec!["transaction_q".to_owned(), "transaction".to_owned()]
    );

    data.notify_dequeue(Duration::from_millis(12));
    data.notify_execute(Duration::from_millis(18));

    let notifications = collector
        .notifications
        .lock()
        .expect("notifications mutex poisoned")
        .clone();
    assert_eq!(
        notifications,
        vec![
            ("transaction_q".to_owned(), Duration::from_millis(12)),
            ("transaction".to_owned(), Duration::from_millis(18)),
        ]
    );
}

#[test]
fn job_type_data_can_build_load_monitor_from_journal_factory_logs_owner() {
    let journal_factory = Arc::new(RecordingJournalFactory::default());
    let info = JobTypeInfo::new(
        JobType::Transaction,
        "transaction",
        4,
        Duration::from_millis(250),
        Duration::from_secs(1),
    );
    let _data = JobTypeData::new_with_collector_and_logs::<
        dyn JobTypeDataCollector,
        dyn LoadMonitorJournalFactory,
    >(
        info,
        None,
        Some(Arc::clone(&journal_factory) as Arc<dyn LoadMonitorJournalFactory>),
    );

    assert_eq!(
        *journal_factory
            .created
            .lock()
            .expect("created journals mutex poisoned"),
        vec!["LoadMonitor".to_owned()]
    );
    assert!(
        journal_factory
            .entries
            .lock()
            .expect("journal entries mutex poisoned")
            .is_empty()
    );
}
