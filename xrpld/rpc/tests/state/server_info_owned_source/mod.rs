mod server_info_state_support;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use app::{
    ApplicationRoot, NetworkOpsOperatingMode, PublishedGrpcPort, ServerPortSetup, ServerPortsSetup,
    ServiceRegistry,
};
use perflog::{JobType, PerfLog};
use protocol::JsonValue;
use rpc::{
    ApplicationServerInfo, JsonContext, JsonContextHeaders, OwnedApplicationServerInfo, RpcRole,
    RpcRuntime, do_server_info, do_server_state,
};
use server_info_state_support::{TestPerfLogReportSource, make_test_perf_log};
use tx::{QueueTxQRpcDrops, QueueTxQRpcLevels, QueueTxQRpcReport};

fn sample_open_tx() -> Arc<protocol::STTx> {
    Arc::new(protocol::STTx::new(protocol::TxType::OFFER_CREATE, |_| {}))
}

fn context<'a, Env>(params: &'a JsonValue, env: &'a Env, role: RpcRole) -> JsonContext<'a, Env> {
    JsonContext {
        params,
        env,
        role,
        api_version: 2,
        headers: JsonContextHeaders::default(),
        unlimited: false,
    }
}

fn strip_live_time(value: JsonValue) -> JsonValue {
    match value {
        JsonValue::Object(mut object) => {
            object.remove("time");
            object.remove("server_state_duration_us");
            object.remove("duration_us");
            JsonValue::Object(
                object
                    .into_iter()
                    .map(|(key, value)| (key, strip_live_time(value)))
                    .collect(),
            )
        }
        JsonValue::Array(values) => {
            JsonValue::Array(values.into_iter().map(strip_live_time).collect())
        }
        other => other,
    }
}

fn sample_queue_report() -> QueueTxQRpcReport {
    QueueTxQRpcReport {
        ledger_current_index: 321,
        expected_ledger_size: "12".to_owned(),
        current_ledger_size: "9".to_owned(),
        current_queue_size: "3".to_owned(),
        max_queue_size: Some("5".to_owned()),
        levels: QueueTxQRpcLevels {
            reference_level: "256".to_owned(),
            minimum_level: "128".to_owned(),
            median_level: "192".to_owned(),
            open_ledger_level: "384".to_owned(),
        },
        drops: QueueTxQRpcDrops {
            base_fee: "10".to_owned(),
            median_fee: "12".to_owned(),
            minimum_fee: "8".to_owned(),
            open_ledger_fee: "16".to_owned(),
        },
    }
}

#[test]
fn borrowed_and_owned_application_server_info_match_for_server_info_and_state() {
    let mut app = ApplicationRoot::new(0).expect("root shell should build");
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
        }],
        client: None,
        overlay: None,
        grpc: Some(PublishedGrpcPort {
            ip: "127.0.0.1".to_owned(),
            port: "50051".to_owned(),
        }),
    }));
    let perf_log = make_test_perf_log(
        &["server_info", "server_state"],
        TestPerfLogReportSource {
            nodestore: serde_json::json!({
                "node_reads_total": "9"
            }),
            state_accounting: serde_json::json!({
                "full": {"transitions": "1", "duration_us": "10"}
            }),
            server_state_duration_us: Some("10".to_owned()),
            initial_sync_duration_us: None,
        },
    );
    perf_log.resize_jobs(1);
    perf_log.job_queue(JobType::Transaction);
    perf_log.job_start(
        JobType::Transaction,
        Duration::from_micros(10),
        Instant::now(),
        0,
    );
    perf_log.rpc_start("server_info", 1);
    app.attach_status_metrics(perf_log.clone());
    assert!(app.status_metrics().is_some());
    app.set_status_rpc_current_ledger_index(Some(321));
    app.set_status_rpc_queue_report(Some(sample_queue_report()));
    app.set_status_rpc_hostid(Some("host-1".to_owned()));
    app.set_status_rpc_server_domain(Some("example.com".to_owned()));
    app.set_status_rpc_node_size(Some("large".to_owned()));
    app.set_status_rpc_io_latency_ms(Some(42));
    app.set_status_rpc_complete_ledgers(Some("1-5".to_owned()));
    app.set_status_rpc_fetch_pack(Some(7));
    app.set_need_network_ledger(true);
    app.set_amendment_blocked(true);
    app.set_unl_blocked(true);
    app.set_path_search_levels(4, 5, 3);
    app.set_path_search_max(8);

    let params = JsonValue::Object(BTreeMap::new());
    let borrowed = ApplicationServerInfo::new(&app);
    let owned = ApplicationServerInfo::new(OwnedApplicationServerInfo::from_application_root(&app));
    assert_eq!(borrowed.path_search_old(), 4);
    assert_eq!(borrowed.path_search(), 5);
    assert_eq!(borrowed.path_search_fast(), 3);
    assert_eq!(borrowed.path_search_max(), 8);
    assert_eq!(owned.path_search_old(), 4);
    assert_eq!(owned.path_search(), 5);
    assert_eq!(owned.path_search_fast(), 3);
    assert_eq!(owned.path_search_max(), 8);

    let borrowed_info = do_server_info(&context(&params, &borrowed, RpcRole::Admin));
    let owned_info = do_server_info(&context(&params, &owned, RpcRole::Admin));
    assert_eq!(strip_live_time(borrowed_info), strip_live_time(owned_info));

    let borrowed_state = do_server_state(&context(&params, &borrowed, RpcRole::Admin));
    let owned_state = do_server_state(&context(&params, &owned, RpcRole::Admin));
    assert_eq!(
        strip_live_time(borrowed_state),
        strip_live_time(owned_state)
    );
}

#[test]
fn borrowed_and_owned_application_server_info_share_live_txq_fallback_when_snapshot_missing() {
    let app = ApplicationRoot::new(0).expect("root shell should build");
    app.set_network_ops_operating_mode(NetworkOpsOperatingMode::Full);
    assert_eq!(app.status_rpc_queue_report(), None);

    let changed = ServiceRegistry::get_open_ledger(&app).modify(|next| {
        next.ledger_current_index = 654;
        next.base_fee_drops = 19;
        next.push_transaction(sample_open_tx());
        true
    });
    assert!(changed);

    let params = JsonValue::Object(BTreeMap::new());
    let borrowed = ApplicationServerInfo::new(&app);
    let owned = ApplicationServerInfo::new(OwnedApplicationServerInfo::from_application_root(&app));

    let borrowed_fee = rpc::do_fee(&borrowed);
    let owned_fee = rpc::do_fee(&owned);
    assert_eq!(borrowed_fee, owned_fee);

    let borrowed_info = do_server_info(&context(&params, &borrowed, RpcRole::Admin));
    let owned_info = do_server_info(&context(&params, &owned, RpcRole::Admin));
    assert_eq!(strip_live_time(borrowed_info), strip_live_time(owned_info));
}

#[test]
fn borrowed_and_owned_application_server_info_share_live_current_ledger_fallback_when_snapshot_missing()
 {
    let app = ApplicationRoot::new(0).expect("root shell should build");
    app.set_network_ops_operating_mode(NetworkOpsOperatingMode::Full);
    assert_eq!(app.status_rpc_current_ledger_index(), None);

    let changed = ServiceRegistry::get_open_ledger(&app).modify(|next| {
        next.ledger_current_index = 4321;
        next.base_fee_drops = 13;
        true
    });
    assert!(changed);

    let borrowed = ApplicationServerInfo::new(&app);
    let owned = ApplicationServerInfo::new(OwnedApplicationServerInfo::from_application_root(&app));

    assert_eq!(
        rpc::do_ledger_current(&borrowed),
        rpc::do_ledger_current(&owned)
    );
}

#[test]
fn owned_application_server_info_tracks_late_live_open_ledger_updates() {
    let app = ApplicationRoot::new(0).expect("root shell should build");
    app.set_network_ops_operating_mode(NetworkOpsOperatingMode::Full);
    let owned = ApplicationServerInfo::new(OwnedApplicationServerInfo::from_application_root(&app));

    let first_change = ServiceRegistry::get_open_ledger(&app).modify(|next| {
        next.ledger_current_index = 5000;
        next.base_fee_drops = 21;
        next.push_transaction(sample_open_tx());
        true
    });
    assert!(first_change);

    assert_eq!(
        rpc::do_ledger_current(&owned),
        protocol::JsonValue::Object(BTreeMap::from([(
            "ledger_current_index".to_owned(),
            protocol::JsonValue::Unsigned(5000),
        )]))
    );

    let protocol::JsonValue::Object(fee) = rpc::do_fee(&owned) else {
        panic!("fee response must be object");
    };
    assert_eq!(
        fee.get("ledger_current_index"),
        Some(&protocol::JsonValue::Unsigned(5000))
    );

    let second_change = ServiceRegistry::get_open_ledger(&app).modify(|next| {
        next.ledger_current_index = 5001;
        next.base_fee_drops = 22;
        next.push_transaction(sample_open_tx());
        true
    });
    assert!(second_change);

    assert_eq!(
        rpc::do_ledger_current(&owned),
        protocol::JsonValue::Object(BTreeMap::from([(
            "ledger_current_index".to_owned(),
            protocol::JsonValue::Unsigned(5001),
        )]))
    );

    let protocol::JsonValue::Object(updated_fee) = rpc::do_fee(&owned) else {
        panic!("fee response must be object");
    };
    assert_eq!(
        updated_fee.get("ledger_current_index"),
        Some(&protocol::JsonValue::Unsigned(5001))
    );
    let protocol::JsonValue::Object(drops) = updated_fee.get("drops").expect("drops") else {
        panic!("drops must be object");
    };
    assert_eq!(
        drops.get("base_fee"),
        Some(&protocol::JsonValue::String("22".to_owned()))
    );
}
