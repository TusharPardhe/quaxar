use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use app::{
    ApplicationRoot, NetworkOpsOperatingMode, OverlayStatusSnapshot, OverlayStatusSource,
    PublishedGrpcPort, PublishedServerPort, PublishedServerPortsSource, StatusMetricsSource,
    StatusRpcGitInfo, StatusRpcLastClose, UnsupportedMajorityWarningDetails,
};
use ledger::{Fees, Ledger, LedgerHeader};
use protocol::{JsonValue, PublicKey};
use serde_json::json;
use server::{
    BuiltinDispatcher, RequestMetadata, RpcDispatcher, RpcRequest, ServerAuth, ServerAuthConfig,
    StreamKind, SubscriptionEvent, SubscriptionManager, WSSession, from_protocol_json,
    to_protocol_json,
};
use tokio::sync::mpsc;

#[derive(Debug, Clone, Copy)]
struct FixedOverlayStatusSource {
    snapshot: OverlayStatusSnapshot,
}

impl OverlayStatusSource for FixedOverlayStatusSource {
    fn status_snapshot(&self) -> OverlayStatusSnapshot {
        self.snapshot
    }
}

#[derive(Debug, Clone)]
struct FixedStatusMetricsSource {
    counters: serde_json::Value,
    current_activities: serde_json::Value,
    nodestore: serde_json::Value,
    state_accounting: serde_json::Value,
    server_state_duration_us: Option<String>,
    initial_sync_duration_us: Option<String>,
}

#[derive(Debug, Clone)]
struct FixedPublishedServerPortsSource {
    ports: Vec<PublishedServerPort>,
    grpc: Option<PublishedGrpcPort>,
}

impl StatusMetricsSource for FixedStatusMetricsSource {
    fn counters_json(&self) -> serde_json::Value {
        self.counters.clone()
    }

    fn current_activities_json(&self) -> serde_json::Value {
        self.current_activities.clone()
    }

    fn nodestore_counts_json(&self) -> serde_json::Value {
        self.nodestore.clone()
    }

    fn state_accounting_json(&self) -> serde_json::Value {
        self.state_accounting.clone()
    }

    fn server_state_duration_us(&self) -> Option<String> {
        self.server_state_duration_us.clone()
    }

    fn initial_sync_duration_us(&self) -> Option<String> {
        self.initial_sync_duration_us.clone()
    }
}

impl PublishedServerPortsSource for FixedPublishedServerPortsSource {
    fn published_server_ports(&self) -> Vec<PublishedServerPort> {
        self.ports.clone()
    }

    fn published_grpc_port(&self) -> Option<PublishedGrpcPort> {
        self.grpc.clone()
    }
}

fn sample_ledger(seq: u32, close_time: u32, hash_byte: u8) -> Arc<Ledger> {
    let mut ledger = Ledger::from_ledger_seq_and_close_time(seq, close_time, false);
    ledger.set_ledger_info(LedgerHeader {
        hash: basics::sha_map_hash::SHAMapHash::new(basics::base_uint::Uint256::from_array(
            [hash_byte; 32],
        )),
        ..ledger.header()
    });
    ledger.set_fees(Fees {
        base: 10,
        reserve: 2_000_000,
        increment: 200_000,
    });
    Arc::new(ledger)
}

#[test]
fn auth_request_role_detects_admin_and_gateway_roles() {
    let auth = ServerAuth::new(ServerAuthConfig {
        user: None,
        password: None,
        admin_user: Some("alice".to_owned()),
        admin_password: Some("secret".to_owned()),
        admin_nets_v4: vec![
            "127.0.0.0/8"
                .parse()
                .expect("loopback admin network should parse"),
        ],
        admin_nets_v6: Vec::new(),
        secure_gateway_nets_v4: vec![
            "10.0.0.0/8"
                .parse()
                .expect("gateway admin network should parse"),
        ],
        secure_gateway_nets_v6: Vec::new(),
    });

    let request = http::Request::builder()
        .method("POST")
        .uri("/")
        .body(axum::body::Body::from(Vec::<u8>::new()))
        .expect("request should build");
    let mut metadata = RequestMetadata::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 51234),
        &request,
    );
    metadata.user = "alice".to_owned();
    metadata.forwarded_for = "10.1.2.3".to_owned();

    let role = server::request_role(
        rpc::RpcRole::User,
        &auth,
        &metadata,
        &JsonValue::Object(BTreeMap::from([
            (
                "admin_user".to_owned(),
                JsonValue::String("alice".to_owned()),
            ),
            (
                "admin_password".to_owned(),
                JsonValue::String("secret".to_owned()),
            ),
        ])),
        "alice",
    );

    assert_eq!(role, rpc::RpcRole::Admin);
}

#[test]
fn request_metadata_preserves_header_values_for_rpc_handoff() {
    let request = http::Request::builder()
        .method("POST")
        .uri("/")
        .header("Forwarded", "for=203.0.113.9")
        .header("X-User", "alice")
        .body(axum::body::Body::from(Vec::<u8>::new()))
        .expect("request should build");
    let metadata = RequestMetadata::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 51234),
        &request,
    );

    assert_eq!(
        metadata.request_headers().get("forwarded"),
        Some(&"for=203.0.113.9".to_owned())
    );
    assert_eq!(
        metadata.request_headers().get("x-user"),
        Some(&"alice".to_owned())
    );
}

#[tokio::test]
async fn websocket_subscription_fanout_emits_json_text() {
    let manager = Arc::new(SubscriptionManager::new(8));
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let request = http::Request::builder()
        .method("GET")
        .uri("/")
        .body(axum::body::Body::from(Vec::<u8>::new()))
        .expect("request should build");
    let metadata = RequestMetadata::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 51234),
        &request,
    );
    let session = WSSession::new(1, metadata, sender, manager.clone());

    session.subscribe_stream(StreamKind::Transactions);
    manager.publish(SubscriptionEvent {
        stream: StreamKind::Transactions,
        payload: bytes::Bytes::from(sonic_rs::to_vec(&JsonValue::Object(BTreeMap::from([(
            "transaction".to_owned(),
            JsonValue::String("abc123".to_owned()),
        )]))).unwrap()),
    });

    let message = receiver
        .recv()
        .await
        .expect("session should emit a message");
    let text = match message {
        axum::extract::ws::Message::Text(text) => text,
        other => panic!("unexpected ws message: {other:?}"),
    };
    let emitted: serde_json::Value = serde_json::from_str(&text).expect("valid json text");
    assert_eq!(
        emitted["transaction"],
        serde_json::Value::String("abc123".to_owned())
    );
}

#[test]
fn builtin_dispatcher_routes_ping_and_server_info() {
    let mut app = ApplicationRoot::new(0).expect("root shell should build");
    app.set_network_ops_operating_mode(NetworkOpsOperatingMode::Tracking);
    app.set_unsupported_majority_warning_details(Some(UnsupportedMajorityWarningDetails {
        expected_date: 1_700_000_000,
        expected_date_utc: "2023-Nov-14 22:13:20 UTC".to_owned(),
    }));
    app.attach_status_metrics(Arc::new(FixedStatusMetricsSource {
        counters: json!({
            "rpc": {
                "server_info": {
                    "started": "1"
                }
            }
        }),
        current_activities: json!({
            "jobs": [{"job": "transaction"}],
            "methods": [{"method": "server_info"}]
        }),
        nodestore: json!({
            "read_threads_running": 1
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
    }));
    app.set_status_rpc_current_ledger_index(Some(101));
    app.attach_overlay_status(Arc::new(FixedOverlayStatusSource {
        snapshot: OverlayStatusSnapshot {
            peers: 9,
            network_id: Some(2_048),
            jq_trans_overflow: 3,
            peer_disconnects: 4,
            peer_disconnect_charges: 5,
        },
    }));
    app.attach_published_server_ports(Arc::new(FixedPublishedServerPortsSource {
        ports: vec![
            PublishedServerPort {
                port: "5005".to_owned(),
                protocols: vec!["ws".to_owned(), "http".to_owned(), "grpc".to_owned()],
                admin_nets_v4_configured: false,
                admin_nets_v6_configured: false,
                admin_user: None,
                admin_password: None,
            },
            PublishedServerPort {
                port: "6006".to_owned(),
                protocols: vec!["peer".to_owned()],
                admin_nets_v4_configured: true,
                admin_nets_v6_configured: false,
                admin_user: Some("rpc".to_owned()),
                admin_password: Some("secret".to_owned()),
            },
        ],
        grpc: Some(PublishedGrpcPort {
            ip: "127.0.0.1".to_owned(),
            port: "50051".to_owned(),
        }),
    }));
    app.set_status_rpc_peer_count(Some(99));
    app.set_status_rpc_network_id(Some(99));
    app.set_status_rpc_last_close(Some(StatusRpcLastClose {
        proposers: 6,
        converge_time: Duration::from_millis(1_250),
    }));
    app.set_status_rpc_hostid(Some("host-dispatch".to_owned()));
    app.set_status_rpc_server_domain(Some("status.example.com".to_owned()));
    app.set_status_rpc_node_size(Some("large".to_owned()));
    app.set_status_rpc_io_latency_ms(Some(13));
    app.set_status_rpc_complete_ledgers(Some("100-101".to_owned()));
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
            minimum_level: "300".to_owned(),
            median_level: "400".to_owned(),
            open_ledger_level: "500".to_owned(),
        },
        drops: tx::QueueTxQRpcDrops {
            base_fee: "10".to_owned(),
            median_fee: "16".to_owned(),
            minimum_fee: "12".to_owned(),
            open_ledger_fee: "20".to_owned(),
        },
    }));
    app.on_closed_ledger(sample_ledger(100, 1_000, 0x11));
    let dispatcher = BuiltinDispatcher::new(
        rpc::ApplicationServerInfo::new(&app),
        SubscriptionManager::default(),
    );
    let request = http::Request::builder()
        .method("POST")
        .uri("/")
        .body(axum::body::Body::from(Vec::<u8>::new()))
        .expect("request should build");
    let mut metadata = RequestMetadata::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 51234),
        &request,
    );
    metadata.role = rpc::RpcRole::Admin;
    metadata.unlimited = true;

    let ping = dispatcher.dispatch(RpcRequest {
        method: "ping",
        params: &JsonValue::Object(BTreeMap::new()),
        metadata: &metadata,
        session: None,
    });
    let ping = match ping {
        server::RpcReply::Result(value) => value,
        other => panic!("unexpected rpc reply: {other:?}"),
    };
    assert_eq!(
        from_protocol_json(&ping)["role"],
        serde_json::Value::String("admin".to_owned())
    );
    assert_eq!(
        from_protocol_json(&ping)["unlimited"],
        serde_json::Value::Bool(true)
    );

    let server_info = dispatcher.dispatch(RpcRequest {
        method: "server_info",
        params: &JsonValue::Object(BTreeMap::new()),
        metadata: &metadata,
        session: None,
    });
    let server_info = match server_info {
        server::RpcReply::Result(value) => value,
        other => panic!("unexpected rpc reply: {other:?}"),
    };
    assert_eq!(
        from_protocol_json(&server_info)["info"]["server_state"],
        serde_json::Value::String("tracking".to_owned())
    );
    assert_eq!(
        from_protocol_json(&server_info)["info"]["peers"],
        serde_json::Value::Number(9_u64.into())
    );
    assert_eq!(
        from_protocol_json(&server_info)["info"]["jq_trans_overflow"],
        serde_json::Value::String("3".to_owned())
    );
    assert_eq!(
        from_protocol_json(&server_info)["info"]["peer_disconnects"],
        serde_json::Value::String("4".to_owned())
    );
    assert_eq!(
        from_protocol_json(&server_info)["info"]["peer_disconnects_resources"],
        serde_json::Value::String("5".to_owned())
    );
    assert_eq!(
        from_protocol_json(&server_info)["info"]["hostid"],
        serde_json::Value::String("host-dispatch".to_owned())
    );
    assert_eq!(
        from_protocol_json(&server_info)["info"]["node_size"],
        serde_json::Value::String("large".to_owned())
    );
    assert_eq!(
        from_protocol_json(&server_info)["info"]["server_domain"],
        serde_json::Value::String("status.example.com".to_owned())
    );
    assert_eq!(
        from_protocol_json(&server_info)["info"]["validation_quorum"],
        serde_json::Value::Number(1_u64.into())
    );
    assert_eq!(
        from_protocol_json(&server_info)["info"]["validator_list"]["status"],
        serde_json::Value::String("unknown".to_owned())
    );
    assert_eq!(
        from_protocol_json(&server_info)["info"]["io_latency_ms"],
        serde_json::Value::Number(13_u64.into())
    );
    assert_eq!(
        from_protocol_json(&server_info)["info"]["pubkey_validator"],
        serde_json::Value::String("none".to_owned())
    );
    assert_eq!(
        from_protocol_json(&server_info)["info"]["git"]["hash"],
        serde_json::Value::String("abc123".to_owned())
    );
    assert_eq!(
        from_protocol_json(&server_info)["info"]["complete_ledgers"],
        serde_json::Value::String("100-101".to_owned())
    );
    assert_eq!(
        from_protocol_json(&server_info)["info"]["fetch_pack"],
        serde_json::Value::Number(1_u64.into())
    );
    assert_eq!(
        from_protocol_json(&server_info)["info"]["load"]["job_count"],
        serde_json::Value::Number(0_u64.into())
    );
    assert_eq!(
        from_protocol_json(&server_info)["info"]["last_close"]["converge_time_s"],
        serde_json::Value::String("1.25".to_owned())
    );
    assert_eq!(
        from_protocol_json(&server_info)["info"]["closed_ledger"]["seq"],
        serde_json::Value::Number(100_u64.into())
    );
    assert_eq!(
        from_protocol_json(&server_info)["info"]["warnings"][0]["details"]["expected_date_UTC"],
        serde_json::Value::String("2023-Nov-14 22:13:20 UTC".to_owned())
    );
    assert_eq!(
        from_protocol_json(&server_info)["info"]["state_accounting"]["full"]["duration_us"],
        serde_json::Value::String("50".to_owned())
    );
    assert_eq!(
        from_protocol_json(&server_info)["info"]["server_state_duration_us"],
        serde_json::Value::String("60".to_owned())
    );
    assert_eq!(
        from_protocol_json(&server_info)["info"]["ports"][0]["protocol"][0],
        serde_json::Value::String("http".to_owned())
    );
    assert_eq!(
        from_protocol_json(&server_info)["info"]["ports"][2]["protocol"][0],
        serde_json::Value::String("grpc".to_owned())
    );
    let server_info_time = from_protocol_json(&server_info)["info"]["time"]
        .as_str()
        .expect("time must be a string")
        .to_owned();
    assert!(server_info_time.ends_with(" UTC"));

    let server_info_counters = dispatcher.dispatch(RpcRequest {
        method: "server_info",
        params: &JsonValue::Object(BTreeMap::from([(
            "counters".to_owned(),
            JsonValue::Bool(true),
        )])),
        metadata: &metadata,
        session: None,
    });
    let server_info_counters = match server_info_counters {
        server::RpcReply::Result(value) => value,
        other => panic!("unexpected rpc reply: {other:?}"),
    };
    assert_eq!(
        from_protocol_json(&server_info_counters)["info"]["counters"]["rpc"]["server_info"]["started"],
        serde_json::Value::String("1".to_owned())
    );
    assert_eq!(
        from_protocol_json(&server_info_counters)["info"]["counters"]["nodestore"]["read_threads_running"],
        serde_json::Value::Number(1_u64.into())
    );
    assert_eq!(
        from_protocol_json(&server_info_counters)["info"]["current_activities"]["methods"][0]["method"],
        serde_json::Value::String("server_info".to_owned())
    );

    let server_state = dispatcher.dispatch(RpcRequest {
        method: "server_state",
        params: &JsonValue::Object(BTreeMap::new()),
        metadata: &metadata,
        session: None,
    });
    let server_state = match server_state {
        server::RpcReply::Result(value) => value,
        other => panic!("unexpected rpc reply: {other:?}"),
    };
    assert_eq!(
        from_protocol_json(&server_state)["state"]["server_state"],
        serde_json::Value::String("tracking".to_owned())
    );
    assert_eq!(
        from_protocol_json(&server_state)["state"]["network_id"],
        serde_json::Value::Number(2048_u64.into())
    );
    assert_eq!(
        from_protocol_json(&server_state)["state"]["jq_trans_overflow"],
        serde_json::Value::String("3".to_owned())
    );
    assert_eq!(
        from_protocol_json(&server_state)["state"]["peer_disconnects"],
        serde_json::Value::String("4".to_owned())
    );
    assert_eq!(
        from_protocol_json(&server_state)["state"]["peer_disconnects_resources"],
        serde_json::Value::String("5".to_owned())
    );
    assert_eq!(
        from_protocol_json(&server_state)["state"]["validation_quorum"],
        serde_json::Value::Number(1_u64.into())
    );
    assert_eq!(
        from_protocol_json(&server_state)["state"]["validator_list_expires"],
        serde_json::Value::Number(0_u64.into())
    );
    assert_eq!(
        from_protocol_json(&server_state)["state"]["load_factor_fee_reference"],
        serde_json::Value::Number(256_u64.into())
    );
    assert_eq!(
        from_protocol_json(&server_state)["state"]["io_latency_ms"],
        serde_json::Value::Number(13_u64.into())
    );
    assert_eq!(
        from_protocol_json(&server_state)["state"]["node_size"],
        serde_json::Value::String("large".to_owned())
    );
    assert_eq!(
        from_protocol_json(&server_state)["state"]["pubkey_validator"],
        serde_json::Value::String("none".to_owned())
    );
    assert_eq!(
        from_protocol_json(&server_state)["state"]["complete_ledgers"],
        serde_json::Value::String("100-101".to_owned())
    );
    assert_eq!(
        from_protocol_json(&server_state)["state"]["fetch_pack"],
        serde_json::Value::Number(1_u64.into())
    );
    assert_eq!(
        from_protocol_json(&server_state)["state"]["load"]["job_count"],
        serde_json::Value::Number(0_u64.into())
    );
    assert_eq!(
        from_protocol_json(&server_state)["state"]["closed_ledger"]["close_time"],
        serde_json::Value::Number(1_000_u64.into())
    );
    assert_eq!(
        from_protocol_json(&server_state)["state"]["state_accounting"]["tracking"]["duration_us"],
        serde_json::Value::String("40".to_owned())
    );
    assert_eq!(
        from_protocol_json(&server_state)["state"]["initial_sync_duration_us"],
        serde_json::Value::String("70".to_owned())
    );
    assert_eq!(
        from_protocol_json(&server_state)["state"]["ports"][2]["protocol"][0],
        serde_json::Value::String("grpc".to_owned())
    );
    let server_state_time = from_protocol_json(&server_state)["state"]["time"]
        .as_str()
        .expect("time must be a string")
        .to_owned();
    assert!(server_state_time.ends_with(" UTC"));

    let server_state_counters = dispatcher.dispatch(RpcRequest {
        method: "server_state",
        params: &JsonValue::Object(BTreeMap::from([(
            "counters".to_owned(),
            JsonValue::Bool(true),
        )])),
        metadata: &metadata,
        session: None,
    });
    let server_state_counters = match server_state_counters {
        server::RpcReply::Result(value) => value,
        other => panic!("unexpected rpc reply: {other:?}"),
    };
    assert_eq!(
        from_protocol_json(&server_state_counters)["state"]["counters"]["rpc"]["server_info"]["started"],
        serde_json::Value::String("1".to_owned())
    );
    assert_eq!(
        from_protocol_json(&server_state_counters)["state"]["current_activities"]["jobs"][0]["job"],
        serde_json::Value::String("transaction".to_owned())
    );

    let fee = dispatcher.dispatch(RpcRequest {
        method: "fee",
        params: &JsonValue::Object(BTreeMap::new()),
        metadata: &metadata,
        session: None,
    });
    let fee = match fee {
        server::RpcReply::Result(value) => value,
        other => panic!("unexpected rpc reply: {other:?}"),
    };
    assert_eq!(
        from_protocol_json(&fee)["ledger_current_index"],
        serde_json::Value::Number(101_u64.into())
    );
    assert_eq!(
        from_protocol_json(&fee)["drops"]["open_ledger_fee"],
        serde_json::Value::String("20".to_owned())
    );

    let ledger_current = dispatcher.dispatch(RpcRequest {
        method: "ledger_current",
        params: &JsonValue::Object(BTreeMap::new()),
        metadata: &metadata,
        session: None,
    });
    let ledger_current = match ledger_current {
        server::RpcReply::Result(value) => value,
        other => panic!("unexpected rpc reply: {other:?}"),
    };
    assert_eq!(
        from_protocol_json(&ledger_current)["ledger_current_index"],
        serde_json::Value::Number(101_u64.into())
    );

    let ledger_closed = dispatcher.dispatch(RpcRequest {
        method: "ledger_closed",
        params: &JsonValue::Object(BTreeMap::new()),
        metadata: &metadata,
        session: None,
    });
    let ledger_closed = match ledger_closed {
        server::RpcReply::Result(value) => value,
        other => panic!("unexpected rpc reply: {other:?}"),
    };
    assert_eq!(
        from_protocol_json(&ledger_closed)["ledger_index"],
        serde_json::Value::Number(100_u64.into())
    );
}

#[test]
fn builtin_dispatcher_surfaces_path_find_not_synced_gate_before_runtime_wiring() {
    let app = ApplicationRoot::new(0).expect("root shell should build");
    let dispatcher = BuiltinDispatcher::new(
        rpc::ApplicationServerInfo::new(&app),
        SubscriptionManager::default(),
    );
    let request = http::Request::builder()
        .method("POST")
        .uri("/")
        .body(axum::body::Body::from(Vec::<u8>::new()))
        .expect("request should build");
    let metadata = RequestMetadata::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 51234),
        &request,
    );

    let reply = dispatcher.dispatch(RpcRequest {
        method: "path_find",
        params: &JsonValue::Object(BTreeMap::from([
            (
                "subcommand".to_owned(),
                JsonValue::String("create".to_owned()),
            ),
            (
                "source_account".to_owned(),
                JsonValue::String(protocol::to_base58(protocol::AccountID::from_array(
                    [1; 20],
                ))),
            ),
            (
                "destination_account".to_owned(),
                JsonValue::String(protocol::to_base58(protocol::AccountID::from_array(
                    [2; 20],
                ))),
            ),
            (
                "destination_amount".to_owned(),
                JsonValue::String("1000000".to_owned()),
            ),
        ])),
        metadata: &metadata,
        session: None,
    });

    let server::RpcReply::Result(reply) = reply else {
        panic!("path_find reply must be a result");
    };
    assert_eq!(
        from_protocol_json(&reply)["error"],
        serde_json::Value::String("noNetwork".to_owned())
    );
}

#[test]
fn builtin_dispatcher_reports_no_events_for_http_path_find_without_runtime_wiring() {
    let mut app = ApplicationRoot::new(0).expect("root shell should build");
    app.set_network_ops_operating_mode(NetworkOpsOperatingMode::Tracking);
    app.set_status_rpc_current_ledger_index(Some(100));
    app.set_path_search_max(3);
    let dispatcher = BuiltinDispatcher::new(
        rpc::ApplicationServerInfo::new(&app),
        SubscriptionManager::default(),
    );
    let request = http::Request::builder()
        .method("POST")
        .uri("/")
        .body(axum::body::Body::from(Vec::<u8>::new()))
        .expect("request should build");
    let metadata = RequestMetadata::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 51234),
        &request,
    );

    let reply = dispatcher.dispatch(RpcRequest {
        method: "path_find",
        params: &JsonValue::Object(BTreeMap::from([
            (
                "subcommand".to_owned(),
                JsonValue::String("create".to_owned()),
            ),
            (
                "source_account".to_owned(),
                JsonValue::String(protocol::to_base58(protocol::AccountID::from_array(
                    [1; 20],
                ))),
            ),
            (
                "destination_account".to_owned(),
                JsonValue::String(protocol::to_base58(protocol::AccountID::from_array(
                    [2; 20],
                ))),
            ),
            (
                "destination_amount".to_owned(),
                JsonValue::String("1000000".to_owned()),
            ),
        ])),
        metadata: &metadata,
        session: None,
    });

    let server::RpcReply::Result(reply) = reply else {
        panic!("path_find reply must be a result");
    };
    assert_eq!(
        from_protocol_json(&reply)["error"],
        serde_json::Value::String("noEvents".to_owned())
    );
}

#[test]
fn builtin_dispatcher_reports_invalid_params_for_malformed_path_find_subcommand() {
    let mut app = ApplicationRoot::new(0).expect("root shell should build");
    app.set_network_ops_operating_mode(NetworkOpsOperatingMode::Tracking);
    app.set_status_rpc_current_ledger_index(Some(100));
    app.set_path_search_max(3);
    let dispatcher = BuiltinDispatcher::new(
        rpc::ApplicationServerInfo::new(&app),
        SubscriptionManager::default(),
    );
    let request = http::Request::builder()
        .method("POST")
        .uri("/")
        .body(axum::body::Body::from(Vec::<u8>::new()))
        .expect("request should build");
    let metadata = RequestMetadata::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 51234),
        &request,
    );

    let reply = dispatcher.dispatch(RpcRequest {
        method: "path_find",
        params: &JsonValue::Object(BTreeMap::from([
            ("subcommand".to_owned(), JsonValue::Unsigned(1)),
            (
                "source_account".to_owned(),
                JsonValue::String(protocol::to_base58(protocol::AccountID::from_array(
                    [1; 20],
                ))),
            ),
            (
                "destination_account".to_owned(),
                JsonValue::String(protocol::to_base58(protocol::AccountID::from_array(
                    [2; 20],
                ))),
            ),
            (
                "destination_amount".to_owned(),
                JsonValue::String("1000000".to_owned()),
            ),
        ])),
        metadata: &metadata,
        session: None,
    });

    let server::RpcReply::Result(reply) = reply else {
        panic!("path_find reply must be a result");
    };
    assert_eq!(
        from_protocol_json(&reply)["error"],
        serde_json::Value::String("invalidParams".to_owned())
    );
}

#[test]
fn builtin_dispatcher_reports_no_path_request_for_status_without_runtime_wiring() {
    let mut app = ApplicationRoot::new(0).expect("root shell should build");
    app.set_network_ops_operating_mode(NetworkOpsOperatingMode::Tracking);
    app.set_status_rpc_current_ledger_index(Some(100));
    app.set_path_search_max(3);
    let dispatcher = BuiltinDispatcher::new(
        rpc::ApplicationServerInfo::new(&app),
        SubscriptionManager::default(),
    );
    let request = http::Request::builder()
        .method("POST")
        .uri("/")
        .body(axum::body::Body::from(Vec::<u8>::new()))
        .expect("request should build");
    let metadata = RequestMetadata::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 51234),
        &request,
    );
    let mut ws_metadata = metadata.clone();
    ws_metadata.is_websocket = true;
    let (sender, _receiver) = mpsc::unbounded_channel();
    let session = WSSession::new(
        7,
        ws_metadata,
        sender,
        Arc::new(SubscriptionManager::new(8)),
    );

    let reply = dispatcher.dispatch(RpcRequest {
        method: "path_find",
        params: &JsonValue::Object(BTreeMap::from([(
            "subcommand".to_owned(),
            JsonValue::String("status".to_owned()),
        )])),
        metadata: &metadata,
        session: Some(&session),
    });

    let server::RpcReply::Result(reply) = reply else {
        panic!("path_find reply must be a result");
    };
    assert_eq!(
        from_protocol_json(&reply)["error"],
        serde_json::Value::String("noPathRequest".to_owned())
    );
}

#[test]
fn builtin_dispatcher_reports_not_supported_for_unwired_path_find_create() {
    let mut app = ApplicationRoot::new(0).expect("root shell should build");
    app.set_network_ops_operating_mode(NetworkOpsOperatingMode::Tracking);
    app.set_status_rpc_current_ledger_index(Some(100));
    app.set_path_search_max(3);
    let dispatcher = BuiltinDispatcher::new(
        rpc::ApplicationServerInfo::new(&app),
        SubscriptionManager::default(),
    );
    let request = http::Request::builder()
        .method("POST")
        .uri("/")
        .body(axum::body::Body::from(Vec::<u8>::new()))
        .expect("request should build");
    let metadata = RequestMetadata::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 51234),
        &request,
    );
    let mut ws_metadata = metadata.clone();
    ws_metadata.is_websocket = true;
    let (sender, _receiver) = mpsc::unbounded_channel();
    let session = WSSession::new(
        7,
        ws_metadata,
        sender,
        Arc::new(SubscriptionManager::new(8)),
    );

    let reply = dispatcher.dispatch(RpcRequest {
        method: "path_find",
        params: &JsonValue::Object(BTreeMap::from([
            (
                "subcommand".to_owned(),
                JsonValue::String("create".to_owned()),
            ),
            (
                "source_account".to_owned(),
                JsonValue::String(protocol::to_base58(protocol::AccountID::from_array(
                    [1; 20],
                ))),
            ),
            (
                "destination_account".to_owned(),
                JsonValue::String(protocol::to_base58(protocol::AccountID::from_array(
                    [2; 20],
                ))),
            ),
            (
                "destination_amount".to_owned(),
                JsonValue::String("1000000".to_owned()),
            ),
        ])),
        metadata: &metadata,
        session: Some(&session),
    });
    let server::RpcReply::Result(reply) = reply else {
        panic!("path_find reply must be a result");
    };
    assert_eq!(
        from_protocol_json(&reply)["error"],
        serde_json::Value::String("notSupported".to_owned())
    );

    let ripple_path_find = dispatcher.dispatch(RpcRequest {
        method: "ripple_path_find",
        params: &JsonValue::Object(BTreeMap::from([
            (
                "source_account".to_owned(),
                JsonValue::String(protocol::to_base58(protocol::AccountID::from_array(
                    [1; 20],
                ))),
            ),
            (
                "destination_account".to_owned(),
                JsonValue::String(protocol::to_base58(protocol::AccountID::from_array(
                    [2; 20],
                ))),
            ),
            (
                "destination_amount".to_owned(),
                JsonValue::String("1000000".to_owned()),
            ),
        ])),
        metadata: &metadata,
        session: Some(&session),
    });
    let server::RpcReply::Result(ripple_path_find) = ripple_path_find else {
        panic!("ripple_path_find reply must be a result");
    };
    assert_eq!(
        from_protocol_json(&ripple_path_find)["error"],
        serde_json::Value::String("notSupported".to_owned())
    );
}

#[test]
fn builtin_dispatcher_reports_not_supported_when_path_search_is_disabled() {
    let mut app = ApplicationRoot::new(0).expect("root shell should build");
    app.set_network_ops_operating_mode(NetworkOpsOperatingMode::Tracking);
    app.set_status_rpc_current_ledger_index(Some(100));
    app.set_path_search_max(0);
    let dispatcher = BuiltinDispatcher::new(
        rpc::ApplicationServerInfo::new(&app),
        SubscriptionManager::default(),
    );
    let request = http::Request::builder()
        .method("POST")
        .uri("/")
        .body(axum::body::Body::from(Vec::<u8>::new()))
        .expect("request should build");
    let metadata = RequestMetadata::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 51234),
        &request,
    );

    let path_find = dispatcher.dispatch(RpcRequest {
        method: "path_find",
        params: &JsonValue::Object(BTreeMap::from([
            (
                "subcommand".to_owned(),
                JsonValue::String("create".to_owned()),
            ),
            (
                "source_account".to_owned(),
                JsonValue::String(protocol::to_base58(protocol::AccountID::from_array(
                    [1; 20],
                ))),
            ),
            (
                "destination_account".to_owned(),
                JsonValue::String(protocol::to_base58(protocol::AccountID::from_array(
                    [2; 20],
                ))),
            ),
            (
                "destination_amount".to_owned(),
                JsonValue::String("1000000".to_owned()),
            ),
        ])),
        metadata: &metadata,
        session: None,
    });
    let server::RpcReply::Result(path_find) = path_find else {
        panic!("path_find reply must be a result");
    };
    assert_eq!(
        from_protocol_json(&path_find)["error"],
        serde_json::Value::String("notSupported".to_owned())
    );

    let ripple_path_find = dispatcher.dispatch(RpcRequest {
        method: "ripple_path_find",
        params: &JsonValue::Object(BTreeMap::from([
            (
                "source_account".to_owned(),
                JsonValue::String(protocol::to_base58(protocol::AccountID::from_array(
                    [1; 20],
                ))),
            ),
            (
                "destination_account".to_owned(),
                JsonValue::String(protocol::to_base58(protocol::AccountID::from_array(
                    [2; 20],
                ))),
            ),
            (
                "destination_amount".to_owned(),
                JsonValue::String("1000000".to_owned()),
            ),
        ])),
        metadata: &metadata,
        session: None,
    });
    let server::RpcReply::Result(ripple_path_find) = ripple_path_find else {
        panic!("ripple_path_find reply must be a result");
    };
    assert_eq!(
        from_protocol_json(&ripple_path_find)["error"],
        serde_json::Value::String("notSupported".to_owned())
    );
}

#[test]
fn builtin_dispatcher_routes_path_find_to_app_source_after_runtime_wiring() {
    let mut app = ApplicationRoot::new(0).expect("root shell should build");
    app.set_network_ops_operating_mode(NetworkOpsOperatingMode::Tracking);
    app.set_status_rpc_current_ledger_index(Some(100));
    app.set_path_search_max(3);

    let source = rpc::ApplicationServerInfo::new(
        rpc::OwnedApplicationServerInfo::from_application_root(&app),
    );
    let path_source: Arc<dyn rpc::PathFinderSource + Send + Sync> = Arc::new(source.clone());
    let dispatcher = BuiltinDispatcher::new(source, SubscriptionManager::default())
        .with_path_find(Arc::new(rpc::PathRequestManager::new()), path_source);
    let request = http::Request::builder()
        .method("POST")
        .uri("/")
        .body(axum::body::Body::from(Vec::<u8>::new()))
        .expect("request should build");
    let metadata = RequestMetadata::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 51234),
        &request,
    );
    let mut ws_metadata = metadata.clone();
    ws_metadata.is_websocket = true;
    let (sender, _receiver) = mpsc::unbounded_channel();
    let session = WSSession::new(
        7,
        ws_metadata,
        sender,
        Arc::new(SubscriptionManager::new(8)),
    );

    let reply = dispatcher.dispatch(RpcRequest {
        method: "path_find",
        params: &JsonValue::Object(BTreeMap::from([
            (
                "subcommand".to_owned(),
                JsonValue::String("create".to_owned()),
            ),
            (
                "source_account".to_owned(),
                JsonValue::String(protocol::to_base58(protocol::AccountID::from_array(
                    [1; 20],
                ))),
            ),
            (
                "destination_account".to_owned(),
                JsonValue::String(protocol::to_base58(protocol::AccountID::from_array(
                    [2; 20],
                ))),
            ),
            (
                "destination_amount".to_owned(),
                JsonValue::String("1000000".to_owned()),
            ),
        ])),
        metadata: &metadata,
        session: Some(&session),
    });

    let server::RpcReply::Result(reply) = reply else {
        panic!("path_find reply must be a result");
    };
    assert!(from_protocol_json(&reply)["error"].is_null());
    assert_eq!(
        from_protocol_json(&reply)["source_account"],
        serde_json::Value::String(protocol::to_base58(protocol::AccountID::from_array(
            [1; 20]
        )))
    );
    assert_eq!(
        from_protocol_json(&reply)["destination_account"],
        serde_json::Value::String(protocol::to_base58(protocol::AccountID::from_array(
            [2; 20]
        )))
    );
    assert_eq!(
        from_protocol_json(&reply)["alternatives"][0]["source_amount"],
        serde_json::Value::String("1000000".to_owned())
    );
}

#[test]
fn builtin_dispatcher_routes_ripple_path_find_with_legacy_shape_after_runtime_wiring() {
    let mut app = ApplicationRoot::new(0).expect("root shell should build");
    app.set_network_ops_operating_mode(NetworkOpsOperatingMode::Tracking);
    app.set_status_rpc_current_ledger_index(Some(100));
    app.set_path_search_max(3);

    let source = rpc::ApplicationServerInfo::new(
        rpc::OwnedApplicationServerInfo::from_application_root(&app),
    );
    let path_source: Arc<dyn rpc::PathFinderSource + Send + Sync> = Arc::new(source.clone());
    let dispatcher = BuiltinDispatcher::new(source, SubscriptionManager::default())
        .with_path_find(Arc::new(rpc::PathRequestManager::new()), path_source);
    let request = http::Request::builder()
        .method("POST")
        .uri("/")
        .body(axum::body::Body::from(Vec::<u8>::new()))
        .expect("request should build");
    let metadata = RequestMetadata::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 51234),
        &request,
    );

    let reply = dispatcher.dispatch(RpcRequest {
        method: "ripple_path_find",
        params: &JsonValue::Object(BTreeMap::from([
            (
                "source_account".to_owned(),
                JsonValue::String(protocol::to_base58(protocol::AccountID::from_array(
                    [1; 20],
                ))),
            ),
            (
                "destination_account".to_owned(),
                JsonValue::String(protocol::to_base58(protocol::AccountID::from_array(
                    [2; 20],
                ))),
            ),
            (
                "destination_amount".to_owned(),
                JsonValue::String("1000000".to_owned()),
            ),
        ])),
        metadata: &metadata,
        session: None,
    });

    let server::RpcReply::Result(reply) = reply else {
        panic!("ripple_path_find reply must be a result");
    };
    assert!(from_protocol_json(&reply)["error"].is_null());
    assert!(
        from_protocol_json(&reply)["destination_currencies"]
            .as_array()
            .expect("legacy destination currencies should be present")
            .is_empty()
    );
    assert_eq!(
        from_protocol_json(&reply)["alternatives"][0]["source_amount"],
        serde_json::Value::String("1000000".to_owned())
    );
}

#[test]
fn builtin_dispatcher_rejects_command_method_mismatches() {
    let app = ApplicationRoot::new(0).expect("root shell should build");
    let dispatcher = BuiltinDispatcher::new(
        rpc::ApplicationServerInfo::new(&app),
        SubscriptionManager::default(),
    );
    let request = http::Request::builder()
        .method("POST")
        .uri("/")
        .body(axum::body::Body::from(Vec::<u8>::new()))
        .expect("request should build");
    let metadata = RequestMetadata::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 51234),
        &request,
    );

    let reply = dispatcher.dispatch(RpcRequest {
        method: "server_info",
        params: &JsonValue::Object(BTreeMap::from([(
            "method".to_owned(),
            JsonValue::String("server_state".to_owned()),
        )])),
        metadata: &metadata,
        session: None,
    });

    let server::RpcReply::Result(reply) = reply else {
        panic!("mismatched request reply must be a result");
    };
    assert_eq!(
        from_protocol_json(&reply)["error"],
        serde_json::Value::String("unknownCmd".to_owned())
    );
}

#[test]
fn builtin_dispatcher_reports_unknown_command_for_registered_but_unwired_handlers() {
    let app = ApplicationRoot::new(0).expect("root shell should build");
    let dispatcher = BuiltinDispatcher::new(
        rpc::ApplicationServerInfo::new(&app),
        SubscriptionManager::default(),
    );
    let request = http::Request::builder()
        .method("POST")
        .uri("/")
        .body(axum::body::Body::from(Vec::<u8>::new()))
        .expect("request should build");
    let metadata = RequestMetadata::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 51234),
        &request,
    );

    let reply = dispatcher.dispatch(RpcRequest {
        method: "ledger",
        params: &JsonValue::Object(BTreeMap::new()),
        metadata: &metadata,
        session: None,
    });

    let server::RpcReply::Result(reply) = reply else {
        panic!("ledger reply must be a result");
    };
    assert_eq!(
        from_protocol_json(&reply)["error"],
        serde_json::Value::String("lgrNotFound".to_owned())
    );
}

#[test]
fn builtin_dispatcher_reports_static_validator_status() {
    let mut app = ApplicationRoot::new(0).expect("root shell should build");
    let local_signing_key = PublicKey::from_bytes([0x02; 33]);
    assert!(app.validators().load(
        Some(local_signing_key),
        &["n949f75evCHwgyP4fPVgaHqNHxUVN15PsJEZ3B3HnXPcPjcZAoy7".to_owned()],
        &[],
        None,
    ));
    app.set_validation_public_key(PublicKey::from_bytes([0x03; 33]));
    let local_public_key = app
        .validators()
        .local_public_key()
        .expect("local validator key should be set");

    let dispatcher = BuiltinDispatcher::new(
        rpc::ApplicationServerInfo::new(&app),
        SubscriptionManager::default(),
    );
    let request = http::Request::builder()
        .method("POST")
        .uri("/")
        .body(axum::body::Body::from(Vec::<u8>::new()))
        .expect("request should build");
    let mut metadata = RequestMetadata::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 51234),
        &request,
    );
    metadata.role = rpc::RpcRole::Admin;

    let server_info = dispatcher.dispatch(RpcRequest {
        method: "server_info",
        params: &JsonValue::Object(BTreeMap::new()),
        metadata: &metadata,
        session: None,
    });
    let server_info = match server_info {
        server::RpcReply::Result(value) => value,
        other => panic!("unexpected rpc reply: {other:?}"),
    };
    assert_eq!(
        from_protocol_json(&server_info)["info"]["validator_list"]["count"],
        serde_json::Value::Number(1_u64.into())
    );
    assert_eq!(
        from_protocol_json(&server_info)["info"]["validator_list"]["expiration"],
        serde_json::Value::String("never".to_owned())
    );
    assert_eq!(
        from_protocol_json(&server_info)["info"]["validator_list"]["status"],
        serde_json::Value::String("active".to_owned())
    );
    assert!(
        from_protocol_json(&server_info)["info"]["validator_list"]
            .get("validator_list_threshold")
            .is_none()
    );
    assert_eq!(
        from_protocol_json(&server_info)["info"]["pubkey_validator"],
        serde_json::Value::String(local_public_key.to_node_public_base58())
    );
    assert!(
        from_protocol_json(&server_info)["info"]
            .get("validator_list_expires")
            .is_none()
    );

    let user_request = http::Request::builder()
        .method("POST")
        .uri("/")
        .body(axum::body::Body::from(Vec::<u8>::new()))
        .expect("request should build");
    let mut user_metadata = RequestMetadata::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 51235),
        &user_request,
    );
    user_metadata.role = rpc::RpcRole::User;

    let user_info = dispatcher.dispatch(RpcRequest {
        method: "server_info",
        params: &JsonValue::Object(BTreeMap::new()),
        metadata: &user_metadata,
        session: None,
    });
    let user_info = match user_info {
        server::RpcReply::Result(value) => value,
        other => panic!("unexpected rpc reply: {other:?}"),
    };
    assert!(
        from_protocol_json(&user_info)["info"]
            .get("validator_list")
            .is_none()
    );
    assert!(
        from_protocol_json(&user_info)["info"]
            .get("pubkey_validator")
            .is_none()
    );

    let server_state = dispatcher.dispatch(RpcRequest {
        method: "server_state",
        params: &JsonValue::Object(BTreeMap::new()),
        metadata: &metadata,
        session: None,
    });
    let server_state = match server_state {
        server::RpcReply::Result(value) => value,
        other => panic!("unexpected rpc reply: {other:?}"),
    };
    assert_eq!(
        from_protocol_json(&server_state)["state"]["validator_list_expires"],
        serde_json::Value::Number((u32::MAX as u64).into())
    );
    assert_eq!(
        from_protocol_json(&server_state)["state"]["pubkey_validator"],
        serde_json::Value::String(local_public_key.to_node_public_base58())
    );
    assert!(
        from_protocol_json(&server_state)["state"]
            .get("validator_list")
            .is_none()
    );

    let user_state = dispatcher.dispatch(RpcRequest {
        method: "server_state",
        params: &JsonValue::Object(BTreeMap::new()),
        metadata: &user_metadata,
        session: None,
    });
    let user_state = match user_state {
        server::RpcReply::Result(value) => value,
        other => panic!("unexpected rpc reply: {other:?}"),
    };
    assert!(
        from_protocol_json(&user_state)["state"]
            .get("validator_list_expires")
            .is_none()
    );
    assert!(
        from_protocol_json(&user_state)["state"]
            .get("pubkey_validator")
            .is_none()
    );
}

#[test]
fn json_conversion_round_trips_nested_objects() {
    let source = serde_json::json!({
        "method": "subscribe",
        "streams": ["ledger", "transactions"],
        "id": 5,
        "nested": {"flag": true},
    });
    let round_trip = from_protocol_json(&to_protocol_json(source.clone()));
    assert_eq!(round_trip, source);
}

#[tokio::test]
async fn websocket_subscription_ledger_stream_receives_ledger_closed_event() {
    let manager = Arc::new(SubscriptionManager::new(8));
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let request = http::Request::builder()
        .method("GET")
        .uri("/")
        .body(axum::body::Body::from(Vec::<u8>::new()))
        .expect("request should build");
    let metadata = RequestMetadata::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 51234),
        &request,
    );
    let session = WSSession::new(2, metadata, sender, manager.clone());

    session.subscribe_stream(StreamKind::Ledger);

    // Simulate ledger close event
    manager.publish(SubscriptionEvent {
        stream: StreamKind::Ledger,
        payload: bytes::Bytes::from(sonic_rs::to_vec(&JsonValue::Object(BTreeMap::from([
            (
                "type".to_owned(),
                JsonValue::String("ledgerClosed".to_owned()),
            ),
            ("ledger_index".to_owned(), JsonValue::Unsigned(42)),
            (
                "ledger_hash".to_owned(),
                JsonValue::String("ABCD".repeat(16)),
            ),
            ("txn_count".to_owned(), JsonValue::Unsigned(5)),
        ]))).unwrap()),
    });

    let message = receiver
        .recv()
        .await
        .expect("session should emit a ledger message");
    let text = match message {
        axum::extract::ws::Message::Text(text) => text,
        other => panic!("unexpected ws message: {other:?}"),
    };
    let emitted: serde_json::Value = serde_json::from_str(&text).expect("valid json");
    assert_eq!(emitted["type"], "ledgerClosed");
    assert_eq!(emitted["ledger_index"], 42);
    assert_eq!(emitted["txn_count"], 5);
}

#[tokio::test]
async fn websocket_subscription_server_stream_receives_fee_change() {
    let manager = Arc::new(SubscriptionManager::new(8));
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let request = http::Request::builder()
        .method("GET")
        .uri("/")
        .body(axum::body::Body::from(Vec::<u8>::new()))
        .expect("request should build");
    let metadata = RequestMetadata::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 51235),
        &request,
    );
    let session = WSSession::new(3, metadata, sender, manager.clone());

    session.subscribe_stream(StreamKind::Server);

    // Simulate server status event
    manager.publish(SubscriptionEvent {
        stream: StreamKind::Server,
        payload: bytes::Bytes::from(sonic_rs::to_vec(&JsonValue::Object(BTreeMap::from([
            (
                "type".to_owned(),
                JsonValue::String("serverStatus".to_owned()),
            ),
            ("load_base".to_owned(), JsonValue::Unsigned(256)),
            ("load_factor".to_owned(), JsonValue::Unsigned(512)),
        ]))).unwrap()),
    });

    let message = receiver
        .recv()
        .await
        .expect("session should emit a server message");
    let text = match message {
        axum::extract::ws::Message::Text(text) => text,
        other => panic!("unexpected ws message: {other:?}"),
    };
    let emitted: serde_json::Value = serde_json::from_str(&text).expect("valid json");
    assert_eq!(emitted["type"], "serverStatus");
    assert_eq!(emitted["load_base"], 256);
    assert_eq!(emitted["load_factor"], 512);
}

#[tokio::test]
async fn websocket_subscription_unsubscribe_stops_receiving() {
    let manager = Arc::new(SubscriptionManager::new(8));
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let request = http::Request::builder()
        .method("GET")
        .uri("/")
        .body(axum::body::Body::from(Vec::<u8>::new()))
        .expect("request should build");
    let metadata = RequestMetadata::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 51236),
        &request,
    );
    let session = WSSession::new(4, metadata, sender, manager.clone());

    session.subscribe_stream(StreamKind::Transactions);
    session.unsubscribe_stream(StreamKind::Transactions);

    // Publish after unsubscribe
    manager.publish(SubscriptionEvent {
        stream: StreamKind::Transactions,
        payload: bytes::Bytes::from(sonic_rs::to_vec(&JsonValue::Object(BTreeMap::from([(
            "type".to_owned(),
            JsonValue::String("transaction".to_owned()),
        )]))).unwrap()),
    });

    // Give a moment for any message to arrive
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Should NOT receive anything
    assert!(
        receiver.try_recv().is_err(),
        "should not receive after unsubscribe"
    );
}

#[tokio::test]
async fn websocket_subscription_multiple_streams_independent() {
    let manager = Arc::new(SubscriptionManager::new(8));
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let request = http::Request::builder()
        .method("GET")
        .uri("/")
        .body(axum::body::Body::from(Vec::<u8>::new()))
        .expect("request should build");
    let metadata = RequestMetadata::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 51237),
        &request,
    );
    let session = WSSession::new(5, metadata, sender, manager.clone());

    session.subscribe_stream(StreamKind::Ledger);
    session.subscribe_stream(StreamKind::Server);

    // Publish to ledger
    manager.publish(SubscriptionEvent {
        stream: StreamKind::Ledger,
        payload: bytes::Bytes::from(sonic_rs::to_vec(&JsonValue::Object(BTreeMap::from([(
            "type".to_owned(),
            JsonValue::String("ledgerClosed".to_owned()),
        )]))).unwrap()),
    });

    // Publish to server
    manager.publish(SubscriptionEvent {
        stream: StreamKind::Server,
        payload: bytes::Bytes::from(sonic_rs::to_vec(&JsonValue::Object(BTreeMap::from([(
            "type".to_owned(),
            JsonValue::String("serverStatus".to_owned()),
        )]))).unwrap()),
    });

    // Should receive both
    let msg1 = receiver.recv().await.expect("first message");
    let msg2 = receiver.recv().await.expect("second message");

    let text1 = match msg1 {
        axum::extract::ws::Message::Text(t) => t,
        other => panic!("unexpected: {other:?}"),
    };
    let text2 = match msg2 {
        axum::extract::ws::Message::Text(t) => t,
        other => panic!("unexpected: {other:?}"),
    };

    let json1: serde_json::Value = serde_json::from_str(&text1).expect("json");
    let json2: serde_json::Value = serde_json::from_str(&text2).expect("json");

    let types: Vec<&str> = vec![
        json1["type"].as_str().unwrap(),
        json2["type"].as_str().unwrap(),
    ];
    assert!(types.contains(&"ledgerClosed"));
    assert!(types.contains(&"serverStatus"));
}

#[tokio::test]
async fn websocket_subscription_does_not_receive_unsubscribed_stream() {
    let manager = Arc::new(SubscriptionManager::new(8));
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let request = http::Request::builder()
        .method("GET")
        .uri("/")
        .body(axum::body::Body::from(Vec::<u8>::new()))
        .expect("request should build");
    let metadata = RequestMetadata::new(
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 51238),
        &request,
    );
    let session = WSSession::new(6, metadata, sender, manager.clone());

    // Only subscribe to ledger, not transactions
    session.subscribe_stream(StreamKind::Ledger);

    // Publish to transactions (not subscribed)
    manager.publish(SubscriptionEvent {
        stream: StreamKind::Transactions,
        payload: bytes::Bytes::from(sonic_rs::to_vec(&JsonValue::Object(BTreeMap::from([(
            "type".to_owned(),
            JsonValue::String("transaction".to_owned()),
        )]))).unwrap()),
    });

    // Publish to ledger (subscribed)
    manager.publish(SubscriptionEvent {
        stream: StreamKind::Ledger,
        payload: bytes::Bytes::from(sonic_rs::to_vec(&JsonValue::Object(BTreeMap::from([(
            "type".to_owned(),
            JsonValue::String("ledgerClosed".to_owned()),
        )]))).unwrap()),
    });

    // Should only receive ledger event
    let message = receiver.recv().await.expect("should receive ledger");
    let text = match message {
        axum::extract::ws::Message::Text(t) => t,
        other => panic!("unexpected: {other:?}"),
    };
    let json: serde_json::Value = serde_json::from_str(&text).expect("json");
    assert_eq!(json["type"], "ledgerClosed");
}
