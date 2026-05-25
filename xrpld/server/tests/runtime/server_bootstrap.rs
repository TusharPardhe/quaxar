use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use app::ManagedComponent;
use server::bootstrap::{
    ServerBootstrapConfig, build_runtime, build_runtime_report, parse_server_bootstrap_args,
};

fn install_crypto() {
    use std::sync::Once;
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

#[test]
fn server_bootstrap_defaults_to_loopback_ephemeral_http_ws() {
    let config = parse_server_bootstrap_args(["xrpld-server".to_owned()])
        .expect("default bootstrap args should parse");
    assert_eq!(
        config.bind,
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0)
    );
    assert_eq!(config.protocols, vec!["http".to_owned(), "ws".to_owned()]);
    assert!(!config.elb_support);
    assert!(!config.start_valid);
}

#[test]
fn server_bootstrap_accepts_peer_and_secure_modes_in_mixed_protocol_sets() {
    let config = parse_server_bootstrap_args([
        "xrpld-server".to_owned(),
        "--protocols".to_owned(),
        "http,peer,https".to_owned(),
    ])
    .expect("mixed transport bootstrap should parse");
    assert_eq!(
        config.protocols,
        vec!["http".to_owned(), "peer".to_owned(), "https".to_owned()]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_bootstrap_runtime_binds_and_answers_ping() {
    let config = ServerBootstrapConfig {
        bind: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        protocols: vec!["http".to_owned(), "ws".to_owned()],
        ssl_key: String::new(),
        ssl_cert: String::new(),
        ssl_chain: String::new(),
        elb_support: false,
        start_valid: false,
        skip_ssl_check: true,
    };
    tokio::task::spawn_blocking(move || {
        let runtime = build_runtime(&config).expect("bootstrap runtime should build");
        runtime.start().expect("runtime should start");
        let addr = runtime.bound_listener_addrs()[0];

        let response = {
            let mut stream = std::net::TcpStream::connect(addr).expect("client should connect");
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("read timeout should set");
            let body = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}";
            let request = format!(
                concat!(
                    "POST / HTTP/1.1\r\n",
                    "Host: localhost\r\n",
                    "Connection: close\r\n",
                    "Content-Type: application/json\r\n",
                    "Content-Length: {}\r\n",
                    "\r\n",
                    "{}"
                ),
                body.len(),
                body
            );
            stream
                .write_all(request.as_bytes())
                .expect("client should write request");

            let mut response = Vec::new();
            stream
                .read_to_end(&mut response)
                .expect("client should read response");

            String::from_utf8(response).expect("response should be utf-8")
        };

        assert!(
            response.starts_with("HTTP/1.1 200 OK"),
            "response: {response}"
        );
        assert!(response.contains("\"jsonrpc\":\"2.0\""));

        runtime.stop();
    })
    .await
    .expect("blocking bootstrap should finish");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_bootstrap_runtime_serves_server_definitions() {
    let config = ServerBootstrapConfig {
        bind: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        protocols: vec!["http".to_owned(), "ws".to_owned()],
        ssl_key: String::new(),
        ssl_cert: String::new(),
        ssl_chain: String::new(),
        elb_support: false,
        start_valid: false,
        skip_ssl_check: true,
    };
    tokio::task::spawn_blocking(move || {
        let runtime = build_runtime(&config).expect("bootstrap runtime should build");
        runtime.start().expect("runtime should start");
        let addr = runtime.bound_listener_addrs()[0];

        let response = {
            let mut stream = std::net::TcpStream::connect(addr).expect("client should connect");
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("read timeout should set");
            let body = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"server_definitions\"}";
            let request = format!(
                concat!(
                    "POST / HTTP/1.1\r\n",
                    "Host: localhost\r\n",
                    "Connection: close\r\n",
                    "Content-Type: application/json\r\n",
                    "Content-Length: {}\r\n",
                    "\r\n",
                    "{}"
                ),
                body.len(),
                body
            );
            stream
                .write_all(request.as_bytes())
                .expect("client should write request");

            let mut response = Vec::new();
            stream
                .read_to_end(&mut response)
                .expect("client should read response");

            String::from_utf8(response).expect("response should be utf-8")
        };

        assert!(
            response.starts_with("HTTP/1.1 200 OK"),
            "response: {response}"
        );
        assert!(response.contains("\"TYPES\""));
        assert!(response.contains("\"hash\""));

        runtime.stop();
    })
    .await
    .expect("blocking bootstrap should finish");
}

#[test]
fn server_bootstrap_runtime_reports_deferred_peer_and_secure_protocols() {
    install_crypto();
    let config = ServerBootstrapConfig {
        bind: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        protocols: vec!["http".to_owned(), "peer".to_owned(), "https".to_owned()],
        ssl_key: String::new(),
        ssl_cert: String::new(),
        ssl_chain: String::new(),
        elb_support: false,
        start_valid: false,
        skip_ssl_check: true,
    };

    let report = build_runtime_report(&config).expect("bootstrap report should build");
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
    report.runtime.stop();
}

#[test]
fn server_bootstrap_runtime_rejects_peer_only_configs_with_deferred_details() {
    let config = ServerBootstrapConfig {
        bind: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        protocols: vec!["peer".to_owned()],
        ssl_key: String::new(),
        ssl_cert: String::new(),
        ssl_chain: String::new(),
        elb_support: false,
        start_valid: false,
        skip_ssl_check: false,
    };

    let error = build_runtime_report(&config).expect_err("peer-only configs should not start");
    assert!(error.contains("deferred peer on"));
    assert!(
        error.contains("server ports setup does not expose any supported Rust HTTP/WS listeners")
    );
}
