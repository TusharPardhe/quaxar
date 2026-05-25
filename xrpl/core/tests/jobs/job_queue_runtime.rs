use basics::local_value::LocalValue;
use serde_json::{Map, Value};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use xrpl_core::{
    JobQueue, JobQueueCollector, JobQueueGauge, JobQueueHook, JobQueueJournal, JobType,
    JobTypeDataCollector, JobTypeDataEvent, LoadMonitorJournal, LoadMonitorJournalFactory, PerfLog,
};

#[derive(Default)]
struct RecordingQueueCollector {
    gauge_values: Arc<Mutex<Vec<usize>>>,
    event_notifications: Arc<Mutex<Vec<(String, Duration)>>>,
}

struct RecordingGauge {
    values: Arc<Mutex<Vec<usize>>>,
}

impl JobQueueGauge for RecordingGauge {
    fn set(&self, value: usize) {
        self.values
            .lock()
            .expect("gauge mutex poisoned")
            .push(value);
    }
}

struct RecordingHook {
    callback: Arc<dyn Fn() + Send + Sync>,
}

impl JobQueueHook for RecordingHook {
    fn trigger(&self) {
        (self.callback)();
    }
}

struct RecordingEvent {
    name: String,
    notifications: Arc<Mutex<Vec<(String, Duration)>>>,
}

#[derive(Default)]
struct RecordingLoadMonitorJournalFactory {
    created: Arc<Mutex<Vec<String>>>,
    entries: Arc<Mutex<Vec<(String, String)>>>,
}

#[derive(Clone)]
struct RecordingLoadMonitorJournal {
    entries: Arc<Mutex<Vec<(String, String)>>>,
}

#[derive(Clone)]
struct RecordingQueueJournal {
    entries: Arc<Mutex<Vec<(String, String)>>>,
}

#[derive(Default)]
struct RecordingPerfLog {
    queued_job_types: Mutex<Vec<JobType>>,
}

impl LoadMonitorJournal for RecordingLoadMonitorJournal {
    fn info(&self, message: &str) {
        self.entries
            .lock()
            .expect("load monitor journal entries mutex poisoned")
            .push(("info".to_owned(), message.to_owned()));
    }

    fn warn(&self, message: &str) {
        self.entries
            .lock()
            .expect("load monitor journal entries mutex poisoned")
            .push(("warn".to_owned(), message.to_owned()));
    }
}

impl LoadMonitorJournal for RecordingQueueJournal {
    fn info(&self, message: &str) {
        self.entries
            .lock()
            .expect("queue journal entries mutex poisoned")
            .push(("info".to_owned(), message.to_owned()));
    }
}

impl PerfLog for RecordingPerfLog {
    fn rpc_start(&self, _method: &str, _request_id: u64) {}

    fn rpc_finish(&self, _method: &str, _request_id: u64) {}

    fn rpc_error(&self, _method: &str, _request_id: u64) {}

    fn job_queue(&self, job_type: JobType) {
        self.queued_job_types
            .lock()
            .expect("queued job types mutex poisoned")
            .push(job_type);
    }

    fn job_start(
        &self,
        _job_type: JobType,
        _queued_duration: Duration,
        _start_time: std::time::Instant,
        _instance: i32,
    ) {
    }

    fn job_finish(&self, _job_type: JobType, _running_duration: Duration, _instance: i32) {}

    fn counters_json(&self) -> Value {
        Value::Object(Map::new())
    }

    fn current_json(&self) -> Value {
        Value::Array(Vec::new())
    }

    fn resize_jobs(&self, _resize: usize) {}
}

impl LoadMonitorJournalFactory for RecordingLoadMonitorJournalFactory {
    fn make_load_monitor_journal(&self, name: &str) -> Arc<dyn LoadMonitorJournal> {
        self.created
            .lock()
            .expect("created journals mutex poisoned")
            .push(name.to_owned());
        Arc::new(RecordingLoadMonitorJournal {
            entries: Arc::clone(&self.entries),
        })
    }
}

impl JobTypeDataEvent for RecordingEvent {
    fn notify(&self, duration: Duration) {
        self.notifications
            .lock()
            .expect("event notifications mutex poisoned")
            .push((self.name.clone(), duration));
    }
}

impl JobTypeDataCollector for RecordingQueueCollector {
    fn make_event(&self, name: &str) -> Arc<dyn JobTypeDataEvent> {
        Arc::new(RecordingEvent {
            name: name.to_owned(),
            notifications: Arc::clone(&self.event_notifications),
        })
    }
}

impl JobQueueCollector for RecordingQueueCollector {
    fn make_gauge(&self, _name: &str) -> Arc<dyn JobQueueGauge> {
        Arc::new(RecordingGauge {
            values: Arc::clone(&self.gauge_values),
        })
    }

    fn make_hook(&self, callback: Arc<dyn Fn() + Send + Sync>) -> Arc<dyn JobQueueHook> {
        Arc::new(RecordingHook { callback })
    }
}

#[test]
fn add_job_runs_and_stop_rejects_new_work_queue_contract() {
    let queue = JobQueue::new(2);
    let ran = Arc::new(AtomicUsize::new(0));

    assert!(queue.add_job(JobType::Client, "job-add", {
        let ran = Arc::clone(&ran);
        move || {
            ran.fetch_add(1, Ordering::SeqCst);
        }
    }));

    queue.rendezvous();
    assert_eq!(ran.load(Ordering::SeqCst), 1);

    queue.stop();
    assert!(queue.is_stopping());
    assert!(queue.is_stopped());
    assert!(!queue.add_job(JobType::Client, "late-job", || {}));
}

#[test]
fn stopped_queue_rejects_post_coro_job_queue_test() {
    let queue = JobQueue::new(1);

    queue.stop();
    assert!(queue.is_stopped());
    assert!(
        queue
            .post_coro(JobType::Client, "late-coro", |_| panic!(
                "stopped coro must not run"
            ))
            .is_none()
    );
}

#[test]
fn invalid_job_type_is_rejected_without_perf_queue_side_effects() {
    let perf_log = Arc::new(RecordingPerfLog::default());
    let queue = JobQueue::new_with_perf_log(1, Some(Arc::clone(&perf_log) as Arc<dyn PerfLog>));

    assert!(!queue.add_job(JobType::Invalid, "invalid", || {}));
    assert_eq!(queue.get_job_count(JobType::Invalid), 0);
    assert_eq!(queue.get_job_count_total(JobType::Invalid), 0);
    assert_eq!(queue.get_job_count_ge(JobType::Invalid), 0);
    assert!(
        perf_log
            .queued_job_types
            .lock()
            .expect("queued job types mutex poisoned")
            .is_empty()
    );
}

#[test]
fn pack_limit_defers_second_job_until_first_finishes() {
    let queue = JobQueue::new(2);
    let first_running = Arc::new(AtomicBool::new(false));
    let release_first = Arc::new(AtomicBool::new(false));
    let second_ran = Arc::new(AtomicBool::new(false));

    assert!(queue.add_job(JobType::Pack, "first-pack", {
        let first_running = Arc::clone(&first_running);
        let release_first = Arc::clone(&release_first);
        move || {
            first_running.store(true, Ordering::SeqCst);
            while !release_first.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(1));
            }
        }
    }));

    while !first_running.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(1));
    }

    assert!(queue.add_job(JobType::Pack, "second-pack", {
        let second_ran = Arc::clone(&second_ran);
        move || {
            second_ran.store(true, Ordering::SeqCst);
        }
    }));

    while queue.get_job_count_total(JobType::Pack) < 2 {
        thread::sleep(Duration::from_millis(1));
    }

    assert_eq!(queue.get_job_count(JobType::Pack), 1);
    assert_eq!(queue.get_job_count_total(JobType::Pack), 2);
    assert_eq!(queue.get_job_count_ge(JobType::Pack), 1);

    thread::sleep(Duration::from_millis(10));
    assert!(!second_ran.load(Ordering::SeqCst));

    release_first.store(true, Ordering::SeqCst);
    queue.rendezvous();
    assert!(second_ran.load(Ordering::SeqCst));
    assert_eq!(queue.get_job_count(JobType::Pack), 0);
    assert_eq!(queue.get_job_count_total(JobType::Pack), 0);
}

#[test]
fn load_samples_drive_over_target_json_runtime_reporting() {
    let queue = JobQueue::new(1);
    queue.add_load_events(JobType::Transaction, 1, Duration::from_millis(1_500));

    assert!(queue.is_overloaded());

    let json = queue.get_json(0);
    let entry = json["job_types"]
        .as_array()
        .expect("job_types array")
        .iter()
        .find(|entry| entry["job_type"] == "transaction")
        .expect("transaction stats entry");

    assert_eq!(json["threads"], 1);
    assert_eq!(entry["avg_time"], 375);
    assert_eq!(entry["peak_time"], 1500);
    assert_eq!(entry["over_target"], true);
}

#[test]
fn job_count_ge_waiting_count_semantics() {
    let queue = JobQueue::new(2);
    let started = Arc::new(AtomicUsize::new(0));
    let release_blockers = Arc::new(AtomicBool::new(false));

    for index in 0..2 {
        assert!(
            queue.add_job(JobType::Client, format!("client-blocker-{index}"), {
                let started = Arc::clone(&started);
                let release_blockers = Arc::clone(&release_blockers);
                move || {
                    started.fetch_add(1, Ordering::SeqCst);
                    while !release_blockers.load(Ordering::SeqCst) {
                        thread::sleep(Duration::from_millis(1));
                    }
                }
            })
        );
    }

    while started.load(Ordering::SeqCst) < 2 {
        thread::sleep(Duration::from_millis(1));
    }

    assert!(queue.add_job(JobType::Client, "queued-client", || {}));
    assert!(queue.add_job(JobType::Transaction, "queued-transaction", || {}));

    assert_eq!(queue.get_job_count(JobType::Client), 1);
    assert_eq!(queue.get_job_count_total(JobType::Client), 3);
    assert_eq!(queue.get_job_count_total(JobType::Transaction), 1);
    assert_eq!(queue.get_job_count_ge(JobType::Client), 2);
    assert_eq!(queue.get_job_count_ge(JobType::Transaction), 1);

    release_blockers.store(true, Ordering::SeqCst);
    queue.rendezvous();

    assert_eq!(queue.get_job_count_ge(JobType::Client), 0);
    assert_eq!(queue.get_job_count_total(JobType::Client), 0);
    assert_eq!(queue.get_job_count_total(JobType::Transaction), 0);
}

#[test]
fn json_detail_level_is_ignored_and_generic_stats_stay_hidden() {
    let queue = JobQueue::new(1);
    assert!(queue.make_load_event(JobType::Invalid, "invalid").is_none());

    queue.add_load_events(JobType::Generic, 1, Duration::from_millis(1_500));
    queue.add_load_events(JobType::Transaction, 1, Duration::from_millis(1_500));

    let json = queue.get_json(0);
    let json_with_detail = queue.get_json(7);
    assert_eq!(json, json_with_detail);
    assert_eq!(json["threads"], 1);

    let job_types = json["job_types"].as_array().expect("job_types array");
    assert!(job_types.iter().all(|entry| entry["job_type"] != "generic"));
    assert!(
        job_types
            .iter()
            .any(|entry| entry["job_type"] == "transaction")
    );
}

#[test]
fn post_coro_repeated_post_and_join_coroutine_shape() {
    let queue = JobQueue::new(1);
    let yield_count = Arc::new(AtomicUsize::new(0));
    let coro = queue
        .post_coro(JobType::Client, "post-coro", {
            let yield_count = Arc::clone(&yield_count);
            move |coro| {
                while yield_count.fetch_add(1, Ordering::SeqCst) + 1 < 4 {
                    coro.yield_now();
                }
            }
        })
        .expect("coro should post");

    coro.join();
    let mut last = yield_count.load(Ordering::SeqCst);
    while coro.runnable() {
        assert!(coro.post());
        while yield_count.load(Ordering::SeqCst) == last {
            thread::sleep(Duration::from_millis(1));
        }
        coro.join();
        last = yield_count.load(Ordering::SeqCst);
    }

    assert_eq!(yield_count.load(Ordering::SeqCst), 4);
}

#[test]
fn post_coro_repeated_resume_synchronous_coroutine_shape() {
    let queue = JobQueue::new(1);
    let yield_count = Arc::new(AtomicUsize::new(0));
    let coro = queue
        .post_coro(JobType::Client, "resume-coro", {
            let yield_count = Arc::clone(&yield_count);
            move |coro| {
                while yield_count.fetch_add(1, Ordering::SeqCst) + 1 < 4 {
                    coro.yield_now();
                }
            }
        })
        .expect("coro should post");

    coro.join();
    let mut last = yield_count.load(Ordering::SeqCst);
    while coro.runnable() {
        coro.resume();
        assert_eq!(yield_count.load(Ordering::SeqCst), last + 1);
        last += 1;
    }

    assert_eq!(yield_count.load(Ordering::SeqCst), 4);
}

#[test]
fn post_before_yield_completes_without_double_resume_incorrect_order() {
    let queue = JobQueue::new(2);
    let (done_tx, done_rx) = mpsc::channel();
    let coro = queue
        .post_coro(JobType::Client, "incorrect-order", move |coro| {
            assert!(coro.post());
            coro.yield_now();
            done_tx.send(()).expect("coro completion should send");
        })
        .expect("coro should post");

    done_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("coro should complete after post-before-yield");
    coro.join();
    assert!(!coro.runnable());

    queue.stop();
    assert!(queue.is_stopped());
}

#[test]
fn collector_hook_updates_job_count_gauge_collect() {
    let collector = Arc::new(RecordingQueueCollector::default());
    let queue = JobQueue::new_with_perf_log_and_collector(
        1,
        None,
        Some(Arc::clone(&collector) as Arc<dyn JobQueueCollector>),
    );
    let first_running = Arc::new(AtomicBool::new(false));
    let release_first = Arc::new(AtomicBool::new(false));

    assert!(queue.add_job(JobType::Client, "first", {
        let first_running = Arc::clone(&first_running);
        let release_first = Arc::clone(&release_first);
        move || {
            first_running.store(true, Ordering::SeqCst);
            while !release_first.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(1));
            }
        }
    }));

    while !first_running.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(1));
    }

    assert!(queue.add_job(JobType::Client, "second", || {}));
    queue
        .metrics_hook()
        .expect("collector hook should exist")
        .trigger();

    assert_eq!(
        collector
            .gauge_values
            .lock()
            .expect("gauge mutex poisoned")
            .last()
            .copied(),
        Some(1)
    );

    release_first.store(true, Ordering::SeqCst);
    queue.rendezvous();
}

#[test]
fn collector_events_fire_for_slow_queue_and_execution_paths() {
    let collector = Arc::new(RecordingQueueCollector::default());
    let queue = JobQueue::new_with_perf_log_and_collector(
        1,
        None,
        Some(Arc::clone(&collector) as Arc<dyn JobQueueCollector>),
    );
    let first_running = Arc::new(AtomicBool::new(false));
    let release_first = Arc::new(AtomicBool::new(false));

    assert!(queue.add_job(JobType::Client, "slow-first", {
        let first_running = Arc::clone(&first_running);
        let release_first = Arc::clone(&release_first);
        move || {
            first_running.store(true, Ordering::SeqCst);
            while !release_first.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(1));
            }
            thread::sleep(Duration::from_millis(12));
        }
    }));

    while !first_running.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(1));
    }

    assert!(queue.add_job(JobType::Client, "slow-second", || {
        thread::sleep(Duration::from_millis(12));
    }));
    thread::sleep(Duration::from_millis(12));
    release_first.store(true, Ordering::SeqCst);
    queue.rendezvous();

    let notifications = collector
        .event_notifications
        .lock()
        .expect("event notifications mutex poisoned")
        .clone();
    assert!(notifications.iter().any(
        |(name, duration)| name == "clientCommand_q" && *duration >= Duration::from_millis(10)
    ));
    assert!(
        notifications
            .iter()
            .any(|(name, duration)| name == "clientCommand"
                && *duration >= Duration::from_millis(10))
    );
}

#[test]
fn job_queue_runtime_dependencies_log_startup_and_build_load_monitors_owner() {
    let factory = Arc::new(RecordingLoadMonitorJournalFactory::default());
    let queue_journal_entries = Arc::new(Mutex::new(Vec::new()));
    let queue = JobQueue::new_with_runtime_dependencies(
        1,
        None,
        Some(Arc::new(RecordingQueueJournal {
            entries: Arc::clone(&queue_journal_entries),
        }) as Arc<JobQueueJournal>),
        Some(Arc::clone(&factory) as Arc<dyn LoadMonitorJournalFactory>),
        None,
    );

    assert!(queue.add_job(JobType::Client, "slow-load-monitor", || {
        thread::sleep(Duration::from_millis(600));
    }));
    queue.rendezvous();

    let created = factory
        .created
        .lock()
        .expect("created journals mutex poisoned")
        .clone();
    assert!(!created.is_empty());
    assert!(created.iter().all(|name| name == "LoadMonitor"));

    let entries = factory
        .entries
        .lock()
        .expect("load monitor journal entries mutex poisoned")
        .clone();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].0, "info");
    assert!(entries[0].1.contains("Job: slow-load-monitor"));

    assert_eq!(
        *queue_journal_entries
            .lock()
            .expect("queue journal entries mutex poisoned"),
        vec![("info".to_owned(), "Using 1  threads".to_owned())]
    );
}

#[test]
fn post_coro_preserves_local_values_across_yield_and_isolates_coroutine_contexts() {
    let queue = JobQueue::new(1);
    let value = Arc::new(LocalValue::new(-1));
    value.set(41);

    let first_seen = Arc::new(Mutex::new(Vec::new()));
    let first = queue
        .post_coro(JobType::Client, "local-value-first", {
            let value = Arc::clone(&value);
            let first_seen = Arc::clone(&first_seen);
            move |coro| {
                first_seen
                    .lock()
                    .expect("first local-value trace mutex poisoned")
                    .push(value.get_cloned());
                value.set(7);
                first_seen
                    .lock()
                    .expect("first local-value trace mutex poisoned")
                    .push(value.get_cloned());
                coro.yield_now();
                first_seen
                    .lock()
                    .expect("first local-value trace mutex poisoned")
                    .push(value.get_cloned());
            }
        })
        .expect("first coro should post");

    first.join();
    assert_eq!(value.get_cloned(), 41);
    assert_eq!(
        *first_seen
            .lock()
            .expect("first local-value trace mutex poisoned"),
        vec![-1, 7]
    );

    assert!(first.runnable());
    assert!(first.post());
    first.join();
    assert!(!first.runnable());
    assert_eq!(value.get_cloned(), 41);
    assert_eq!(
        *first_seen
            .lock()
            .expect("first local-value trace mutex poisoned"),
        vec![-1, 7, 7]
    );

    let second_seen = Arc::new(Mutex::new(Vec::new()));
    let second = queue
        .post_coro(JobType::Client, "local-value-second", {
            let value = Arc::clone(&value);
            let second_seen = Arc::clone(&second_seen);
            move |_| {
                second_seen
                    .lock()
                    .expect("second local-value trace mutex poisoned")
                    .push(value.get_cloned());
                value.set(99);
                second_seen
                    .lock()
                    .expect("second local-value trace mutex poisoned")
                    .push(value.get_cloned());
            }
        })
        .expect("second coro should post");

    second.join();
    assert!(!second.runnable());
    assert_eq!(value.get_cloned(), 41);
    assert_eq!(
        *second_seen
            .lock()
            .expect("second local-value trace mutex poisoned"),
        vec![-1, 99]
    );
}

#[test]
fn worker_thread_local_values_persist_across_jobs_while_coroutines_stay_isolated() {
    let queue = JobQueue::new(1);
    let value = Arc::new(LocalValue::new(-1));
    let worker_seen = Arc::new(Mutex::new(Vec::new()));

    assert!(queue.add_job(JobType::Client, "local-value-worker-first", {
        let value = Arc::clone(&value);
        let worker_seen = Arc::clone(&worker_seen);
        move || {
            worker_seen
                .lock()
                .expect("worker local-value trace mutex poisoned")
                .push(value.get_cloned());
            value.set(-2);
            worker_seen
                .lock()
                .expect("worker local-value trace mutex poisoned")
                .push(value.get_cloned());
        }
    }));
    queue.rendezvous();

    assert_eq!(value.get_cloned(), -1);
    assert_eq!(
        *worker_seen
            .lock()
            .expect("worker local-value trace mutex poisoned"),
        vec![-1, -2]
    );

    let coro_seen = Arc::new(Mutex::new(Vec::new()));
    let coros: Vec<_> = (0..4)
        .map(|id| {
            queue
                .post_coro(JobType::Client, format!("local-value-coro-{id}"), {
                    let value = Arc::clone(&value);
                    let coro_seen = Arc::clone(&coro_seen);
                    move |coro| {
                        coro_seen
                            .lock()
                            .expect("coro local-value trace mutex poisoned")
                            .push((id, value.get_cloned()));
                        value.set(id);
                        coro_seen
                            .lock()
                            .expect("coro local-value trace mutex poisoned")
                            .push((id, value.get_cloned()));
                        coro.yield_now();
                        coro_seen
                            .lock()
                            .expect("coro local-value trace mutex poisoned")
                            .push((id, value.get_cloned()));
                    }
                })
                .expect("coro should post")
        })
        .collect();

    for coro in &coros {
        coro.join();
    }
    for coro in &coros {
        assert!(coro.runnable());
        assert!(coro.post());
        coro.join();
        assert!(!coro.runnable());
    }

    assert_eq!(value.get_cloned(), -1);
    assert_eq!(
        *coro_seen
            .lock()
            .expect("coro local-value trace mutex poisoned"),
        vec![
            (0, -1),
            (0, 0),
            (1, -1),
            (1, 1),
            (2, -1),
            (2, 2),
            (3, -1),
            (3, 3),
            (0, 0),
            (1, 1),
            (2, 2),
            (3, 3),
        ]
    );

    assert!(
        queue.add_job(JobType::Client, "local-value-worker-second", {
            let value = Arc::clone(&value);
            let worker_seen = Arc::clone(&worker_seen);
            move || {
                worker_seen
                    .lock()
                    .expect("worker local-value trace mutex poisoned")
                    .push(value.get_cloned());
            }
        })
    );
    queue.rendezvous();

    assert_eq!(value.get_cloned(), -1);
    assert_eq!(
        *worker_seen
            .lock()
            .expect("worker local-value trace mutex poisoned"),
        vec![-1, -2, -2]
    );
}
