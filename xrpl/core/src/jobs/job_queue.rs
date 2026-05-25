use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::atomic::AtomicI32;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::time::Duration;

use basics::local_value::{LocalSlotOwner, install_local_slot_owner};
use generator::{Generator, Gn};
use serde_json::{Value, json};

use crate::closure_counter::ClosureCounter;
use crate::job::{Job, JobType};
use crate::job_type_data::{JobTypeDataCollector, JobTypeDataEvent};
use crate::job_type_info::JobTypeInfo;
use crate::job_types::JobTypes;
use crate::load_event::LoadEvent;
use crate::load_monitor::{LoadMonitor, LoadMonitorJournal, LoadMonitorJournalFactory};
use crate::perf_log::PerfLog;
use crate::workers::{Callback as WorkersCallback, Workers};

pub trait JobQueueGauge: Send + Sync {
    fn set(&self, value: usize);
}

pub trait JobQueueHook: Send + Sync {
    fn trigger(&self);
}

pub trait JobQueueCollector: JobTypeDataCollector + Send + Sync {
    fn make_gauge(&self, name: &str) -> Arc<dyn JobQueueGauge>;
    fn make_hook(&self, callback: Arc<dyn Fn() + Send + Sync>) -> Arc<dyn JobQueueHook>;
}

pub type JobQueueJournal = dyn LoadMonitorJournal;

struct RuntimeJobTypeData {
    info: JobTypeInfo,
    waiting: i32,
    running: i32,
    deferred: i32,
    load: Arc<LoadMonitor>,
    dequeue_sink: Option<Arc<dyn JobTypeDataEvent>>,
    execute_sink: Option<Arc<dyn JobTypeDataEvent>>,
}

impl std::fmt::Debug for RuntimeJobTypeData {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RuntimeJobTypeData")
            .field("info", &self.info)
            .field("waiting", &self.waiting)
            .field("running", &self.running)
            .field("deferred", &self.deferred)
            .field("load", &self.load)
            .finish()
    }
}

impl RuntimeJobTypeData {
    fn new(
        info: JobTypeInfo,
        collector: Option<Arc<dyn JobQueueCollector>>,
        journal_factory: Option<Arc<dyn LoadMonitorJournalFactory>>,
    ) -> Self {
        let load = if let Some(factory) = journal_factory {
            Arc::new(LoadMonitor::with_journal(
                factory.make_load_monitor_journal("LoadMonitor"),
            ))
        } else {
            Arc::new(LoadMonitor::new())
        };
        load.set_target_latency(info.get_average_latency(), info.get_peak_latency());
        let (dequeue_sink, execute_sink) = if info.special() {
            (None, None)
        } else if let Some(collector) = collector {
            (
                Some(collector.make_event(&format!("{}_q", info.name()))),
                Some(collector.make_event(info.name())),
            )
        } else {
            (None, None)
        };
        Self {
            info,
            waiting: 0,
            running: 0,
            deferred: 0,
            load,
            dequeue_sink,
            execute_sink,
        }
    }
}

#[derive(Debug)]
struct JobQueueState {
    last_job: u64,
    jobs: BTreeSet<Job>,
    job_data: BTreeMap<JobType, RuntimeJobTypeData>,
    process_count: i32,
}

impl JobQueueState {
    fn new(
        collector: Option<Arc<dyn JobQueueCollector>>,
        journal_factory: Option<Arc<dyn LoadMonitorJournalFactory>>,
    ) -> Self {
        let mut job_data = BTreeMap::new();
        for info in JobTypes::instance().iter() {
            job_data.insert(
                info.job_type(),
                RuntimeJobTypeData::new(info, collector.clone(), journal_factory.clone()),
            );
        }

        Self {
            last_job: 0,
            jobs: BTreeSet::new(),
            job_data,
            process_count: 0,
        }
    }

    fn is_idle(&self) -> bool {
        self.process_count == 0 && self.jobs.is_empty()
    }
}

struct JobQueueCallback {
    inner: Weak<JobQueueInner>,
}

impl WorkersCallback for JobQueueCallback {
    fn process_task(&self, instance: i32) {
        if let Some(inner) = self.inner.upgrade() {
            inner.process_task(instance);
        }
    }
}

struct JobQueueInner {
    state: Mutex<JobQueueState>,
    cv: Condvar,
    job_counter: ClosureCounter<fn()>,
    suspended_coros: AtomicI32,
    stopping: AtomicBool,
    stopped: AtomicBool,
    perf_log: Option<Arc<dyn PerfLog>>,
    collector_hook: Option<Arc<dyn JobQueueHook>>,
    collector_gauge: Option<Arc<dyn JobQueueGauge>>,
    workers: Workers,
}

impl JobQueueInner {
    fn collect(&self) {
        let state = self.state.lock().expect("job queue mutex poisoned");
        if let Some(gauge) = &self.collector_gauge {
            gauge.set(state.jobs.len());
        }
    }

    fn process_task(&self, _instance: i32) {
        let (mut job, load_monitor, job_type, queue_time) = {
            let mut state = self.state.lock().expect("job queue mutex poisoned");
            let Some((selected_key, selected_type, load_monitor)) = select_next_job(&state) else {
                return;
            };

            let job = state
                .jobs
                .take(&selected_key)
                .expect("selected job must still exist");
            let data = state
                .job_data
                .get_mut(&selected_type)
                .expect("selected job type must exist");
            data.waiting -= 1;
            data.running += 1;
            state.process_count += 1;
            let queue_time = job.queue_time();
            (job, load_monitor, selected_type, queue_time)
        };

        let mut load_event =
            LoadEvent::new(Arc::clone(&load_monitor), job.name().to_owned(), false);
        let start_time = std::time::Instant::now();
        if let Some(perf_log) = &self.perf_log {
            perf_log.job_start(
                job_type,
                queue_time
                    .map(|queued| start_time.saturating_duration_since(queued))
                    .unwrap_or(Duration::ZERO),
                start_time,
                _instance,
            );
        }
        let result = catch_unwind(AssertUnwindSafe(|| {
            load_event.start();
            load_event.set_name(job.name().to_owned());
            job.do_job();
        }));
        let running_duration = start_time.elapsed();
        drop(load_event);
        let queue_duration = queue_time
            .map(|queued| start_time.saturating_duration_since(queued))
            .unwrap_or(Duration::ZERO);

        if running_duration >= Duration::from_millis(10)
            || queue_duration >= Duration::from_millis(10)
        {
            let state = self.state.lock().expect("job queue mutex poisoned");
            let data = state
                .job_data
                .get(&job_type)
                .expect("finished job type must exist");
            if let Some(sink) = &data.dequeue_sink {
                sink.notify(queue_duration);
            }
            if let Some(sink) = &data.execute_sink {
                sink.notify(running_duration);
            }
        }

        let should_add_task = {
            let mut state = self.state.lock().expect("job queue mutex poisoned");
            let data = state
                .job_data
                .get_mut(&job_type)
                .expect("finished job type must exist");
            if data.deferred > 0 {
                data.deferred -= 1;
                true
            } else {
                false
            }
        };

        {
            let mut state = self.state.lock().expect("job queue mutex poisoned");
            let data = state
                .job_data
                .get_mut(&job_type)
                .expect("finished job type must exist");
            data.running -= 1;
            state.process_count -= 1;
            if state.is_idle() {
                self.cv.notify_all();
            }
        }

        if should_add_task {
            self.workers.add_task();
        }

        if let Some(perf_log) = &self.perf_log {
            perf_log.job_finish(job_type, running_duration, _instance);
        }

        if let Err(payload) = result {
            resume_unwind(payload);
        }
    }
}

fn select_next_job(state: &JobQueueState) -> Option<(Job, JobType, Arc<LoadMonitor>)> {
    state.jobs.iter().find_map(|job| {
        let job_type = job.get_type();
        let data = state.job_data.get(&job_type)?;
        if data.info.limit() <= 0 || data.running >= data.info.limit() {
            return None;
        }

        Some((
            Job::new(job_type, job.job_index()),
            job_type,
            Arc::clone(&data.load),
        ))
    })
}

#[derive(Clone)]
pub struct JobQueue {
    inner: Arc<JobQueueInner>,
}

pub struct Coro {
    jq: JobQueue,
    job_type: JobType,
    name: String,
    local_slots: LocalSlotOwner,
    generator: Mutex<Generator<'static, (), ()>>,
    resume_mutex: Mutex<()>,
    running: Mutex<bool>,
    cv: Condvar,
    early_exit_expected: AtomicBool,
}

impl fmt::Debug for JobQueue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.inner.state.lock().expect("job queue mutex poisoned");
        f.debug_struct("JobQueue")
            .field("threads", &self.inner.workers.get_number_of_threads())
            .field("queued_jobs", &state.jobs.len())
            .field("process_count", &state.process_count)
            .field("stopping", &self.is_stopping())
            .field("stopped", &self.is_stopped())
            .finish()
    }
}

impl JobQueue {
    pub fn new(thread_count: i32) -> Self {
        Self::new_with_runtime_dependencies(thread_count, None, None, None, None)
    }

    pub fn new_with_perf_log(thread_count: i32, perf_log: Option<Arc<dyn PerfLog>>) -> Self {
        Self::new_with_runtime_dependencies(thread_count, None, None, None, perf_log)
    }

    pub fn new_with_perf_log_and_collector(
        thread_count: i32,
        perf_log: Option<Arc<dyn PerfLog>>,
        collector: Option<Arc<dyn JobQueueCollector>>,
    ) -> Self {
        Self::new_with_runtime_dependencies(thread_count, collector, None, None, perf_log)
    }

    pub fn new_with_perf_log_collector_and_journal_factory(
        thread_count: i32,
        perf_log: Option<Arc<dyn PerfLog>>,
        collector: Option<Arc<dyn JobQueueCollector>>,
        journal_factory: Option<Arc<dyn LoadMonitorJournalFactory>>,
    ) -> Self {
        Self::new_with_runtime_dependencies(
            thread_count,
            collector,
            None,
            journal_factory,
            perf_log,
        )
    }

    pub fn new_with_runtime_dependencies(
        thread_count: i32,
        collector: Option<Arc<dyn JobQueueCollector>>,
        journal: Option<Arc<JobQueueJournal>>,
        logs: Option<Arc<dyn LoadMonitorJournalFactory>>,
        perf_log: Option<Arc<dyn PerfLog>>,
    ) -> Self {
        let inner = Arc::new_cyclic(|weak| {
            let callback: Arc<dyn WorkersCallback> = Arc::new(JobQueueCallback {
                inner: weak.clone(),
            });
            let collector_hook = collector.as_ref().map(|collector| {
                let weak = weak.clone();
                collector.make_hook(Arc::new(move || {
                    if let Some(inner) = weak.upgrade() {
                        inner.collect();
                    }
                }))
            });
            let collector_gauge = collector
                .as_ref()
                .map(|collector| collector.make_gauge("job_count"));
            JobQueueInner {
                state: Mutex::new(JobQueueState::new(collector.clone(), logs.clone())),
                cv: Condvar::new(),
                job_counter: ClosureCounter::new(),
                suspended_coros: AtomicI32::new(0),
                stopping: AtomicBool::new(false),
                stopped: AtomicBool::new(false),
                perf_log: perf_log.clone(),
                collector_hook,
                collector_gauge,
                workers: Workers::new(callback, perf_log.clone(), "JobQueue", thread_count),
            }
        });

        if let Some(journal) = journal {
            journal.info(&format!("Using {thread_count}  threads"));
        }

        Self { inner }
    }

    pub fn get_number_of_threads(&self) -> i32 {
        self.inner.workers.get_number_of_threads()
    }

    pub fn post_coro<F>(
        &self,
        job_type: JobType,
        name: impl Into<String>,
        f: F,
    ) -> Option<Arc<Coro>>
    where
        F: FnOnce(Arc<Coro>) + Send + 'static,
    {
        let coro = Coro::new(self.clone(), job_type, name.into(), f);
        if coro.post() {
            Some(coro)
        } else {
            coro.expect_early_exit();
            None
        }
    }

    pub fn add_job<F>(&self, job_type: JobType, name: impl Into<String>, job_handler: F) -> bool
    where
        F: Fn() + Send + Sync + 'static,
    {
        if self.is_stopping() || self.is_stopped() {
            return false;
        }

        let Some(counted_job) = self.inner.job_counter.wrap(job_handler) else {
            return false;
        };

        let mut state = self.inner.state.lock().expect("job queue mutex poisoned");
        if !state.job_data.contains_key(&job_type) {
            return false;
        }
        state.last_job = state.last_job.wrapping_add(1);
        let index = state.last_job;
        let data = state
            .job_data
            .get_mut(&job_type)
            .expect("job type should exist after contains_key");
        let should_add_task = data.waiting + data.running < data.info.limit();
        if should_add_task {
            self.inner.workers.add_task();
        } else {
            data.deferred += 1;
        }
        data.waiting += 1;

        let inserted = state
            .jobs
            .insert(Job::new_with_closure(job_type, name, index, move || {
                counted_job()
            }));
        debug_assert!(inserted, "JobQueue::add_job should enqueue a fresh job");
        if let Some(perf_log) = &self.inner.perf_log {
            perf_log.job_queue(job_type);
        }
        true
    }

    pub fn get_job_count(&self, job_type: JobType) -> i32 {
        let state = self.inner.state.lock().expect("job queue mutex poisoned");
        state.job_data.get(&job_type).map_or(0, |data| data.waiting)
    }

    pub fn get_job_count_total(&self, job_type: JobType) -> i32 {
        let state = self.inner.state.lock().expect("job queue mutex poisoned");
        state
            .job_data
            .get(&job_type)
            .map_or(0, |data| data.waiting + data.running)
    }

    pub fn get_job_count_ge(&self, job_type: JobType) -> i32 {
        let state = self.inner.state.lock().expect("job queue mutex poisoned");
        state
            .job_data
            .iter()
            .filter(|(kind, _)| **kind >= job_type)
            .map(|(_, data)| data.waiting)
            .sum()
    }

    pub fn make_load_event(&self, job_type: JobType, name: impl Into<String>) -> Option<LoadEvent> {
        let state = self.inner.state.lock().expect("job queue mutex poisoned");
        state
            .job_data
            .get(&job_type)
            .map(|data| LoadEvent::new(Arc::clone(&data.load), name, true))
    }

    pub fn add_load_events(&self, job_type: JobType, count: i32, elapsed: Duration) {
        assert!(
            !self.is_stopped(),
            "JobQueue::add_load_events called after stop"
        );

        let load = {
            let state = self.inner.state.lock().expect("job queue mutex poisoned");
            state
                .job_data
                .get(&job_type)
                .map(|data| Arc::clone(&data.load))
        };

        if let Some(load) = load {
            load.add_samples(count, elapsed);
        }
    }

    pub fn is_overloaded(&self) -> bool {
        let state = self.inner.state.lock().expect("job queue mutex poisoned");
        state.job_data.values().any(|data| data.load.is_over())
    }

    pub fn get_json(&self, _detail_level: usize) -> Value {
        let state = self.inner.state.lock().expect("job queue mutex poisoned");
        let priorities = state
            .job_data
            .iter()
            .filter_map(|(job_type, data)| {
                if *job_type == JobType::Generic {
                    return None;
                }

                let stats = data.load.get_stats();
                if stats.count == 0
                    && data.waiting == 0
                    && stats.latency_peak.is_zero()
                    && data.running == 0
                {
                    return None;
                }

                let mut entry = json!({
                    "job_type": data.info.name(),
                });

                if stats.is_overloaded {
                    entry["over_target"] = Value::Bool(true);
                }
                if data.waiting != 0 {
                    entry["waiting"] = json!(data.waiting);
                }
                if stats.count != 0 {
                    entry["per_second"] = json!(stats.count);
                }
                if !stats.latency_peak.is_zero() {
                    entry["peak_time"] = json!(stats.latency_peak.as_millis());
                }
                if !stats.latency_avg.is_zero() {
                    entry["avg_time"] = json!(stats.latency_avg.as_millis());
                }
                if data.running != 0 {
                    entry["in_progress"] = json!(data.running);
                }

                Some(entry)
            })
            .collect::<Vec<_>>();

        json!({
            "threads": self.inner.workers.get_number_of_threads(),
            "job_types": priorities,
        })
    }

    pub fn rendezvous(&self) {
        let mut state = self.inner.state.lock().expect("job queue mutex poisoned");
        while !state.is_idle() {
            state = self
                .inner
                .cv
                .wait(state)
                .expect("job queue rendezvous wait poisoned");
        }
    }

    pub fn stop(&self) {
        if self.is_stopped() {
            return;
        }

        self.inner.stopping.store(true, Ordering::Release);
        self.inner
            .job_counter
            .join("JobQueue", Duration::from_secs(1), || {});
        self.rendezvous();
        self.inner.workers.stop();
        debug_assert_eq!(
            self.inner.suspended_coros.load(Ordering::Acquire),
            0,
            "JobQueue::stop should not leave suspended coroutines behind"
        );
        self.inner.stopped.store(true, Ordering::Release);
    }

    pub fn is_stopping(&self) -> bool {
        self.inner.stopping.load(Ordering::Acquire)
    }

    pub fn is_stopped(&self) -> bool {
        self.inner.stopped.load(Ordering::Acquire)
    }

    pub fn get_job_limit(job_type: JobType) -> i32 {
        JobTypes::instance().get(job_type).limit()
    }

    pub fn metrics_hook(&self) -> Option<Arc<dyn JobQueueHook>> {
        self.inner.collector_hook.clone()
    }
}

impl Default for JobQueue {
    fn default() -> Self {
        Self::new(1)
    }
}

impl fmt::Debug for Coro {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Coro")
            .field("job_type", &self.job_type)
            .field("name", &self.name)
            .field("runnable", &self.runnable())
            .finish()
    }
}

impl Coro {
    const STACK_SIZE: usize = 1536 * 1024;

    fn new<F>(jq: JobQueue, job_type: JobType, name: String, f: F) -> Arc<Self>
    where
        F: FnOnce(Arc<Coro>) + Send + 'static,
    {
        let coro = Arc::new_cyclic(|weak: &Weak<Coro>| {
            let weak_for_generator = weak.clone();
            let generator = Gn::new_opt(Self::STACK_SIZE, move || {
                if let Some(coro) = weak_for_generator.upgrade() {
                    coro.note_suspend();
                }
                #[allow(deprecated)]
                let _ = generator::yield_::<(), ()>(());
                if let Some(coro) = weak_for_generator.upgrade() {
                    f(coro);
                }
            });

            Self {
                jq: jq.clone(),
                job_type,
                name,
                local_slots: LocalSlotOwner::new(),
                generator: Mutex::new(generator),
                resume_mutex: Mutex::new(()),
                running: Mutex::new(false),
                cv: Condvar::new(),
                early_exit_expected: AtomicBool::new(false),
            }
        });

        coro.prime();
        coro
    }

    fn prime(&self) {
        let _guard = self
            .resume_mutex
            .lock()
            .expect("coro resume mutex poisoned");
        let _context = install_local_slot_owner(&self.local_slots);
        let mut generator = self
            .generator
            .lock()
            .expect("coro generator mutex poisoned");
        if !generator.is_done() {
            let _ = generator.resume();
        }
    }

    fn note_suspend(&self) {
        self.jq.inner.suspended_coros.fetch_add(1, Ordering::AcqRel);
    }

    fn note_resume(&self) {
        self.jq.inner.suspended_coros.fetch_sub(1, Ordering::AcqRel);
    }

    pub fn yield_now(&self) {
        self.note_suspend();
        #[allow(deprecated)]
        let _ = generator::yield_::<(), ()>(());
    }

    pub fn post(self: &Arc<Self>) -> bool {
        {
            let mut running = self.running.lock().expect("coro running mutex poisoned");
            *running = true;
        }

        let this = Arc::clone(self);
        if self
            .jq
            .add_job(self.job_type, self.name.clone(), move || this.resume())
        {
            return true;
        }

        let mut running = self.running.lock().expect("coro running mutex poisoned");
        *running = false;
        self.cv.notify_all();
        false
    }

    pub fn resume(&self) {
        {
            let mut running = self.running.lock().expect("coro running mutex poisoned");
            *running = true;
        }

        self.note_resume();
        let _resume_guard = self
            .resume_mutex
            .lock()
            .expect("coro resume mutex poisoned");
        let _context_guard = install_local_slot_owner(&self.local_slots);
        {
            let mut generator = self
                .generator
                .lock()
                .expect("coro generator mutex poisoned");
            if !generator.is_done() {
                let _ = generator.resume();
            }
        }

        let mut running = self.running.lock().expect("coro running mutex poisoned");
        *running = false;
        self.cv.notify_all();
    }

    pub fn runnable(&self) -> bool {
        !self
            .generator
            .lock()
            .expect("coro generator mutex poisoned")
            .is_done()
    }

    pub fn expect_early_exit(&self) {
        if !self.early_exit_expected.swap(true, Ordering::AcqRel) {
            self.note_resume();
        }
    }

    pub fn join(&self) {
        let mut running = self.running.lock().expect("coro running mutex poisoned");
        while *running {
            running = self.cv.wait(running).expect("coro condvar wait poisoned");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;
    use std::time::Duration;

    use crate::{JobQueue, JobType};

    #[test]
    fn queue_runs_jobs_and_stop_prevents_new_additions() {
        let queue = JobQueue::new(2);
        let calls = Arc::new(AtomicUsize::new(0));

        assert!(queue.add_job(JobType::Client, "client", {
            let calls = Arc::clone(&calls);
            move || {
                calls.fetch_add(1, Ordering::SeqCst);
            }
        }));

        queue.rendezvous();
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        queue.stop();
        assert!(queue.is_stopping());
        assert!(queue.is_stopped());
        assert!(!queue.add_job(JobType::Client, "late", || {}));
    }

    #[test]
    fn invalid_job_add_does_not_advance_last_job_counter() {
        let queue = JobQueue::new(1);
        let before = queue
            .inner
            .state
            .lock()
            .expect("job queue mutex poisoned")
            .last_job;

        assert!(!queue.add_job(JobType::Invalid, "invalid", || {}));

        let after = queue
            .inner
            .state
            .lock()
            .expect("job queue mutex poisoned")
            .last_job;

        assert_eq!(before, after);
    }

    #[test]
    fn queue_load_stats_render_in_json() {
        let queue = JobQueue::new(1);
        queue.add_load_events(JobType::Transaction, 1, Duration::from_millis(1_500));

        let json = queue.get_json(0);
        let entry = json["job_types"]
            .as_array()
            .expect("job_types array")
            .iter()
            .find(|entry| entry["job_type"] == "transaction")
            .expect("transaction entry");

        assert_eq!(entry["avg_time"], 375);
        assert_eq!(entry["peak_time"], 1500);
        assert_eq!(entry["over_target"], true);
    }

    #[test]
    fn make_load_event_tracks_manual_runtime() {
        let queue = JobQueue::new(1);
        let mut event = queue
            .make_load_event(JobType::Client, "manual")
            .expect("client load event");
        thread::sleep(Duration::from_millis(3));
        event.stop();

        assert_eq!(event.name(), "manual");
        assert!(event.run_time() >= Duration::from_millis(1));
    }

    #[test]
    fn post_coro_supports_post_resume_and_join_shape() {
        let queue = JobQueue::new(1);
        let yield_count = Arc::new(AtomicUsize::new(0));
        let coro = queue
            .post_coro(JobType::Client, "coro", {
                let yield_count = Arc::clone(&yield_count);
                move |coro| {
                    while yield_count.fetch_add(1, Ordering::SeqCst) + 1 < 4 {
                        coro.yield_now();
                    }
                }
            })
            .expect("coro should post");

        coro.join();
        while coro.runnable() {
            assert!(coro.post());
            coro.join();
        }

        assert_eq!(yield_count.load(Ordering::SeqCst), 4);
    }
}
