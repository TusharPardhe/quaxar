use basics::basic_config::Section;
use serde_json::Value;
use tempfile::TempDir;
use xrpl_core::perf_log::{
    NullPerfLogReportProvider, PerfLogReportProvider, PerfLogSetup, measure_duration_and_log,
    setup_perf_log,
};
use xrpl_core::{JobType, PerfLog, PerfLogImp};

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug)]
struct ExtraReportProvider;

impl PerfLogReportProvider for ExtraReportProvider {
    fn extend_report(&self, report: &mut serde_json::Map<String, Value>) {
        report.insert("extra".to_owned(), Value::String("present".to_owned()));
    }
}

fn wait_for_non_empty_file(path: &std::path::Path) {
    for _ in 0..100 {
        if fs::metadata(path)
            .map(|metadata| metadata.len() > 0)
            .unwrap_or(false)
        {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }

    panic!("timed out waiting for non-empty file at {}", path.display());
}

fn read_last_json_line(path: &std::path::Path) -> Value {
    let contents = fs::read_to_string(path).expect("read perf log");
    let line = contents
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .expect("perf log line");
    serde_json::from_str(line).expect("parse perf log json")
}

fn json_string_u128(value: &Value, field: &str) -> u128 {
    value[field]
        .as_str()
        .unwrap_or_else(|| panic!("{field} should be a string"))
        .parse::<u128>()
        .unwrap_or_else(|err| panic!("{field} should parse as u128: {err}"))
}

#[test]
fn setup_perf_log_resolves_relative_paths_and_current_cpp_seconds_interval() {
    let mut section = Section::new("perf");
    section.set("perf_log", "logs/perf.json");
    section.set("log_interval", "7");

    let setup = setup_perf_log(&section, &PathBuf::from("/tmp/config"));

    assert_eq!(
        setup.perf_log,
        Some(PathBuf::from("/tmp/config/logs/perf.json"))
    );
    assert_eq!(setup.log_interval, Duration::from_secs(7));
}

#[test]
fn setup_perf_log_keeps_absolute_paths_and_default_interval() {
    let mut section = Section::new("perf");
    section.set("perf_log", "/var/log/xrpld/perf.json");

    let setup = setup_perf_log(&section, &PathBuf::from("/tmp/config"));

    assert_eq!(
        setup.perf_log,
        Some(PathBuf::from("/var/log/xrpld/perf.json"))
    );
    assert_eq!(setup.log_interval, Duration::from_secs(1));
}

#[test]
fn measure_duration_and_log_only_triggers_when_over_threshold() {
    let log_count = Arc::new(AtomicUsize::new(0));

    let under = measure_duration_and_log(|| 7_u32, "under threshold", Duration::from_secs(1), {
        let log_count = Arc::clone(&log_count);
        move |_, _| {
            log_count.fetch_add(1, Ordering::SeqCst);
        }
    });
    assert_eq!(under, 7);
    assert_eq!(log_count.load(Ordering::SeqCst), 0);

    let over = measure_duration_and_log(
        || {
            std::thread::sleep(Duration::from_millis(2));
            "done"
        },
        "over threshold",
        Duration::ZERO,
        {
            let log_count = Arc::clone(&log_count);
            move |message, duration| {
                assert_eq!(message, "over threshold");
                assert!(duration >= Duration::from_millis(2));
                log_count.fetch_add(1, Ordering::SeqCst);
            }
        },
    );
    assert_eq!(over, "done");
    assert_eq!(log_count.load(Ordering::SeqCst), 1);
}

#[test]
fn perf_log_json_counter_totals_and_current_shape() {
    let log = PerfLogImp::new();
    log.resize_jobs(4);

    log.rpc_start("ping", 10);
    log.rpc_finish("ping", 10);
    log.job_queue(JobType::Client);
    log.job_start(JobType::Client, Duration::from_micros(7), Instant::now(), 2);

    let counters = log.counters_json();
    assert_eq!(counters["rpc"]["ping"]["started"], "1");
    assert_eq!(counters["rpc"]["total"]["started"], "1");
    assert_eq!(counters["rpc"]["total"]["finished"], "1");
    assert_eq!(counters["job_queue"]["clientCommand"]["queued"], "1");
    assert_eq!(counters["job_queue"]["total"]["queued"], "1");
    assert_eq!(counters["job_queue"]["total"]["started"], "1");

    let current = log.current_json();
    assert!(current.is_object());
    assert!(current.get("jobs").expect("jobs entry").is_array());
    assert!(current.get("methods").expect("methods entry").is_array());
    assert_eq!(current["jobs"].as_array().expect("jobs array").len(), 1);
    assert_eq!(
        current["methods"].as_array().expect("methods array").len(),
        0
    );
    assert!(current["jobs"][0].get("instance").is_none());
    assert_eq!(current["jobs"][0]["job"], "clientCommand");
}

#[test]
fn perf_log_rpc_error_updates_totals_and_clears_current_method() {
    let log = PerfLogImp::new();

    log.rpc_start("submit", 77);
    let current = log.current_json();
    assert_eq!(
        current["methods"].as_array().expect("methods array").len(),
        1
    );
    assert_eq!(current["methods"][0]["method"], "submit");

    log.rpc_error("submit", 77);

    let counters = log.counters_json();
    assert_eq!(counters["rpc"]["submit"]["started"], "1");
    assert_eq!(counters["rpc"]["submit"]["finished"], "0");
    assert_eq!(counters["rpc"]["submit"]["errored"], "1");
    assert_eq!(counters["rpc"]["total"]["started"], "1");
    assert_eq!(counters["rpc"]["total"]["finished"], "0");
    assert_eq!(counters["rpc"]["total"]["errored"], "1");

    let current = log.current_json();
    assert_eq!(
        current["methods"].as_array().expect("methods array").len(),
        0
    );
}

#[test]
fn perf_log_interleaved_rpc_lifecycle_keeps_one_unfinished_method() {
    let log = PerfLogImp::new();

    log.rpc_start("server_info", 1);
    thread::sleep(Duration::from_millis(1));
    log.rpc_start("server_info", 2);
    thread::sleep(Duration::from_millis(1));
    log.rpc_start("submit", 3);
    thread::sleep(Duration::from_millis(1));
    log.rpc_start("submit", 4);

    log.rpc_finish("submit", 4);
    log.rpc_error("submit", 3);
    log.rpc_finish("server_info", 2);
    // Intentionally leave request 1 in flight, matching the C++ test shape.

    let counters = log.counters_json();
    assert_eq!(counters["rpc"]["server_info"]["started"], "2");
    assert_eq!(counters["rpc"]["server_info"]["finished"], "1");
    assert_eq!(counters["rpc"]["server_info"]["errored"], "0");
    assert_ne!(counters["rpc"]["server_info"]["duration_us"], "0");
    assert_eq!(counters["rpc"]["submit"]["started"], "2");
    assert_eq!(counters["rpc"]["submit"]["finished"], "1");
    assert_eq!(counters["rpc"]["submit"]["errored"], "1");
    assert_ne!(counters["rpc"]["submit"]["duration_us"], "0");
    assert_eq!(counters["rpc"]["total"]["started"], "4");
    assert_eq!(counters["rpc"]["total"]["finished"], "2");
    assert_eq!(counters["rpc"]["total"]["errored"], "1");
    assert_ne!(counters["rpc"]["total"]["duration_us"], "0");

    let current = log.current_json();
    let methods = current["methods"].as_array().expect("methods array");
    assert_eq!(methods.len(), 1);
    assert_eq!(methods[0]["method"], "server_info");
    assert_ne!(methods[0]["duration_us"], "0");
    assert_eq!(current["jobs"].as_array().expect("jobs array").len(), 0);
}

#[test]
fn perf_log_current_json_sorts_entries_by_duration_descending() {
    let log = PerfLogImp::new();
    log.resize_jobs(2);

    let older = Instant::now() - Duration::from_millis(20);
    let newer = Instant::now() - Duration::from_millis(10);
    log.job_start(JobType::Client, Duration::ZERO, older, 0);
    log.job_start(JobType::Pack, Duration::ZERO, newer, 1);

    log.rpc_start("submit", 1);
    thread::sleep(Duration::from_millis(1));
    log.rpc_start("server_info", 2);

    let current = log.current_json();
    let jobs = current["jobs"].as_array().expect("jobs array");
    assert_eq!(jobs.len(), 2);
    assert_eq!(jobs[0]["job"], "clientCommand");
    assert_eq!(jobs[1]["job"], "makeFetchPack");
    assert!(json_string_u128(&jobs[0], "duration_us") >= json_string_u128(&jobs[1], "duration_us"));

    let methods = current["methods"].as_array().expect("methods array");
    assert_eq!(methods.len(), 2);
    assert_eq!(methods[0]["method"], "submit");
    assert_eq!(methods[1]["method"], "server_info");
    assert!(
        json_string_u128(&methods[0], "duration_us")
            >= json_string_u128(&methods[1], "duration_us")
    );
}

#[test]
fn perf_log_ignores_invalid_worker_ids_but_keeps_counter_totals() {
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
    assert_eq!(
        log.current_json()["jobs"]
            .as_array()
            .expect("jobs array")
            .len(),
        0
    );
}

#[test]
fn runtime_perf_log_writes_json_reports_with_extended_fields() {
    let temp = TempDir::new().expect("tempdir");
    let path = temp.path().join("perf.log");
    let log = PerfLogImp::with_runtime(
        PerfLogSetup {
            perf_log: Some(path.clone()),
            log_interval: Duration::from_millis(10),
        },
        "test-host",
        || {},
        Arc::new(ExtraReportProvider),
    );

    log.resize_jobs(3);
    log.rpc_start("ping", 1);
    log.job_queue(JobType::Client);
    log.job_start(
        JobType::Client,
        Duration::from_micros(11),
        Instant::now(),
        7,
    );
    log.start();

    wait_for_non_empty_file(&path);
    log.stop();

    let report = read_last_json_line(&path);
    assert!(report.get("time").is_some());
    assert_eq!(report["workers"], 3);
    assert_eq!(report["hostid"], "test-host");
    assert_eq!(report["extra"], "present");
    assert_eq!(report["counters"]["rpc"]["ping"]["started"], "1");
    assert_eq!(
        report["counters"]["job_queue"]["clientCommand"]["queued"],
        "1"
    );
    assert!(report["current_activities"]["jobs"].is_array());
    assert!(report["current_activities"]["methods"].is_array());
}

#[test]
fn runtime_rotate_reopens_the_log_path_after_file_replacement() {
    let temp = TempDir::new().expect("tempdir");
    let path = temp.path().join("perf.log");
    let rotated_path = temp.path().join("perf.log.1");
    let log = PerfLogImp::with_runtime(
        PerfLogSetup {
            perf_log: Some(path.clone()),
            log_interval: Duration::from_millis(10),
        },
        "rotate-host",
        || {},
        Arc::new(NullPerfLogReportProvider),
    );

    log.start();
    wait_for_non_empty_file(&path);

    fs::rename(&path, &rotated_path).expect("rotate old file");
    fs::write(&path, "").expect("create new file");
    log.rotate();
    wait_for_non_empty_file(&path);
    log.stop();

    let rotated = fs::read_to_string(&rotated_path).expect("read rotated file");
    let current = fs::read_to_string(&path).expect("read reopened file");
    assert!(!rotated.trim().is_empty());
    assert!(!current.trim().is_empty());
}

#[test]
fn runtime_stop_flag_stays_sticky_across_restart_like_current_cpp() {
    let temp = TempDir::new().expect("tempdir");
    let path = temp.path().join("perf.log");
    let log = PerfLogImp::with_runtime(
        PerfLogSetup {
            perf_log: Some(path.clone()),
            log_interval: Duration::from_millis(10),
        },
        "sticky-stop",
        || {},
        Arc::new(NullPerfLogReportProvider),
    );

    log.start();
    wait_for_non_empty_file(&path);
    log.stop();

    let first_contents = fs::read_to_string(&path).expect("read initial perf log");
    let first_line_count = first_contents
        .lines()
        .filter(|line| !line.is_empty())
        .count();

    log.start();
    thread::sleep(Duration::from_millis(40));
    log.stop();

    let restarted_contents = fs::read_to_string(&path).expect("read restarted perf log");
    let restarted_line_count = restarted_contents
        .lines()
        .filter(|line| !line.is_empty())
        .count();

    assert_eq!(restarted_line_count, first_line_count);
}

#[test]
fn runtime_setup_signals_stop_when_parent_directory_cannot_be_created() {
    let temp = TempDir::new().expect("tempdir");
    let blocked_parent = temp.path().join("blocked");
    fs::write(&blocked_parent, "not a directory").expect("create blocking file");

    let stop_signals = Arc::new(AtomicUsize::new(0));
    let _log = PerfLogImp::with_runtime(
        PerfLogSetup {
            perf_log: Some(blocked_parent.join("perf.log")),
            log_interval: Duration::from_millis(10),
        },
        "bad-host",
        {
            let stop_signals = Arc::clone(&stop_signals);
            move || {
                stop_signals.fetch_add(1, Ordering::SeqCst);
            }
        },
        Arc::new(NullPerfLogReportProvider),
    );

    assert_eq!(stop_signals.load(Ordering::SeqCst), 1);
}
