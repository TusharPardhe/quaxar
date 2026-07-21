//! Acquisition job queue and delayed timer service.
//!
//! This mirrors rippled's split between `JobQueue` work and `TimeoutCounter`
//! timers: delayed callbacks only enqueue work; they never run acquisition
//! logic on the timer thread.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const LEDGER_DATA_JOB_LIMIT: usize = 5;

type Job = Box<dyn FnOnce() + Send>;

struct TimerTask {
    due: Instant,
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
                callback,
            });
        wake.notify_one();
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
    queue: Arc<(Mutex<VecDeque<Job>>, Condvar)>,
    stop: Arc<AtomicBool>,
    workers: Mutex<Vec<JoinHandle<()>>>,
    ledger_data_jobs: Arc<AtomicUsize>,
    timers: TimerService,
}

impl WorkerPool {
    pub fn new(size: usize) -> Self {
        let queue = Arc::new((Mutex::new(VecDeque::<Job>::new()), Condvar::new()));
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
                                jobs.pop_front().expect("non-empty acquisition queue")
                            };
                            job();
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
            timers: TimerService::new(),
        }
    }

    fn enqueue_reserved_ledger_data(&self, job: Job) {
        let count = Arc::clone(&self.ledger_data_jobs);
        let wrapped: Job = Box::new(move || {
            job();
            count.fetch_sub(1, Ordering::AcqRel);
        });
        let (lock, wake) = &*self.queue;
        lock.lock()
            .expect("acquisition queue lock")
            .push_back(wrapped);
        wake.notify_one();
    }

    fn enqueue_ledger_data(&self, job: Job) {
        self.ledger_data_jobs.fetch_add(1, Ordering::AcqRel);
        self.enqueue_reserved_ledger_data(job);
    }

    /// Queue a response-processing job. rippled queues this whenever
    /// `InboundLedger::gotData` transitions its dispatch flag to true.
    pub fn submit_ledger_data(&self, job: Job) {
        if !self.stop.load(Ordering::Acquire) {
            self.enqueue_ledger_data(job);
        }
    }

    /// Queue a TimeoutCounter job if the reference `JtLedgerData` limit permits.
    /// The caller re-arms its own timer when this returns false.
    pub fn try_submit_timeout(&self, job: Job) -> bool {
        if self.stop.load(Ordering::Acquire) {
            return false;
        }

        let mut queued = self.ledger_data_jobs.load(Ordering::Acquire);
        loop {
            if queued >= LEDGER_DATA_JOB_LIMIT {
                return false;
            }
            match self.ledger_data_jobs.compare_exchange_weak(
                queued,
                queued + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    self.enqueue_reserved_ledger_data(job);
                    return true;
                }
                Err(observed) => queued = observed,
            }
        }
    }

    /// Schedule one delayed timer callback. The callback must decide whether
    /// to re-arm itself, exactly as `TimeoutCounter::invokeOnTimer` does.
    pub fn schedule_after(&self, delay: Duration, callback: Job) {
        if !self.stop.load(Ordering::Acquire) {
            self.timers.schedule(delay, callback);
        }
    }

    pub fn stop(&self) {
        if self.stop.swap(true, Ordering::AcqRel) {
            return;
        }
        self.timers.stop();
        let (_, wake) = &*self.queue;
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

impl Drop for WorkerPool {
    fn drop(&mut self) {
        self.stop();
    }
}
