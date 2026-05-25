//! Owner-level `TimeoutCounter` port for ledger acquisition-style tasks.
//!
//! This keeps the reference timer/job loop shape explicit:
//! - `set_timer()` schedules a delayed callback,
//! - the delayed callback queues a job unless load defers it,
//! - `invoke_on_timer()` increments timeouts when no progress was marked,
//! - `cancel()` marks the task failed without unscheduling existing work.

use basics::base_uint::Uint256;
use std::sync::{Arc, Mutex, Weak};
use time::Duration;

pub type TimeoutCounterJob = Box<dyn FnOnce() + Send + 'static>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeoutCounterJobConfig {
    pub job_name: String,
    pub job_limit: Option<u32>,
}

impl TimeoutCounterJobConfig {
    pub fn new(job_name: impl Into<String>, job_limit: Option<u32>) -> Self {
        Self {
            job_name: job_name.into(),
            job_limit,
        }
    }
}

pub trait TimeoutCounterRuntime: Send + Sync + 'static {
    fn schedule_after(&self, delay: Duration, job: TimeoutCounterJob);
    fn outstanding_jobs(&self, job_name: &str) -> u32;
    fn enqueue_job(&self, job_name: &str, job: TimeoutCounterJob);
}

#[derive(Debug, Default)]
pub struct NullTimeoutCounterRuntime;

impl TimeoutCounterRuntime for NullTimeoutCounterRuntime {
    fn schedule_after(&self, _delay: Duration, _job: TimeoutCounterJob) {}

    fn outstanding_jobs(&self, _job_name: &str) -> u32 {
        0
    }

    fn enqueue_job(&self, _job_name: &str, _job: TimeoutCounterJob) {}
}

pub trait TimeoutCounterJournal: Send + Sync + 'static {
    fn debug(&self, _message: &str) {}
    fn info(&self, _message: &str) {}
}

#[derive(Debug, Default)]
pub struct NullTimeoutCounterJournal;

impl TimeoutCounterJournal for NullTimeoutCounterJournal {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeoutCounterSnapshot {
    pub hash: Uint256,
    pub timeouts: u32,
    pub complete: bool,
    pub failed: bool,
    pub progress: bool,
    pub timer_interval: Duration,
    pub job_name: String,
    pub job_limit: Option<u32>,
}

#[derive(Clone)]
pub struct TimeoutCounter {
    inner: Arc<TimeoutCounterInner>,
}

struct TimeoutCounterInner {
    runtime: Arc<dyn TimeoutCounterRuntime>,
    journal: Arc<dyn TimeoutCounterJournal>,
    on_timer: Arc<dyn Fn(bool, TimeoutCounter) + Send + Sync>,
    state: Mutex<TimeoutCounterState>,
}

#[derive(Debug)]
struct TimeoutCounterState {
    hash: Uint256,
    timeouts: u32,
    complete: bool,
    failed: bool,
    progress: bool,
    timer_interval: Duration,
    job: TimeoutCounterJobConfig,
}

impl TimeoutCounter {
    pub fn new(
        runtime: Arc<dyn TimeoutCounterRuntime>,
        journal: Arc<dyn TimeoutCounterJournal>,
        hash: Uint256,
        timer_interval: Duration,
        job: TimeoutCounterJobConfig,
        on_timer: impl Fn(bool, TimeoutCounter) + Send + Sync + 'static,
    ) -> Self {
        assert!(
            timer_interval > Duration::milliseconds(10) && timer_interval < Duration::seconds(30),
            "xrpl::TimeoutCounter::TimeoutCounter : interval input inside range"
        );

        Self {
            inner: Arc::new(TimeoutCounterInner {
                runtime,
                journal,
                on_timer: Arc::new(on_timer),
                state: Mutex::new(TimeoutCounterState {
                    hash,
                    timeouts: 0,
                    complete: false,
                    failed: false,
                    progress: false,
                    timer_interval,
                    job,
                }),
            }),
        }
    }

    pub fn set_timer(&self) {
        if self.is_done() {
            return;
        }

        let delay = self
            .inner
            .state
            .lock()
            .expect("timeout-counter mutex must not be poisoned")
            .timer_interval;
        let weak = Arc::downgrade(&self.inner);
        self.inner
            .runtime
            .schedule_after(delay, Box::new(move || run_set_timer(weak)));
    }

    pub fn queue_job(&self) {
        let (job_name, job_limit) = {
            let state = self
                .inner
                .state
                .lock()
                .expect("timeout-counter mutex must not be poisoned");
            if state.complete || state.failed {
                return;
            }
            (state.job.job_name.clone(), state.job.job_limit)
        };

        if job_limit.is_some_and(|limit| self.inner.runtime.outstanding_jobs(&job_name) >= limit) {
            self.inner
                .journal
                .debug(&format!("Deferring {} timer due to load", job_name));
            self.set_timer();
            return;
        }

        let weak = Arc::downgrade(&self.inner);
        self.inner.runtime.enqueue_job(
            &job_name,
            Box::new(move || {
                if let Some(inner) = weak.upgrade() {
                    TimeoutCounter { inner }.invoke_on_timer();
                }
            }),
        );
    }

    pub fn invoke_on_timer(&self) {
        let (progress, hash, timeouts) = {
            let mut state = self
                .inner
                .state
                .lock()
                .expect("timeout-counter mutex must not be poisoned");
            if state.complete || state.failed {
                return;
            }

            if !state.progress {
                state.timeouts += 1;
                let timeouts = state.timeouts;
                let hash = state.hash;
                (false, hash, timeouts)
            } else {
                state.progress = false;
                (true, state.hash, state.timeouts)
            }
        };

        if !progress {
            self.inner
                .journal
                .debug(&format!("Timeout({}) acquiring {}", timeouts, hash));
        }

        (self.inner.on_timer)(progress, self.clone());

        if !self.is_done() {
            self.set_timer();
        }
    }

    pub fn cancel(&self) {
        let mut state = self
            .inner
            .state
            .lock()
            .expect("timeout-counter mutex must not be poisoned");
        if state.complete || state.failed {
            return;
        }

        state.failed = true;
        self.inner.journal.info(&format!("Cancel {}", state.hash));
    }

    pub fn mark_complete(&self) {
        self.inner
            .state
            .lock()
            .expect("timeout-counter mutex must not be poisoned")
            .complete = true;
    }

    pub fn mark_failed(&self) {
        self.inner
            .state
            .lock()
            .expect("timeout-counter mutex must not be poisoned")
            .failed = true;
    }

    pub fn mark_progress(&self) {
        self.inner
            .state
            .lock()
            .expect("timeout-counter mutex must not be poisoned")
            .progress = true;
    }

    pub fn is_done(&self) -> bool {
        let state = self
            .inner
            .state
            .lock()
            .expect("timeout-counter mutex must not be poisoned");
        state.complete || state.failed
    }

    pub fn snapshot(&self) -> TimeoutCounterSnapshot {
        let state = self
            .inner
            .state
            .lock()
            .expect("timeout-counter mutex must not be poisoned");
        TimeoutCounterSnapshot {
            hash: state.hash,
            timeouts: state.timeouts,
            complete: state.complete,
            failed: state.failed,
            progress: state.progress,
            timer_interval: state.timer_interval,
            job_name: state.job.job_name.clone(),
            job_limit: state.job.job_limit,
        }
    }
}

fn run_set_timer(inner: Weak<TimeoutCounterInner>) {
    let Some(inner) = inner.upgrade() else {
        return;
    };
    TimeoutCounter { inner }.queue_job();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[derive(Default)]
    struct RecordingRuntime {
        timers: Mutex<VecDeque<TimeoutCounterJob>>,
        jobs: Mutex<VecDeque<TimeoutCounterJob>>,
        outstanding: AtomicU32,
    }

    impl RecordingRuntime {
        fn run_timer(&self) {
            let job = self
                .timers
                .lock()
                .expect("timers mutex")
                .pop_front()
                .expect("timer job should exist");
            job();
        }

        fn run_job(&self) {
            let job = self
                .jobs
                .lock()
                .expect("jobs mutex")
                .pop_front()
                .expect("queued job should exist");
            self.outstanding.fetch_add(1, Ordering::SeqCst);
            job();
            self.outstanding.fetch_sub(1, Ordering::SeqCst);
        }
    }

    impl TimeoutCounterRuntime for RecordingRuntime {
        fn schedule_after(&self, _delay: Duration, job: TimeoutCounterJob) {
            self.timers.lock().expect("timers mutex").push_back(job);
        }

        fn outstanding_jobs(&self, _job_name: &str) -> u32 {
            self.outstanding.load(Ordering::SeqCst)
        }

        fn enqueue_job(&self, _job_name: &str, job: TimeoutCounterJob) {
            self.jobs.lock().expect("jobs mutex").push_back(job);
        }
    }

    #[test]
    fn timeout_counter_increments_timeouts_without_progress() {
        let runtime = Arc::new(RecordingRuntime::default());
        let fired = Arc::new(AtomicU32::new(0));
        let fired_clone = Arc::clone(&fired);
        let counter = TimeoutCounter::new(
            runtime.clone(),
            Arc::new(NullTimeoutCounterJournal),
            Uint256::from_array([7; 32]),
            Duration::seconds(1),
            TimeoutCounterJobConfig::new("Acquire", Some(2)),
            move |progress, _counter| {
                assert!(!progress);
                fired_clone.fetch_add(1, Ordering::SeqCst);
            },
        );

        counter.set_timer();
        runtime.run_timer();
        runtime.run_job();

        let snapshot = counter.snapshot();
        assert_eq!(snapshot.timeouts, 1);
        assert_eq!(fired.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn timeout_counter_clears_progress_before_callback() {
        let runtime = Arc::new(RecordingRuntime::default());
        let saw_progress = Arc::new(AtomicU32::new(0));
        let saw_progress_clone = Arc::clone(&saw_progress);
        let counter = TimeoutCounter::new(
            runtime.clone(),
            Arc::new(NullTimeoutCounterJournal),
            Uint256::from_array([9; 32]),
            Duration::seconds(1),
            TimeoutCounterJobConfig::new("Acquire", None),
            move |progress, counter| {
                if progress && !counter.snapshot().progress {
                    saw_progress_clone.fetch_add(1, Ordering::SeqCst);
                }
            },
        );

        counter.mark_progress();
        counter.set_timer();
        runtime.run_timer();
        runtime.run_job();

        assert_eq!(counter.snapshot().timeouts, 0);
        assert_eq!(saw_progress.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn timeout_counter_defers_when_job_limit_is_reached() {
        let runtime = Arc::new(RecordingRuntime::default());
        runtime.outstanding.store(1, Ordering::SeqCst);
        let counter = TimeoutCounter::new(
            runtime.clone(),
            Arc::new(NullTimeoutCounterJournal),
            Uint256::from_array([5; 32]),
            Duration::seconds(1),
            TimeoutCounterJobConfig::new("Acquire", Some(1)),
            |_progress, _counter| {},
        );

        counter.queue_job();

        assert_eq!(runtime.jobs.lock().expect("jobs mutex").len(), 0);
        assert_eq!(runtime.timers.lock().expect("timers mutex").len(), 1);
    }
}
