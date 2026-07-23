//! A pool of worker threads that execute jobs to completion, dispatched by
//! priority with per-type concurrency limits. Ported from rippled's
//! `JobQueue.h`/`JobQueue.cpp` and `detail/Workers.h`.
//!
//! # Deviations from the reference
//!
//! - The reference's `Workers` class implements its thread pool with a
//!   custom semaphore, a lock-free stack of `Worker` objects, and an
//!   explicit active/idle/paused state machine so the pool can grow and
//!   shrink at runtime. This port collapses that into a fixed-size pool of
//!   `std::thread` workers parked on a `parking_lot::Condvar` guarding a
//!   shared `Mutex<State>` -- the standard idiomatic-Rust pattern for a
//!   bounded worker pool. Runtime resizing (`setNumberOfThreads`) is not
//!   ported: the pool size is fixed at construction, which is how this
//!   engine is actually used (a fixed thread count read from config at
//!   startup). If runtime resizing is needed later, it can be added by
//!   spawning/parking additional threads against the same `State`.
//! - There is no `JobCounter`/`ClosureCounter` reference-counting wrapper
//!   for in-flight closures. Rust's ownership model means a `Job`'s
//!   captured closure is naturally dropped exactly once when it finishes
//!   running; `stop()` achieves the reference's "wait for all in-flight
//!   work to finish" guarantee directly by joining worker threads after
//!   signaling shutdown and draining the queue, rather than via a separate
//!   counter object.
//! - `Coro`/`postCoro` (the boost::coroutines2-based suspend/resume
//!   mechanism for RPC handlers) is out of scope currently. Nothing
//!   in the consensus/RCL rewrite path requires coroutine
//!   support; it is deferred to whenever RPC handler code needs it.
//! - `LoadMonitor`/`PerfLog`/`beast::insight::Collector` integration
//!   (latency histograms, overload detection, metrics hooks) is omitted.
//!   These are diagnostic/observability concerns layered on top of a
//!   working scheduler, not part of the scheduling algorithm itself, and
//!   are deferred to whenever the app wires up its metrics story.
//! - `getJson`/`isOverloaded` diagnostic surfaces are omitted for the same
//!   reason `Consensus::getJson` was omitted: presentation
//!   concerns belong at a higher layer once the underlying data exists.

use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering};
use std::thread::JoinHandle;
use std::time::Instant;

use parking_lot::{Condvar, Mutex};

use crate::job::job_types::JobType;

/// A unit of work submitted to the [`JobQueue`]. Matches the reference's
/// `Job`, minus the `LoadEvent`/latency-tracking fields (see module-level
/// deviation note).
struct Job {
    job_type: JobType,
    name: String,
    index: u64,
    queue_time: Instant,
    func: Box<dyn FnOnce() + Send + 'static>,
}

impl Job {
    /// How long this job waited in the queue before starting execution.
    /// Matches the reference's use of `Job::queueTime()` in
    /// `JobQueue::processTask` to compute queuing latency. Exposed here
    /// for whenever latency instrumentation (deferred per the
    /// module-level deviation note) is wired up.
    fn queued_for(&self, now: Instant) -> std::time::Duration {
        now.saturating_duration_since(self.queue_time)
    }
}

impl std::fmt::Debug for Job {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Job")
            .field("job_type", &self.job_type)
            .field("name", &self.name)
            .field("index", &self.index)
            .finish()
    }
}

/// Jobs order by `(job_type, index)`: higher-priority job types run first;
/// within the same type, earlier-submitted jobs (lower index) run first.
/// Matches the reference's `Job::operator<` (which the `std::set<Job>`
/// ordering, combined with `getNextJob`'s forward scan, uses to always
/// prefer the highest-priority runnable job).
impl PartialEq for Job {
    fn eq(&self, other: &Self) -> bool {
        self.job_type == other.job_type && self.index == other.index
    }
}
impl Eq for Job {}
impl PartialOrd for Job {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Job {
    fn cmp(&self, other: &Self) -> Ordering {
        // BinaryHeap is a max-heap; we want the highest JobType (priority)
        // popped first, and within a type, the lowest index (earliest
        // submitted) popped first -- so index comparison is reversed.
        self.job_type
            .cmp(&other.job_type)
            .then_with(|| other.index.cmp(&self.index))
    }
}

/// Per-job-type dynamic counters. Matches the dynamic portion of the
/// reference's `JobTypeData` (the static portion lives in
/// [`crate::job::job_types::JobTypeInfo`]).
#[derive(Debug, Default, Clone, Copy)]
struct JobTypeCounters {
    waiting: usize,
    running: usize,
    deferred: usize,
    /// Count of externally-recorded load events for this job type (e.g.
    /// node-store read/write reports for the "special" job types that
    /// never actually flow through [`JobQueue::add_job`]). Matches the
    /// reference's `JobTypeData::updateLatency`'s event count, minus the
    /// latency histogram itself (see module-level deviation note).
    load_events: u64,
}

/// Shared mutable state protected by [`JobQueue`]'s mutex. Matches the
/// reference's `jobSet_`/`jobData_`/`processCount_`/`stopping_` fields.
struct State {
    /// All jobs waiting to run, ordered by priority. Matches `jobSet_`
    /// (a `std::set<Job>`); a `BinaryHeap` is the idiomatic Rust
    /// equivalent for "always extract the highest-priority element".
    queue: BinaryHeap<Job>,
    counters: std::collections::BTreeMap<JobType, JobTypeCounters>,
    /// Number of jobs currently executing inside `processTask`/a worker's
    /// job-running loop. Matches `processCount_`.
    process_count: usize,
}

impl State {
    fn new() -> Self {
        Self {
            queue: BinaryHeap::new(),
            counters: std::collections::BTreeMap::new(),
            process_count: 0,
        }
    }

    fn counters_mut(&mut self, jt: JobType) -> &mut JobTypeCounters {
        self.counters.entry(jt).or_default()
    }

    /// Pop and return the highest-priority job whose type is under its
    /// concurrency limit. Matches `getNextJob`.
    ///
    /// The reference scans `jobSet_` (ordered by priority) for the first
    /// entry under its limit; since `BinaryHeap` doesn't support scanning
    /// in order without draining, this pops into a temporary buffer and
    /// restores the jobs it skips over. In practice the scan depth is
    /// small (limited by how many high-priority job types are currently
    /// saturated), so this stays cheap.
    fn take_next_runnable_job(&mut self) -> Option<Job> {
        let mut skipped = Vec::new();
        let found = loop {
            let Some(job) = self.queue.pop() else {
                // Exhausted the queue without finding a runnable job.
                // Restore everything we skipped over before returning,
                // otherwise those jobs are silently lost.
                for job in skipped {
                    self.queue.push(job);
                }
                return None;
            };
            let running = self
                .counters
                .get(&job.job_type)
                .map(|c| c.running)
                .unwrap_or(0);
            if running < job.job_type.limit() {
                break job;
            }
            skipped.push(job);
        };
        for job in skipped {
            self.queue.push(job);
        }

        let counters = self.counters_mut(found.job_type);
        counters.waiting = counters.waiting.saturating_sub(1);
        counters.running += 1;
        Some(found)
    }
}

/// A pool of worker threads that execute [`Job`]s to completion, dispatched
/// by priority with per-`JobType` concurrency limits. Matches the
/// reference's `JobQueue`.
///
/// # Concurrency model
///
/// Jobs are pushed into a shared, mutex-guarded priority queue. A fixed
/// pool of worker threads parks on a `Condvar` when there is no runnable
/// job; `add_job` (and `finish_job`, when it frees up a deferred slot)
/// notifies the condvar so a parked worker wakes and claims the job. This
/// is a real, signal-driven dispatch: no worker ever busy-polls the queue.
///
/// `JobQueue` is cheaply `Clone` (all shared state lives behind an inner
/// `Arc`) so callers can hold their own handle without wrapping it in an
/// `Arc` themselves, matching how several call sites in `application_root.rs`
/// and `node_store_scheduler.rs` pass owned `JobQueue` values around.
#[derive(Clone)]
pub struct JobQueue {
    inner: Arc<JobQueueInner>,
}

struct JobQueueInner {
    state: Mutex<State>,
    not_empty: Condvar,
    /// Notified when `process_count == 0 && queue.is_empty()`. Matches the
    /// reference's `cv_`, used by `rendezvous`/`stop`.
    idle: Condvar,
    stopping: AtomicBool,
    stopped: AtomicBool,
    next_index: AtomicU64,
    /// Total jobs ever completed (waiting + running + deferred history),
    /// used by [`JobQueue::is_overloaded`]'s simple heuristic and
    /// [`JobQueue::get_json`]'s diagnostic summary.
    load_events: AtomicU64,
    /// Cumulative count of jobs ever deferred (submitted while their type
    /// was at its concurrency limit). Unlike the per-type `deferred`
    /// counter in [`JobTypeCounters`] (which resets to zero once the
    /// backlog drains), this never decreases -- it exists so
    /// [`JobQueue::is_overloaded`] can still report recent overload
    /// pressure even after the jobs that caused it have already finished
    /// running, matching the reference's higher-level intent of flagging
    /// a queue that recently couldn't keep up, not just its
    /// instantaneous state at the moment of the check.
    ever_deferred: AtomicU64,
    workers: Mutex<Vec<JoinHandle<()>>>,
}

/// An RAII handle representing a job reserved via
/// [`JobQueue::reserve_next_job`], owning that job's "running" slot until
/// [`RunningJob::finish`] is called or the handle is dropped. Exposes the
/// same descriptive fields as the reference's `Job` (`job_type`, `name`,
/// `index`, `queue_time`) for callers driving execution manually.
///
/// If the handle is dropped without an explicit call to
/// [`RunningJob::finish`] (e.g. the reserving thread panics, or the
/// caller simply lets it go out of scope after running the job inline),
/// `Drop` performs the same `finish_job` bookkeeping and wakes a worker
/// in case a deferred job of this type is now runnable -- so the
/// concurrency-limit slot is never leaked regardless of how the handle's
/// lifetime ends.
pub struct RunningJob {
    inner: Arc<JobQueueInner>,
    job_type: JobType,
    name: String,
    index: u64,
    queue_time: Instant,
    func: Option<Box<dyn FnOnce() + Send + 'static>>,
    finished: bool,
}

impl RunningJob {
    /// The job type this reservation holds a "running" slot for.
    pub fn job_type(&self) -> JobType {
        self.job_type
    }

    /// The name the job was submitted with.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The monotonically increasing submission index used to break ties
    /// between same-priority jobs (earliest submitted runs first).
    pub fn index(&self) -> u64 {
        self.index
    }

    /// When this job was originally submitted to the queue (not when it
    /// was reserved), matching the reference's `Job::queueTime()`.
    pub fn queue_time(&self) -> Instant {
        self.queue_time
    }

    /// Explicitly release this job's "running" slot, promoting a
    /// deferred job of the same type if one is waiting and waking a
    /// worker to check for newly-runnable work. Idempotent: calling this
    /// more than once (or letting the handle drop afterward) has no
    /// further effect.
    pub fn finish(mut self) {
        self.finish_inner();
    }

    fn finish_inner(&mut self) {
        if self.finished {
            return;
        }
        self.finished = true;

        let mut guard = self.inner.state.lock();
        let promoted_deferred = finish_job(&mut guard, self.job_type);
        guard.process_count -= 1;
        let now_idle = guard.process_count == 0 && guard.queue.is_empty();
        drop(guard);

        self.inner.load_events.fetch_add(1, AtomicOrdering::Relaxed);

        if now_idle {
            self.inner.idle.notify_all();
        }
        if promoted_deferred {
            self.inner.not_empty.notify_one();
        }
    }
}

impl Drop for RunningJob {
    fn drop(&mut self) {
        self.finish_inner();
    }
}

impl std::fmt::Debug for JobQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JobQueue")
            .field("worker_thread_count", &self.worker_thread_count())
            .field("is_stopping", &self.is_stopping())
            .field("is_stopped", &self.is_stopped())
            .finish_non_exhaustive()
    }
}

impl Default for JobQueue {
    /// A small default thread count, used by call sites (and tests) that
    /// don't need to tune the pool size explicitly.
    fn default() -> Self {
        Self::new(2)
    }
}

impl JobQueue {
    /// Construct a new `JobQueue` and spawn `thread_count` persistent
    /// worker threads immediately. Matches the reference's constructor,
    /// which builds the `Workers` pool (and therefore its threads) as part
    /// of `JobQueue` construction.
    pub fn new(thread_count: usize) -> Self {
        let state = Mutex::new(State::new());
        let not_empty = Condvar::new();
        let idle = Condvar::new();
        let stopping = AtomicBool::new(false);
        let stopped = AtomicBool::new(false);
        let next_index = AtomicU64::new(0);
        let load_events = AtomicU64::new(0);

        let inner = Arc::new(JobQueueInner {
            state,
            not_empty,
            idle,
            stopping,
            stopped,
            next_index,
            load_events,
            ever_deferred: AtomicU64::new(0),
            workers: Mutex::new(Vec::new()),
        });

        let mut workers = Vec::with_capacity(thread_count);
        for instance in 0..thread_count {
            let inner = Arc::clone(&inner);
            let handle = std::thread::Builder::new()
                .name(format!("JobQueue-{instance}"))
                .spawn(move || worker_loop(instance, inner))
                .expect("failed to spawn JobQueue worker thread");
            workers.push(handle);
        }
        *inner.workers.lock() = workers;

        Self { inner }
    }

    /// Alias for [`JobQueue::new`], matching the reference's naming at
    /// call sites that construct the queue directly from a configured
    /// thread count (`application_root.rs`'s startup wiring).
    pub fn with_worker_threads(thread_count: usize) -> Self {
        Self::new(thread_count)
    }

    /// No-op: this port's workers are already spawned and running by the
    /// time [`JobQueue::new`] returns (see the module-level deviation
    /// note), unlike the reference's `Workers` pool which separates
    /// construction from starting the thread pool. Retained so call
    /// sites written against that two-step reference lifecycle (spawn,
    /// then explicitly start) still compile and behave correctly.
    pub fn run_worker_loop(&self) {}

    /// The number of persistent worker threads in this pool. Matches
    /// `Workers::getNumberOfThreads` in spirit (this pool's size is fixed
    /// at construction; see the module-level deviation note).
    pub fn worker_thread_count(&self) -> usize {
        self.inner.workers.lock().len()
    }

    /// Submit a job for execution. Matches `JobQueue::addJob`, minus the
    /// `JobCounter::wrap` reference-counting step (see module-level
    /// deviation note: Rust ownership makes that unnecessary).
    ///
    /// Returns `false` (and does not enqueue) if the queue is stopping or
    /// stopped, matching the reference's refusal to accept jobs after
    /// `stop()` has been called.
    ///
    /// # Panics (debug builds only)
    ///
    /// `job_type` must not be a "special" job type (`is_special()`,
    /// i.e. limit `0`). The reference documents special job types as
    /// "not dispatched by the job pool" -- they exist only to categorize
    /// work that runs inline on other threads (peer I/O, disk access,
    /// RPC handlers) rather than through `JobQueue`. Submitting one here
    /// would create a job that can never satisfy `running < limit()`
    /// (since the limit is zero) and would therefore sit in the queue
    /// forever. This is checked with a `debug_assert!` rather than a
    /// silent `false` return so the bug surfaces immediately in tests
    /// rather than manifesting as a mysteriously stuck job queue.
    pub fn add_job<F>(&self, job_type: JobType, name: impl Into<String>, func: F) -> bool
    where
        F: FnOnce() + Send + 'static,
    {
        debug_assert!(
            !job_type.is_special(),
            "JobQueue::add_job: {job_type:?} is a special job type (limit 0) and must never be \
             dispatched through the job pool -- it belongs on whatever thread the reference \
             documents it as running on instead"
        );

        if self.inner.stopping.load(AtomicOrdering::SeqCst)
            || self.inner.stopped.load(AtomicOrdering::SeqCst)
        {
            return false;
        }

        let index = self.inner.next_index.fetch_add(1, AtomicOrdering::SeqCst);
        let job = Job {
            job_type,
            name: name.into(),
            index,
            queue_time: Instant::now(),
            func: Box::new(func),
        };

        let mut state = self.inner.state.lock();
        let counters = state.counters_mut(job_type);
        let under_limit = counters.waiting + counters.running < job_type.limit();
        if !under_limit {
            // Defer: still enqueue the job itself (it's runnable once a
            // slot frees up), but track it as deferred for bookkeeping
            // parity with the reference's `data.deferred` counter.
            counters.deferred += 1;
            self.inner
                .ever_deferred
                .fetch_add(1, AtomicOrdering::Relaxed);
        }
        counters.waiting += 1;
        state.queue.push(job);
        drop(state);

        self.inner.not_empty.notify_one();
        true
    }

    /// Jobs waiting (not yet running) at this priority. Matches
    /// `getJobCount`.
    pub fn job_count(&self, job_type: JobType) -> usize {
        self.inner
            .state
            .lock()
            .counters
            .get(&job_type)
            .map(|c| c.waiting)
            .unwrap_or(0)
    }

    /// Jobs waiting plus running at this priority. Matches
    /// `getJobCountTotal`.
    pub fn job_count_total(&self, job_type: JobType) -> usize {
        let state = self.inner.state.lock();
        state
            .counters
            .get(&job_type)
            .map(|c| c.waiting + c.running)
            .unwrap_or(0)
    }

    /// All waiting jobs at or above this priority. Matches `getJobCountGE`.
    pub fn job_count_ge(&self, job_type: JobType) -> usize {
        let state = self.inner.state.lock();
        state
            .counters
            .iter()
            .filter(|&(&jt, _)| jt >= job_type)
            .map(|(_, c)| c.waiting)
            .sum()
    }

    /// Record `count` completed jobs of `job_type` having taken `elapsed`
    /// each, for the overload heuristic and diagnostic summary. Matches
    /// the reference's `JobQueue::addLoadEvents`, minus the latency
    /// histogram this port omits (see module-level deviation note) --
    /// only the count is tracked, which is enough to detect a queue
    /// that never stops accumulating load.
    pub fn add_load_events(&self, job_type: JobType, count: u64, _elapsed: std::time::Duration) {
        self.inner
            .load_events
            .fetch_add(count, AtomicOrdering::Relaxed);
        let mut state = self.inner.state.lock();
        state.counters_mut(job_type).load_events += count;
    }

    /// Reserve the highest-priority runnable job without running it,
    /// returning an RAII handle ([`RunningJob`]) that owns the "running"
    /// slot until it is explicitly finished (via [`RunningJob::finish`])
    /// or dropped. Returns `None` if no job is currently runnable (the
    /// queue is empty, or every waiting job's type is at its concurrency
    /// limit).
    ///
    /// This is a manual, synchronous alternative to the persistent
    /// worker-pool dispatch `add_job` normally triggers automatically --
    /// useful for callers that want to drive job execution themselves
    /// (e.g. a single-threaded test harness, or a custom worker loop) on
    /// a `JobQueue` that was constructed without its own workers doing
    /// the dispatching. It reuses the same underlying scheduling state
    /// and concurrency-limit bookkeeping as the automatic worker loop
    /// (`take_next_runnable_job`/`finish_job`), so counts observed via
    /// `job_count`/`job_count_total`/`job_count_ge` stay consistent
    /// regardless of which dispatch path is used.
    pub fn reserve_next_job(&self) -> Option<RunningJob> {
        let mut state = self.inner.state.lock();
        let job = state.take_next_runnable_job()?;
        state.process_count += 1;
        Some(RunningJob {
            inner: Arc::clone(&self.inner),
            job_type: job.job_type,
            name: job.name,
            index: job.index,
            queue_time: job.queue_time,
            func: Some(job.func),
            finished: false,
        })
    }

    /// Reserve and immediately run the next runnable job on the calling
    /// thread, then mark it finished. Returns `None` (without blocking)
    /// if no job is currently runnable. Matches the same manual-dispatch
    /// use case as [`JobQueue::reserve_next_job`], but folds
    /// reserve-run-finish into a single call for callers that just want
    /// to drive the queue forward one job at a time on their own thread
    /// (e.g. `while queue.dispatch_next_job().is_some() {}`).
    pub fn dispatch_next_job(&self) -> Option<()> {
        let mut running = self.reserve_next_job()?;
        let func = running
            .func
            .take()
            .expect("RunningJob::func is only taken once, by dispatch_next_job or Drop");
        func();
        running.finish();
        Some(())
    }

    /// Whether the queue is under enough sustained load to be considered
    /// overloaded. Matches `JobQueue::isOverloaded`'s intent (the
    /// reference checks whether any job type's average latency exceeds
    /// its configured threshold); this port instead flags overload
    /// whenever the queue has ever had to defer a job past its
    /// concurrency limit, since per-type latency histograms are out of
    /// scope (see module-level deviation note) but a saturated job type
    /// is exactly the condition those latency thresholds are meant to
    /// detect. Uses the cumulative [`JobQueueInner::ever_deferred`]
    /// counter (rather than the transient per-type `deferred` count,
    /// which resets once the backlog drains) so a brief overload spike
    /// stays visible to callers that check after the triggering jobs
    /// have already finished running.
    pub fn is_overloaded(&self) -> bool {
        self.inner.ever_deferred.load(AtomicOrdering::Relaxed) > 0
            || self
                .inner
                .state
                .lock()
                .counters
                .values()
                .any(|c| c.waiting > 1_000)
    }

    /// A minimal JSON diagnostic summary. Matches `JobQueue::getJson`'s
    /// intent (a per-type breakdown), reduced to the aggregate counters
    /// this port tracks -- full per-type latency percentiles are out of
    /// scope (see module-level deviation note).
    pub fn get_json(&self, _threshold: u32) -> serde_json::Value {
        let state = self.inner.state.lock();
        let jobs: Vec<serde_json::Value> = state
            .counters
            .iter()
            .filter(|&(_, c)| c.waiting > 0 || c.running > 0 || c.load_events > 0)
            .map(|(&job_type, c)| {
                serde_json::json!({
                    "job_type": job_type.name(),
                    "waiting": c.waiting,
                    "running": c.running,
                    "load_events": c.load_events,
                })
            })
            .collect();
        serde_json::json!({
            "threads": self.worker_thread_count(),
            "load_events": self.inner.load_events.load(AtomicOrdering::Relaxed),
            "jobs": jobs,
        })
    }

    /// Block until no jobs are waiting or running. Matches `rendezvous`.
    pub fn rendezvous(&self) {
        let mut state = self.inner.state.lock();
        while !(state.process_count == 0 && state.queue.is_empty()) {
            self.inner.idle.wait(&mut state);
        }
    }

    /// Alias for [`JobQueue::rendezvous`], matching call sites (and
    /// tests) that phrase the same "wait for the queue to drain" wait in
    /// terms of the pool becoming idle rather than a rendezvous point.
    pub fn run_until_idle(&self) {
        self.rendezvous();
    }

    /// Whether shutdown has been requested. Matches `isStopping`.
    pub fn is_stopping(&self) -> bool {
        self.inner.stopping.load(AtomicOrdering::SeqCst)
    }

    /// Whether the queue has fully stopped (all workers joined, all jobs
    /// drained). Matches `isStopped`.
    pub fn is_stopped(&self) -> bool {
        self.inner.stopped.load(AtomicOrdering::SeqCst)
    }

    /// Signal shutdown, wait for all in-flight and queued jobs to finish,
    /// wake and join every worker thread. Matches `JobQueue::stop`, with
    /// the `JobCounter::join` step folded into the same wait-then-join
    /// sequence (see module-level deviation note).
    ///
    /// Takes `&self` (the reference takes `&self` too): worker handles
    /// live behind an inner `Mutex` specifically so shutdown can be
    /// triggered through a shared `JobQueue` handle (e.g. `Arc<JobQueue>`
    /// or a cheap `Clone`) without requiring exclusive ownership.
    pub fn stop(&self) {
        self.inner.stopping.store(true, AtomicOrdering::SeqCst);

        // Wake all parked workers so they notice `stopping` and exit their
        // wait loop once the queue drains, rather than staying parked
        // forever waiting for a job that will never come.
        self.inner.not_empty.notify_all();

        {
            let mut state = self.inner.state.lock();
            while !(state.process_count == 0 && state.queue.is_empty()) {
                self.inner.idle.wait(&mut state);
            }
        }

        // Every worker will now observe `stopping` on its next wake and
        // exit its loop; wake them again in case they parked between the
        // notify_all above and now, then join.
        self.inner.not_empty.notify_all();
        let handles: Vec<JoinHandle<()>> = std::mem::take(&mut *self.inner.workers.lock());
        for handle in handles {
            let _ = handle.join();
        }

        self.inner.stopped.store(true, AtomicOrdering::SeqCst);
    }
}

impl Drop for JobQueueInner {
    fn drop(&mut self) {
        if !self.stopped.load(AtomicOrdering::SeqCst) && !self.workers.lock().is_empty() {
            self.stopping.store(true, AtomicOrdering::SeqCst);
            self.not_empty.notify_all();
            let handles: Vec<JoinHandle<()>> = std::mem::take(&mut *self.workers.lock());
            for handle in handles {
                let _ = handle.join();
            }
            self.stopped.store(true, AtomicOrdering::SeqCst);
        }
    }
}

/// The body of each persistent worker thread. Matches the reference's
/// `Worker::run` / `JobQueue::processTask`, minus latency instrumentation.
fn worker_loop(instance: usize, inner: Arc<JobQueueInner>) {
    loop {
        let job = {
            let mut guard = inner.state.lock();
            loop {
                if let Some(job) = guard.take_next_runnable_job() {
                    guard.process_count += 1;
                    break job;
                }
                if inner.stopping.load(AtomicOrdering::SeqCst) && guard.queue.is_empty() {
                    return;
                }
                // Real condvar park: no busy-polling. Woken by
                // `add_job`'s notify, `finish_job`'s deferred-slot notify,
                // or `stop`'s shutdown notify.
                inner.not_empty.wait(&mut guard);
            }
        };

        tracing::trace!(job_type = ?job.job_type, name = %job.name, queued_for = ?job.queued_for(Instant::now()), instance, "JobQueue: starting job");

        // Run the job behind `catch_unwind` so a panicking job cannot take
        // down this worker thread or leave `running`/`process_count`
        // permanently incremented (which would silently deadlock every
        // future job of that type). The reference's C++ jobs are not
        // expected to throw across `doJob()`, but Rust's job closures are
        // arbitrary application code and *can* panic; the queue's
        // scheduling invariants must survive that. The panic is logged
        // and then re-silenced (not re-panicked) so one bad job cannot
        // bring down the whole pool -- callers that need to know about
        // job failures should catch and report errors themselves inside
        // their job closure instead of relying on unwinding.
        let job_type = job.job_type;
        let job_name = job.name.clone();
        if let Err(payload) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job.func)) {
            let message = panic_message(&payload);
            tracing::error!(job_type = ?job_type, name = %job_name, %message, "JobQueue: job panicked");
        }

        let mut guard = inner.state.lock();
        let promoted_deferred = finish_job(&mut guard, job_type);
        guard.process_count -= 1;
        let now_idle = guard.process_count == 0 && guard.queue.is_empty();
        drop(guard);

        inner.load_events.fetch_add(1, AtomicOrdering::Relaxed);

        if now_idle {
            inner.idle.notify_all();
        }
        if promoted_deferred {
            // A deferred job for this type may now be runnable since a
            // slot just freed up; wake another worker to check.
            inner.not_empty.notify_one();
        }
    }
}

/// Best-effort extraction of a human-readable message from a caught panic
/// payload, for logging. Falls back to a generic message for payloads
/// that aren't `&str`/`String` (the two types `panic!`/`assert!` produce).
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "non-string panic payload".to_string()
    }
}

/// Decrement the running count for `job_type` and promote one deferred job
/// to waiting if the type was at its limit. Matches `JobQueue::finishJob`.
/// Returns whether a deferred job was promoted, so the caller knows
/// whether it's worth waking another worker to check for newly-runnable
/// work of this type.
fn finish_job(state: &mut State, job_type: JobType) -> bool {
    let counters = state.counters_mut(job_type);
    let promoted = counters.deferred > 0;
    if promoted {
        counters.deferred -= 1;
        // The deferred job itself is already sitting in `state.queue`
        // (see the module-level deviation note on `add_job`); freeing a
        // running slot is what makes it runnable. `waiting` was already
        // incremented at submission time, so nothing to adjust there.
    }
    counters.running = counters.running.saturating_sub(1);
    promoted
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn worker_thread_count_matches_construction_argument() {
        let jq = JobQueue::new(4);
        assert_eq!(jq.worker_thread_count(), 4);
        jq.stop();
    }

    #[test]
    fn a_submitted_job_actually_runs() {
        let jq = JobQueue::new(2);
        let (tx, rx) = mpsc::channel();

        assert!(jq.add_job(JobType::JtClient, "test", move || {
            tx.send(42).unwrap();
        }));

        let value = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("job should run and send a value");
        assert_eq!(value, 42);
        jq.stop();
    }

    #[test]
    fn higher_priority_jobs_run_before_lower_priority_ones() {
        // Use a single worker thread so execution order is deterministic,
        // and hold it busy with a "gate" job while we enqueue a low- and
        // a high-priority job, then release the gate and observe order.
        let jq = JobQueue::new(1);
        let (gate_tx, gate_rx) = mpsc::channel::<()>();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let (order_tx, order_rx) = mpsc::channel::<&'static str>();

        assert!(jq.add_job(JobType::JtWal, "gate", move || {
            gate_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        }));
        gate_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("gate job should start");

        // While the single worker is blocked in the gate job, queue a
        // low-priority and a high-priority job.
        let order_tx_low = order_tx.clone();
        assert!(jq.add_job(JobType::JtPack, "low", move || {
            order_tx_low.send("low").unwrap();
        }));
        let order_tx_high = order_tx.clone();
        assert!(jq.add_job(JobType::JtAdmin, "high", move || {
            order_tx_high.send("high").unwrap();
        }));

        release_tx.send(()).unwrap();

        let first = order_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("first job should run");
        let second = order_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("second job should run");
        assert_eq!(first, "high");
        assert_eq!(second, "low");
        jq.stop();
    }

    #[test]
    fn job_count_reflects_waiting_jobs_before_they_run() {
        let jq = JobQueue::new(1);
        let (gate_tx, gate_rx) = mpsc::channel::<()>();
        let (release_tx, release_rx) = mpsc::channel::<()>();

        assert!(jq.add_job(JobType::JtWal, "gate", move || {
            gate_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        }));
        gate_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("gate job should start");

        assert!(jq.add_job(JobType::JtPack, "waiting-job", || {}));
        assert_eq!(jq.job_count(JobType::JtPack), 1);
        assert_eq!(jq.job_count_total(JobType::JtPack), 1);

        release_tx.send(()).unwrap();
        jq.rendezvous();
        assert_eq!(jq.job_count(JobType::JtPack), 0);
        jq.stop();
    }

    #[test]
    fn per_type_limit_defers_jobs_beyond_the_limit() {
        // JtPack has a limit of 1. Submitting two JtPack jobs at once
        // should run only one concurrently even with multiple workers
        // available, and both should eventually complete.
        let jq = JobQueue::new(4);
        let (tx, rx) = mpsc::channel::<u32>();
        let barrier_state: Arc<Mutex<usize>> = Arc::new(Mutex::new(0));

        for i in 0..2u32 {
            let tx = tx.clone();
            let barrier_state = Arc::clone(&barrier_state);
            assert!(jq.add_job(JobType::JtPack, "limited", move || {
                {
                    let mut running = barrier_state.lock();
                    *running += 1;
                    assert_eq!(
                        *running, 1,
                        "JtPack limit of 1 must not be exceeded concurrently"
                    );
                }
                std::thread::sleep(Duration::from_millis(20));
                {
                    let mut running = barrier_state.lock();
                    *running -= 1;
                }
                tx.send(i).unwrap();
            }));
        }

        let mut results = vec![
            rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            rx.recv_timeout(Duration::from_secs(5)).unwrap(),
        ];
        results.sort();
        assert_eq!(results, vec![0, 1]);
        jq.stop();
    }

    #[test]
    fn stop_drains_queue_and_prevents_new_jobs() {
        let jq = JobQueue::new(2);
        let (tx, rx) = mpsc::channel();
        assert!(jq.add_job(JobType::JtClient, "final", move || {
            tx.send(()).unwrap();
        }));
        rx.recv_timeout(Duration::from_secs(5)).unwrap();

        jq.stop();
        assert!(jq.is_stopped());
        assert!(!jq.add_job(JobType::JtClient, "after-stop", || {}));
    }

    #[test]
    fn rendezvous_returns_once_queue_and_running_jobs_are_empty() {
        let jq = JobQueue::new(2);
        for _ in 0..5 {
            assert!(jq.add_job(JobType::JtClient, "batch", || {
                std::thread::sleep(Duration::from_millis(5));
            }));
        }
        jq.rendezvous();
        assert_eq!(jq.job_count_total(JobType::JtClient), 0);
        jq.stop();
    }
}
