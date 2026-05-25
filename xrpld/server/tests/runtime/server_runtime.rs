use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use app::{
    ApplicationRoot, ApplicationRootOptions, ManagedComponent, ServerPortClientSetup,
    ServerPortSetup, ServerPortsSetup,
};
use server::{
    RpcDispatcher, RpcReply, RpcRequest, RpcServerPortPolicy, ServerAuthConfig, ServerRuntime,
};
use tokio::runtime::Handle;
use tokio::task;

#[derive(Debug, Clone, Default)]
struct RecordingDispatcher {
    calls: Arc<Mutex<Vec<String>>>,
}

impl RecordingDispatcher {
    fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .expect("dispatcher calls mutex poisoned")
            .clone()
    }
}

impl RpcDispatcher for RecordingDispatcher {
    fn dispatch(&self, request: RpcRequest<'_>) -> RpcReply {
        self.calls
            .lock()
            .expect("dispatcher calls mutex poisoned")
            .push(request.method.to_owned());

        RpcReply::result(protocol::JsonValue::Object(BTreeMap::from([(
            "ok".to_owned(),
            protocol::JsonValue::Bool(true),
        )])))
    }
}

fn policy(name: &str, port: u16, allow_http: bool, allow_ws: bool) -> RpcServerPortPolicy {
    RpcServerPortPolicy {
        name: name.to_owned(),
        socket_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
        allow_http,
        allow_ws,
        auth: ServerAuthConfig::default(),
        tls_config: None,
    }
}

fn server_ports_setup(protocols: Vec<String>) -> Arc<ServerPortsSetup> {
    Arc::new(ServerPortsSetup {
        ports: vec![ServerPortSetup {
            name: "port_rpc".to_owned(),
            ip: "127.0.0.1".to_owned(),
            port: 0,
            limit: 0,
            protocols,
            user: String::new(),
            password: String::new(),
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
        client: Some(ServerPortClientSetup {
            secure: false,
            ip: "127.0.0.1".to_owned(),
            port: 0,
            user: String::new(),
            password: String::new(),
            admin_user: String::new(),
            admin_password: String::new(),
        }),
        overlay: None,
        grpc: None,
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_runtime_binds_multiple_plain_listeners_and_serves_requests() {
    let dispatcher = RecordingDispatcher::default();
    let runtime = ServerRuntime::new(
        Handle::current(),
        dispatcher.clone(),
        vec![policy("http", 0, true, false), policy("ws", 0, false, true)],
    );

    runtime.start().expect("runtime should start");
    let addrs = runtime.bound_listener_addrs();
    assert_eq!(addrs.len(), 2);
    assert_ne!(addrs[0], addrs[1]);

    let http_response = task::spawn_blocking({
        let addr = addrs[0];
        move || {
            let mut stream =
                std::net::TcpStream::connect(addr).expect("client should connect to http listener");
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("read timeout should set");
            stream
                .write_all(
                    concat!(
                        "POST / HTTP/1.1\r\n",
                        "Host: localhost\r\n",
                        "Content-Type: application/json\r\n",
                        "Content-Length: 40\r\n",
                        "\r\n",
                        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}"
                    )
                    .as_bytes(),
                )
                .expect("client should write request");
            let mut response = Vec::new();
            let mut chunk = [0_u8; 256];
            loop {
                let read = stream
                    .read(&mut chunk)
                    .expect("client should read response");
                if read == 0 {
                    break;
                }
                response.extend_from_slice(&chunk[..read]);
                if response.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            String::from_utf8(response).expect("response should be utf-8")
        }
    })
    .await
    .expect("blocking client should finish");
    assert!(http_response.contains("200 OK"));

    let ws_response = task::spawn_blocking({
        let addr = addrs[1];
        move || {
            let mut stream =
                std::net::TcpStream::connect(addr).expect("client should connect to ws listener");
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("read timeout should set");
            stream
                .write_all(
                    concat!(
                        "GET / HTTP/1.1\r\n",
                        "Host: localhost\r\n",
                        "Connection: upgrade\r\n",
                        "Upgrade: websocket\r\n",
                        "Sec-WebSocket-Version: 13\r\n",
                        "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n",
                        "\r\n",
                    )
                    .as_bytes(),
                )
                .expect("client should write request");
            let mut response = Vec::new();
            let mut chunk = [0_u8; 256];
            loop {
                let read = stream
                    .read(&mut chunk)
                    .expect("client should read response");
                if read == 0 {
                    break;
                }
                response.extend_from_slice(&chunk[..read]);
                if response.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            String::from_utf8(response).expect("response should be utf-8")
        }
    })
    .await
    .expect("blocking client should finish");
    assert!(ws_response.starts_with("HTTP/1.1 101 Switching Protocols"));

    runtime.stop();
    assert_eq!(dispatcher.calls(), vec!["ping".to_owned()]);
}

#[test]
fn server_runtime_reports_deferred_protocols_but_keeps_supported_listeners() {
    let mut app =
        ApplicationRoot::with_options(ApplicationRootOptions::default()).expect("app should build");
    app.attach_server_ports_setup(server_ports_setup(vec![
        "http".to_owned(),
        "peer".to_owned(),
        "https".to_owned(),
    ]));

    let report = ServerRuntime::from_application_root_with_report(&app)
        .expect("runtime should build from mixed ports");
    let tuning = report
        .runtime
        .path_find_tuning()
        .expect("runtime should attach path source");
    assert_eq!(tuning.old, 2);
    assert_eq!(tuning.search, 2);
    assert_eq!(tuning.fast, 2);
    assert_eq!(tuning.max, 3);
    assert_eq!(report.deferred_protocols.len(), 1);
    let transport_report = report.runtime.transport_report();
    assert_eq!(
        transport_report.deferred_protocols,
        report.deferred_protocols
    );
    assert_eq!(transport_report.deferred_peer_handoff_count(), 1);
    assert_eq!(transport_report.deferred_secure_listener_count(), 0);
    assert_eq!(
        transport_report.deferred_transport_summary(),
        "1 peer handoff transport(s) deferred, 0 secure listener transport(s) deferred"
    );
    assert!(transport_report.bound_addresses.is_empty());
    assert_eq!(transport_report.active_listener_count, 0);
    assert!(
        report
            .deferred_protocols
            .iter()
            .any(|protocol| protocol.protocol == "peer")
    );

    report.runtime.start().expect("runtime should start");
    let transport_report = report.runtime.transport_report();
    assert_eq!(transport_report.bound_addresses.len(), 1);
    assert_eq!(transport_report.active_listener_count, 1);
    assert_eq!(transport_report.deferred_peer_handoff_count(), 1);
    assert_eq!(transport_report.deferred_secure_listener_count(), 0);
    report.runtime.stop();
    let transport_report = report.runtime.transport_report();
    assert!(transport_report.bound_addresses.is_empty());
    assert_eq!(transport_report.active_listener_count, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_runtime_stop_releases_listener_ports_cleanly() {
    let runtime = ServerRuntime::new(
        Handle::current(),
        RecordingDispatcher::default(),
        vec![policy("http", 0, true, false)],
    );

    runtime.start().expect("runtime should start");
    let addr = runtime.bound_listener_addrs()[0];

    runtime.stop();

    let rebound = std::net::TcpListener::bind(addr);
    assert!(
        rebound.is_ok(),
        "listener port should be released after stop"
    );
}

#[test]
fn application_root_runtime_serves_ws_status_page_from_owned_health_source() {
    let mut app = ApplicationRoot::with_options(ApplicationRootOptions {
        elb_support: true,
        ..ApplicationRootOptions::default()
    })
    .expect("app should build");
    app.attach_server_ports_setup(server_ports_setup(vec!["ws".to_owned()]));

    let runtime = ServerRuntime::from_application_root(&app)
        .expect("runtime should build from application root");
    runtime.start().expect("runtime should start");
    let addr = runtime.bound_listener_addrs()[0];

    std::thread::sleep(Duration::from_millis(50));

    let response = {
        let mut stream =
            std::net::TcpStream::connect(addr).expect("client should connect to ws listener");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("read timeout should set");
        stream
            .write_all(concat!("GET / HTTP/1.1\r\n", "Host: localhost\r\n", "\r\n",).as_bytes())
            .expect("client should write request");
        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .expect("client should read response");
        String::from_utf8(response).expect("response should be utf-8")
    };

    assert!(response.starts_with("HTTP/1.1 500 Internal Server Error"));
    assert!(response.contains("Server cannot accept clients"));

    runtime.stop();
}
