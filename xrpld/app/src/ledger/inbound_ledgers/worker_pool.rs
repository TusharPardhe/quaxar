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

use acquisition::{AcquisitionEvent, OperationRef, TimerKind};

use super::acquisition::AcquisitionState;
use super::coordinator_adapter::RetainedControlEvents;

type Job = Box<dyn FnOnce() + Send>;

/// A coordinator timer wakeup. The record is a fixed-size typed identity plus
/// the completion channel; the timer thread only posts the typed `TimerFired`
/// event and never runs session logic.
struct CoordinatorTimerRecord {
    operation: OperationRef,
    timer: TimerKind,
    completions: RetainedControlEvents,
}

/// Production timer work is a fixed record: it retains only the acquisition
/// identity through a weak Arc, or a coordinator operation identity plus the
/// completion channel. Test callbacks remain behind cfg(test) and never
/// participate in the production ownership boundary.
enum TimerWork {
    AcquisitionTimeout(std::sync::Weak<AcquisitionState>),
    Coordinator(CoordinatorTimerRecord),
    #[cfg(test)]
    Test(Job),
}

impl TimerWork {
    fn run(self) {
        match self {
            Self::AcquisitionTimeout(state) => {
                if let Some(state) = state.upgrade() {
                    state.on_acquisition_timer_fired();
                }
            }
            Self::Coordinator(record) => {
                record.completions.push(AcquisitionEvent::TimerFired {
                    operation: record.operation,
                    timer: record.timer,
                });
            }
            #[cfg(test)]
            Self::Test(callback) => callback(),
        }
    }
}

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
    bytes: Arc<AtomicUsize>,
    charged_bytes: usize,
}

impl Drop for LedgerDataReservation {
    fn drop(&mut self) {
        self.count.fetch_sub(1, Ordering::AcqRel);
        self.bytes.fetch_sub(self.charged_bytes, Ordering::AcqRel);
    }
}

fn reserve_ledger_data_job(job: Job, count: Arc<AtomicUsize>, bytes: Arc<AtomicUsize>) -> Job {
    // Measure the concrete FnOnce object while it is still typed behind the
    // trait object. The reservation, not the queue/container, owns release on
    // run, panic unwind, executor rejection, or WorkerPool stop/drop.
    let charged_bytes = std::mem::size_of_val(&*job).saturating_add(std::mem::size_of::<Job>());
    let reservation = LedgerDataReservation {
        count,
        bytes,
        charged_bytes,
    };
    Box::new(move || {
        let _reservation = reservation;
        job();
    })
}

/// Worker-pool state for acquisition diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkerPoolSnapshot {
    pub queued_jobs: usize,
    pub outstanding_ledger_data_jobs: usize,
    pub outstanding_ledger_data_job_bytes: usize,
    pub worker_count: usize,
}

struct TimerTask {
    due: Instant,
    #[cfg_attr(not(test), allow(dead_code))] // read by test introspection helpers
    delay: Duration,
    /// Exact operation identity for coordinator timers; `None` for legacy and
    /// test tasks. Disarm removes only tasks whose key matches, so a stale
    /// rearm can never be silently dropped while a newer arm of the same
    /// operation is pending.
    key: Option<OperationRef>,
    work: TimerWork,
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
                            break state.tasks.swap_remove(index).work;
                        }
                    };
                    callback.run();
                }
            })
            .expect("acquisition timer thread should spawn");
        Self {
            state,
            stopped,
            thread: Mutex::new(Some(thread)),
        }
    }

    #[cfg(test)]
    fn new_manual_for_test() -> Self {
        Self {
            state: Arc::new((Mutex::new(TimerState { tasks: Vec::new() }), Condvar::new())),
            stopped: Arc::new(AtomicBool::new(false)),
            thread: Mutex::new(None),
        }
    }

    fn schedule_acquisition_timeout(&self, delay: Duration, state: &Arc<AcquisitionState>) {
        self.schedule(delay, TimerWork::AcquisitionTimeout(Arc::downgrade(state)));
    }

    fn schedule_coordinator(&self, delay: Duration, record: CoordinatorTimerRecord) {
        let key = Some(record.operation);
        self.schedule_with_key(delay, key, TimerWork::Coordinator(record));
    }

    /// Remove every pending timer whose exact operation identity matches. A
    /// disarm race with the thread picking the task either removes it here or
    /// delivers the typed wakeup; the coordinator validates the operation and
    /// treats a late delivery for a cancelled/disarmed operation as stale.
    fn disarm(&self, operation: OperationRef) {
        let (lock, wake) = &*self.state;
        let mut state = lock.lock().expect("acquisition timer lock");
        state.tasks.retain(|task| task.key != Some(operation));
        drop(state);
        // The timer thread may be waiting for the removed earliest task. Wake
        // it so cancellation never leaves it sleeping on a stale deadline.
        wake.notify_one();
    }

    fn schedule(&self, delay: Duration, work: TimerWork) {
        self.schedule_with_key(delay, None, work);
    }

    fn schedule_with_key(&self, delay: Duration, key: Option<OperationRef>, work: TimerWork) {
        let (lock, wake) = &*self.state;
        let mut state = lock.lock().expect("acquisition timer lock");
        // Serialize stopped observation with queue insertion. A caller that
        // raced stop either inserts before stop (and is cleared after join) or
        // observes stopped while holding this same lock and retains nothing.
        if self.stopped.load(Ordering::Acquire) {
            return;
        }
        state.tasks.push(TimerTask {
            due: Instant::now() + delay,
            delay,
            key,
            work,
        });
        drop(state);
        wake.notify_one();
    }

    #[cfg(test)]
    fn schedule_for_test(&self, delay: Duration, callback: Job) {
        self.schedule(delay, TimerWork::Test(callback));
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
        (task.work).run();
        Some(delay)
    }

    fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        self.state.1.notify_all();
        if let Some(thread) = self.thread.lock().expect("timer thread lock").take() {
            let _ = thread.join();
        }
        // The timer thread exits without consuming delayed work. Drop every
        // queued record after join so callback/reservation ownership has no
        // stop-race survivor.
        self.state
            .0
            .lock()
            .expect("acquisition timer lock")
            .tasks
            .clear();
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
    ledger_data_job_bytes: Arc<AtomicUsize>,
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
            ledger_data_job_bytes: Arc::new(AtomicUsize::new(0)),
            timers: TimerService::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn new_with_manual_timer_for_test(size: usize) -> Self {
        let mut pool = Self::new(size);
        // `TimerService::new` always starts its thread. Replace it only in a
        // test-only constructor after stopping that isolated service, so a
        // deterministic timer test can drive queued callbacks explicitly.
        pool.timers.stop();
        pool.timers = TimerService::new_manual_for_test();
        pool
    }

    fn enqueue_ledger_data(&self, job: Job) {
        let (lock, wake) = &*self.queue;
        let mut jobs = lock.lock().expect("acquisition queue lock");
        if self.stop.load(Ordering::Acquire) {
            return;
        }

        self.ledger_data_jobs.fetch_add(1, Ordering::AcqRel);
        let charged_bytes = std::mem::size_of_val(&*job).saturating_add(std::mem::size_of::<Job>());
        self.ledger_data_job_bytes
            .fetch_add(charged_bytes, Ordering::AcqRel);
        jobs.ledger_data.push_back(reserve_ledger_data_job(
            job,
            Arc::clone(&self.ledger_data_jobs),
            Arc::clone(&self.ledger_data_job_bytes),
        ));
        wake.notify_all();
    }

    /// Submit an actor turn for which `AcquisitionReadyScheduler` already
    /// reserved one of the five JtLedgerData-equivalent slots.  WorkerPool is
    /// deliberately only an executor; it no longer admits acquisition
    /// identities or owns an unbounded normal acquisition FIFO.
    pub(crate) fn submit_reserved_turn(&self, job: Job) {
        if !self.stop.load(Ordering::Acquire) {
            self.enqueue_ledger_data(job);
        }
    }

    #[cfg(test)]
    pub fn submit_ledger_data(&self, job: Job) {
        self.submit_reserved_turn(job);
    }

    /// Schedule the fixed typed timeout record owned by an acquisition.
    pub(crate) fn schedule_acquisition_timeout(
        &self,
        delay: Duration,
        state: &Arc<AcquisitionState>,
    ) {
        if !self.stop.load(Ordering::Acquire) {
            self.timers.schedule_acquisition_timeout(delay, state);
        }
    }

    /// Schedule a coordinator timer wakeup. The timer thread posts a typed
    /// `TimerFired` completion carrying the exact arming operation; it never
    /// runs session logic.
    pub(crate) fn schedule_coordinator_timer(
        &self,
        operation: OperationRef,
        timer: TimerKind,
        delay: Duration,
        completions: RetainedControlEvents,
    ) {
        if !self.stop.load(Ordering::Acquire) {
            self.timers.schedule_coordinator(
                delay,
                CoordinatorTimerRecord {
                    operation,
                    timer,
                    completions,
                },
            );
        }
    }

    /// Disarm a pending coordinator timer by its exact operation identity.
    pub(crate) fn disarm_coordinator_timer(&self, operation: OperationRef) {
        self.timers.disarm(operation);
    }

    #[cfg(test)]
    pub(crate) fn schedule_after_for_test(&self, delay: Duration, callback: Job) {
        if !self.stop.load(Ordering::Acquire) {
            self.timers.schedule_for_test(delay, callback);
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
            outstanding_ledger_data_job_bytes: self.ledger_data_job_bytes.load(Ordering::Acquire),
            worker_count,
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
    fn timer_stop_drops_queued_work_and_refuses_a_racing_late_schedule() {
        let pool = WorkerPool::new(0);
        let dropped = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let retained = Arc::clone(&dropped);
        pool.schedule_after_for_test(
            Duration::from_secs(60),
            Box::new(move || {
                retained.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
            }),
        );
        assert_eq!(pool.scheduled_timer_delays_for_test().len(), 1);

        pool.stop();
        assert!(pool.scheduled_timer_delays_for_test().is_empty());
        pool.schedule_after_for_test(
            Duration::from_secs(60),
            Box::new(|| panic!("stopped timer must not retain or run work")),
        );
        assert!(pool.scheduled_timer_delays_for_test().is_empty());
        assert_eq!(dropped.load(std::sync::atomic::Ordering::Acquire), 0);
    }

    #[test]
    fn manual_timer_fires_callback_before_manually_drained_worker_job() {
        let pool = Arc::new(WorkerPool::new(0));
        let events = Arc::new(Mutex::new(Vec::new()));
        let callback_pool = Arc::clone(&pool);
        let callback_events = Arc::clone(&events);

        pool.schedule_after_for_test(
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
}

impl Drop for WorkerPool {
    fn drop(&mut self) {
        self.stop();
    }
}
