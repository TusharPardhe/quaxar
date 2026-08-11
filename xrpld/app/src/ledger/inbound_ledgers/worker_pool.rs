//! Acquisition job queue and delayed timer service.
//!
//! This mirrors rippled's split between `JobQueue` work and `TimeoutCounter`
//! timers: delayed callbacks only enqueue work; they never run acquisition
//! logic on the timer thread.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

// `TimeoutCounter` admits recovery work only while fewer than this many
// JtLedgerData-equivalent jobs are live (queued or executing). The isolated
// throughput experiment permits a bounded recovery backlog alongside eight
// workers; ordinary packet jobs remain independently coalesced.
const LEDGER_DATA_JOB_LIMIT: usize = 16;

type Job = Box<dyn FnOnce() + Send>;

struct QueueState {
    ledger_data: VecDeque<Job>,
}

impl QueueState {
    fn is_empty(&self) -> bool {
        self.ledger_data.is_empty()
    }

    fn dequeue(&mut self) -> Job {
        self.ledger_data
            .pop_front()
            .expect("non-empty ledger-data queue")
    }
}

/// Releases a JobQueue-equivalent reservation even when the job unwinds or is
/// discarded during shutdown.
struct LedgerDataReservation {
    count: Arc<AtomicUsize>,
}

impl Drop for LedgerDataReservation {
    fn drop(&mut self) {
        self.count.fetch_sub(1, Ordering::AcqRel);
    }
}

fn reserve_ledger_data_job(job: Job, count: Arc<AtomicUsize>) -> Job {
    let reservation = LedgerDataReservation { count };
    Box::new(move || {
        let _reservation = reservation;
        job();
    })
}

/// Bounded, read-only worker-pool state for acquisition diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkerPoolSnapshot {
    pub queued_jobs: usize,
    pub outstanding_ledger_data_jobs: usize,
    pub worker_count: usize,
    pub ledger_data_job_limit: usize,
    pub timeout_submission_attempts: u64,
    pub timeout_submission_rejected: u64,
}

struct TimerTask {
    due: Instant,
    delay: Duration,
    callback: Job,
}

struct TimerState {
    tasks: Vec<TimerTask>,
}

/// A single delayed-callback service. Each acquisition owns its timer lifecycle,
/// while this service provides the shared event-loop equivalent.
struct TimerService {
    state: Arc<(Mutex<TimerState>, Condvar)>,
    stopped: Arc<AtomicBool>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl TimerService {
    fn new() -> Self {
        let state = Arc::new((Mutex::new(TimerState { tasks: Vec::new() }), Condvar::new()));
        let stopped = Arc::new(AtomicBool::new(false));
        let thread_state = Arc::clone(&state);
        let thread_stopped = Arc::clone(&stopped);
        let thread = thread::Builder::new()
            .name("xrpld-acq-timer".to_owned())
            .spawn(move || {
                loop {
                    let callback = {
                        let (lock, wake) = &*thread_state;
                        let mut state = lock.lock().expect("acquisition timer lock");
                        loop {
                            if thread_stopped.load(Ordering::Acquire) {
                                return;
                            }
                            let Some((index, due)) = state
                                .tasks
                                .iter()
                                .enumerate()
                                .min_by_key(|(_, task)| task.due)
                                .map(|(index, task)| (index, task.due))
                            else {
                                state = wake.wait(state).expect("acquisition timer wait");
                                continue;
                            };
                            let now = Instant::now();
                            if due > now {
                                let (next, _) = wake
                                    .wait_timeout(state, due.duration_since(now))
                                    .expect("acquisition timer wait_timeout");
                                state = next;
                                continue;
                            }
                            break state.tasks.swap_remove(index).callback;
                        }
                    };
                    callback();
                }
            })
            .expect("acquisition timer thread should spawn");
        Self {
            state,
            stopped,
            thread: Mutex::new(Some(thread)),
        }
    }

    fn schedule(&self, delay: Duration, callback: Job) {
        if self.stopped.load(Ordering::Acquire) {
            return;
        }
        let (lock, wake) = &*self.state;
        lock.lock()
            .expect("acquisition timer lock")
            .tasks
            .push(TimerTask {
                due: Instant::now() + delay,
                delay,
                callback,
            });
        wake.notify_one();
    }

    #[cfg(test)]
    fn scheduled_delays_for_test(&self) -> Vec<Duration> {
        self.state
            .0
            .lock()
            .expect("acquisition timer lock")
            .tasks
            .iter()
            .map(|task| task.delay)
            .collect()
    }

    #[cfg(test)]
    fn fire_next_for_test(&self) -> Option<Duration> {
        let task = {
            let (lock, _) = &*self.state;
            let mut state = lock.lock().expect("acquisition timer lock");
            let index = state
                .tasks
                .iter()
                .enumerate()
                .min_by_key(|(_, task)| task.due)
                .map(|(index, _)| index)?;
            state.tasks.swap_remove(index)
        };
        let delay = task.delay;
        (task.callback)();
        Some(delay)
    }

    fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        self.state.1.notify_all();
        if let Some(thread) = self.thread.lock().expect("timer thread lock").take() {
            let _ = thread.join();
        }
    }
}

impl Drop for TimerService {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Fixed-size worker pool for `JtLedgerData`-equivalent jobs.
pub struct WorkerPool {
    queue: Arc<(Mutex<QueueState>, Condvar)>,
    stop: Arc<AtomicBool>,
    workers: Mutex<Vec<JoinHandle<()>>>,
    ledger_data_jobs: Arc<AtomicUsize>,
    timeout_submission_attempts: AtomicU64,
    timeout_submission_rejected: AtomicU64,
    timers: TimerService,
}

impl WorkerPool {
    pub fn new(size: usize) -> Self {
        let queue = Arc::new((
            Mutex::new(QueueState {
                ledger_data: VecDeque::new(),
            }),
            Condvar::new(),
        ));
        let stop = Arc::new(AtomicBool::new(false));
        let mut workers = Vec::with_capacity(size);

        for index in 0..size {
            let queue = Arc::clone(&queue);
            let stop = Arc::clone(&stop);
            workers.push(
                thread::Builder::new()
                    .name(format!("xrpld-acq-pool-{index}"))
                    .spawn(move || {
                        loop {
                            let job = {
                                let (lock, wake) = &*queue;
                                let mut jobs = lock.lock().expect("acquisition queue lock");
                                while jobs.is_empty() && !stop.load(Ordering::Acquire) {
                                    jobs = wake.wait(jobs).expect("acquisition queue wait");
                                }
                                if stop.load(Ordering::Acquire) {
                                    return;
                                }
                                jobs.dequeue()
                            };
                            if let Err(payload) =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(job))
                            {
                                let message = payload
                                    .downcast_ref::<String>()
                                    .cloned()
                                    .or_else(|| {
                                        payload
                                            .downcast_ref::<&str>()
                                            .map(|message| (*message).to_owned())
                                    })
                                    .unwrap_or_else(|| "non-string panic payload".to_owned());
                                tracing::error!(
                                    target: "inbound_ledger",
                                    worker = index,
                                    %message,
                                    "acquisition worker job panicked; worker remains available"
                                );
                            }
                        }
                    })
                    .expect("acquisition worker should spawn"),
            );
        }

        Self {
            queue,
            stop,
            workers: Mutex::new(workers),
            ledger_data_jobs: Arc::new(AtomicUsize::new(0)),
            timeout_submission_attempts: AtomicU64::new(0),
            timeout_submission_rejected: AtomicU64::new(0),
            timers: TimerService::new(),
        }
    }

    fn enqueue_ledger_data(&self, job: Job) {
        let (lock, wake) = &*self.queue;
        let mut jobs = lock.lock().expect("acquisition queue lock");
        if self.stop.load(Ordering::Acquire) {
            return;
        }

        self.ledger_data_jobs.fetch_add(1, Ordering::AcqRel);
        jobs.ledger_data.push_back(reserve_ledger_data_job(
            job,
            Arc::clone(&self.ledger_data_jobs),
        ));
        wake.notify_all();
    }

    /// Queue a response-processing job. rippled queues this whenever
    /// `InboundLedger::gotData` transitions its dispatch flag to true. This
    /// must not wait for queue capacity: the coalesced dispatch remains live
    /// until its queued job runs.
    pub fn submit_ledger_data(&self, job: Job) {
        if !self.stop.load(Ordering::Acquire) {
            self.enqueue_ledger_data(job);
        }
    }

    /// Queue TimeoutCounter recovery work only while the aggregate live
    /// JtLedgerData-equivalent count is below rippled's admission limit. A
    /// false result lets the timer re-arm rather than dropping recovery work.
    pub fn try_submit_timeout(&self, job: Job) -> bool {
        self.timeout_submission_attempts
            .fetch_add(1, Ordering::Relaxed);
        if self.stop.load(Ordering::Acquire) {
            self.timeout_submission_rejected
                .fetch_add(1, Ordering::Relaxed);
            return false;
        }

        let (lock, wake) = &*self.queue;
        let mut jobs = lock.lock().expect("acquisition queue lock");
        if self.stop.load(Ordering::Acquire)
            || self.ledger_data_jobs.load(Ordering::Acquire) >= LEDGER_DATA_JOB_LIMIT
        {
            self.timeout_submission_rejected
                .fetch_add(1, Ordering::Relaxed);
            return false;
        }

        self.ledger_data_jobs.fetch_add(1, Ordering::AcqRel);
        jobs.ledger_data.push_back(reserve_ledger_data_job(
            job,
            Arc::clone(&self.ledger_data_jobs),
        ));
        wake.notify_all();
        true
    }

    /// Schedule one delayed timer callback. The callback must decide whether
    /// to re-arm itself, exactly as `TimeoutCounter::invokeOnTimer` does.
    pub fn schedule_after(&self, delay: Duration, callback: Job) {
        if !self.stop.load(Ordering::Acquire) {
            self.timers.schedule(delay, callback);
        }
    }

    /// Read the current queue pressure without mutating scheduling state.
    pub fn snapshot(&self) -> WorkerPoolSnapshot {
        let jobs = self.queue.0.lock().expect("acquisition queue lock");
        let queued_jobs = jobs.ledger_data.len();
        let worker_count = self.workers.lock().expect("acquisition workers lock").len();
        WorkerPoolSnapshot {
            queued_jobs,
            outstanding_ledger_data_jobs: self.ledger_data_jobs.load(Ordering::Acquire),
            worker_count,
            ledger_data_job_limit: LEDGER_DATA_JOB_LIMIT,
            timeout_submission_attempts: self.timeout_submission_attempts.load(Ordering::Relaxed),
            timeout_submission_rejected: self.timeout_submission_rejected.load(Ordering::Relaxed),
        }
    }

    #[cfg(test)]
    pub(crate) fn scheduled_timer_delays_for_test(&self) -> Vec<Duration> {
        self.timers.scheduled_delays_for_test()
    }

    #[cfg(test)]
    pub(crate) fn fire_next_timer_for_test(&self) -> Option<Duration> {
        self.timers.fire_next_for_test()
    }

    #[cfg(test)]
    pub(crate) fn run_next_job_for_test(&self) -> bool {
        let job = {
            let (lock, _) = &*self.queue;
            let mut jobs = lock.lock().expect("acquisition queue lock");
            jobs.ledger_data.pop_front()
        };
        let Some(job) = job else {
            return false;
        };
        job();
        true
    }

    pub fn stop(&self) {
        if self.stop.swap(true, Ordering::AcqRel) {
            return;
        }
        self.timers.stop();
        let (lock, wake) = &*self.queue;
        {
            let mut jobs = lock.lock().expect("acquisition queue lock");
            // Workers stop without consuming queued work. Drop it now so each
            // queued reservation is released.
            jobs.ledger_data.clear();
        }
        wake.notify_all();
        for worker in self
            .workers
            .lock()
            .expect("acquisition workers lock")
            .drain(..)
        {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::WorkerPool;
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::Duration;

    #[test]
    fn worker_survives_a_panicking_job_and_runs_the_next_job() {
        let pool = WorkerPool::new(1);
        let (done_tx, done_rx) = mpsc::channel();

        pool.submit_ledger_data(Box::new(|| panic!("intentional acquisition job panic")));
        pool.submit_ledger_data(Box::new(move || {
            done_tx.send(()).expect("test receiver should remain live");
        }));

        done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("worker should continue after a panicking job");
        pool.stop();
    }

    #[test]
    fn manual_timer_fires_callback_before_manually_drained_worker_job() {
        let pool = Arc::new(WorkerPool::new(0));
        let events = Arc::new(Mutex::new(Vec::new()));
        let callback_pool = Arc::clone(&pool);
        let callback_events = Arc::clone(&events);

        pool.schedule_after(
            Duration::from_secs(3),
            Box::new(move || {
                callback_events
                    .lock()
                    .expect("event recorder lock")
                    .push("timer_callback");
                let job_events = Arc::clone(&callback_events);
                callback_pool.submit_ledger_data(Box::new(move || {
                    job_events
                        .lock()
                        .expect("event recorder lock")
                        .push("worker_job");
                }));
            }),
        );

        assert_eq!(
            pool.scheduled_timer_delays_for_test(),
            vec![Duration::from_secs(3)]
        );
        assert_eq!(
            pool.fire_next_timer_for_test(),
            Some(Duration::from_secs(3))
        );
        assert_eq!(
            *events.lock().expect("event recorder lock"),
            vec!["timer_callback"]
        );
        assert_eq!(pool.snapshot().queued_jobs, 1);

        assert!(pool.run_next_job_for_test());
        assert_eq!(
            *events.lock().expect("event recorder lock"),
            vec!["timer_callback", "worker_job"]
        );
        assert!(!pool.run_next_job_for_test());
    }

    #[test]
    fn timeout_admission_snapshot_counts_rejected_submission() {
        let pool = WorkerPool::new(1);
        pool.stop();

        assert!(!pool.try_submit_timeout(Box::new(|| {})));
        let snapshot = pool.snapshot();
        assert_eq!(snapshot.timeout_submission_attempts, 1);
        assert_eq!(snapshot.timeout_submission_rejected, 1);
    }
}

impl Drop for WorkerPool {
    fn drop(&mut self) {
        self.stop();
    }
}
