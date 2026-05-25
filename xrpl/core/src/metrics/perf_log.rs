use crate::job::JobType;
use crate::job_types::JobTypes;
use basics::basic_config::{Section, get_if_exists, get_string};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fmt;
use std::fs::{File, OpenOptions, create_dir_all};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Condvar;
use std::sync::Mutex;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PerfLogSetup {
    pub perf_log: Option<std::path::PathBuf>,
    pub log_interval: Duration,
}

impl Default for PerfLogSetup {
    fn default() -> Self {
        Self {
            perf_log: None,
            log_interval: Duration::from_secs(1),
        }
    }
}

pub fn setup_perf_log(section: &Section, config_dir: &Path) -> PerfLogSetup {
    let mut setup = PerfLogSetup::default();

    let perf_log = get_string(section, "perf_log", "");
    if !perf_log.is_empty() {
        let mut path = PathBuf::from(perf_log);
        if path.is_relative() {
            path = config_dir.join(path);
        }
        setup.perf_log = Some(path);
    }

    let mut log_interval = 0_u64;
    if get_if_exists(section, "log_interval", &mut log_interval) {
        // The the reference implementation implementation applies this config value as whole
        // seconds even though the header comment still mentions milliseconds.
        setup.log_interval = Duration::from_secs(log_interval);
    }

    setup
}

pub trait PerfLog: Send + Sync {
    fn start(&self) {}
    fn stop(&self) {}
    fn rpc_start(&self, method: &str, request_id: u64);
    fn rpc_finish(&self, method: &str, request_id: u64);
    fn rpc_error(&self, method: &str, request_id: u64);
    fn job_queue(&self, job_type: JobType);
    fn job_start(
        &self,
        job_type: JobType,
        queued_duration: Duration,
        start_time: Instant,
        instance: i32,
    );
    fn job_finish(&self, job_type: JobType, running_duration: Duration, instance: i32);
    fn counters_json(&self) -> Value;
    fn current_json(&self) -> Value;
    fn resize_jobs(&self, resize: usize);
    fn rotate(&self) {}
}

pub trait PerfLogReportProvider: Send + Sync {
    fn extend_report(&self, _report: &mut Map<String, Value>) {}
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct NullPerfLogReportProvider;

impl PerfLogReportProvider for NullPerfLogReportProvider {}

pub fn measure_duration_and_log<R, F, L>(
    func: F,
    action_description: &str,
    max_delay: Duration,
    log: L,
) -> R
where
    F: FnOnce() -> R,
    L: FnOnce(&str, Duration),
{
    let start_time = Instant::now();
    let result = func();
    // threshold comparison and the reported value.
    let duration = Duration::from_millis(start_time.elapsed().as_millis() as u64);
    if duration > max_delay {
        log(action_description, duration);
    }
    result
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct RpcCounter {
    started: u64,
    finished: u64,
    errored: u64,
    duration_us: u128,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct JobCounter {
    queued: u64,
    started: u64,
    finished: u64,
    queued_duration_us: u128,
    running_duration_us: u128,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CurrentJob {
    job_type: JobType,
    start_time: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CurrentRpc {
    method: String,
    start_time: Instant,
}

#[derive(Debug, Default)]
struct PerfLogState {
    worker_count: usize,
    rpc_counters: BTreeMap<String, RpcCounter>,
    job_counters: BTreeMap<JobType, JobCounter>,
    current_jobs: BTreeMap<i32, CurrentJob>,
    current_rpcs: BTreeMap<u64, CurrentRpc>,
}

struct PerfLogRuntimeState {
    file: Option<File>,
    thread: Option<JoinHandle<()>>,
    stop: bool,
    rotate: bool,
}

#[derive(Default)]
pub struct PerfLogImp {
    state: Arc<Mutex<PerfLogState>>,
    runtime: Option<PerfLogRuntime>,
}

impl fmt::Debug for PerfLogImp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.state.lock().expect("perf log mutex poisoned");
        f.debug_struct("PerfLogImp")
            .field("worker_count", &state.worker_count)
            .field("rpc_methods", &state.rpc_counters.len())
            .field("job_types", &state.job_counters.len())
            .finish()
    }
}

struct PerfLogRuntime {
    setup: PerfLogSetup,
    hostid: String,
    signal_stop: Arc<dyn Fn() + Send + Sync>,
    report_provider: Arc<dyn PerfLogReportProvider>,
    state: Arc<(Mutex<PerfLogRuntimeState>, Condvar)>,
}

impl PerfLogImp {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_runtime(
        setup: PerfLogSetup,
        hostid: impl Into<String>,
        signal_stop: impl Fn() + Send + Sync + 'static,
        report_provider: Arc<dyn PerfLogReportProvider>,
    ) -> Self {
        let runtime = PerfLogRuntime {
            setup,
            hostid: hostid.into(),
            signal_stop: Arc::new(signal_stop),
            report_provider,
            state: Arc::new((
                Mutex::new(PerfLogRuntimeState {
                    file: None,
                    thread: None,
                    stop: false,
                    rotate: false,
                }),
                Condvar::new(),
            )),
        };
        let _ = open_runtime_file(&runtime);

        Self {
            state: Arc::new(Mutex::new(PerfLogState::default())),
            runtime: Some(runtime),
        }
    }
}

impl PerfLog for PerfLogImp {
    fn start(&self) {
        let Some(runtime) = &self.runtime else {
            return;
        };
        if runtime.setup.perf_log.is_none() {
            return;
        }

        let (lock, cond) = &*runtime.state;
        let runtime_state = lock.lock().expect("perf log runtime mutex poisoned");
        if runtime_state.thread.is_some() {
            return;
        }
        drop(runtime_state);

        let state = Arc::clone(&runtime.state);
        let perf_state = Arc::clone(&self.state);
        let setup = runtime.setup.clone();
        let hostid = runtime.hostid.clone();
        let signal_stop = Arc::clone(&runtime.signal_stop);
        let report_provider = Arc::clone(&runtime.report_provider);
        let handle = thread::Builder::new()
            .name("perflog".to_owned())
            .spawn(move || {
                run_perf_log_thread(
                    perf_state,
                    PerfLogRuntime {
                        setup,
                        hostid,
                        signal_stop,
                        report_provider,
                        state,
                    },
                );
            })
            .expect("perf log thread must spawn");

        let mut runtime_state = lock.lock().expect("perf log runtime mutex poisoned");
        runtime_state.thread = Some(handle);
        cond.notify_all();
    }

    fn stop(&self) {
        let Some(runtime) = &self.runtime else {
            return;
        };

        let (lock, cond) = &*runtime.state;
        let handle = {
            let mut runtime_state = lock.lock().expect("perf log runtime mutex poisoned");
            runtime_state.stop = true;
            cond.notify_all();
            runtime_state.thread.take()
        };

        if let Some(handle) = handle {
            let _ = handle.join();
        }
    }

    fn rpc_start(&self, method: &str, request_id: u64) {
        let mut state = self.state.lock().expect("perf log mutex poisoned");
        let counter = state.rpc_counters.entry(method.to_owned()).or_default();
        counter.started += 1;
        state.current_rpcs.insert(
            request_id,
            CurrentRpc {
                method: method.to_owned(),
                start_time: Instant::now(),
            },
        );
    }

    fn rpc_finish(&self, method: &str, request_id: u64) {
        let mut state = self.state.lock().expect("perf log mutex poisoned");
        let now = Instant::now();
        let duration_us = state
            .current_rpcs
            .remove(&request_id)
            .map(|current| now.duration_since(current.start_time).as_micros())
            .unwrap_or(0);
        let counter = state.rpc_counters.entry(method.to_owned()).or_default();
        counter.finished += 1;
        counter.duration_us += duration_us;
    }

    fn rpc_error(&self, method: &str, request_id: u64) {
        let mut state = self.state.lock().expect("perf log mutex poisoned");
        let now = Instant::now();
        let duration_us = state
            .current_rpcs
            .remove(&request_id)
            .map(|current| now.duration_since(current.start_time).as_micros())
            .unwrap_or(0);
        let counter = state.rpc_counters.entry(method.to_owned()).or_default();
        counter.errored += 1;
        counter.duration_us += duration_us;
    }

    fn job_queue(&self, job_type: JobType) {
        let mut state = self.state.lock().expect("perf log mutex poisoned");
        state.job_counters.entry(job_type).or_default().queued += 1;
    }

    fn job_start(
        &self,
        job_type: JobType,
        queued_duration: Duration,
        start_time: Instant,
        instance: i32,
    ) {
        let mut state = self.state.lock().expect("perf log mutex poisoned");
        let counter = state.job_counters.entry(job_type).or_default();
        counter.started += 1;
        counter.queued_duration_us += queued_duration.as_micros();
        if valid_worker_instance(&state, instance) {
            state.current_jobs.insert(
                instance,
                CurrentJob {
                    job_type,
                    start_time,
                },
            );
        }
    }

    fn job_finish(&self, job_type: JobType, running_duration: Duration, instance: i32) {
        let mut state = self.state.lock().expect("perf log mutex poisoned");
        let counter = state.job_counters.entry(job_type).or_default();
        counter.finished += 1;
        counter.running_duration_us += running_duration.as_micros();
        if valid_worker_instance(&state, instance) {
            state.current_jobs.remove(&instance);
        }
    }

    fn counters_json(&self) -> Value {
        let state = self.state.lock().expect("perf log mutex poisoned");
        counters_json_from_state(&state)
    }

    fn current_json(&self) -> Value {
        let state = self.state.lock().expect("perf log mutex poisoned");
        current_json_from_state(&state)
    }

    fn resize_jobs(&self, resize: usize) {
        let mut state = self.state.lock().expect("perf log mutex poisoned");
        state.worker_count = state.worker_count.max(resize);
    }

    fn rotate(&self) {
        let Some(runtime) = &self.runtime else {
            return;
        };
        if runtime.setup.perf_log.is_none() {
            return;
        }
        let (lock, cond) = &*runtime.state;
        let mut runtime_state = lock.lock().expect("perf log runtime mutex poisoned");
        runtime_state.rotate = true;
        cond.notify_all();
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct NullPerfLog;

impl PerfLog for NullPerfLog {
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
        let mut root = Map::new();
        root.insert("rpc".to_owned(), Value::Object(Map::new()));
        root.insert("job_queue".to_owned(), Value::Object(Map::new()));
        Value::Object(root)
    }
    fn current_json(&self) -> Value {
        let mut root = Map::new();
        root.insert("jobs".to_owned(), Value::Array(Vec::new()));
        root.insert("methods".to_owned(), Value::Array(Vec::new()));
        Value::Object(root)
    }
    fn resize_jobs(&self, _resize: usize) {}
}

fn now_micros_string() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_micros().to_string())
        .unwrap_or_else(|_| "0".to_owned())
}

fn valid_worker_instance(state: &PerfLogState, instance: i32) -> bool {
    instance >= 0 && (instance as usize) < state.worker_count
}

fn open_runtime_file(runtime: &PerfLogRuntime) -> Result<(), ()> {
    let Some(path) = &runtime.setup.perf_log else {
        return Ok(());
    };

    if let Some(parent) = path.parent()
        && create_dir_all(parent).is_err()
    {
        (runtime.signal_stop)();
        return Err(());
    }

    match OpenOptions::new().create(true).append(true).open(path) {
        Ok(file) => {
            let (lock, _) = &*runtime.state;
            lock.lock().expect("perf log runtime mutex poisoned").file = Some(file);
            Ok(())
        }
        Err(_) => {
            (runtime.signal_stop)();
            Err(())
        }
    }
}

fn run_perf_log_thread(perf_state: Arc<Mutex<PerfLogState>>, runtime: PerfLogRuntime) {
    let (lock, cond) = &*runtime.state;
    loop {
        let mut runtime_state = lock.lock().expect("perf log runtime mutex poisoned");
        let wait_result = cond
            .wait_timeout(runtime_state, runtime.setup.log_interval)
            .expect("perf log wait must not be poisoned");
        runtime_state = wait_result.0;

        if runtime_state.stop {
            runtime_state.file.take();
            return;
        }

        if runtime_state.rotate {
            runtime_state.rotate = false;
            runtime_state.file.take();
            drop(runtime_state);
            if open_runtime_file(&runtime).is_err() {
                return;
            }
            continue;
        }

        let report = serde_json::to_string(&report_json(&perf_state, &runtime))
            .expect("perf log report json must serialize");
        if let Some(file) = runtime_state.file.as_mut()
            && writeln!(file, "{report}")
                .and_then(|_| file.flush())
                .is_err()
        {
            (runtime.signal_stop)();
            runtime_state.file.take();
            return;
        }
    }
}

fn counters_json_from_state(state: &PerfLogState) -> Value {
    let mut root = Map::new();

    let mut rpc = Map::new();
    let mut rpc_total = RpcCounter::default();
    for (method, counter) in &state.rpc_counters {
        if counter.started == 0 && counter.finished == 0 && counter.errored == 0 {
            continue;
        }
        let mut entry = Map::new();
        entry.insert(
            "started".to_owned(),
            Value::String(counter.started.to_string()),
        );
        entry.insert(
            "finished".to_owned(),
            Value::String(counter.finished.to_string()),
        );
        entry.insert(
            "errored".to_owned(),
            Value::String(counter.errored.to_string()),
        );
        entry.insert(
            "duration_us".to_owned(),
            Value::String(counter.duration_us.to_string()),
        );
        rpc.insert(method.clone(), Value::Object(entry));
        rpc_total.started += counter.started;
        rpc_total.finished += counter.finished;
        rpc_total.errored += counter.errored;
        rpc_total.duration_us += counter.duration_us;
    }
    if rpc_total.started != 0 {
        let mut total = Map::new();
        total.insert(
            "started".to_owned(),
            Value::String(rpc_total.started.to_string()),
        );
        total.insert(
            "finished".to_owned(),
            Value::String(rpc_total.finished.to_string()),
        );
        total.insert(
            "errored".to_owned(),
            Value::String(rpc_total.errored.to_string()),
        );
        total.insert(
            "duration_us".to_owned(),
            Value::String(rpc_total.duration_us.to_string()),
        );
        rpc.insert("total".to_owned(), Value::Object(total));
    }
    root.insert("rpc".to_owned(), Value::Object(rpc));

    let mut jq = Map::new();
    let mut jq_total = JobCounter::default();
    for (job_type, counter) in &state.job_counters {
        if counter.queued == 0 && counter.started == 0 && counter.finished == 0 {
            continue;
        }
        let mut entry = Map::new();
        entry.insert(
            "queued".to_owned(),
            Value::String(counter.queued.to_string()),
        );
        entry.insert(
            "started".to_owned(),
            Value::String(counter.started.to_string()),
        );
        entry.insert(
            "finished".to_owned(),
            Value::String(counter.finished.to_string()),
        );
        entry.insert(
            "queued_duration_us".to_owned(),
            Value::String(counter.queued_duration_us.to_string()),
        );
        entry.insert(
            "running_duration_us".to_owned(),
            Value::String(counter.running_duration_us.to_string()),
        );
        jq.insert(JobTypes::name(*job_type).to_owned(), Value::Object(entry));
        jq_total.queued += counter.queued;
        jq_total.started += counter.started;
        jq_total.finished += counter.finished;
        jq_total.queued_duration_us += counter.queued_duration_us;
        jq_total.running_duration_us += counter.running_duration_us;
    }
    if jq_total.queued != 0 {
        let mut total = Map::new();
        total.insert(
            "queued".to_owned(),
            Value::String(jq_total.queued.to_string()),
        );
        total.insert(
            "started".to_owned(),
            Value::String(jq_total.started.to_string()),
        );
        total.insert(
            "finished".to_owned(),
            Value::String(jq_total.finished.to_string()),
        );
        total.insert(
            "queued_duration_us".to_owned(),
            Value::String(jq_total.queued_duration_us.to_string()),
        );
        total.insert(
            "running_duration_us".to_owned(),
            Value::String(jq_total.running_duration_us.to_string()),
        );
        jq.insert("total".to_owned(), Value::Object(total));
    }
    root.insert("job_queue".to_owned(), Value::Object(jq));

    Value::Object(root)
}

fn current_json_from_state(state: &PerfLogState) -> Value {
    let now = Instant::now();
    let mut jobs = Vec::new();
    let mut methods = Vec::new();

    for current in state.current_jobs.values() {
        let duration_us = now.duration_since(current.start_time).as_micros();
        let mut entry = Map::new();
        entry.insert(
            "job".to_owned(),
            Value::String(JobTypes::name(current.job_type).to_owned()),
        );
        entry.insert(
            "duration_us".to_owned(),
            Value::String(duration_us.to_string()),
        );
        jobs.push((
            duration_us,
            JobTypes::name(current.job_type).to_owned(),
            Value::Object(entry),
        ));
    }
    jobs.sort_by(|lhs, rhs| rhs.0.cmp(&lhs.0).then_with(|| lhs.1.cmp(&rhs.1)));

    for current in state.current_rpcs.values() {
        let duration_us = now.duration_since(current.start_time).as_micros();
        let mut entry = Map::new();
        entry.insert("method".to_owned(), Value::String(current.method.clone()));
        entry.insert(
            "duration_us".to_owned(),
            Value::String(duration_us.to_string()),
        );
        methods.push((duration_us, current.method.clone(), Value::Object(entry)));
    }
    methods.sort_by(|lhs, rhs| rhs.0.cmp(&lhs.0).then_with(|| lhs.1.cmp(&rhs.1)));

    let mut root = Map::new();
    root.insert(
        "jobs".to_owned(),
        Value::Array(jobs.into_iter().map(|(_, _, value)| value).collect()),
    );
    root.insert(
        "methods".to_owned(),
        Value::Array(methods.into_iter().map(|(_, _, value)| value).collect()),
    );
    Value::Object(root)
}

fn report_json(perf_state: &Arc<Mutex<PerfLogState>>, runtime: &PerfLogRuntime) -> Value {
    let state = perf_state.lock().expect("perf log mutex poisoned");
    let mut report = Map::new();
    report.insert("time".to_owned(), Value::String(now_micros_string()));
    report.insert("workers".to_owned(), Value::from(state.worker_count as u64));
    report.insert("hostid".to_owned(), Value::String(runtime.hostid.clone()));
    report.insert("counters".to_owned(), counters_json_from_state(&state));
    report.insert(
        "current_activities".to_owned(),
        current_json_from_state(&state),
    );
    runtime.report_provider.extend_report(&mut report);
    Value::Object(report)
}

#[cfg(test)]
mod tests {
    use super::{PerfLog, PerfLogImp, PerfLogRuntime, PerfLogRuntimeState, report_json};
    use crate::JobType;
    use std::sync::{Arc, Condvar, Mutex};
    use std::thread;
    use std::time::{Duration, Instant};

    #[test]
    fn perf_log_tracks_rpc_and_job_counters() {
        let log = PerfLogImp::new();
        log.resize_jobs(2);
        log.rpc_start("ping", 7);
        thread::sleep(Duration::from_millis(1));
        log.rpc_finish("ping", 7);

        let start = Instant::now();
        log.job_queue(JobType::Transaction);
        log.job_start(JobType::Transaction, Duration::from_millis(3), start, 0);
        thread::sleep(Duration::from_millis(1));
        log.job_finish(JobType::Transaction, Duration::from_millis(4), 0);

        let counters = log.counters_json();
        assert_eq!(counters["rpc"]["ping"]["started"], "1");
        assert_eq!(counters["rpc"]["ping"]["finished"], "1");
        assert_eq!(counters["rpc"]["total"]["started"], "1");
        assert_eq!(counters["rpc"]["total"]["finished"], "1");
        assert_eq!(counters["job_queue"]["transaction"]["queued"], "1");
        assert_eq!(counters["job_queue"]["transaction"]["started"], "1");
        assert_eq!(counters["job_queue"]["transaction"]["finished"], "1");
        assert_eq!(counters["job_queue"]["total"]["queued"], "1");
        assert_eq!(counters["job_queue"]["total"]["started"], "1");
        assert_eq!(counters["job_queue"]["total"]["finished"], "1");
    }

    #[test]
    fn perf_log_current_json_tracks_in_flight_work() {
        let log = PerfLogImp::new();
        log.resize_jobs(4);
        log.rpc_start("ping", 1);
        log.job_start(JobType::Client, Duration::ZERO, Instant::now(), 3);
        let current = log.current_json();
        assert_eq!(current["jobs"].as_array().expect("jobs array").len(), 1);
        assert_eq!(
            current["methods"].as_array().expect("methods array").len(),
            1
        );
        assert!(current["jobs"][0].get("instance").is_none());
    }

    #[test]
    fn perf_log_ignores_invalid_worker_ids_and_only_grows_worker_count() {
        let log = PerfLogImp::new();
        log.resize_jobs(1);
        log.job_start(JobType::Client, Duration::from_micros(7), Instant::now(), 2);
        log.job_start(
            JobType::Client,
            Duration::from_micros(11),
            Instant::now(),
            -1,
        );

        let current = log.current_json();
        assert_eq!(current["jobs"].as_array().expect("jobs array").len(), 0);

        log.job_finish(JobType::Client, Duration::from_micros(13), 2);
        log.job_finish(JobType::Client, Duration::from_micros(17), -1);

        let counters = log.counters_json();
        assert_eq!(counters["job_queue"]["clientCommand"]["started"], "2");
        assert_eq!(counters["job_queue"]["clientCommand"]["finished"], "2");
        assert_eq!(
            counters["job_queue"]["clientCommand"]["queued_duration_us"],
            "18"
        );
        assert_eq!(
            counters["job_queue"]["clientCommand"]["running_duration_us"],
            "30"
        );

        log.resize_jobs(3);
        log.resize_jobs(1);
        let report = log.current_json();
        assert_eq!(report["jobs"].as_array().expect("jobs array").len(), 0);
        assert_eq!(perf_log_report(&log)["workers"], 3);
    }

    #[test]
    fn runtime_start_and_rotate_are_noops_without_a_perf_log_path() {
        let log = PerfLogImp::with_runtime(
            Default::default(),
            "test-host",
            || {},
            Arc::new(super::NullPerfLogReportProvider),
        );

        log.start();
        log.rotate();

        let runtime = log.runtime.as_ref().expect("runtime");
        let runtime_state = runtime
            .state
            .0
            .lock()
            .expect("perf log runtime mutex poisoned");
        assert!(runtime_state.thread.is_none());
        assert!(!runtime_state.rotate);
        assert!(!runtime_state.stop);
    }

    fn perf_log_report(log: &PerfLogImp) -> serde_json::Value {
        let runtime = PerfLogRuntime {
            setup: Default::default(),
            hostid: "test-host".to_owned(),
            signal_stop: Arc::new(|| {}),
            report_provider: Arc::new(super::NullPerfLogReportProvider),
            state: Arc::new((
                Mutex::new(PerfLogRuntimeState {
                    file: None,
                    thread: None,
                    stop: false,
                    rotate: false,
                }),
                Condvar::new(),
            )),
        };
        report_json(&log.state, &runtime)
    }

    #[test]
    fn null_perf_log_uses_the_same_empty_report_shape_as_the_real_reporter() {
        let log = super::NullPerfLog;

        let counters = log.counters_json();
        assert!(counters.is_object());
        assert!(counters.get("rpc").expect("rpc entry").is_object());
        assert!(
            counters
                .get("job_queue")
                .expect("job_queue entry")
                .is_object()
        );

        let current = log.current_json();
        assert!(current.is_object());
        assert_eq!(current["jobs"].as_array().expect("jobs array").len(), 0);
        assert_eq!(
            current["methods"].as_array().expect("methods array").len(),
            0
        );
    }
}
