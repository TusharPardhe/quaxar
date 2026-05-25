//! Worker-pool substrate compatible with the current `xrpl/core/detail/Workers.h`.
//!
//! The reference implementation is a pause/resume thread pool built around a
//! semaphore and a paused-worker stack. This Rust version mirrors the
//! externally visible behavior:
//! - `add_task()` posts one task token,
//! - `set_number_of_threads()` grows by reusing paused workers before spawning
//!   new ones,
//! - shrinking posts "pause" tokens,
//! - `stop()` waits until all workers are paused and no callback is running.

use crate::perf_log::PerfLog;
use crate::semaphore::Semaphore;
use std::fmt;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};

static WORKER_INSTANCE_COUNTER: AtomicI32 = AtomicI32::new(0);

/// Called to perform tasks as needed.
pub trait Callback: Send + Sync {
    fn process_task(&self, instance: i32);
}

#[derive(Debug)]
struct WorkerControl {
    instance: i32,
    wake_state: Mutex<WakeState>,
    wakeup: Condvar,
}

#[derive(Debug, Default)]
struct WakeState {
    wake_count: usize,
    should_exit: bool,
}

impl WorkerControl {
    fn new(instance: i32) -> Self {
        Self {
            instance,
            wake_state: Mutex::new(WakeState::default()),
            wakeup: Condvar::new(),
        }
    }

    fn notify(&self) {
        let mut wake_state = self.wake_state.lock().expect("worker wake mutex poisoned");
        wake_state.wake_count += 1;
        self.wakeup.notify_one();
    }

    fn request_exit(&self) {
        let mut wake_state = self.wake_state.lock().expect("worker wake mutex poisoned");
        wake_state.should_exit = true;
        wake_state.wake_count += 1;
        self.wakeup.notify_one();
    }

    fn wait_for_wakeup(&self) -> bool {
        let mut wake_state = self.wake_state.lock().expect("worker wake mutex poisoned");
        while wake_state.wake_count == 0 {
            wake_state = self
                .wakeup
                .wait(wake_state)
                .expect("worker wake condvar poisoned");
        }

        wake_state.wake_count -= 1;
        wake_state.should_exit
    }
}

#[derive(Debug)]
struct WorkerHandle {
    control: Arc<WorkerControl>,
    thread: JoinHandle<()>,
}

struct WorkersInner {
    callback: Arc<dyn Callback>,
    perf_log: Option<Arc<dyn PerfLog>>,
    thread_names: String,
    semaphore: Semaphore,
    number_of_threads: AtomicI32,
    active_count: AtomicI32,
    pause_count: AtomicI32,
    running_task_count: AtomicI32,
    paused_workers: Mutex<Vec<Arc<WorkerControl>>>,
    worker_handles: Mutex<Vec<WorkerHandle>>,
    pause_mutex: Mutex<bool>,
    pause_cv: Condvar,
}

/// Thread-pool style worker manager.
pub struct Workers {
    inner: Arc<WorkersInner>,
}

impl fmt::Debug for Workers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Workers")
            .field("number_of_threads", &self.get_number_of_threads())
            .field(
                "number_of_currently_running_tasks",
                &self.number_of_currently_running_tasks(),
            )
            .finish()
    }
}

impl Workers {
    pub fn new(
        callback: Arc<dyn Callback>,
        perf_log: Option<Arc<dyn PerfLog>>,
        thread_names: impl Into<String>,
        number_of_threads: i32,
    ) -> Self {
        let workers = Self {
            inner: Arc::new(WorkersInner {
                callback,
                perf_log,
                thread_names: thread_names.into(),
                semaphore: Semaphore::default(),
                number_of_threads: AtomicI32::new(0),
                active_count: AtomicI32::new(0),
                pause_count: AtomicI32::new(0),
                running_task_count: AtomicI32::new(0),
                paused_workers: Mutex::new(Vec::new()),
                worker_handles: Mutex::new(Vec::new()),
                pause_mutex: Mutex::new(true),
                pause_cv: Condvar::new(),
            }),
        };

        workers.set_number_of_threads(number_of_threads);
        workers
    }

    /// This just returns the requested number of active threads.
    pub fn get_number_of_threads(&self) -> i32 {
        self.inner.number_of_threads.load(Ordering::SeqCst)
    }

    pub fn set_number_of_threads(&self, number_of_threads: i32) {
        let number_of_threads = number_of_threads.max(0);
        let current = self.get_number_of_threads();
        if current == number_of_threads {
            return;
        }

        if let Some(perf_log) = &self.inner.perf_log {
            perf_log.resize_jobs(number_of_threads as usize);
        }

        if number_of_threads > current {
            let amount = usize::try_from(number_of_threads - current)
                .expect("positive thread delta must fit usize");
            for _ in 0..amount {
                if let Some(worker) = self
                    .inner
                    .paused_workers
                    .lock()
                    .expect("paused worker stack poisoned")
                    .pop()
                {
                    worker.notify();
                } else {
                    self.spawn_worker();
                }
            }
        } else {
            let amount = usize::try_from(current - number_of_threads)
                .expect("positive thread delta must fit usize");
            for _ in 0..amount {
                self.inner.pause_count.fetch_add(1, Ordering::SeqCst);
                self.inner.semaphore.notify();
            }
        }

        self.inner
            .number_of_threads
            .store(number_of_threads, Ordering::SeqCst);
    }

    /// Pause all threads and wait until they are paused.
    pub fn stop(&self) {
        self.set_number_of_threads(0);

        let mut paused = self
            .inner
            .pause_mutex
            .lock()
            .expect("workers pause mutex poisoned");
        while !*paused || self.number_of_currently_running_tasks() != 0 {
            paused = self
                .inner
                .pause_cv
                .wait(paused)
                .expect("workers pause condvar poisoned");
        }
    }

    /// Add a task to be performed.
    pub fn add_task(&self) {
        self.inner.semaphore.notify();
    }

    pub fn number_of_currently_running_tasks(&self) -> i32 {
        self.inner.running_task_count.load(Ordering::SeqCst)
    }

    fn spawn_worker(&self) {
        let instance = WORKER_INSTANCE_COUNTER.fetch_add(1, Ordering::SeqCst);
        let control = Arc::new(WorkerControl::new(instance));
        let thread_name = self.inner.thread_names.clone();
        let inner = Arc::clone(&self.inner);
        let thread_control = Arc::clone(&control);

        let thread = thread::Builder::new()
            .name(thread_name.clone())
            .spawn(move || worker_main(inner, thread_control))
            .expect("worker thread must spawn");

        self.inner
            .worker_handles
            .lock()
            .expect("worker handle stack poisoned")
            .push(WorkerHandle { control, thread });
    }

    fn finish_running_task(&self) {
        if self.inner.running_task_count.fetch_sub(1, Ordering::SeqCst) == 1 {
            let _lock = self
                .inner
                .pause_mutex
                .lock()
                .expect("workers pause mutex poisoned");
            self.inner.pause_cv.notify_all();
        }
    }

    fn acquire_pause_request(&self) -> bool {
        loop {
            let current = self.inner.pause_count.load(Ordering::SeqCst);
            if current <= 0 {
                return false;
            }

            if self
                .inner
                .pause_count
                .compare_exchange(current, current - 1, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                return true;
            }
        }
    }
}

impl Drop for Workers {
    fn drop(&mut self) {
        if Arc::strong_count(&self.inner) != 1 {
            return;
        }

        self.stop();

        let handles = {
            let mut handles = self
                .inner
                .worker_handles
                .lock()
                .expect("worker handle stack poisoned");
            std::mem::take(&mut *handles)
        };

        for handle in &handles {
            handle.control.request_exit();
        }

        for handle in handles {
            handle.thread.join().expect("worker thread join failed");
        }
    }
}

fn worker_main(inner: Arc<WorkersInner>, control: Arc<WorkerControl>) {
    let workers = Workers { inner };
    loop {
        if workers.inner.active_count.fetch_add(1, Ordering::SeqCst) + 1 == 1 {
            let mut paused = workers
                .inner
                .pause_mutex
                .lock()
                .expect("workers pause mutex poisoned");
            *paused = false;
        }

        loop {
            workers.inner.semaphore.wait();

            if workers.acquire_pause_request() {
                break;
            }

            workers
                .inner
                .running_task_count
                .fetch_add(1, Ordering::SeqCst);
            workers.inner.callback.process_task(control.instance);
            workers.finish_running_task();
        }

        workers
            .inner
            .paused_workers
            .lock()
            .expect("paused worker stack poisoned")
            .push(Arc::clone(&control));

        if workers.inner.active_count.fetch_sub(1, Ordering::SeqCst) == 1 {
            let mut paused = workers
                .inner
                .pause_mutex
                .lock()
                .expect("workers pause mutex poisoned");
            *paused = true;
            workers.inner.pause_cv.notify_all();
        }

        if control.wait_for_wakeup() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Callback, Workers};
    use crate::perf_log::PerfLog;
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::Duration;

    #[derive(Default)]
    struct RecordingCallback {
        calls: Mutex<Vec<i32>>,
        cv: Condvar,
    }

    impl RecordingCallback {
        fn wait_for_len(&self, expected: usize) {
            let mut calls = self.calls.lock().expect("callback mutex poisoned");
            while calls.len() < expected {
                calls = self.cv.wait(calls).expect("callback condvar poisoned");
            }
        }

        fn snapshot(&self) -> Vec<i32> {
            self.calls.lock().expect("callback mutex poisoned").clone()
        }
    }

    impl Callback for RecordingCallback {
        fn process_task(&self, instance: i32) {
            let mut calls = self.calls.lock().expect("callback mutex poisoned");
            calls.push(instance);
            self.cv.notify_all();
        }
    }

    #[derive(Default)]
    struct RecordingPerfLog {
        resizes: Mutex<Vec<i32>>,
    }

    impl PerfLog for RecordingPerfLog {
        fn rpc_start(&self, _method: &str, _request_id: u64) {}
        fn rpc_finish(&self, _method: &str, _request_id: u64) {}
        fn rpc_error(&self, _method: &str, _request_id: u64) {}
        fn job_queue(&self, _job_type: crate::job::JobType) {}
        fn job_start(
            &self,
            _job_type: crate::job::JobType,
            _queued_duration: std::time::Duration,
            _start_time: std::time::Instant,
            _instance: i32,
        ) {
        }
        fn job_finish(
            &self,
            _job_type: crate::job::JobType,
            _running_duration: std::time::Duration,
            _instance: i32,
        ) {
        }
        fn counters_json(&self) -> serde_json::Value {
            serde_json::Value::Object(serde_json::Map::new())
        }
        fn current_json(&self) -> serde_json::Value {
            serde_json::Value::Array(Vec::new())
        }
        fn resize_jobs(&self, number_of_threads: usize) {
            self.resizes
                .lock()
                .expect("perf log mutex poisoned")
                .push(number_of_threads as i32);
        }
    }

    #[test]
    fn workers_match_pause_resume_stop_shape() {
        let callback = Arc::new(RecordingCallback::default());
        let perf_log = Arc::new(RecordingPerfLog::default());
        let workers = Workers::new(callback.clone(), Some(perf_log.clone()), "Worker", 2);

        assert_eq!(workers.get_number_of_threads(), 2);
        assert_eq!(
            perf_log
                .resizes
                .lock()
                .expect("perf log mutex poisoned")
                .as_slice(),
            &[2]
        );

        workers.add_task();
        workers.add_task();
        callback.wait_for_len(2);

        workers.stop();
        assert_eq!(workers.get_number_of_threads(), 0);
        assert_eq!(
            perf_log
                .resizes
                .lock()
                .expect("perf log mutex poisoned")
                .as_slice(),
            &[2, 0]
        );
        assert_eq!(workers.number_of_currently_running_tasks(), 0);

        workers.add_task();
        std::thread::sleep(Duration::from_millis(25));
        assert_eq!(callback.snapshot().len(), 2);

        workers.set_number_of_threads(1);
        assert_eq!(
            perf_log
                .resizes
                .lock()
                .expect("perf log mutex poisoned")
                .as_slice(),
            &[2, 0, 1]
        );
        callback.wait_for_len(3);
        assert_eq!(workers.number_of_currently_running_tasks(), 0);
    }

    #[test]
    fn workers_can_grow_after_pause_by_reusing_workers_or_spawning_new_ones() {
        let callback = Arc::new(RecordingCallback::default());
        let workers = Workers::new(callback.clone(), None, "Worker", 1);
        assert_eq!(workers.get_number_of_threads(), 1);

        workers.add_task();
        callback.wait_for_len(1);
        workers.set_number_of_threads(0);
        workers.set_number_of_threads(2);
        assert_eq!(workers.get_number_of_threads(), 2);

        workers.add_task();
        workers.add_task();
        callback.wait_for_len(3);
        assert_eq!(workers.number_of_currently_running_tasks(), 0);
    }
}
