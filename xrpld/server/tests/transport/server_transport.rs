use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use app::ServerPortSetup;
use axum::body::{Body, to_bytes};
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode, header};
use protocol::JsonValue;
use serde_json::json;
use server::{RpcDispatcher, RpcReply, RpcRequest, RpcServer, ServerStatusSource};
use tower::ServiceExt;

#[derive(Debug, Clone, Default)]
struct RecordingDispatcher {
    calls: Arc<Mutex<Vec<RecordedCall>>>,
}

#[derive(Debug, Clone, PartialEq)]
struct RecordedCall {
    method: String,
    role: rpc::RpcRole,
    unlimited: bool,
    user: String,
    forwarded_for: String,
    api_version: u32,
    is_websocket: bool,
}

impl RecordingDispatcher {
    fn calls(&self) -> Vec<RecordedCall> {
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
            .push(RecordedCall {
                method: request.method.to_owned(),
                role: request.metadata.role,
                unlimited: request.metadata.unlimited,
                user: request.metadata.user.clone(),
                forwarded_for: request.metadata.forwarded_for.clone(),
                api_version: request.metadata.api_version,
                is_websocket: request.metadata.is_websocket,
            });

        RpcReply::result(JsonValue::Object(BTreeMap::from([(
            "ok".to_owned(),
            JsonValue::Bool(true),
        )])))
    }
}

struct FixedStatusSource(Result<(), String>);

impl ServerStatusSource for FixedStatusSource {
    fn validated_ledger_hash(&self) -> Option<String> {
        None
    }
    fn server_okay(&self) -> Result<(), String> {
        self.0.clone()
    }
}

fn make_request(method: &str, body: serde_json::Value, remote_addr: SocketAddr) -> Request<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri("/")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .expect("request should build");
    request.extensions_mut().insert(ConnectInfo(remote_addr));
    request
}

fn make_get_request(remote_addr: SocketAddr) -> Request<Body> {
    let mut request = Request::builder()
        .method("GET")
        .uri("/")
        .body(Body::empty())
        .expect("request should build");
    request.extensions_mut().insert(ConnectInfo(remote_addr));
    request
}

fn make_server(dispatcher: RecordingDispatcher) -> RpcServer<RecordingDispatcher> {
    RpcServer::with_server_port(
        dispatcher,
        &ServerPortSetup {
            name: "port_rpc".to_owned(),
            ip: "127.0.0.1".to_owned(),
            port: 5005,
            limit: 0,
            protocols: vec!["http".to_owned(), "ws".to_owned()],
            user: "rpc".to_owned(),
            password: "secret".to_owned(),
            admin_user: "rpc".to_owned(),
            admin_password: "secret".to_owned(),
            ssl_key: String::new(),
            ssl_cert: String::new(),
            ssl_chain: String::new(),
            ssl_ciphers: String::new(),
            admin_nets_v4: vec!["127.0.0.0/8".parse().expect("loopback should parse")],
            admin_nets_v6: Vec::new(),
            secure_gateway_nets_v4: Vec::new(),
            secure_gateway_nets_v6: Vec::new(),
            standalone_mode: false,
        },
    )
    .expect("server port should build")
}

#[tokio::test]
async fn http_post_uses_basic_auth_and_clears_untrusted_headers() {
    let dispatcher = RecordingDispatcher::default();
    let server = make_server(dispatcher.clone());
    let mut request = make_request(
        "POST",
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "ping",
            "params": {}
        }),
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 51_234),
    );
    request.headers_mut().insert(
        header::AUTHORIZATION,
        header::HeaderValue::from_static("Basic cnBjOnNlY3JldA=="),
    );
    request.headers_mut().insert(
        header::HeaderName::from_static("x-user"),
        header::HeaderValue::from_static("rpc"),
    );
    request.headers_mut().insert(
        header::HeaderName::from_static("x-forwarded-for"),
        header::HeaderValue::from_static("10.1.2.3, 10.2.3.4"),
    );

    let response = server
        .router()
        .oneshot(request)
        .await
        .expect("router should respond");

    assert_eq!(response.status(), StatusCode::OK);
    let calls = dispatcher.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].method, "ping");
    assert_eq!(calls[0].role, rpc::RpcRole::Guest);
    assert!(!calls[0].unlimited);
    assert_eq!(calls[0].user, "");
    assert_eq!(calls[0].forwarded_for, "");
    assert_eq!(calls[0].api_version, 1);
    assert!(!calls[0].is_websocket);
}

#[tokio::test]
async fn http_post_promotes_admin_from_json_credentials_but_still_clears_untrusted_headers() {
    let dispatcher = RecordingDispatcher::default();
    let server = make_server(dispatcher.clone());
    let mut request = make_request(
        "POST",
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "ping",
            "params": {
                "admin_user": "rpc",
                "admin_password": "secret"
            }
        }),
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 51_234),
    );
    request.headers_mut().insert(
        header::AUTHORIZATION,
        header::HeaderValue::from_static("Basic cnBjOnNlY3JldA=="),
    );
    request.headers_mut().insert(
        header::HeaderName::from_static("x-user"),
        header::HeaderValue::from_static("rpc"),
    );
    request.headers_mut().insert(
        header::HeaderName::from_static("x-forwarded-for"),
        header::HeaderValue::from_static("10.1.2.3, 10.2.3.4"),
    );

    let response = server
        .router()
        .oneshot(request)
        .await
        .expect("router should respond");

    assert_eq!(response.status(), StatusCode::OK);
    let calls = dispatcher.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].method, "ping");
    assert_eq!(calls[0].role, rpc::RpcRole::Admin);
    assert!(calls[0].unlimited);
    assert_eq!(calls[0].user, "");
    assert_eq!(calls[0].forwarded_for, "");
    assert_eq!(calls[0].api_version, 1);
    assert!(!calls[0].is_websocket);
}

#[tokio::test]
async fn http_post_rejects_ports_without_http_protocol() {
    let dispatcher = RecordingDispatcher::default();
    let server = RpcServer::with_server_port(
        dispatcher,
        &ServerPortSetup {
            name: "port_ws".to_owned(),
            ip: "127.0.0.1".to_owned(),
            port: 5005,
            limit: 0,
            protocols: vec!["ws".to_owned()],
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
            standalone_mode: false,
        },
    )
    .expect("server port should build");
    let request = make_request(
        "POST",
        json!({"jsonrpc": "2.0", "id": 1, "method": "ping", "params": {}}),
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 51_234),
    );

    let response = server
        .router()
        .oneshot(request)
        .await
        .expect("router should respond");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn http_post_preserves_trusted_headers_for_secure_gateway() {
    let dispatcher = RecordingDispatcher::default();
    let server = RpcServer::with_server_port(
        dispatcher.clone(),
        &ServerPortSetup {
            name: "port_gateway".to_owned(),
            ip: "127.0.0.1".to_owned(),
            port: 5005,
            limit: 0,
            protocols: vec!["http".to_owned()],
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
            secure_gateway_nets_v4: vec![
                "127.0.0.0/8"
                    .parse()
                    .expect("secure gateway network should parse"),
            ],
            secure_gateway_nets_v6: Vec::new(),
            standalone_mode: false,
        },
    )
    .expect("server port should build");
    let mut request = make_request(
        "POST",
        json!({"jsonrpc": "2.0", "id": 1, "method": "ping", "params": {}}),
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 51_234),
    );
    request.headers_mut().insert(
        header::HeaderName::from_static("x-user"),
        header::HeaderValue::from_static("gateway-user"),
    );
    request.headers_mut().insert(
        header::HeaderName::from_static("x-forwarded-for"),
        header::HeaderValue::from_static("10.1.2.3, 10.2.3.4"),
    );

    let response = server
        .router()
        .oneshot(request)
        .await
        .expect("router should respond");

    assert_eq!(response.status(), StatusCode::OK);
    let calls = dispatcher.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].role, rpc::RpcRole::Identified);
    assert!(calls[0].unlimited);
    assert_eq!(calls[0].user, "gateway-user");
    assert_eq!(calls[0].forwarded_for, "10.1.2.3");
}

#[tokio::test]
async fn websocket_upgrade_route_accepts_ws_handshake() {
    let dispatcher = RecordingDispatcher::default();
    let server = make_server(dispatcher);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let local_addr = listener
        .local_addr()
        .expect("listener should have local addr");
    let server_task = tokio::spawn(async move {
        server.serve(listener).await.expect("server should run");
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let response = tokio::task::spawn_blocking(move || {
        let mut stream =
            std::net::TcpStream::connect(local_addr).expect("client should connect to server");
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
            let n = stream
                .read(&mut chunk)
                .expect("client should read response");
            if n == 0 {
                break;
            }
            response.extend_from_slice(&chunk[..n]);
            if response.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }

        std::str::from_utf8(&response)
            .expect("response should be utf-8")
            .to_owned()
    })
    .await
    .expect("blocking websocket client should finish");
    assert!(
        response.starts_with("HTTP/1.1 101 Switching Protocols"),
        "unexpected websocket response: {response}"
    );
    assert!(response.contains("upgrade: websocket") || response.contains("Upgrade: websocket"));

    server_task.abort();
}

#[tokio::test]
async fn websocket_get_status_page_reports_server_health() {
    let dispatcher = RecordingDispatcher::default();
    let server = RpcServer::with_server_port_and_status_source(
        dispatcher,
        &ServerPortSetup {
            name: "port_ws".to_owned(),
            ip: "127.0.0.1".to_owned(),
            port: 5005,
            limit: 0,
            protocols: vec!["ws".to_owned()],
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
            standalone_mode: false,
        },
        Arc::new(FixedStatusSource(Ok(()))),
    )
    .expect("server port should build");

    let response = server
        .router()
        .oneshot(make_get_request(SocketAddr::new(
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            51_234,
        )))
        .await
        .expect("router should respond");

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should read");
    let body = std::str::from_utf8(&body).expect("body should be utf-8");
    assert!(body.contains("Test page for quaxar"));
    assert!(body.contains("connectivity is working"));
}

#[tokio::test]
async fn websocket_upgrade_rejects_non_ws_ports_with_invalid_protocol_html() {
    let dispatcher = RecordingDispatcher::default();
    let server = RpcServer::with_server_port(
        dispatcher,
        &ServerPortSetup {
            name: "port_http".to_owned(),
            ip: "127.0.0.1".to_owned(),
            port: 5005,
            limit: 0,
            protocols: vec!["http".to_owned()],
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
            standalone_mode: false,
        },
    )
    .expect("server port should build");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let local_addr = listener
        .local_addr()
        .expect("listener should have local addr");
    let server_task = tokio::spawn(async move {
        server.serve(listener).await.expect("server should run");
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let response = tokio::task::spawn_blocking(move || {
        let mut stream =
            std::net::TcpStream::connect(local_addr).expect("client should connect to server");
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
        stream
            .read_to_end(&mut response)
            .expect("client should read response");
        std::str::from_utf8(&response)
            .expect("response should be utf-8")
            .to_owned()
    })
    .await
    .expect("blocking websocket client should finish");

    assert!(response.starts_with("HTTP/1.1 401 Unauthorized"));
    assert!(response.contains("Invalid protocol."));

    server_task.abort();
}
