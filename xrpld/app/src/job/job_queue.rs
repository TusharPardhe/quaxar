//! Priority job queue slice for `xrpld/app`.
//!
//! This is a parity-oriented scheduler core, not a full clone of the reference
//! worker pool. It preserves the current queue ordering, per-type running
//! limits, wait counts, and stop/rendezvous contract. The actual thread-pool
//! orchestration is intentionally left out because the live Rust migration
//! surface does not need the full `Workers` stack yet.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::job::job_types::{JobType, JobTypes};
use serde_json::{Value, json};

#[derive(Default)]
struct JobQueueState {
    jobs: BTreeSet<Job>,
    job_types: BTreeMap<JobType, JobTypeState>,
    load_events: BTreeMap<JobType, LoadEventState>,
    last_job_index: u64,
    process_count: usize,
}

#[derive(Debug, Clone)]
struct JobTypeState {
    limit: i32,
    waiting: usize,
    running: usize,
    deferred: usize,
}

impl JobTypeState {
    fn new(limit: i32) -> Self {
        Self {
            limit,
            waiting: 0,
            running: 0,
            deferred: 0,
        }
    }

    fn limit_usize(&self) -> usize {
        usize::try_from(self.limit).unwrap_or(usize::MAX)
    }

    fn can_run(&self) -> bool {
        self.limit > 0 && self.running < self.limit_usize()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct LoadEventState {
    count: usize,
    elapsed_millis: u128,
}

impl JobQueueState {
    fn new() -> Self {
        let mut job_types = BTreeMap::new();
        let mut load_events = BTreeMap::new();
        for info in JobTypes::instance().iter() {
            job_types.insert(info.job_type(), JobTypeState::new(info.limit()));
            load_events.insert(info.job_type(), LoadEventState::default());
        }

        Self {
            jobs: BTreeSet::new(),
            job_types,
            load_events,
            last_job_index: 0,
            process_count: 0,
        }
    }

    fn job_type_state(&self, job_type: JobType) -> Option<&JobTypeState> {
        self.job_types.get(&job_type)
    }

    fn job_type_state_mut(&mut self, job_type: JobType) -> Option<&mut JobTypeState> {
        self.job_types.get_mut(&job_type)
    }

    fn selected_runnable_key(&self) -> Option<Job> {
        self.jobs.iter().find_map(|job| {
            let state = self.job_type_state(job.job_type())?;
            if state.can_run() {
                Some(Job::key(job.job_type(), job.index()))
            } else {
                None
            }
        })
    }

    fn is_idle(&self) -> bool {
        self.process_count == 0 && self.jobs.is_empty()
    }

    fn is_overloaded(&self) -> bool {
        self.job_types.values().any(|state| state.deferred > 0)
    }
}

#[derive(Clone)]
pub struct JobQueue {
    inner: Arc<JobQueueInner>,
    worker_threads: usize,
}

impl fmt::Debug for JobQueue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.inner.state.lock().expect("job queue mutex poisoned");
        f.debug_struct("JobQueue")
            .field("worker_threads", &self.worker_threads)
            .field("queued_jobs", &state.jobs.len())
            .field("process_count", &state.process_count)
            .field("stopping", &self.is_stopping())
            .field("stopped", &self.is_stopped())
            .finish()
    }
}

struct JobQueueInner {
    state: Mutex<JobQueueState>,
    cv: Condvar,
    stopping: AtomicBool,
    stopped: AtomicBool,
}

impl JobQueue {
    pub fn new() -> Self {
        Self::with_worker_threads(1)
    }

    pub fn with_worker_threads(worker_threads: usize) -> Self {
        Self {
            inner: Arc::new(JobQueueInner {
                state: Mutex::new(JobQueueState::new()),
                cv: Condvar::new(),
                stopping: AtomicBool::new(false),
                stopped: AtomicBool::new(false),
            }),
            worker_threads,
        }
    }

    pub fn is_stopping(&self) -> bool {
        self.inner.stopping.load(AtomicOrdering::Acquire)
    }

    pub fn is_stopped(&self) -> bool {
        self.inner.stopped.load(AtomicOrdering::Acquire)
    }

    pub fn add_job<F>(&self, job_type: JobType, name: impl Into<String>, handler: F) -> bool
    where
        F: FnMut() + Send + 'static,
    {
        if self.is_stopping() || self.is_stopped() {
            return false;
        }

        let mut state = self.inner.state.lock().expect("job queue mutex poisoned");
        if !state.job_types.contains_key(&job_type) {
            return false;
        }

        let index = state.last_job_index.wrapping_add(1);
        state.last_job_index = index;

        let job = Job::new(job_type, name.into(), index, handler);
        let job_type_state = state
            .job_type_state_mut(job_type)
            .expect("job type must exist in job table");
        if job_type_state.waiting + job_type_state.running >= job_type_state.limit_usize() {
            job_type_state.deferred += 1;
        }
        job_type_state.waiting += 1;
        state.jobs.insert(job);
        true
    }

    pub fn get_job_count(&self, job_type: JobType) -> usize {
        let state = self.inner.state.lock().expect("job queue mutex poisoned");
        state
            .job_type_state(job_type)
            .map_or(0, |info| info.waiting)
    }

    pub fn get_job_count_total(&self, job_type: JobType) -> usize {
        let state = self.inner.state.lock().expect("job queue mutex poisoned");
        state
            .job_type_state(job_type)
            .map_or(0, |info| info.waiting + info.running)
    }

    pub fn get_job_count_ge(&self, job_type: JobType) -> usize {
        let state = self.inner.state.lock().expect("job queue mutex poisoned");
        state
            .job_types
            .iter()
            .filter(|(t, _)| **t >= job_type)
            .map(|(_, info)| info.waiting)
            .sum()
    }

    pub fn get_job_limit(job_type: JobType) -> i32 {
        JobTypes::instance().get(job_type).limit()
    }

    #[allow(dead_code)]
    pub fn worker_threads(&self) -> usize {
        self.worker_threads
    }

    pub fn add_load_events(&self, job_type: JobType, count: i32, elapsed: Duration) {
        let Ok(count) = usize::try_from(count) else {
            return;
        };

        let mut state = self.inner.state.lock().expect("job queue mutex poisoned");
        let Some(load_state) = state.load_events.get_mut(&job_type) else {
            return;
        };
        load_state.count += count;
        load_state.elapsed_millis += elapsed.as_millis();
    }

    pub fn is_overloaded(&self) -> bool {
        let state = self.inner.state.lock().expect("job queue mutex poisoned");
        state.is_overloaded()
    }

    pub fn get_json(&self, _detail_level: usize) -> Value {
        let state = self.inner.state.lock().expect("job queue mutex poisoned");

        let jobs = state
            .job_types
            .iter()
            .filter_map(|(job_type, info)| {
                let load = state.load_events.get(job_type).copied().unwrap_or_default();
                if info.waiting == 0
                    && info.running == 0
                    && info.deferred == 0
                    && load.count == 0
                    && load.elapsed_millis == 0
                {
                    return None;
                }

                Some(json!({
                    "job_type": job_type.to_string(),
                    "waiting": info.waiting,
                    "running": info.running,
                    "deferred": info.deferred,
                    "limit": info.limit,
                    "load_event_count": load.count,
                    "load_event_elapsed_millis": load.elapsed_millis,
                }))
            })
            .collect::<Vec<_>>();

        json!({
            "worker_threads": self.worker_threads,
            "stopping": self.is_stopping(),
            "stopped": self.is_stopped(),
            "overloaded": state.is_overloaded(),
            "process_count": state.process_count,
            "job_count": state.jobs.len(),
            "jobs": jobs,
        })
    }

    pub fn reserve_next_job(&self) -> Option<RunningJob> {
        let mut state = self.inner.state.lock().expect("job queue mutex poisoned");
        let key = state.selected_runnable_key()?;
        let job = state.jobs.take(&key).expect("selected job must exist");
        let job_type = job.job_type();
        let job_type_state = state
            .job_type_state_mut(job_type)
            .expect("job type must exist in job table");
        debug_assert!(job_type_state.waiting > 0);
        debug_assert!(
            job_type_state.running < job_type_state.limit_usize() || job_type_state.limit == 0
        );
        job_type_state.waiting -= 1;
        job_type_state.running += 1;
        state.process_count += 1;

        Some(RunningJob {
            queue: self.clone(),
            job_type,
            job: Some(job),
            finished: false,
        })
    }

    pub fn dispatch_next_job(&self) -> Option<JobType> {
        let mut reservation = self.reserve_next_job()?;
        let job_type = reservation.job_type;
        let job = reservation.job.take().expect("reserved job must exist");

        let result = catch_unwind(AssertUnwindSafe(|| job.execute()));
        reservation.finish();

        if let Err(payload) = result {
            resume_unwind(payload);
        }

        Some(job_type)
    }

    pub fn run_until_idle(&self) {
        while self.dispatch_next_job().is_some() {}
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

        self.inner.stopping.store(true, AtomicOrdering::Release);
        self.rendezvous();
        self.inner.stopped.store(true, AtomicOrdering::Release);
        self.inner.cv.notify_all();
    }

    fn finish_job(&self, job_type: JobType) {
        let mut state = self.inner.state.lock().expect("job queue mutex poisoned");
        let job_type_state = state
            .job_type_state_mut(job_type)
            .expect("job type must exist in job table");

        debug_assert!(job_type_state.running > 0);
        job_type_state.running -= 1;
        if job_type_state.deferred > 0 {
            job_type_state.deferred -= 1;
        }

        debug_assert!(state.process_count > 0);
        state.process_count -= 1;
        if state.is_idle() {
            self.inner.cv.notify_all();
        }
    }
}

impl Default for JobQueue {
    fn default() -> Self {
        Self::new()
    }
}

pub struct RunningJob {
    queue: JobQueue,
    job_type: JobType,
    job: Option<Job>,
    finished: bool,
}

impl RunningJob {
    pub fn job_type(&self) -> JobType {
        self.job_type
    }

    pub fn name(&self) -> &str {
        self.job.as_ref().expect("reserved job must exist").name()
    }

    pub fn index(&self) -> u64 {
        self.job.as_ref().expect("reserved job must exist").index()
    }

    pub fn queue_time(&self) -> Instant {
        self.job
            .as_ref()
            .expect("reserved job must exist")
            .queue_time()
    }

    pub fn finish(mut self) {
        if !self.finished {
            self.finished = true;
            self.queue.finish_job(self.job_type);
        }
    }
}

impl Drop for RunningJob {
    fn drop(&mut self) {
        if !self.finished {
            self.finished = true;
            self.queue.finish_job(self.job_type);
        }
    }
}

pub struct Job {
    job_type: JobType,
    index: u64,
    name: String,
    queued_at: Instant,
    handler: Option<Box<dyn FnMut() + Send + 'static>>,
}

impl Job {
    pub fn new<F>(job_type: JobType, name: String, index: u64, handler: F) -> Self
    where
        F: FnMut() + Send + 'static,
    {
        Self {
            job_type,
            index,
            name,
            queued_at: Instant::now(),
            handler: Some(Box::new(handler)),
        }
    }

    fn key(job_type: JobType, index: u64) -> Self {
        Self {
            job_type,
            index,
            name: String::new(),
            queued_at: Instant::now(),
            handler: None,
        }
    }

    pub fn job_type(&self) -> JobType {
        self.job_type
    }

    pub fn index(&self) -> u64 {
        self.index
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn queue_time(&self) -> Instant {
        self.queued_at
    }

    pub fn execute(mut self) {
        let mut handler = self.handler.take().expect("job handler must exist");
        handler();
    }
}

impl PartialEq for Job {
    fn eq(&self, other: &Self) -> bool {
        self.job_type == other.job_type && self.index == other.index
    }
}

impl Eq for Job {}

impl Ord for Job {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .job_type
            .cmp(&self.job_type)
            .then(self.index.cmp(&other.index))
    }
}

impl PartialOrd for Job {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Debug for Job {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Job")
            .field("job_type", &self.job_type)
            .field("index", &self.index)
            .field("name", &self.name)
            .field("queued_at", &self.queued_at)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::{Job, JobQueue};
    use crate::job::job_types::JobType;
    use std::time::Duration;

    #[test]
    fn job_priority_orders_later_job_types_first() {
        let lower = Job::key(JobType::Pack, 1);
        let higher = Job::key(JobType::Accept, 1);
        assert!(higher < lower);
        assert!(lower > higher);
    }

    #[test]
    fn job_queue_limit_helpers_match_job_table() {
        assert_eq!(JobQueue::get_job_limit(JobType::Pack), 1);
        assert_eq!(JobQueue::get_job_limit(JobType::Peer), 0);
        assert_eq!(JobQueue::get_job_limit(JobType::Accept), i32::MAX);
    }

    #[test]
    fn job_queue_tracks_load_events_and_overload_json() {
        let queue = JobQueue::with_worker_threads(4);
        assert!(queue.add_job(JobType::Pack, "first", || {}));
        assert!(queue.add_job(JobType::Pack, "second", || {}));
        queue.add_load_events(JobType::NsAsyncRead, 2, Duration::from_millis(25));

        assert!(queue.is_overloaded());
        let json = queue.get_json(0);
        assert_eq!(json["worker_threads"], 4);
        assert!(
            json["overloaded"]
                .as_bool()
                .expect("overloaded flag should be boolean")
        );
        assert!(
            json["jobs"]
                .as_array()
                .expect("jobs should be an array")
                .iter()
                .any(|entry| entry["job_type"] == "makeFetchPack")
        );
    }
}
