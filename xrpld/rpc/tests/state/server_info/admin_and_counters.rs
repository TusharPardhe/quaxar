//! server info tests part B.

use super::*;
use app::JobType;
use perflog::PerfLog;
use std::time::Instant;

#[test]
fn server_info_admin_uses_none_pubkey_validator_when_unset() {
    let app = ApplicationRoot::new(0).expect("root shell should build");
    let source = ApplicationServerInfo::new(&app);

    let result = do_server_info(&context(&object([]), &source, RpcRole::Admin));
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    let JsonValue::Object(info) = result.get("info").expect("info must exist") else {
        panic!("info must be an object");
    };

    assert_eq!(
        info.get("pubkey_validator"),
        Some(&JsonValue::String("none".to_owned()))
    );
}

#[test]
fn server_info_pins_validated_age_status_flags_and_current_offset_omission() {
    let app = ApplicationRoot::new(0).expect("root shell should build");
    app.set_network_ops_operating_mode(NetworkOpsOperatingMode::Tracking);
    app.set_need_network_ledger(true);
    app.set_amendment_blocked(true);
    assert_eq!(
        app.time_keeper()
            .adjust_close_time(time::Duration::seconds(8)),
        time::Duration::seconds(2)
    );

    let validated = sample_ledger(101, 1_005, 0x44);
    app.on_validated_ledger(validated);
    let now_close_time = app.time_keeper().close_time().as_seconds();
    app.ledger_master_state()
        .set_validated_close_time(now_close_time.saturating_sub(42));

    let source = ApplicationServerInfo::new(&app);
    let result = do_server_info(&context(&object([]), &source, RpcRole::User));
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    let JsonValue::Object(info) = result.get("info").expect("info must exist") else {
        panic!("info must be an object");
    };

    assert_eq!(
        info.get("server_state"),
        Some(&JsonValue::String("tracking".to_owned()))
    );
    assert_eq!(
        info.get("network_ledger"),
        Some(&JsonValue::String("waiting".to_owned()))
    );
    assert_eq!(info.get("amendment_blocked"), Some(&JsonValue::Bool(true)));
    assert!(!info.contains_key("close_time_offset"));

    let JsonValue::Object(validated_ledger) = info
        .get("validated_ledger")
        .expect("validated_ledger must exist")
    else {
        panic!("validated_ledger must be an object");
    };
    assert_eq!(validated_ledger.get("seq"), Some(&JsonValue::Unsigned(101)));
    assert_eq!(validated_ledger.get("age"), Some(&JsonValue::Unsigned(42)));
}

#[test]
fn server_info_counters_reads_attached_status_metrics_when_requested() {
    let mut app = ApplicationRoot::new(0).expect("root shell should build");
    attach_test_server_ports(&mut app, None);
    let perf_log = make_test_perf_log(
        &["ping", "server_info"],
        TestPerfLogReportSource {
            nodestore: json!({
                "node_reads_total": "9",
                "read_threads_total": 2
            }),
            state_accounting: json!({
                "full": {"transitions": "1", "duration_us": "10"}
            }),
            server_state_duration_us: Some("10".to_owned()),
            initial_sync_duration_us: None,
        },
    );
    perf_log.rpc_start("ping", 1);
    perf_log.rpc_finish("ping", 1);
    perf_log.rpc_start("ping", 2);
    perf_log.rpc_finish("ping", 2);
    perf_log.resize_jobs(1);
    perf_log.job_queue(JobType::Transaction);
    perf_log.job_start(
        JobType::Transaction,
        Duration::from_micros(10),
        Instant::now(),
        0,
    );
    app.attach_status_metrics(perf_log.clone());

    let source = ApplicationServerInfo::new(&app);
    let params = object([("counters", JsonValue::Bool(true))]);
    let result = do_server_info(&context(&params, &source, RpcRole::Admin));
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    let JsonValue::Object(info) = result.get("info").expect("info must exist") else {
        panic!("info must be an object");
    };

    let JsonValue::Object(counters) = info.get("counters").expect("counters must exist") else {
        panic!("counters must be an object");
    };
    let JsonValue::Object(rpc) = counters.get("rpc").expect("rpc counters must exist") else {
        panic!("rpc counters must be an object");
    };
    let JsonValue::Object(ping) = rpc.get("ping").expect("ping counters must exist") else {
        panic!("ping counters must be an object");
    };
    assert_eq!(
        ping.get("started"),
        Some(&JsonValue::String("2".to_owned()))
    );
    assert_eq!(
        ping.get("finished"),
        Some(&JsonValue::String("2".to_owned()))
    );

    let JsonValue::Object(nodestore) = counters
        .get("nodestore")
        .expect("nodestore counters must exist")
    else {
        panic!("nodestore counters must be an object");
    };
    assert_eq!(
        nodestore.get("node_reads_total"),
        Some(&JsonValue::String("9".to_owned()))
    );
    assert_eq!(
        nodestore.get("read_threads_total"),
        Some(&JsonValue::Unsigned(2))
    );

    let JsonValue::Object(current) = info
        .get("current_activities")
        .expect("current_activities must exist")
    else {
        panic!("current_activities must be an object");
    };
    let JsonValue::Array(jobs) = current.get("jobs").expect("jobs must exist") else {
        panic!("jobs must be an array");
    };
    assert_eq!(jobs.len(), 1);
    let JsonValue::Object(job) = &jobs[0] else {
        panic!("job must be an object");
    };
    assert_eq!(
        job.get("job"),
        Some(&JsonValue::String("transaction".to_owned()))
    );
    let JsonValue::Array(methods) = current.get("methods").expect("methods must exist") else {
        panic!("methods must be an array");
    };
    assert_eq!(methods.len(), 0);
}

#[test]
fn server_info_hides_admin_ports_from_non_admin_users() {
    let mut app = ApplicationRoot::new(0).expect("root shell should build");
    attach_test_server_ports(&mut app, None);

    let source = ApplicationServerInfo::new(&app);
    let result = do_server_info(&context(&object([]), &source, RpcRole::User));
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    let JsonValue::Object(info) = result.get("info").expect("info must exist") else {
        panic!("info must be an object");
    };
    let JsonValue::Array(ports) = info.get("ports").expect("ports must exist") else {
        panic!("ports must be an array");
    };
    assert_eq!(ports.len(), 1);
    assert_eq!(
        ports[0],
        JsonValue::Object(BTreeMap::from([
            ("port".to_owned(), JsonValue::String("5005".to_owned())),
            (
                "protocol".to_owned(),
                JsonValue::Array(vec![
                    JsonValue::String("http".to_owned()),
                    JsonValue::String("ws".to_owned()),
                ]),
            ),
        ]))
    );
}
