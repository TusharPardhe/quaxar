use basics::basic_config::Section;
use perflog::{
    JobType, JobTypes, JournalLevel, NullJournal, PerfLog, PerfLogImp, PerfLogJournal,
    PerfLogReportSource, PerfLogSetup, make_perf_log, measure_duration_and_log, setup_perf_log,
};
use serde_json::{Value, json};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Default)]
struct RecordingJournal {
    entries: Mutex<Vec<(JournalLevel, String)>>,
}

impl RecordingJournal {
    fn entries(&self) -> Vec<(JournalLevel, String)> {
        self.entries.lock().expect("journal mutex poisoned").clone()
    }
}

impl PerfLogJournal for RecordingJournal {
    fn log(&self, level: JournalLevel, message: &str) {
        self.entries
            .lock()
            .expect("journal mutex poisoned")
            .push((level, message.to_owned()));
    }
}

#[derive(Default)]
struct RecordingReportSource {
    state_calls: Mutex<u32>,
}

impl RecordingReportSource {
    fn state_calls(&self) -> u32 {
        *self.state_calls.lock().expect("state calls mutex poisoned")
    }
}

impl PerfLogReportSource for RecordingReportSource {
    fn node_store_counts_json(&self) -> Value {
        json!({"entries": 5})
    }

    fn state_accounting(&self, report: &mut Value) {
        *self.state_calls.lock().expect("state calls mutex poisoned") += 1;
        if let Some(object) = report.as_object_mut() {
            object.insert("state".to_owned(), Value::String("ok".to_owned()));
        }
    }
}

fn unique_path(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .expect("system clock should be after unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("perflog-{name}-{nanos}-{}", std::process::id()))
}

#[test]
fn job_types_match_cpp_table() {
    let job_types = JobTypes::instance();

    assert_eq!(job_types.size(), 46);
    assert_eq!(JobTypes::name(JobType::Pack), "makeFetchPack");
    assert_eq!(JobTypes::name(JobType::Rpc), "RPC");
    assert_eq!(job_types.get(JobType::Peer).limit(), 0);
    assert!(job_types.get(JobType::Peer).special());
    assert_eq!(job_types.get(JobType::Batch).limit(), i32::MAX);
    assert_eq!(job_types.get(JobType::NsWrite).name(), "WriteNode");
}

#[test]
fn setup_perf_log_resolves_relative_paths_and_seconds() {
    let mut section = Section::new("perf");
    section.set("perf_log", "logs/perf.json");
    section.set("log_interval", "7");

    let config_dir = unique_path("config");
    let setup = setup_perf_log(&section, &config_dir);

    assert_eq!(setup.perf_log, config_dir.join("logs/perf.json"));
    assert_eq!(setup.log_interval, Duration::from_secs(7));
}

#[test]
fn setup_perf_log_resolves_relative_config_dirs_absolute_paths() {
    let mut section = Section::new("perf");
    section.set("perf_log", "logs/perf.json");

    let cwd = std::env::current_dir().expect("current dir should exist");
    struct RestoreCwd(PathBuf);
    impl Drop for RestoreCwd {
        fn drop(&mut self) {
            std::env::set_current_dir(&self.0).expect("test should restore cwd");
        }
    }
    let _restore = RestoreCwd(cwd.clone());
    let temp_dir = unique_path("relative-config");
    fs::create_dir_all(&temp_dir).expect("temp dir should be creatable");
    std::env::set_current_dir(&temp_dir).expect("test should be able to chdir");

    let config_dir = PathBuf::from("config/subdir");
    let setup = setup_perf_log(&section, &config_dir);

    let expected_suffix = temp_dir.join("config/subdir").join("logs/perf.json");
    // On macOS /var is symlinked to /private/var, so compare suffixes
    assert!(
        setup
            .perf_log
            .ends_with(expected_suffix.file_name().unwrap())
            || setup
                .perf_log
                .to_string_lossy()
                .ends_with("config/subdir/logs/perf.json"),
        "perf_log path should end with config/subdir/logs/perf.json, got: {:?}",
        setup.perf_log
    );

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn setup_perf_log_keeps_absolute_paths_and_default_interval() {
    let mut section = Section::new("perf");
    section.set("perf_log", "/var/log/xrpld/perf.json");

    let setup = setup_perf_log(&section, &unique_path("config"));

    assert_eq!(setup.perf_log, PathBuf::from("/var/log/xrpld/perf.json"));
    assert_eq!(setup.log_interval, Duration::from_secs(1));
    assert_eq!(JournalLevel::Trace.to_string(), "trace");
    assert_eq!(JournalLevel::Fatal.to_string(), "fatal");
}

#[test]
fn counters_and_current_json_match_cpp_shape() {
    let journal = Arc::new(RecordingJournal::default());
    let report_source = Arc::new(RecordingReportSource::default());
    let signal_stop = Arc::new(|| {});
    let setup = PerfLogSetup::default();
    let log = PerfLogImp::new_with_hostname(
        setup,
        vec!["ping".to_owned()],
        report_source,
        journal,
        signal_stop,
        "test-host",
    );

    log.resize_jobs(1);
    log.rpc_start("ping", 7);
    let job_start = Instant::now();
    log.job_queue(JobType::Transaction);
    log.job_start(JobType::Transaction, Duration::from_millis(3), job_start, 0);

    thread::sleep(Duration::from_millis(2));
    let current = log.current_json();
    assert_eq!(current["jobs"].as_array().expect("jobs array").len(), 1);
    assert_eq!(
        current["methods"].as_array().expect("methods array").len(),
        1
    );
    assert_eq!(current["jobs"][0]["job"], "transaction");
    assert_eq!(current["methods"][0]["method"], "ping");

    log.rpc_finish("ping", 7);
    log.job_finish(JobType::Transaction, Duration::from_millis(4), 0);

    let counters = log.counters_json();
    assert_eq!(counters["rpc"]["ping"]["started"], "1");
    assert_eq!(counters["rpc"]["ping"]["finished"], "1");
    assert_eq!(counters["rpc"]["ping"]["errored"], "0");
    assert_eq!(counters["rpc"]["total"]["started"], "1");
    assert_eq!(counters["job_queue"]["transaction"]["queued"], "1");
    assert_eq!(counters["job_queue"]["transaction"]["started"], "1");
    assert_eq!(counters["job_queue"]["transaction"]["finished"], "1");
    assert_eq!(
        counters["job_queue"]["transaction"]["queued_duration_us"],
        "3000"
    );
    assert_eq!(
        counters["job_queue"]["transaction"]["running_duration_us"],
        "4000"
    );
    assert_eq!(counters["job_queue"]["total"]["queued"], "1");
}

#[test]
fn report_once_writes_cpp_style_json() {
    let report_path = unique_path("report").with_extension("json");
    let journal = Arc::new(RecordingJournal::default());
    let report_source = Arc::new(RecordingReportSource::default());
    let signal_stop = Arc::new(|| {});
    let setup = PerfLogSetup {
        perf_log: report_path.clone(),
        log_interval: Duration::from_secs(1),
    };
    let log = PerfLogImp::new_with_hostname(
        setup,
        vec!["ping".to_owned()],
        report_source,
        journal,
        signal_stop,
        "test-host",
    );

    log.resize_jobs(3);
    log.rpc_start("ping", 1);
    log.rpc_finish("ping", 1);
    log.report_once();

    let contents = fs::read_to_string(&report_path).expect("report file should exist");
    let report: Value = serde_json::from_str(contents.trim()).expect("report should parse");

    assert_eq!(report["hostid"], "test-host");
    assert_eq!(report["workers"], 3);
    assert_eq!(report["nodestore"]["entries"], 5);
    assert_eq!(report["state"], "ok");
    assert_eq!(report["counters"]["rpc"]["ping"]["started"], "1");
    assert_eq!(report["counters"]["rpc"]["total"]["finished"], "1");
    assert_eq!(
        report["current_activities"]["jobs"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(log.snapshot_report()["state"], "ok");
    assert_eq!(log.snapshot_report()["nodestore"]["entries"], 5);

    let _ = fs::remove_file(&report_path);
}

#[test]
fn stop_before_start_noop_and_does_not_disable_later_start() {
    let report_path = unique_path("stop-before-start").with_extension("json");
    let journal = Arc::new(RecordingJournal::default());
    let report_source = Arc::new(RecordingReportSource::default());
    let signal_stop = Arc::new(|| {});
    let setup = PerfLogSetup {
        perf_log: report_path.clone(),
        log_interval: Duration::from_millis(10),
    };
    let log = PerfLogImp::new_with_hostname(
        setup,
        vec!["ping".to_owned()],
        report_source,
        journal,
        signal_stop,
        "test-host",
    );

    log.stop();
    log.start();
    thread::sleep(Duration::from_millis(30));
    log.stop();

    let contents = fs::read_to_string(&report_path).expect("report file should exist");
    assert!(
        !contents.trim().is_empty(),
        "perflog should still report after a pre-start stop"
    );

    let _ = fs::remove_file(&report_path);
}

#[test]
fn start_after_stop_preserves_current_cpp_sticky_stop_behavior() {
    let report_path = unique_path("restart").with_extension("json");
    let journal = Arc::new(RecordingJournal::default());
    let report_source = Arc::new(RecordingReportSource::default());
    let signal_stop = Arc::new(|| {});
    let setup = PerfLogSetup {
        perf_log: report_path.clone(),
        log_interval: Duration::from_millis(10),
    };
    let log = PerfLogImp::new_with_hostname(
        setup,
        vec!["ping".to_owned()],
        report_source,
        journal,
        signal_stop,
        "test-host",
    );

    log.start();
    thread::sleep(Duration::from_millis(30));
    log.stop();
    let first_contents = fs::read_to_string(&report_path).expect("report file should exist");
    let first_lines = first_contents.lines().count();

    log.start();
    thread::sleep(Duration::from_millis(30));
    log.stop();
    let second_contents = fs::read_to_string(&report_path).expect("report file should exist");
    let second_lines = second_contents.lines().count();

    assert_eq!(
        second_lines, first_lines,
        "post-stop start should respawn like C++ but exit immediately under the preserved stop flag"
    );

    let _ = fs::remove_file(&report_path);
}

#[test]
fn constructor_open_failure_logs_fatal_and_signals_stop() {
    let blocked_parent = unique_path("blocked-parent");
    fs::write(&blocked_parent, b"obstacle").expect("blocked parent file should be created");

    let report_path = blocked_parent.join("perf.json");
    let journal = Arc::new(RecordingJournal::default());
    let report_source = Arc::new(RecordingReportSource::default());
    let signal_count = Arc::new(AtomicUsize::new(0));
    let signal_stop = {
        let signal_count = signal_count.clone();
        Arc::new(move || {
            signal_count.fetch_add(1, Ordering::SeqCst);
        })
    };

    let _log = PerfLogImp::new_with_hostname(
        PerfLogSetup {
            perf_log: report_path,
            log_interval: Duration::from_millis(10),
        },
        vec!["ping".to_owned()],
        report_source,
        journal.clone(),
        signal_stop,
        "test-host",
    );

    assert_eq!(signal_count.load(Ordering::SeqCst), 1);
    assert!(journal.entries().iter().any(|(level, message)| {
        *level == JournalLevel::Fatal
            && message.contains("Unable to create performance log directory")
    }));

    let _ = fs::remove_file(&blocked_parent);
}

#[test]
fn rotate_reopens_the_log_path() {
    let report_path = unique_path("rotate").with_extension("json");
    let rotated_path = report_path.with_extension("old");
    let journal = Arc::new(RecordingJournal::default());
    let report_source = Arc::new(RecordingReportSource::default());
    let signal_stop = Arc::new(|| {});
    let setup = PerfLogSetup {
        perf_log: report_path.clone(),
        log_interval: Duration::from_millis(10),
    };
    let log = PerfLogImp::new_with_hostname(
        setup,
        vec!["ping".to_owned()],
        report_source.clone(),
        journal,
        signal_stop,
        "test-host",
    );

    log.report_once();
    fs::rename(&report_path, &rotated_path).expect("initial report should be movable");

    log.start();
    log.rotate();
    thread::sleep(Duration::from_millis(30));
    log.report_once();
    log.stop();

    assert!(
        rotated_path.exists(),
        "rotated-away file should still exist"
    );
    assert!(
        report_path.exists(),
        "rotate should recreate the active log path"
    );
    assert!(
        !fs::read_to_string(&report_path)
            .expect("recreated log file should be readable")
            .trim()
            .is_empty(),
        "recreated log file should contain a report"
    );
    assert!(
        report_source.state_calls() >= 2,
        "state accounting should run for both pre- and post-rotate reports"
    );

    let _ = fs::remove_file(&report_path);
    let _ = fs::remove_file(&rotated_path);
}

#[test]
fn double_start_panics_joinable_thread_contract() {
    let report_path = unique_path("double-start").with_extension("json");
    let journal = Arc::new(RecordingJournal::default());
    let report_source = Arc::new(RecordingReportSource::default());
    let signal_stop = Arc::new(|| {});
    let setup = PerfLogSetup {
        perf_log: report_path.clone(),
        log_interval: Duration::from_millis(10),
    };
    let log = PerfLogImp::new_with_hostname(
        setup,
        vec!["ping".to_owned()],
        report_source,
        journal,
        signal_stop,
        "test-host",
    );

    log.start();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        log.start();
    }));
    log.stop();

    assert!(
        result.is_err(),
        "a second start while the worker thread is already running should fail like C++"
    );

    let _ = fs::remove_file(&report_path);
}

#[test]
fn measure_duration_and_log_warns_after_threshold() {
    let journal = RecordingJournal::default();
    let result = measure_duration_and_log(
        || {
            thread::sleep(Duration::from_millis(2));
            42
        },
        "slow action",
        Duration::from_millis(0),
        &journal,
    );

    assert_eq!(result, 42);
    let entries = journal.entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].0, JournalLevel::Warn);
    assert!(entries[0].1.contains("slow action took"));
}

#[test]
fn make_perf_log_returns_trait_object() {
    let report_source = Arc::new(RecordingReportSource::default());
    let journal = Arc::new(NullJournal);
    let signal_stop = Arc::new(|| {});
    let perf_log = make_perf_log(
        PerfLogSetup::default(),
        vec!["ping".to_owned()],
        report_source,
        journal,
        signal_stop,
    );

    perf_log.counters_json();
    perf_log.current_json();
}
