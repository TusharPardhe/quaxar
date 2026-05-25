//! server info tests part A.

use super::*;

#[test]
fn server_info_wraps_the_source_result() {
    let params = object([("counters", JsonValue::Bool(true))]);
    let source = FakeServerInfoSource::default();
    let result = do_server_info(&context(&params, &source, RpcRole::Admin));
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };

    let JsonValue::Object(info) = result.get("info").expect("info must exist") else {
        panic!("info must be an object");
    };
    assert_eq!(info.get("human"), Some(&JsonValue::Bool(true)));
    assert_eq!(info.get("admin"), Some(&JsonValue::Bool(true)));
    assert_eq!(info.get("counters"), Some(&JsonValue::Bool(true)));
    assert_eq!(source.calls.borrow().as_slice(), &[(true, true, true)]);
}

#[test]
fn server_info_defaults_counters_to_false() {
    let params = JsonValue::Object(Default::default());
    let source = FakeServerInfoSource::default();
    let result = do_server_info(&context(&params, &source, RpcRole::User));
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };

    let JsonValue::Object(info) = result.get("info").expect("info must exist") else {
        panic!("info must be an object");
    };
    assert_eq!(info.get("human"), Some(&JsonValue::Bool(true)));
    assert_eq!(info.get("admin"), Some(&JsonValue::Bool(false)));
    assert_eq!(info.get("counters"), Some(&JsonValue::Bool(false)));
    assert_eq!(source.calls.borrow().as_slice(), &[(true, false, false)]);
}

#[test]
fn server_info_reads_application_owner_state_boundary() {
    let mut app = ApplicationRoot::new(0).expect("root shell should build");
    app.set_network_ops_operating_mode(NetworkOpsOperatingMode::Full);
    app.set_need_network_ledger(true);
    app.set_amendment_blocked(true);
    app.set_unl_blocked(true);
    app.load_fee_track().set_remote_fee(512);
    app.load_fee_track().set_cluster_fee(384);
    app.set_unsupported_majority_warning_details(Some(UnsupportedMajorityWarningDetails {
        expected_date: 1_700_000_000,
        expected_date_utc: "2023-Nov-14 22:13:20 UTC".to_owned(),
    }));
    assert!(!app.load_fee_track().raise_local_fee());
    assert!(app.load_fee_track().raise_local_fee());
    let overlay = Arc::new(
        OverlayImpl::new(overlay_setup(Some(21_338)), Arc::new(TestHandoff)).expect("overlay"),
    );
    let first = overlay_peer(1, 11);
    let second = overlay_peer(2, 12);
    let third = overlay_peer(3, 13);
    overlay.activate(Arc::clone(&first));
    overlay.activate(Arc::clone(&second));
    overlay.activate(Arc::clone(&third));
    overlay.inc_jq_trans_overflow();
    overlay.inc_jq_trans_overflow();
    overlay.inc_jq_trans_overflow();
    overlay.inc_jq_trans_overflow();
    overlay.inc_peer_disconnect();
    overlay.inc_peer_disconnect();
    overlay.inc_peer_disconnect();
    overlay.inc_peer_disconnect();
    overlay.inc_peer_disconnect();
    overlay.inc_peer_disconnect_charges();
    overlay.inc_peer_disconnect_charges();
    overlay.inc_peer_disconnect_charges();
    overlay.inc_peer_disconnect_charges();
    overlay.inc_peer_disconnect_charges();
    overlay.inc_peer_disconnect_charges();
    let overlay_status: Arc<dyn app::OverlayStatusSource> = overlay.clone();
    app.attach_overlay_status(overlay_status);
    attach_test_server_ports(
        &mut app,
        Some(PublishedGrpcPort {
            ip: "127.0.0.1".to_owned(),
            port: "50051".to_owned(),
        }),
    );
    let perf_log = make_test_perf_log(
        &["server_info", "ping"],
        TestPerfLogReportSource {
            nodestore: json!({
                "node_reads_total": "9"
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
    app.attach_status_metrics(perf_log.clone());
    app.set_status_rpc_peer_count(Some(99));
    app.set_status_rpc_network_id(Some(99));
    app.set_status_rpc_last_close(Some(StatusRpcLastClose {
        proposers: 5,
        converge_time: Duration::from_millis(1_500),
    }));
    app.set_status_rpc_hostid(Some("host-a".to_owned()));
    app.set_status_rpc_server_domain(Some("status.example.com".to_owned()));
    app.set_status_rpc_node_size(Some("huge".to_owned()));
    app.set_status_rpc_io_latency_ms(Some(19));
    app.set_status_rpc_complete_ledgers(Some("98-99,101".to_owned()));
    app.set_status_rpc_fetch_pack(Some(1));
    app.set_status_rpc_git_info(Some(StatusRpcGitInfo {
        hash: Some("abc123".to_owned()),
        branch: Some("main".to_owned()),
    }));
    app.set_status_rpc_queue_report(Some(tx::QueueTxQRpcReport {
        ledger_current_index: 101,
        expected_ledger_size: "32".to_owned(),
        current_ledger_size: "31".to_owned(),
        current_queue_size: "4".to_owned(),
        max_queue_size: Some("200".to_owned()),
        levels: tx::QueueTxQRpcLevels {
            reference_level: "256".to_owned(),
            minimum_level: "512".to_owned(),
            median_level: "640".to_owned(),
            open_ledger_level: "768".to_owned(),
        },
        drops: tx::QueueTxQRpcDrops {
            base_fee: "10".to_owned(),
            median_fee: "16".to_owned(),
            minimum_fee: "12".to_owned(),
            open_ledger_fee: "20".to_owned(),
        },
    }));
    app.set_node_identity((
        PublicKey::from_bytes([0x02; 33]),
        protocol::SecretKey::from_bytes([0x11; 32]),
    ));
    let closed = sample_ledger(100, 1_000, 0x11);
    let validated = sample_ledger(101, 1_005, 0x22);
    let published = sample_ledger(99, 999, 0x33);
    app.on_closed_ledger(closed);
    app.on_validated_ledger(validated);
    app.on_published_ledger(published);
    let now_close_time = app.time_keeper().close_time().as_seconds();
    app.ledger_master_state()
        .set_validated_close_time(now_close_time.saturating_sub(20));

    let source = ApplicationServerInfo::new(&app);
    let result = do_server_info(&context(&object([]), &source, RpcRole::Admin));
    let JsonValue::Object(result) = result else {
        panic!("result must be an object");
    };
    let JsonValue::Object(info) = result.get("info").expect("info must exist") else {
        panic!("info must be an object");
    };

    assert_eq!(
        info.get("server_state"),
        Some(&JsonValue::String("full".to_owned()))
    );
    assert_eq!(
        info.get("network_ledger"),
        Some(&JsonValue::String("waiting".to_owned()))
    );
    assert_eq!(info.get("amendment_blocked"), Some(&JsonValue::Bool(true)));
    assert_eq!(
        info.get("hostid"),
        Some(&JsonValue::String("host-a".to_owned()))
    );
    assert_eq!(
        info.get("server_domain"),
        Some(&JsonValue::String("status.example.com".to_owned()))
    );
    assert_eq!(info.get("validation_quorum"), Some(&JsonValue::Unsigned(1)));
    let JsonValue::Object(validator_list) = info
        .get("validator_list")
        .expect("validator_list must exist for admin")
    else {
        panic!("validator_list must be an object");
    };
    assert_eq!(validator_list.get("count"), Some(&JsonValue::Unsigned(0)));
    assert_eq!(
        validator_list.get("status"),
        Some(&JsonValue::String("unknown".to_owned()))
    );
    assert_eq!(
        validator_list.get("expiration"),
        Some(&JsonValue::String("unknown".to_owned()))
    );
    assert_eq!(info.get("peers"), Some(&JsonValue::Unsigned(3)));
    assert_eq!(info.get("network_id"), Some(&JsonValue::Unsigned(21338)));
    assert_eq!(
        info.get("jq_trans_overflow"),
        Some(&JsonValue::String("4".to_owned()))
    );
    assert_eq!(
        info.get("peer_disconnects"),
        Some(&JsonValue::String("5".to_owned()))
    );
    assert_eq!(
        info.get("peer_disconnects_resources"),
        Some(&JsonValue::String("6".to_owned()))
    );
    let Some(JsonValue::String(server_time)) = info.get("time") else {
        panic!("time must be a string");
    };
    assert!(server_time.ends_with(" UTC"));
    let JsonValue::Object(last_close) = info.get("last_close").expect("last_close must exist")
    else {
        panic!("last_close must be an object");
    };
    assert_eq!(last_close.get("proposers"), Some(&JsonValue::Unsigned(5)));
    assert_eq!(
        last_close.get("converge_time_s"),
        Some(&JsonValue::String("1.5".to_owned()))
    );
    assert_eq!(
        info.get("load_factor"),
        Some(&JsonValue::String("3".to_owned()))
    );
    assert_eq!(
        info.get("load_factor_server"),
        Some(&JsonValue::String("2.5".to_owned()))
    );
    assert_eq!(
        info.get("load_factor_fee_escalation"),
        Some(&JsonValue::String("3".to_owned()))
    );
    assert_eq!(
        info.get("load_factor_fee_queue"),
        Some(&JsonValue::String("2".to_owned()))
    );
    assert_eq!(
        info.get("load_factor_local"),
        Some(&JsonValue::String("2.5".to_owned()))
    );
    assert_eq!(
        info.get("load_factor_net"),
        Some(&JsonValue::String("2".to_owned()))
    );
    assert_eq!(
        info.get("load_factor_cluster"),
        Some(&JsonValue::String("1.5".to_owned()))
    );
    assert_eq!(
        info.get("node_size"),
        Some(&JsonValue::String("huge".to_owned()))
    );
    assert_eq!(info.get("io_latency_ms"), Some(&JsonValue::Unsigned(19)));
    assert_eq!(
        info.get("pubkey_validator"),
        Some(&JsonValue::String("none".to_owned()))
    );
    assert_eq!(
        info.get("build_version"),
        Some(&JsonValue::String(get_version_string().to_owned()))
    );
    assert!(matches!(info.get("uptime"), Some(JsonValue::Unsigned(_))));
    let JsonValue::Object(git) = info.get("git").expect("git must exist") else {
        panic!("git must be object");
    };
    assert_eq!(
        git.get("hash"),
        Some(&JsonValue::String("abc123".to_owned()))
    );
    assert_eq!(
        git.get("branch"),
        Some(&JsonValue::String("main".to_owned()))
    );
    assert_eq!(
        info.get("complete_ledgers"),
        Some(&JsonValue::String("98-99,101".to_owned()))
    );
    assert_eq!(info.get("fetch_pack"), Some(&JsonValue::Unsigned(1)));
    let JsonValue::Object(load) = info.get("load").expect("load must exist for admin") else {
        panic!("load must be object");
    };
    assert_eq!(load.get("job_count"), Some(&JsonValue::Unsigned(0)));
    assert_eq!(load.get("process_count"), Some(&JsonValue::Unsigned(0)));
    assert_eq!(load.get("overloaded"), Some(&JsonValue::Bool(false)));
    let JsonValue::Object(validated_ledger) = info
        .get("validated_ledger")
        .expect("validated ledger must exist")
    else {
        panic!("validated_ledger must be an object");
    };
    assert_eq!(validated_ledger.get("seq"), Some(&JsonValue::Unsigned(101)));
    assert_eq!(
        validated_ledger.get("base_fee_xrp"),
        Some(&JsonValue::String("0.00001".to_owned()))
    );
    assert_eq!(
        validated_ledger.get("reserve_base_xrp"),
        Some(&JsonValue::String("2".to_owned()))
    );
    assert_eq!(
        validated_ledger.get("reserve_inc_xrp"),
        Some(&JsonValue::String("0.2".to_owned()))
    );
    assert!(matches!(
        validated_ledger.get("age"),
        Some(JsonValue::Unsigned(_))
    ));
    assert_eq!(info.get("published_ledger"), Some(&JsonValue::Unsigned(99)));

    let JsonValue::Array(warnings) = info.get("warnings").expect("warnings") else {
        panic!("warnings must be an array");
    };
    assert_eq!(warnings.len(), 3);
    let JsonValue::Object(first) = &warnings[0] else {
        panic!("warning must be object");
    };
    let JsonValue::Object(second) = &warnings[1] else {
        panic!("warning must be object");
    };
    assert_eq!(
        first.get("id"),
        Some(&JsonValue::Signed(WARN_RPC_AMENDMENT_BLOCKED))
    );
    assert_eq!(
        second.get("id"),
        Some(&JsonValue::Signed(WARN_RPC_EXPIRED_VALIDATOR_LIST))
    );
    let JsonValue::Object(third) = &warnings[2] else {
        panic!("warning must be object");
    };
    assert_eq!(
        third.get("id"),
        Some(&JsonValue::Signed(WARN_RPC_UNSUPPORTED_MAJORITY))
    );
    let JsonValue::Object(details) = third.get("details").expect("details must exist") else {
        panic!("details must be an object");
    };
    assert_eq!(
        details.get("expected_date"),
        Some(&JsonValue::Signed(1_700_000_000))
    );
    assert_eq!(
        details.get("expected_date_UTC"),
        Some(&JsonValue::String("2023-Nov-14 22:13:20 UTC".to_owned()))
    );
    let JsonValue::Object(state_accounting) = info
        .get("state_accounting")
        .expect("state accounting must exist")
    else {
        panic!("state_accounting must be an object");
    };
    let JsonValue::Object(full_state) = state_accounting.get("full").expect("full must exist")
    else {
        panic!("full state must be an object");
    };
    assert_eq!(
        full_state.get("duration_us"),
        Some(&JsonValue::String("50".to_owned()))
    );
    assert_eq!(
        info.get("server_state_duration_us"),
        Some(&JsonValue::String("60".to_owned()))
    );
    assert_eq!(
        info.get("initial_sync_duration_us"),
        Some(&JsonValue::String("70".to_owned()))
    );
    assert!(state_accounting.contains_key("full"));
    let JsonValue::Array(ports) = info.get("ports").expect("ports must exist") else {
        panic!("ports must be an array");
    };
    assert_eq!(ports.len(), 3);
    let JsonValue::Object(http_port) = &ports[0] else {
        panic!("port entry must be object");
    };
    assert_eq!(
        http_port.get("port"),
        Some(&JsonValue::String("5005".to_owned()))
    );
    let JsonValue::Array(protocols) = http_port.get("protocol").expect("protocol array") else {
        panic!("protocol must be an array");
    };
    assert_eq!(
        protocols,
        &vec![
            JsonValue::String("http".to_owned()),
            JsonValue::String("ws".to_owned())
        ]
    );
    let JsonValue::Object(peer_port) = &ports[1] else {
        panic!("port entry must be object");
    };
    assert_eq!(
        peer_port.get("port"),
        Some(&JsonValue::String("6006".to_owned()))
    );
    let JsonValue::Object(grpc_port) = &ports[2] else {
        panic!("grpc port entry must be object");
    };
    assert_eq!(
        grpc_port.get("port"),
        Some(&JsonValue::String("50051".to_owned()))
    );
    assert!(info.contains_key("pubkey_node"));
}
