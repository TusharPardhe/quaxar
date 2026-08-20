use crate::Task;
use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchType {
    Synchronous,
    Async,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FetchReport {
    pub elapsed: Duration,
    pub fetch_type: FetchType,
    pub was_found: bool,
}

impl FetchReport {
    pub fn new(fetch_type: FetchType) -> Self {
        Self {
            elapsed: Duration::default(),
            fetch_type,
            was_found: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BatchWriteReport {
    pub elapsed: Duration,
    pub write_count: i32,
}

pub trait Scheduler: Send + Sync + 'static {
    fn schedule_task(&self, task: Arc<dyn Task>);

    /// Bounded schedulers return false without retaining `task`; dropping that
    /// task is the caller's explicit cancellation handoff. Existing adapters
    /// retain the legacy method and conservatively report accepted work.
    fn try_schedule_task(&self, task: Arc<dyn Task>) -> bool {
        self.schedule_task(task);
        true
    }

    fn on_fetch(&self, report: FetchReport);
    fn on_batch_write(&self, report: BatchWriteReport);
}

struct QueuedTask {
    task: Arc<dyn Task>,
    bytes: usize,
}

#[derive(Default)]
struct RealSchedulerQueue {
    tasks: VecDeque<QueuedTask>,
    retained_bytes: usize,
}

struct RealSchedulerInner {
    queue: Mutex<RealSchedulerQueue>,
    wakeup: Condvar,
    stopping: AtomicBool,
    max_scheduled_tasks: usize,
    max_scheduled_bytes: usize,
    rejected_tasks: std::sync::atomic::AtomicUsize,
    high_water_tasks: std::sync::atomic::AtomicUsize,
}

impl RealSchedulerInner {
    fn new(max_scheduled_tasks: usize) -> Self {
        assert!(
            max_scheduled_tasks > 0,
            "xrpl::NodeStore::RealScheduler queue capacity must be nonzero"
        );
        let max_scheduled_bytes = max_scheduled_tasks
            .checked_mul(512)
            .expect("NodeStore scheduler byte capacity overflow");
        Self {
            queue: Mutex::new(RealSchedulerQueue::default()),
            wakeup: Condvar::new(),
            stopping: AtomicBool::new(false),
            max_scheduled_tasks,
            max_scheduled_bytes,
            rejected_tasks: std::sync::atomic::AtomicUsize::new(0),
            high_water_tasks: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

pub struct RealScheduler {
    inner: Arc<RealSchedulerInner>,
    workers: Mutex<Vec<JoinHandle<()>>>,
}

impl RealScheduler {
    pub fn new(worker_count: usize) -> Self {
        assert!(
            worker_count > 0,
            "xrpl::NodeStore::RealScheduler::new requires at least one worker"
        );

        // This queue budget is a bounded Rust adaptation derived from the
        // constructor's worker-count resource contract: each worker may have
        // one additional task retained while it is executing another. It is
        // not a rippled numeric-parity value.
        let inner = Arc::new(RealSchedulerInner::new(worker_count));
        let mut workers = Vec::with_capacity(worker_count);
        for index in 0..worker_count {
            let worker_inner = Arc::clone(&inner);
            let handle = thread::Builder::new()
                .name(format!("nodestore scheduler #{}", index + 1))
                .spawn(move || worker_loop(worker_inner))
                .expect("failed to spawn nodestore scheduler worker");
            workers.push(handle);
        }

        Self {
            inner,
            workers: Mutex::new(workers),
        }
    }

    /// Test and embedding constructor for an explicit finite queue budget.
    pub fn with_task_limit(worker_count: usize, max_scheduled_tasks: usize) -> Self {
        assert!(
            worker_count > 0,
            "xrpl::NodeStore::RealScheduler::with_task_limit requires at least one worker"
        );
        let inner = Arc::new(RealSchedulerInner::new(max_scheduled_tasks));
        let mut workers = Vec::with_capacity(worker_count);
        for index in 0..worker_count {
            let worker_inner = Arc::clone(&inner);
            let handle = thread::Builder::new()
                .name(format!("nodestore scheduler #{}", index + 1))
                .spawn(move || worker_loop(worker_inner))
                .expect("failed to spawn nodestore scheduler worker");
            workers.push(handle);
        }
        Self {
            inner,
            workers: Mutex::new(workers),
        }
    }

    pub fn pending_task_count(&self) -> usize {
        self.inner
            .queue
            .lock()
            .expect("nodestore scheduler queue mutex must not be poisoned")
            .tasks
            .len()
    }

    pub fn task_metrics(&self) -> (usize, usize) {
        (
            self.inner.rejected_tasks.load(Ordering::Relaxed),
            self.inner.high_water_tasks.load(Ordering::Relaxed),
        )
    }

    pub fn with_available_parallelism() -> Self {
        let worker_count = thread::available_parallelism()
            .map(|parallelism| parallelism.get())
            .unwrap_or(1);
        Self::new(worker_count)
    }
}

#[derive(Debug, Default)]
pub struct DummyScheduler;

impl Scheduler for DummyScheduler {
    fn schedule_task(&self, task: Arc<dyn Task>) {
        task.perform_scheduled_task();
    }

    fn on_fetch(&self, _report: FetchReport) {}

    fn on_batch_write(&self, _report: BatchWriteReport) {}
}

impl Scheduler for RealScheduler {
    fn schedule_task(&self, task: Arc<dyn Task>) {
        let _ = self.try_schedule_task(task);
    }

    fn try_schedule_task(&self, task: Arc<dyn Task>) -> bool {
        let mut queue = self
            .inner
            .queue
            .lock()
            .expect("nodestore scheduler queue mutex must not be poisoned");
        let task_bytes = std::mem::size_of_val(task.as_ref()).saturating_add(task.retained_bytes());
        if self.inner.stopping.load(Ordering::Acquire)
            || queue.tasks.len() >= self.inner.max_scheduled_tasks
            || queue
                .retained_bytes
                .checked_add(task_bytes)
                .is_none_or(|total| total > self.inner.max_scheduled_bytes)
        {
            self.inner.rejected_tasks.fetch_add(1, Ordering::Relaxed);
            return false;
        }

        queue.retained_bytes += task_bytes;
        queue.tasks.push_back(QueuedTask {
            task,
            bytes: task_bytes,
        });
        self.inner
            .high_water_tasks
            .fetch_max(queue.tasks.len(), Ordering::Relaxed);
        self.inner.wakeup.notify_one();
        true
    }

    fn on_fetch(&self, _report: FetchReport) {}

    fn on_batch_write(&self, _report: BatchWriteReport) {}
}

impl Drop for RealScheduler {
    fn drop(&mut self) {
        self.inner.stopping.store(true, Ordering::Release);
        self.inner.wakeup.notify_all();

        let mut workers = self
            .workers
            .lock()
            .expect("nodestore scheduler workers mutex must not be poisoned");
        for handle in workers.drain(..) {
            let _ = handle.join();
        }
    }
}

fn worker_loop(inner: Arc<RealSchedulerInner>) {
    loop {
        let task = {
            let mut queue = inner
                .queue
                .lock()
                .expect("nodestore scheduler queue mutex must not be poisoned");

            loop {
                if let Some(task) = queue.tasks.pop_front() {
                    queue.retained_bytes = queue.retained_bytes.saturating_sub(task.bytes);
                    break Some(task);
                }

                if inner.stopping.load(Ordering::Acquire) {
                    break None;
                }

                queue = inner
                    .wakeup
                    .wait(queue)
                    .expect("nodestore scheduler queue mutex must not be poisoned");
            }
        };

        let Some(task) = task else {
            return;
        };

        let _ = catch_unwind(AssertUnwindSafe(|| task.task.perform_scheduled_task()));
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BatchWriteReport, DummyScheduler, FetchReport, FetchType, RealScheduler, Scheduler,
    };
    use crate::Task;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;

    struct ThreadRecordingTask {
        tx: mpsc::Sender<std::thread::ThreadId>,
    }

    impl Task for ThreadRecordingTask {
        fn perform_scheduled_task(&self) {
            self.tx
                .send(std::thread::current().id())
                .expect("worker thread id should send");
        }
    }

    struct CountingTask {
        counter: Arc<AtomicUsize>,
        should_panic: bool,
    }

    struct GateTask {
        entered: mpsc::Sender<()>,
        release: std::sync::Mutex<mpsc::Receiver<()>>,
    }

    impl Task for GateTask {
        fn perform_scheduled_task(&self) {
            self.entered.send(()).expect("gate task should start");
            self.release
                .lock()
                .expect("gate release mutex")
                .recv()
                .expect("gate task should be released");
        }
    }

    impl Task for CountingTask {
        fn perform_scheduled_task(&self) {
            if self.should_panic {
                panic!("scheduler worker task panic");
            }

            self.counter.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn dummy_scheduler_runs_tasks_inline_sync_scheduler() {
        let scheduler = DummyScheduler;
        let ran = Arc::new(AtomicBool::new(false));
        let task = InlineTask {
            ran: Arc::clone(&ran),
        };

        scheduler.schedule_task(Arc::new(task));

        assert!(ran.load(Ordering::Relaxed));
    }

    struct InlineTask {
        ran: Arc<AtomicBool>,
    }

    impl Task for InlineTask {
        fn perform_scheduled_task(&self) {
            self.ran.store(true, Ordering::Relaxed);
        }
    }

    #[test]
    fn real_scheduler_executes_tasks_on_background_workers() {
        let scheduler = RealScheduler::new(1);
        let (tx, rx) = mpsc::channel();
        let caller = std::thread::current().id();

        scheduler.schedule_task(Arc::new(ThreadRecordingTask { tx }));

        let worker = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("scheduled task should complete");
        assert_ne!(worker, caller);
    }

    #[test]
    fn real_scheduler_survives_panicking_tasks_and_keeps_running() {
        let scheduler = RealScheduler::with_task_limit(1, 2);
        let counter = Arc::new(AtomicUsize::new(0));

        scheduler.schedule_task(Arc::new(CountingTask {
            counter: Arc::clone(&counter),
            should_panic: true,
        }));
        scheduler.schedule_task(Arc::new(CountingTask {
            counter: Arc::clone(&counter),
            should_panic: false,
        }));

        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while counter.load(Ordering::Relaxed) == 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }

        assert_eq!(counter.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn real_scheduler_default_worker_bound_accepts_one_queued_task_then_rejects() {
        let scheduler = RealScheduler::new(1);
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let counter = Arc::new(AtomicUsize::new(0));

        assert!(scheduler.try_schedule_task(Arc::new(GateTask {
            entered: entered_tx,
            release: std::sync::Mutex::new(release_rx),
        })));
        entered_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("worker must hold the active task");

        assert!(scheduler.try_schedule_task(Arc::new(CountingTask {
            counter: Arc::clone(&counter),
            should_panic: false,
        })));
        assert!(
            !scheduler.try_schedule_task(Arc::new(CountingTask {
                counter: Arc::clone(&counter),
                should_panic: false,
            })),
            "with one active worker, the one-worker derived queue admits exactly one waiting task"
        );
        assert_eq!(scheduler.pending_task_count(), 1);
        assert_eq!(scheduler.task_metrics(), (1, 1));

        release_tx.send(()).expect("release active task");
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while counter.load(Ordering::Acquire) == 0 && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert_eq!(counter.load(Ordering::Acquire), 1);
    }

    #[test]
    fn report_types_keep_cpp_facing_defaults() {
        let fetch = FetchReport::new(FetchType::Async);
        assert_eq!(fetch.elapsed, Duration::default());
        assert_eq!(fetch.fetch_type, FetchType::Async);
        assert!(!fetch.was_found);

        let batch = BatchWriteReport::default();
        assert_eq!(batch.elapsed, Duration::default());
        assert_eq!(batch.write_count, 0);
    }
}
