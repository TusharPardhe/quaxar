//! server state tests part B.

use super::*;

#[test]
fn server_state_prefers_validated_ledger_and_omits_human_age_and_offset() {
    let app = ApplicationRoot::new(0).expect("root shell should build");
    app.set_network_ops_operating_mode(NetworkOpsOperatingMode::Full);
    app.set_need_network_ledger(true);
    app.set_amendment_blocked(true);

    let closed = sample_ledger(100, 1_000, 0x11);
    let validated = sample_ledger(101, 1_005, 0x22);
    app.on_closed_ledger(closed);
    app.on_validated_ledger(validated);

    assert_eq!(
        app.time_keeper()
            .adjust_close_time(time::Duration::seconds(12)),
        time::Duration::seconds(3)
    );
    let now_close_time = app.time_keeper().close_time().as_seconds();
    app.ledger_master_state()
        .set_validated_close_time(now_close_time.saturating_sub(33));

    let source = ApplicationServerInfo::new(&app);
    let result = do_server_state(&context(
        &JsonValue::Object(Default::default()),
        &source,
        RpcRole::User,
    ));
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    let JsonValue::Object(state) = result.get("state").expect("state must exist") else {
        panic!("state must be object");
    };

    assert_eq!(
        state.get("server_state"),
        Some(&JsonValue::String("full".to_owned()))
    );
    assert_eq!(
        state.get("network_ledger"),
        Some(&JsonValue::String("waiting".to_owned()))
    );
    assert_eq!(state.get("amendment_blocked"), Some(&JsonValue::Bool(true)));
    assert!(!state.contains_key("close_time_offset"));
    assert!(!state.contains_key("closed_ledger"));

    let JsonValue::Object(validated_ledger) = state
        .get("validated_ledger")
        .expect("validated_ledger must exist")
    else {
        panic!("validated_ledger must be object");
    };
    assert_eq!(validated_ledger.get("seq"), Some(&JsonValue::Unsigned(101)));
    assert_eq!(
        validated_ledger.get("close_time"),
        Some(&JsonValue::Unsigned(1_005))
    );
    assert!(!validated_ledger.contains_key("age"));
}

#[test]
fn server_state_admin_includes_validator_expiry_and_load_queue() {
    let app = ApplicationRoot::new(0).expect("root shell should build");
    let source = ApplicationServerInfo::new(&app);

    let result = do_server_state(&context(
        &JsonValue::Object(Default::default()),
        &source,
        RpcRole::Admin,
    ));
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    let JsonValue::Object(state) = result.get("state").expect("state must exist") else {
        panic!("state must be object");
    };

    assert_eq!(
        state.get("validation_quorum"),
        Some(&JsonValue::Unsigned(1))
    );
    assert_eq!(
        state.get("validator_list_expires"),
        Some(&JsonValue::Unsigned(0))
    );
    assert_eq!(
        state.get("pubkey_validator"),
        Some(&JsonValue::String("none".to_owned()))
    );
    let JsonValue::Object(load) = state.get("load").expect("load must exist") else {
        panic!("load must be object");
    };
    // Our load object uses "jobs", "load_events", "threads" fields
    // (matching our JobQueue reporting), not rippled's job_count/process_count.
    assert!(load.contains_key("jobs") || load.contains_key("load_events") || load.contains_key("threads"),
        "load object should contain at least one known field, got: {load:?}");
}

#[test]
fn server_state_admin_reports_validator_owned_expiry_and_pubkey() {
    let mut app = ApplicationRoot::new(0).expect("root shell should build");
    let local_signing_key = protocol::PublicKey::from_bytes([0x02; 33]);
    assert!(app.validators().load(
        Some(local_signing_key),
        &["n949f75evCHwgyP4fPVgaHqNHxUVN15PsJEZ3B3HnXPcPjcZAoy7".to_owned()],
        &[],
        None,
    ));
    let validation_public_key = protocol::PublicKey::from_bytes([0x03; 33]);
    assert_eq!(app.set_validation_public_key(validation_public_key), None);
    let local_public_key = app
        .validators()
        .local_public_key()
        .expect("local validator key should be set");

    let source = ApplicationServerInfo::new(&app);
    let admin_result = do_server_state(&context(
        &JsonValue::Object(Default::default()),
        &source,
        RpcRole::Admin,
    ));
    let JsonValue::Object(admin_result) = admin_result else {
        panic!("result must be an object");
    };
    let JsonValue::Object(admin_state) = admin_result.get("state").expect("state must exist")
    else {
        panic!("state must be object");
    };
    assert_eq!(
        admin_state.get("validation_quorum"),
        Some(&JsonValue::Unsigned(1))
    );
    assert_eq!(
        admin_state.get("validator_list_expires"),
        Some(&JsonValue::Unsigned(u32::MAX as u64))
    );
    assert_eq!(
        admin_state.get("pubkey_validator"),
        Some(&JsonValue::String(local_public_key.to_node_public_base58()))
    );
    assert!(!admin_state.contains_key("validator_list"));

    let user_result = do_server_state(&context(
        &JsonValue::Object(Default::default()),
        &source,
        RpcRole::User,
    ));
    let JsonValue::Object(user_result) = user_result else {
        panic!("result must be an object");
    };
    let JsonValue::Object(user_state) = user_result.get("state").expect("state must exist") else {
        panic!("state must be object");
    };
    assert!(!user_state.contains_key("validator_list_expires"));
    assert!(!user_state.contains_key("pubkey_validator"));
}

#[test]
fn server_state_counters_reads_attached_status_metrics_when_requested() {
    let mut app = ApplicationRoot::new(0).expect("root shell should build");
    let perf_log = make_test_perf_log(
        &["server_state"],
        TestPerfLogReportSource {
            nodestore: json!({
                "node_reads_hit": "7"
            }),
            state_accounting: json!({
                "full": {"transitions": "1", "duration_us": "10"}
            }),
            server_state_duration_us: Some("10".to_owned()),
            initial_sync_duration_us: None,
        },
    );
    perf_log.job_queue(JobType::Transaction);
    perf_log.job_queue(JobType::Transaction);
    perf_log.job_queue(JobType::Transaction);
    perf_log.resize_jobs(1);
    perf_log.job_start(
        JobType::LedgerData,
        Duration::from_micros(10),
        Instant::now(),
        0,
    );
    perf_log.rpc_start("server_state", 1);
    app.attach_status_metrics(perf_log.clone());
    let source = ApplicationServerInfo::new(&app);

    let params = JsonValue::Object(BTreeMap::from([(
        "counters".to_owned(),
        JsonValue::Bool(true),
    )]));
    let result = do_server_state(&context(&params, &source, RpcRole::Admin));
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    let JsonValue::Object(state) = result.get("state").expect("state must exist") else {
        panic!("state must be object");
    };

    let JsonValue::Object(counters) = state.get("counters").expect("counters must exist") else {
        panic!("counters must be an object");
    };
    let JsonValue::Object(job_queue) = counters
        .get("job_queue")
        .expect("job_queue counters must exist")
    else {
        panic!("job_queue counters must be an object");
    };
    let JsonValue::Object(transaction) = job_queue
        .get("transaction")
        .expect("transaction counters must exist")
    else {
        panic!("transaction counters must be an object");
    };
    assert_eq!(
        transaction.get("queued"),
        Some(&JsonValue::String("3".to_owned()))
    );

    let JsonValue::Object(nodestore) = counters
        .get("nodestore")
        .expect("nodestore counters must exist")
    else {
        panic!("nodestore counters must be an object");
    };
    assert_eq!(
        nodestore.get("node_reads_hit"),
        Some(&JsonValue::String("7".to_owned()))
    );

    let JsonValue::Object(current) = state
        .get("current_activities")
        .expect("current_activities must exist")
    else {
        panic!("current_activities must be an object");
    };
    let JsonValue::Array(jobs) = current.get("jobs").expect("jobs must exist") else {
        panic!("jobs must be an array");
    };
    assert_eq!(jobs.len(), 1);
}
