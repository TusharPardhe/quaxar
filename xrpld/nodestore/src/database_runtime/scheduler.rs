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
    fn on_fetch(&self, report: FetchReport);
    fn on_batch_write(&self, report: BatchWriteReport);
}

#[derive(Default)]
struct RealSchedulerQueue {
    tasks: VecDeque<Arc<dyn Task>>,
}

struct RealSchedulerInner {
    queue: Mutex<RealSchedulerQueue>,
    wakeup: Condvar,
    stopping: AtomicBool,
}

impl RealSchedulerInner {
    fn new() -> Self {
        Self {
            queue: Mutex::new(RealSchedulerQueue::default()),
            wakeup: Condvar::new(),
            stopping: AtomicBool::new(false),
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

        let inner = Arc::new(RealSchedulerInner::new());
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
        let mut queue = self
            .inner
            .queue
            .lock()
            .expect("nodestore scheduler queue mutex must not be poisoned");
        if self.inner.stopping.load(Ordering::Acquire) {
            return;
        }

        queue.tasks.push_back(task);
        self.inner.wakeup.notify_one();
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

        let _ = catch_unwind(AssertUnwindSafe(|| task.perform_scheduled_task()));
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
        let scheduler = RealScheduler::new(1);
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
