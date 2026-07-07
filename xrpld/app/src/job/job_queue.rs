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
//!   mechanism for RPC handlers) is out of scope for this phase. Nothing
//!   in the consensus/RCL rewrite path (Phases 5-7) requires coroutine
//!   support; it is deferred to whenever RPC handler code needs it.
//! - `LoadMonitor`/`PerfLog`/`beast::insight::Collector` integration
//!   (latency histograms, overload detection, metrics hooks) is omitted.
//!   These are diagnostic/observability concerns layered on top of a
//!   working scheduler, not part of the scheduling algorithm itself, and
//!   are deferred to whenever the app wires up its metrics story.
//! - `getJson`/`isOverloaded` diagnostic surfaces are omitted for the same
//!   reason `Consensus::getJson` was omitted in Phase 3: presentation
//!   concerns belong at a higher layer once the underlying data exists.

use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering};
use std::sync::Arc;
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
        f.debug_struct("Job").field("job_type", &self.job_type).field("name", &self.name).field("index", &self.index).finish()
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
        self.job_type.cmp(&other.job_type).then_with(|| other.index.cmp(&self.index))
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
        Self { queue: BinaryHeap::new(), counters: std::collections::BTreeMap::new(), process_count: 0 }
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
            let running = self.counters.get(&job.job_type).map(|c| c.running).unwrap_or(0);
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
pub struct JobQueue {
    state: Arc<Mutex<State>>,
    not_empty: Arc<Condvar>,
    /// Notified when `process_count == 0 && queue.is_empty()`. Matches the
    /// reference's `cv_`, used by `rendezvous`/`stop`.
    idle: Arc<Condvar>,
    stopping: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
    next_index: Arc<AtomicU64>,
    workers: Vec<JoinHandle<()>>,
}

impl JobQueue {
    /// Construct a new `JobQueue` and spawn `thread_count` persistent
    /// worker threads immediately. Matches the reference's constructor,
    /// which builds the `Workers` pool (and therefore its threads) as part
    /// of `JobQueue` construction.
    pub fn new(thread_count: usize) -> Self {
        let state = Arc::new(Mutex::new(State::new()));
        let not_empty = Arc::new(Condvar::new());
        let idle = Arc::new(Condvar::new());
        let stopping = Arc::new(AtomicBool::new(false));
        let stopped = Arc::new(AtomicBool::new(false));
        let next_index = Arc::new(AtomicU64::new(0));

        let mut workers = Vec::with_capacity(thread_count);
        for instance in 0..thread_count {
            let state = Arc::clone(&state);
            let not_empty = Arc::clone(&not_empty);
            let idle = Arc::clone(&idle);
            let stopping = Arc::clone(&stopping);

            let handle = std::thread::Builder::new()
                .name(format!("JobQueue-{instance}"))
                .spawn(move || worker_loop(instance, state, not_empty, idle, stopping))
                .expect("failed to spawn JobQueue worker thread");
            workers.push(handle);
        }

        Self { state, not_empty, idle, stopping, stopped, next_index, workers }
    }

    /// The number of persistent worker threads in this pool. Matches
    /// `Workers::getNumberOfThreads` in spirit (this pool's size is fixed
    /// at construction; see the module-level deviation note).
    pub fn worker_thread_count(&self) -> usize {
        self.workers.len()
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

        if self.stopping.load(AtomicOrdering::SeqCst) || self.stopped.load(AtomicOrdering::SeqCst) {
            return false;
        }

        let index = self.next_index.fetch_add(1, AtomicOrdering::SeqCst);
        let job = Job { job_type, name: name.into(), index, queue_time: Instant::now(), func: Box::new(func) };

        let mut state = self.state.lock();
        let counters = state.counters_mut(job_type);
        let under_limit = counters.waiting + counters.running < job_type.limit();
        if !under_limit {
            // Defer: still enqueue the job itself (it's runnable once a
            // slot frees up), but track it as deferred for bookkeeping
            // parity with the reference's `data.deferred` counter.
            counters.deferred += 1;
        }
        counters.waiting += 1;
        state.queue.push(job);
        drop(state);

        self.not_empty.notify_one();
        true
    }

    /// Jobs waiting (not yet running) at this priority. Matches
    /// `getJobCount`.
    pub fn job_count(&self, job_type: JobType) -> usize {
        self.state.lock().counters.get(&job_type).map(|c| c.waiting).unwrap_or(0)
    }

    /// Jobs waiting plus running at this priority. Matches
    /// `getJobCountTotal`.
    pub fn job_count_total(&self, job_type: JobType) -> usize {
        let state = self.state.lock();
        state.counters.get(&job_type).map(|c| c.waiting + c.running).unwrap_or(0)
    }

    /// All waiting jobs at or above this priority. Matches `getJobCountGE`.
    pub fn job_count_ge(&self, job_type: JobType) -> usize {
        let state = self.state.lock();
        state.counters.iter().filter(|&(&jt, _)| jt >= job_type).map(|(_, c)| c.waiting).sum()
    }

    /// Block until no jobs are waiting or running. Matches `rendezvous`.
    pub fn rendezvous(&self) {
        let mut state = self.state.lock();
        while !(state.process_count == 0 && state.queue.is_empty()) {
            self.idle.wait(&mut state);
        }
    }

    /// Whether shutdown has been requested. Matches `isStopping`.
    pub fn is_stopping(&self) -> bool {
        self.stopping.load(AtomicOrdering::SeqCst)
    }

    /// Whether the queue has fully stopped (all workers joined, all jobs
    /// drained). Matches `isStopped`.
    pub fn is_stopped(&self) -> bool {
        self.stopped.load(AtomicOrdering::SeqCst)
    }

    /// Signal shutdown, wait for all in-flight and queued jobs to finish,
    /// wake and join every worker thread. Matches `JobQueue::stop`, with
    /// the `JobCounter::join` step folded into the same wait-then-join
    /// sequence (see module-level deviation note).
    ///
    /// Takes `&mut self` (rather than the reference's `&self`) because
    /// joining worker threads requires taking ownership of their
    /// `JoinHandle`s, which callers can't do through a shared reference.
    pub fn stop(&mut self) {
        self.stopping.store(true, AtomicOrdering::SeqCst);

        // Wake all parked workers so they notice `stopping` and exit their
        // wait loop once the queue drains, rather than staying parked
        // forever waiting for a job that will never come.
        self.not_empty.notify_all();

        {
            let mut state = self.state.lock();
            while !(state.process_count == 0 && state.queue.is_empty()) {
                self.idle.wait(&mut state);
            }
        }

        // Every worker will now observe `stopping` on its next wake and
        // exit its loop; wake them again in case they parked between the
        // notify_all above and now, then join.
        self.not_empty.notify_all();
        for handle in self.workers.drain(..) {
            let _ = handle.join();
        }

        self.stopped.store(true, AtomicOrdering::SeqCst);
    }
}

impl Drop for JobQueue {
    fn drop(&mut self) {
        if !self.stopped.load(AtomicOrdering::SeqCst) {
            self.stop();
        }
    }
}

/// The body of each persistent worker thread. Matches the reference's
/// `Worker::run` / `JobQueue::processTask`, minus latency instrumentation.
fn worker_loop(instance: usize, state: Arc<Mutex<State>>, not_empty: Arc<Condvar>, idle: Arc<Condvar>, stopping: Arc<AtomicBool>) {
    loop {
        let job = {
            let mut guard = state.lock();
            loop {
                if let Some(job) = guard.take_next_runnable_job() {
                    guard.process_count += 1;
                    break job;
                }
                if stopping.load(AtomicOrdering::SeqCst) && guard.queue.is_empty() {
                    return;
                }
                // Real condvar park: no busy-polling. Woken by
                // `add_job`'s notify, `finish_job`'s deferred-slot notify,
                // or `stop`'s shutdown notify.
                not_empty.wait(&mut guard);
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

        let mut guard = state.lock();
        let promoted_deferred = finish_job(&mut guard, job_type);
        guard.process_count -= 1;
        let now_idle = guard.process_count == 0 && guard.queue.is_empty();
        drop(guard);

        if now_idle {
            idle.notify_all();
        }
        if promoted_deferred {
            // A deferred job for this type may now be runnable since a
            // slot just freed up; wake another worker to check.
            not_empty.notify_one();
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
        let mut jq = JobQueue::new(4);
        assert_eq!(jq.worker_thread_count(), 4);
        jq.stop();
    }

    #[test]
    fn a_submitted_job_actually_runs() {
        let mut jq = JobQueue::new(2);
        let (tx, rx) = mpsc::channel();

        assert!(jq.add_job(JobType::JtClient, "test", move || {
            tx.send(42).unwrap();
        }));

        let value = rx.recv_timeout(Duration::from_secs(5)).expect("job should run and send a value");
        assert_eq!(value, 42);
        jq.stop();
    }

    #[test]
    fn higher_priority_jobs_run_before_lower_priority_ones() {
        // Use a single worker thread so execution order is deterministic,
        // and hold it busy with a "gate" job while we enqueue a low- and
        // a high-priority job, then release the gate and observe order.
        let mut jq = JobQueue::new(1);
        let (gate_tx, gate_rx) = mpsc::channel::<()>();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let (order_tx, order_rx) = mpsc::channel::<&'static str>();

        assert!(jq.add_job(JobType::JtWal, "gate", move || {
            gate_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        }));
        gate_rx.recv_timeout(Duration::from_secs(5)).expect("gate job should start");

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

        let first = order_rx.recv_timeout(Duration::from_secs(5)).expect("first job should run");
        let second = order_rx.recv_timeout(Duration::from_secs(5)).expect("second job should run");
        assert_eq!(first, "high");
        assert_eq!(second, "low");
        jq.stop();
    }

    #[test]
    fn job_count_reflects_waiting_jobs_before_they_run() {
        let mut jq = JobQueue::new(1);
        let (gate_tx, gate_rx) = mpsc::channel::<()>();
        let (release_tx, release_rx) = mpsc::channel::<()>();

        assert!(jq.add_job(JobType::JtWal, "gate", move || {
            gate_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        }));
        gate_rx.recv_timeout(Duration::from_secs(5)).expect("gate job should start");

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
        let mut jq = JobQueue::new(4);
        let (tx, rx) = mpsc::channel::<u32>();
        let barrier_state: Arc<Mutex<usize>> = Arc::new(Mutex::new(0));

        for i in 0..2u32 {
            let tx = tx.clone();
            let barrier_state = Arc::clone(&barrier_state);
            assert!(jq.add_job(JobType::JtPack, "limited", move || {
                {
                    let mut running = barrier_state.lock();
                    *running += 1;
                    assert_eq!(*running, 1, "JtPack limit of 1 must not be exceeded concurrently");
                }
                std::thread::sleep(Duration::from_millis(20));
                {
                    let mut running = barrier_state.lock();
                    *running -= 1;
                }
                tx.send(i).unwrap();
            }));
        }

        let mut results = vec![rx.recv_timeout(Duration::from_secs(5)).unwrap(), rx.recv_timeout(Duration::from_secs(5)).unwrap()];
        results.sort();
        assert_eq!(results, vec![0, 1]);
        jq.stop();
    }

    #[test]
    fn stop_drains_queue_and_prevents_new_jobs() {
        let mut jq = JobQueue::new(2);
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
        let mut jq = JobQueue::new(2);
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
