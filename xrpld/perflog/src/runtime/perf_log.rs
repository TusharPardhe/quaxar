use crate::job_types::{JobType, JobTypeInfo, JobTypes};
use crate::journal::{JournalLevel, PerfLogJournal};
use crate::setup::PerfLogSetup;
use basics::chrono::to_string as xrpl_to_string;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime};
use time::{Duration as TimeDuration, OffsetDateTime};

pub trait PerfLogReportSource: Send + Sync + 'static {
    fn node_store_counts_json(&self) -> Value;

    fn state_accounting(&self, report: &mut Value);
}

pub struct NullReportSource;

impl PerfLogReportSource for NullReportSource {
    fn node_store_counts_json(&self) -> Value {
        Value::Object(Map::new())
    }

    fn state_accounting(&self, _report: &mut Value) {}
}

pub trait PerfLog: Send + Sync {
    fn start(&self);
    fn stop(&self);
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
    fn rotate(&self);
}

#[derive(Clone)]
pub struct PerfLogImp {
    inner: Arc<PerfLogInner>,
}

impl PerfLogImp {
    pub fn new(
        setup: PerfLogSetup,
        rpc_methods: Vec<String>,
        report_source: Arc<dyn PerfLogReportSource>,
        journal: Arc<dyn PerfLogJournal>,
        signal_stop: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        Self::new_with_hostname(
            setup,
            rpc_methods,
            report_source,
            journal,
            signal_stop,
            default_hostname(),
        )
    }

    pub fn new_with_hostname(
        setup: PerfLogSetup,
        mut rpc_methods: Vec<String>,
        report_source: Arc<dyn PerfLogReportSource>,
        journal: Arc<dyn PerfLogJournal>,
        signal_stop: Arc<dyn Fn() + Send + Sync>,
        hostname: impl Into<String>,
    ) -> Self {
        rpc_methods.sort_unstable();
        rpc_methods.dedup();

        let inner = Arc::new(PerfLogInner {
            setup,
            counters: Counters::new(rpc_methods, JobTypes::instance().all()),
            report_source,
            journal,
            signal_stop,
            hostname: hostname.into(),
            file_state: Mutex::new(FileState::default()),
            control: Mutex::new(ControlState::new()),
            condvar: Condvar::new(),
            thread: Mutex::new(None),
        });

        let log = Self { inner };
        log.inner.open_log();
        log
    }

    pub fn report_once(&self) {
        if self.inner.current_log_file_is_missing() {
            return;
        }

        {
            let mut control = self
                .inner
                .control
                .lock()
                .expect("perflog control mutex poisoned");
            control.last_report = Instant::now();
        }

        self.inner.write_snapshot(self.snapshot_report());
    }

    pub fn snapshot_report(&self) -> Value {
        self.inner.snapshot_report()
    }
}

impl PerfLog for PerfLogImp {
    fn start(&self) {
        if self.inner.setup.perf_log.as_os_str().is_empty() {
            return;
        }

        let already_running = self
            .inner
            .thread
            .lock()
            .expect("perflog thread mutex poisoned")
            .is_some();
        assert!(
            !already_running,
            "perflog start called while thread already running"
        );

        let inner = Arc::clone(&self.inner);
        let handle = thread::Builder::new()
            .name("perflog".to_owned())
            .spawn(move || inner.run())
            .expect("perflog thread should spawn");
        *self
            .inner
            .thread
            .lock()
            .expect("perflog thread mutex poisoned") = Some(handle);
    }

    fn stop(&self) {
        let handle = self
            .inner
            .thread
            .lock()
            .expect("perflog thread mutex poisoned")
            .take();
        if let Some(handle) = handle {
            {
                let mut control = self
                    .inner
                    .control
                    .lock()
                    .expect("perflog control mutex poisoned");
                control.stop = true;
                self.inner.condvar.notify_all();
            }
            let _ = handle.join();
        }
    }

    fn rpc_start(&self, method: &str, request_id: u64) {
        tracing::debug!(target: "perflog", method, request_id, "RPC started");
        self.inner.counters.rpc_start(method, request_id);
    }

    fn rpc_finish(&self, method: &str, request_id: u64) {
        let duration_us = self.inner.counters.rpc_duration_us(method, request_id);
        tracing::debug!(target: "perflog", method, request_id, duration_us, "RPC finished");
        self.inner.counters.rpc_end(method, request_id, true);
    }

    fn rpc_error(&self, method: &str, request_id: u64) {
        self.inner.counters.rpc_end(method, request_id, false);
    }

    fn job_queue(&self, job_type: JobType) {
        self.inner.counters.job_queue(job_type);
    }

    fn job_start(
        &self,
        job_type: JobType,
        queued_duration: Duration,
        start_time: Instant,
        instance: i32,
    ) {
        let job_type_name = JobTypes::name(job_type);
        let queue_time_us = queued_duration.as_micros() as u64;
        tracing::debug!(target: "perflog", job_type = job_type_name, queue_time_us, "Job started");
        self.inner
            .counters
            .job_start(job_type, queued_duration, start_time, instance);
    }

    fn job_finish(&self, job_type: JobType, running_duration: Duration, instance: i32) {
        let job_type_name = JobTypes::name(job_type);
        let run_time_us = running_duration.as_micros() as u64;
        tracing::debug!(target: "perflog", job_type = job_type_name, run_time_us, "Job finished");
        self.inner
            .counters
            .job_finish(job_type, running_duration, instance);
    }

    fn counters_json(&self) -> Value {
        self.inner.counters.counters_json()
    }

    fn current_json(&self) -> Value {
        self.inner.counters.current_json()
    }

    fn resize_jobs(&self, resize: usize) {
        self.inner.counters.resize_jobs(resize);
    }

    fn rotate(&self) {
        if self.inner.setup.perf_log.as_os_str().is_empty() {
            return;
        }

        let mut control = self
            .inner
            .control
            .lock()
            .expect("perflog control mutex poisoned");
        control.rotate = true;
        self.inner.condvar.notify_one();
    }
}

impl Drop for PerfLogImp {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn make_perf_log(
    setup: PerfLogSetup,
    rpc_methods: Vec<String>,
    report_source: Arc<dyn PerfLogReportSource>,
    journal: Arc<dyn PerfLogJournal>,
    signal_stop: Arc<dyn Fn() + Send + Sync>,
) -> Box<dyn PerfLog> {
    Box::new(PerfLogImp::new(
        setup,
        rpc_methods,
        report_source,
        journal,
        signal_stop,
    ))
}

pub fn measure_duration_and_log<Func, Output>(
    func: Func,
    action_description: &str,
    max_delay: Duration,
    journal: &dyn PerfLogJournal,
) -> Output
where
    Func: FnOnce() -> Output,
{
    let start_time = Instant::now();
    let result = func();
    let duration = start_time.elapsed();
    if duration > max_delay {
        journal.log(
            JournalLevel::Warn,
            &format!("{} took {} ms", action_description, duration.as_millis()),
        );
    }
    result
}

struct PerfLogInner {
    setup: PerfLogSetup,
    counters: Counters,
    report_source: Arc<dyn PerfLogReportSource>,
    journal: Arc<dyn PerfLogJournal>,
    signal_stop: Arc<dyn Fn() + Send + Sync>,
    hostname: String,
    file_state: Mutex<FileState>,
    control: Mutex<ControlState>,
    condvar: Condvar,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl PerfLogInner {
    fn current_log_file_is_missing(&self) -> bool {
        let file_state = self.file_state.lock().expect("perflog file mutex poisoned");
        file_state.file.is_none()
    }

    fn snapshot_report(&self) -> Value {
        let now = SystemTime::now();
        let mut report = Map::new();
        report.insert("time".to_owned(), Value::String(format_system_time(now)));
        report.insert(
            "workers".to_owned(),
            Value::Number(self.counters.worker_count().into()),
        );
        report.insert("hostid".to_owned(), Value::String(self.hostname.clone()));
        report.insert("counters".to_owned(), self.counters.counters_json());
        report.insert(
            "nodestore".to_owned(),
            self.report_source.node_store_counts_json(),
        );
        report.insert(
            "current_activities".to_owned(),
            self.counters.current_json(),
        );

        let mut report = Value::Object(report);
        self.report_source.state_accounting(&mut report);
        report
    }

    fn open_log(&self) {
        if self.setup.perf_log.as_os_str().is_empty() {
            return;
        }

        let path = self.setup.perf_log.clone();
        {
            let mut file_state = self.file_state.lock().expect("perflog file mutex poisoned");
            let _ = file_state.file.take();
        }

        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            && let Err(error) = fs::create_dir_all(parent)
        {
            self.journal.log(
                JournalLevel::Fatal,
                &format!(
                    "Unable to create performance log directory {}: {}",
                    parent.display(),
                    error
                ),
            );
            (self.signal_stop.as_ref())();
            return;
        }

        match OpenOptions::new().create(true).append(true).open(&path) {
            Ok(file) => {
                let mut file_state = self.file_state.lock().expect("perflog file mutex poisoned");
                file_state.file = Some(file);
            }
            Err(error) => {
                self.journal.log(
                    JournalLevel::Fatal,
                    &format!(
                        "Unable to open performance log {}: {}",
                        path.display(),
                        error
                    ),
                );
                (self.signal_stop.as_ref())();
            }
        }
    }

    fn run(self: Arc<Self>) {
        loop {
            let mut control = self.control.lock().expect("perflog control mutex poisoned");
            if control.stop {
                return;
            }

            let wait_result = self
                .condvar
                .wait_timeout(control, self.setup.log_interval)
                .expect("perflog condvar wait should not poison");
            control = wait_result.0;
            if control.stop {
                return;
            }

            let rotate = control.rotate;
            control.rotate = false;
            drop(control);

            if rotate {
                self.open_log();
            }

            self.report_periodic();
        }
    }

    fn report_periodic(&self) {
        let now = Instant::now();
        {
            let mut control = self.control.lock().expect("perflog control mutex poisoned");
            if now < control.last_report + self.setup.log_interval {
                return;
            }
            control.last_report = now;
        }

        tracing::info!(target: "perflog", "Performance log snapshot taken");
        self.write_snapshot(self.snapshot_report());
    }

    fn write_snapshot(&self, report: Value) {
        let mut file_state = self.file_state.lock().expect("perflog file mutex poisoned");
        let Some(file) = file_state.file.as_mut() else {
            return;
        };

        let _ = serde_json::to_writer(&mut *file, &report);
        let _ = file.write_all(b"\n");
        let _ = file.flush();
    }
}

#[derive(Default)]
struct FileState {
    file: Option<File>,
}

struct ControlState {
    stop: bool,
    rotate: bool,
    last_report: Instant,
}

impl ControlState {
    fn new() -> Self {
        Self {
            stop: false,
            rotate: false,
            last_report: Instant::now(),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct RpcCounter {
    started: u64,
    finished: u64,
    errored: u64,
    duration: Duration,
}

#[derive(Debug, Clone, Default)]
struct JqCounter {
    queued: u64,
    started: u64,
    finished: u64,
    queued_duration: Duration,
    running_duration: Duration,
}

struct Counters {
    rpc: HashMap<String, Mutex<RpcCounter>>,
    jq: HashMap<JobType, Mutex<JqCounter>>,
    jobs: Mutex<Vec<(JobType, Instant)>>,
    methods: Mutex<HashMap<u64, (String, Instant)>>,
}

impl Counters {
    fn new(rpc_methods: Vec<String>, job_types: &'static [JobTypeInfo]) -> Self {
        let mut rpc = HashMap::with_capacity(rpc_methods.len());
        for method in rpc_methods {
            rpc.insert(method, Mutex::new(RpcCounter::default()));
        }

        let mut jq = HashMap::with_capacity(job_types.len());
        for job_type in job_types.iter().map(JobTypeInfo::job_type) {
            jq.insert(job_type, Mutex::new(JqCounter::default()));
        }

        Self {
            rpc,
            jq,
            jobs: Mutex::new(Vec::new()),
            methods: Mutex::new(HashMap::new()),
        }
    }

    fn counters_json(&self) -> Value {
        let mut rpc_object = Map::new();
        let mut total_rpc = RpcCounter::default();

        for (method, counter) in &self.rpc {
            let value = counter.lock().expect("perflog rpc counter mutex poisoned");
            if value.started == 0 && value.finished == 0 && value.errored == 0 {
                continue;
            }

            let mut object = Map::new();
            object.insert(
                "started".to_owned(),
                Value::String(value.started.to_string()),
            );
            object.insert(
                "finished".to_owned(),
                Value::String(value.finished.to_string()),
            );
            object.insert(
                "errored".to_owned(),
                Value::String(value.errored.to_string()),
            );
            object.insert(
                "duration_us".to_owned(),
                Value::String(value.duration.as_micros().to_string()),
            );
            rpc_object.insert(method.clone(), Value::Object(object));

            total_rpc.started += value.started;
            total_rpc.finished += value.finished;
            total_rpc.errored += value.errored;
            total_rpc.duration += value.duration;
        }

        if total_rpc.started != 0 {
            let mut object = Map::new();
            object.insert(
                "started".to_owned(),
                Value::String(total_rpc.started.to_string()),
            );
            object.insert(
                "finished".to_owned(),
                Value::String(total_rpc.finished.to_string()),
            );
            object.insert(
                "errored".to_owned(),
                Value::String(total_rpc.errored.to_string()),
            );
            object.insert(
                "duration_us".to_owned(),
                Value::String(total_rpc.duration.as_micros().to_string()),
            );
            rpc_object.insert("total".to_owned(), Value::Object(object));
        }

        let mut job_queue_object = Map::new();
        let mut total_jq = JqCounter::default();

        for (job_type, counter) in &self.jq {
            let value = counter.lock().expect("perflog job counter mutex poisoned");
            if value.queued == 0 && value.started == 0 && value.finished == 0 {
                continue;
            }

            let mut object = Map::new();
            object.insert("queued".to_owned(), Value::String(value.queued.to_string()));
            object.insert(
                "started".to_owned(),
                Value::String(value.started.to_string()),
            );
            object.insert(
                "finished".to_owned(),
                Value::String(value.finished.to_string()),
            );
            object.insert(
                "queued_duration_us".to_owned(),
                Value::String(value.queued_duration.as_micros().to_string()),
            );
            object.insert(
                "running_duration_us".to_owned(),
                Value::String(value.running_duration.as_micros().to_string()),
            );
            job_queue_object.insert(JobTypes::name(*job_type).to_owned(), Value::Object(object));

            total_jq.queued += value.queued;
            total_jq.started += value.started;
            total_jq.finished += value.finished;
            total_jq.queued_duration += value.queued_duration;
            total_jq.running_duration += value.running_duration;
        }

        if total_jq.queued != 0 {
            let mut object = Map::new();
            object.insert(
                "queued".to_owned(),
                Value::String(total_jq.queued.to_string()),
            );
            object.insert(
                "started".to_owned(),
                Value::String(total_jq.started.to_string()),
            );
            object.insert(
                "finished".to_owned(),
                Value::String(total_jq.finished.to_string()),
            );
            object.insert(
                "queued_duration_us".to_owned(),
                Value::String(total_jq.queued_duration.as_micros().to_string()),
            );
            object.insert(
                "running_duration_us".to_owned(),
                Value::String(total_jq.running_duration.as_micros().to_string()),
            );
            job_queue_object.insert("total".to_owned(), Value::Object(object));
        }

        let mut counters = Map::new();
        counters.insert("rpc".to_owned(), Value::Object(rpc_object));
        counters.insert("job_queue".to_owned(), Value::Object(job_queue_object));
        Value::Object(counters)
    }

    fn current_json(&self) -> Value {
        let present = Instant::now();

        let jobs = self
            .jobs
            .lock()
            .expect("perflog jobs mutex poisoned")
            .clone();
        let mut jobs_array = Vec::new();
        for (job_type, start_time) in jobs {
            if job_type == JobType::Invalid {
                continue;
            }

            let mut object = Map::new();
            object.insert(
                "job".to_owned(),
                Value::String(JobTypes::name(job_type).to_owned()),
            );
            object.insert(
                "duration_us".to_owned(),
                Value::String(
                    present
                        .saturating_duration_since(start_time)
                        .as_micros()
                        .to_string(),
                ),
            );
            jobs_array.push(Value::Object(object));
        }

        let methods = self
            .methods
            .lock()
            .expect("perflog methods mutex poisoned")
            .clone();
        let mut methods_array = Vec::with_capacity(methods.len());
        for (method, start_time) in methods.values() {
            let mut object = Map::new();
            object.insert("method".to_owned(), Value::String(method.clone()));
            object.insert(
                "duration_us".to_owned(),
                Value::String(
                    present
                        .saturating_duration_since(*start_time)
                        .as_micros()
                        .to_string(),
                ),
            );
            methods_array.push(Value::Object(object));
        }

        let mut current = Map::new();
        current.insert("jobs".to_owned(), Value::Array(jobs_array));
        current.insert("methods".to_owned(), Value::Array(methods_array));
        Value::Object(current)
    }

    fn worker_count(&self) -> u64 {
        self.jobs.lock().expect("perflog jobs mutex poisoned").len() as u64
    }

    fn rpc_start(&self, method: &str, request_id: u64) {
        let counter = self.rpc.get(method).expect("perflog rpc method must exist");
        {
            let mut counter = counter.lock().expect("perflog rpc counter mutex poisoned");
            counter.started += 1;
        }
        let mut methods = self.methods.lock().expect("perflog methods mutex poisoned");
        methods.insert(request_id, (method.to_owned(), Instant::now()));
    }

    fn rpc_end(&self, method: &str, request_id: u64, finish: bool) {
        let counter = self.rpc.get(method).expect("perflog rpc method must exist");
        let start_time = {
            let mut methods = self.methods.lock().expect("perflog methods mutex poisoned");
            methods
                .remove(&request_id)
                .expect("perflog rpc request id must exist")
                .1
        };

        let mut counter = counter.lock().expect("perflog rpc counter mutex poisoned");
        if finish {
            counter.finished += 1;
        } else {
            counter.errored += 1;
        }
        counter.duration += start_time.elapsed();
    }

    fn rpc_duration_us(&self, _method: &str, request_id: u64) -> u64 {
        let methods = self.methods.lock().expect("perflog methods mutex poisoned");
        methods
            .get(&request_id)
            .map(|(_, start)| start.elapsed().as_micros() as u64)
            .unwrap_or(0)
    }

    fn job_queue(&self, job_type: JobType) {
        let counter = self.jq.get(&job_type).expect("perflog job type must exist");
        let mut counter = counter.lock().expect("perflog job counter mutex poisoned");
        counter.queued += 1;
    }

    fn job_start(
        &self,
        job_type: JobType,
        queued_duration: Duration,
        start_time: Instant,
        instance: i32,
    ) {
        let counter = self.jq.get(&job_type).expect("perflog job type must exist");
        {
            let mut counter = counter.lock().expect("perflog job counter mutex poisoned");
            counter.started += 1;
            counter.queued_duration += queued_duration;
        }

        if instance >= 0 {
            let mut jobs = self.jobs.lock().expect("perflog jobs mutex poisoned");
            let instance = instance as usize;
            if instance < jobs.len() {
                jobs[instance] = (job_type, start_time);
            }
        }
    }

    fn job_finish(&self, job_type: JobType, running_duration: Duration, instance: i32) {
        let counter = self.jq.get(&job_type).expect("perflog job type must exist");
        {
            let mut counter = counter.lock().expect("perflog job counter mutex poisoned");
            counter.finished += 1;
            counter.running_duration += running_duration;
        }

        if instance >= 0 {
            let mut jobs = self.jobs.lock().expect("perflog jobs mutex poisoned");
            let instance = instance as usize;
            if instance < jobs.len() {
                jobs[instance] = (JobType::Invalid, Instant::now());
            }
        }
    }

    fn resize_jobs(&self, resize: usize) {
        let mut jobs = self.jobs.lock().expect("perflog jobs mutex poisoned");
        if resize > jobs.len() {
            jobs.resize(resize, (JobType::Invalid, Instant::now()));
        }
    }
}

fn default_hostname() -> String {
    system_hostname()
        .or_else(|| std::env::var("HOSTNAME").ok())
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .map(|hostname| hostname.trim().to_owned())
        .filter(|hostname| !hostname.is_empty())
        .unwrap_or_else(|| "localhost".to_owned())
}

#[cfg(unix)]
fn system_hostname() -> Option<String> {
    use std::ffi::c_char;

    let mut buffer = [0u8; 256];
    let result = unsafe { libc::gethostname(buffer.as_mut_ptr() as *mut c_char, buffer.len()) };
    if result != 0 {
        return None;
    }

    let len = buffer
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(buffer.len());
    let hostname = std::str::from_utf8(&buffer[..len]).ok()?.trim();
    (!hostname.is_empty()).then(|| hostname.to_owned())
}

#[cfg(not(unix))]
fn system_hostname() -> Option<String> {
    None
}

fn format_system_time(time: SystemTime) -> String {
    xrpl_to_string(system_time_to_offset_date_time(time))
}

fn system_time_to_offset_date_time(time: SystemTime) -> OffsetDateTime {
    match time.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(duration) => {
            OffsetDateTime::UNIX_EPOCH
                + TimeDuration::new(duration.as_secs() as i64, duration.subsec_nanos() as i32)
        }
        Err(error) => {
            let duration = error.duration();
            OffsetDateTime::UNIX_EPOCH
                - TimeDuration::new(duration.as_secs() as i64, duration.subsec_nanos() as i32)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    use super::system_hostname;

    #[test]
    fn default_hostname_is_never_blank() {
        let hostname = default_hostname();
        assert!(!hostname.trim().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn default_hostname_prefers_system_hostname_on_unix() {
        let hostname = default_hostname();
        let system = system_hostname().expect("system hostname should resolve");
        assert_eq!(hostname, system);
    }
}
