//! server state tests part A.

use super::*;

#[test]
fn server_state_wraps_the_source_result() {
    let params = JsonValue::Object(Default::default());
    let source = FakeServerStateSource::default();
    let result = do_server_state(&context(&params, &source, RpcRole::Admin));
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };

    let JsonValue::Object(state) = result.get("state").expect("state must exist") else {
        panic!("state must be an object");
    };
    assert_eq!(state.get("human"), Some(&JsonValue::Bool(false)));
    assert_eq!(state.get("admin"), Some(&JsonValue::Bool(true)));
    assert_eq!(state.get("counters"), Some(&JsonValue::Bool(false)));
    assert_eq!(source.calls.borrow().as_slice(), &[(false, true, false)]);
}

#[test]
fn server_state_treats_counters_as_optional() {
    let params = JsonValue::Object(BTreeMap::from([(
        "counters".to_owned(),
        JsonValue::Unsigned(1),
    )]));
    let source = FakeServerStateSource::default();
    let result = do_server_state(&context(&params, &source, RpcRole::User));
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };

    let JsonValue::Object(state) = result.get("state").expect("state must exist") else {
        panic!("state must be an object");
    };
    assert_eq!(state.get("human"), Some(&JsonValue::Bool(false)));
    assert_eq!(state.get("admin"), Some(&JsonValue::Bool(false)));
    assert_eq!(state.get("counters"), Some(&JsonValue::Bool(true)));
    assert_eq!(source.calls.borrow().as_slice(), &[(false, false, true)]);
}

#[test]
fn server_state_reads_integer_load_fields_from_application_source() {
    let mut app = ApplicationRoot::new(0).expect("root shell should build");
    app.set_network_ops_operating_mode(NetworkOpsOperatingMode::Tracking);
    app.load_fee_track().set_remote_fee(512);
    app.load_fee_track().set_cluster_fee(384);
    app.set_unsupported_majority_warning_details(Some(UnsupportedMajorityWarningDetails {
        expected_date: 1_700_000_000,
        expected_date_utc: "2023-Nov-14 22:13:20 UTC".to_owned(),
    }));
    let overlay = Arc::new(
        OverlayImpl::new(overlay_setup(Some(1_025)), Arc::new(TestHandoff)).expect("overlay"),
    );
    let first = overlay_peer(1, 11);
    let second = overlay_peer(2, 12);
    overlay.activate(Arc::clone(&first));
    overlay.activate(Arc::clone(&second));
    overlay.inc_jq_trans_overflow();
    overlay.inc_jq_trans_overflow();
    overlay.inc_peer_disconnect();
    overlay.inc_peer_disconnect();
    overlay.inc_peer_disconnect();
    overlay.inc_peer_disconnect_charges();
    overlay.inc_peer_disconnect_charges();
    overlay.inc_peer_disconnect_charges();
    overlay.inc_peer_disconnect_charges();
    let overlay_status: Arc<dyn app::OverlayStatusSource> = overlay.clone();
    app.attach_overlay_status(overlay_status);
    app.attach_server_ports_setup(Arc::new(ServerPortsSetup {
        ports: vec![ServerPortSetup {
            name: "port_rpc".to_owned(),
            ip: "127.0.0.1".to_owned(),
            port: 5005,
            limit: 0,
            protocols: vec!["http".to_owned(), "ws".to_owned(), "grpc".to_owned()],
            user: "rpc".to_owned(),
            password: "secret".to_owned(),
            admin_user: String::new(),
            admin_password: String::new(),
            ssl_key: String::new(),
            ssl_cert: String::new(),
            ssl_chain: String::new(),
            ssl_ciphers: String::new(),
            admin_nets_v4: Vec::new(),
            admin_nets_v6: Vec::new(),
            secure_gateway_nets_v4: Vec::new(),
            secure_gateway_nets_v6: Vec::new(),
            standalone_mode: false,
        }],
        client: None,
        overlay: None,
        grpc: Some(PublishedGrpcPort {
            ip: "127.0.0.1".to_owned(),
            port: "50051".to_owned(),
        }),
    }));
    let perf_log = make_test_perf_log(
        &["server_state", "ping"],
        TestPerfLogReportSource {
            nodestore: json!({
                "node_reads_hit": "7"
            }),
            state_accounting: json!({
                "disconnected": {"transitions": "1", "duration_us": "10"},
                "connected": {"transitions": "2", "duration_us": "20"},
                "syncing": {"transitions": "3", "duration_us": "30"},
                "tracking": {"transitions": "4", "duration_us": "40"},
                "full": {"transitions": "5", "duration_us": "50"}
            }),
            server_state_duration_us: Some("60".to_owned()),
            initial_sync_duration_us: Some("70".to_owned()),
        },
    );
    perf_log.resize_jobs(1);
    perf_log.job_queue(JobType::Transaction);
    perf_log.job_queue(JobType::Transaction);
    perf_log.job_queue(JobType::Transaction);
    perf_log.job_start(
        JobType::LedgerData,
        Duration::from_micros(10),
        Instant::now(),
        0,
    );
    perf_log.rpc_start("server_state", 1);
    app.attach_status_metrics(perf_log.clone());
    app.set_status_rpc_peer_count(Some(77));
    app.set_status_rpc_network_id(Some(77));
    app.set_status_rpc_last_close(Some(StatusRpcLastClose {
        proposers: 7,
        converge_time: Duration::from_millis(1_250),
    }));
    app.set_status_rpc_hostid(Some("host-state".to_owned()));
    app.set_status_rpc_server_domain(Some("status.example.com".to_owned()));
    app.set_status_rpc_node_size(Some("medium".to_owned()));
    app.set_status_rpc_io_latency_ms(Some(11));
    app.set_status_rpc_complete_ledgers(Some("90,100-101".to_owned()));
    app.set_status_rpc_fetch_pack(Some(1));
    app.set_status_rpc_git_info(Some(StatusRpcGitInfo {
        hash: Some("abc123".to_owned()),
        branch: Some("main".to_owned()),
    }));
    app.set_status_rpc_queue_report(Some(tx::QueueTxQRpcReport {
        ledger_current_index: 100,
        expected_ledger_size: "32".to_owned(),
        current_ledger_size: "31".to_owned(),
        current_queue_size: "4".to_owned(),
        max_queue_size: Some("200".to_owned()),
        levels: tx::QueueTxQRpcLevels {
            reference_level: "256".to_owned(),
            minimum_level: "300".to_owned(),
            median_level: "400".to_owned(),
            open_ledger_level: "640".to_owned(),
        },
        drops: tx::QueueTxQRpcDrops {
            base_fee: "10".to_owned(),
            median_fee: "16".to_owned(),
            minimum_fee: "12".to_owned(),
            open_ledger_fee: "20".to_owned(),
        },
    }));
    app.on_closed_ledger(sample_ledger(100, 1_000, 0x11));
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
        Some(&JsonValue::String("tracking".to_owned()))
    );
    assert_eq!(state.get("load_base"), Some(&JsonValue::Unsigned(256)));
    assert_eq!(state.get("load_factor"), Some(&JsonValue::Unsigned(640)));
    assert_eq!(
        state.get("load_factor_server"),
        Some(&JsonValue::Unsigned(512))
    );
    assert_eq!(
        state.get("load_factor_fee_escalation"),
        Some(&JsonValue::Unsigned(640))
    );
    assert_eq!(
        state.get("load_factor_fee_queue"),
        Some(&JsonValue::Unsigned(300))
    );
    assert_eq!(
        state.get("load_factor_fee_reference"),
        Some(&JsonValue::Unsigned(256))
    );
    assert_eq!(
        state.get("validation_quorum"),
        Some(&JsonValue::Unsigned(1))
    );
    assert_eq!(state.get("peers"), Some(&JsonValue::Unsigned(2)));
    assert_eq!(state.get("network_id"), Some(&JsonValue::Unsigned(1025)));
    assert_eq!(
        state.get("jq_trans_overflow"),
        Some(&JsonValue::String("2".to_owned()))
    );
    assert_eq!(
        state.get("peer_disconnects"),
        Some(&JsonValue::String("3".to_owned()))
    );
    assert_eq!(
        state.get("peer_disconnects_resources"),
        Some(&JsonValue::String("4".to_owned()))
    );
    let Some(JsonValue::String(server_time)) = state.get("time") else {
        panic!("time must be a string");
    };
    assert!(server_time.ends_with(" UTC"));
    let JsonValue::Object(last_close) = state.get("last_close").expect("last_close must exist")
    else {
        panic!("last_close must be object");
    };
    assert_eq!(last_close.get("proposers"), Some(&JsonValue::Unsigned(7)));
    assert_eq!(
        last_close.get("converge_time"),
        Some(&JsonValue::Unsigned(1_250))
    );
    assert_eq!(
        state.get("build_version"),
        Some(&JsonValue::String(get_version_string().to_owned()))
    );
    assert!(matches!(state.get("uptime"), Some(JsonValue::Unsigned(_))));
    assert_eq!(state.get("io_latency_ms"), Some(&JsonValue::Unsigned(11)));
    assert_eq!(
        state.get("server_domain"),
        Some(&JsonValue::String("status.example.com".to_owned()))
    );
    assert_eq!(
        state.get("complete_ledgers"),
        Some(&JsonValue::String("90,100-101".to_owned()))
    );
    assert_eq!(state.get("fetch_pack"), Some(&JsonValue::Unsigned(1)));
    assert!(!state.contains_key("hostid"));
    assert!(!state.contains_key("node_size"));
    assert!(!state.contains_key("pubkey_validator"));
    assert!(!state.contains_key("git"));
    assert!(!state.contains_key("validator_list_expires"));
    assert!(!state.contains_key("load"));
    let JsonValue::Object(closed_ledger) = state
        .get("closed_ledger")
        .expect("closed_ledger must exist")
    else {
        panic!("closed_ledger must be object");
    };
    assert_eq!(closed_ledger.get("seq"), Some(&JsonValue::Unsigned(100)));
    assert_eq!(closed_ledger.get("base_fee"), Some(&JsonValue::Signed(10)));
    assert_eq!(
        closed_ledger.get("reserve_base"),
        Some(&JsonValue::Signed(2_000_000))
    );
    assert_eq!(
        closed_ledger.get("reserve_inc"),
        Some(&JsonValue::Signed(200_000))
    );
    assert_eq!(
        closed_ledger.get("close_time"),
        Some(&JsonValue::Unsigned(1_000))
    );
    assert_eq!(
        state.get("published_ledger"),
        Some(&JsonValue::String("none".to_owned()))
    );
    let JsonValue::Object(state_accounting) = state
        .get("state_accounting")
        .expect("state accounting must exist")
    else {
        panic!("state_accounting must be object");
    };
    let JsonValue::Object(tracking_state) = state_accounting
        .get("tracking")
        .expect("tracking must exist")
    else {
        panic!("tracking must be object");
    };
    assert_eq!(
        tracking_state.get("duration_us"),
        Some(&JsonValue::String("40".to_owned()))
    );
    assert_eq!(
        state.get("server_state_duration_us"),
        Some(&JsonValue::String("60".to_owned()))
    );
    assert_eq!(
        state.get("initial_sync_duration_us"),
        Some(&JsonValue::String("70".to_owned()))
    );
    let JsonValue::Array(ports) = state.get("ports").expect("ports must exist") else {
        panic!("ports must be an array");
    };
    assert_eq!(ports.len(), 2);
    let JsonValue::Object(http_port) = &ports[0] else {
        panic!("port entry must be object");
    };
    assert_eq!(
        http_port.get("port"),
        Some(&JsonValue::String("5005".to_owned()))
    );
    let JsonValue::Object(grpc_port) = &ports[1] else {
        panic!("grpc port entry must be object");
    };
    assert_eq!(
        grpc_port.get("port"),
        Some(&JsonValue::String("50051".to_owned()))
    );
}
