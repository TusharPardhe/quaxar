use serde_json::{Map, Value};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use xrpl_core::{JobType, PerfLog, Semaphore, Workers, WorkersCallback};

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

impl WorkersCallback for RecordingCallback {
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
    fn job_queue(&self, _job_type: JobType) {}
    fn job_start(
        &self,
        _job_type: JobType,
        _queued_duration: Duration,
        _start_time: Instant,
        _instance: i32,
    ) {
    }
    fn job_finish(&self, _job_type: JobType, _running_duration: Duration, _instance: i32) {}
    fn counters_json(&self) -> Value {
        Value::Object(Map::new())
    }
    fn current_json(&self) -> Value {
        Value::Array(Vec::new())
    }
    fn resize_jobs(&self, resize: usize) {
        self.resizes
            .lock()
            .expect("perf log mutex poisoned")
            .push(resize as i32);
    }
}

#[test]
fn semaphore_matches_cxx_counting_shape() {
    let sem = Semaphore::new(0);
    assert!(!sem.try_wait());

    sem.notify();
    assert!(sem.try_wait());
    assert!(!sem.try_wait());

    sem.notify();
    sem.wait();
    assert!(!sem.try_wait());
}

#[test]
fn workers_pause_resume_and_stop_like_current_cxx_shape() {
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
fn workers_reuse_paused_threads_before_spawning_new_ones() {
    let callback = Arc::new(RecordingCallback::default());
    let workers = Workers::new(callback.clone(), None, "Worker", 1);

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
