use app::ServerPortSetup;
use basics::make_ssl_context::{
    TlsIdentityDer, anonymous_tls_identity_der, authenticated_tls_identity_der,
};
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::{Arc, Once};

use axum::Router;
use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Json, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use futures::{SinkExt, StreamExt};
use protocol::JsonValue;
use rustls::ServerConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::sync::mpsc;

use crate::auth::{ServerAuth, ServerAuthConfig, authorized_http, forwarded_for, request_role};
use crate::json::{Envelope, EnvelopeField, RawRequest, from_protocol_json, result_reports_error};
use crate::session::{RequestMetadata, WSSession};
use crate::status::{ServerStatusSource, invalid_protocol_response, status_page_response};
use crate::subscriptions::SubscriptionManager;
use crate::transport::{RpcDispatcher, RpcReply, RpcRequest};
use rpc::RpcRole;

/// Concurrent ledger-reading RPC handlers per listener (blocking pool).
///
/// rippled runs RPC as JobQueue jobs on a worker pool sized to the machine;
/// running far more CPU-bound handlers than cores only adds contention on
/// shared SHAMap nodes (spinning on locks whose holder is descheduled) and
/// lowers throughput. Override with `QUAXAR_RPC_HANDLER_THREADS`.
fn handler_concurrency() -> usize {
    std::env::var("QUAXAR_RPC_HANDLER_THREADS")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|threads: &usize| *threads > 0)
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(usize::from)
                .unwrap_or(4)
                .max(2)
        })
}

/// Methods whose handlers are constant-time and touch no ledger state, so
/// they are dispatched on the async worker instead of the blocking pool.
const INLINE_METHODS: &[&str] = &["ping", "random"];

/// Per-connection outbound WebSocket queue (frames). RPC replies and
/// subscription forwarders wait for capacity; bursts are absorbed by the
/// shared per-stream broadcast ring (`DEFAULT_STREAM_CAPACITY`). A client
/// that falls a whole ring behind, or overflows a non-waiting `send_text`, is
/// closed with rippled's policy_error "Policy error: client is too slow."
/// (`BaseWSPeer::send`) instead of silently losing messages. rippled's
/// per-port `send_queue_limit` (default 100) is not yet plumbed through
/// `ServerPortSetup`; this matches its default.
pub const WS_SEND_QUEUE_LIMIT: usize = 100;

/// Upper bound on frames written per flush by the WebSocket writer.
const WS_MAX_FRAMES_PER_FLUSH: usize = 64;

#[derive(Clone)]
pub struct RpcServerConfig {
    pub request_path: String,
    pub websocket_path: String,
    pub port_policy: Option<RpcServerPortPolicy>,
    pub status_source: Option<Arc<dyn ServerStatusSource>>,
}

impl std::fmt::Debug for RpcServerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RpcServerConfig")
            .field("request_path", &self.request_path)
            .field("websocket_path", &self.websocket_path)
            .field("port_policy", &self.port_policy)
            .field("has_status_source", &self.status_source.is_some())
            .finish()
    }
}

impl Default for RpcServerConfig {
    fn default() -> Self {
        Self {
            request_path: "/".to_owned(),
            websocket_path: "/".to_owned(),
            port_policy: None,
            status_source: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RpcServerPortPolicy {
    pub name: String,
    pub socket_addr: SocketAddr,
    pub allow_http: bool,
    pub allow_ws: bool,
    pub auth: ServerAuthConfig,
    pub tls_config: Option<Arc<ServerConfig>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RpcServerPortDeferredProtocol {
    pub port_name: String,
    pub protocol: String,
    pub reason: String,
}

impl RpcServerPortDeferredProtocol {
    pub fn is_peer_handoff(&self) -> bool {
        self.protocol == "peer"
    }

    pub fn is_secure_listener(&self) -> bool {
        matches!(self.protocol.as_str(), "https" | "wss" | "wss2")
    }
}

#[derive(Debug, Clone)]
pub struct RpcServerPortBuild {
    pub policy: Option<RpcServerPortPolicy>,
    pub deferred_protocols: Vec<RpcServerPortDeferredProtocol>,
}

impl TryFrom<&ServerPortSetup> for RpcServerPortPolicy {
    type Error = String;

    fn try_from(port: &ServerPortSetup) -> Result<Self, Self::Error> {
        let build = RpcServerPortBuild::from_server_port(port)?;
        build.policy.ok_or_else(|| {
            format!(
                "port [{}] does not expose a supported Rust HTTP/WS protocol",
                port.name
            )
        })
    }
}

fn install_rustls_crypto_provider() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

fn build_tls_config(port: &ServerPortSetup) -> Result<Arc<ServerConfig>, String> {
    // The server crate enables Rustls' `ring` provider explicitly. Install it
    // before using the builder so secure listeners do not depend on Rustls'
    // feature-based default-provider inference.
    install_rustls_crypto_provider();

    let (certs, key) = if port.ssl_key.is_empty()
        && port.ssl_cert.is_empty()
        && port.ssl_chain.is_empty()
    {
        let identity = anonymous_tls_identity_der()
            .map_err(|error| format!("failed to build anonymous TLS identity: {error}"))?;
        (rustls_cert_chain(&identity), rustls_private_key(&identity))
    } else {
        let identity =
            authenticated_tls_identity_der(&port.ssl_key, &port.ssl_cert, &port.ssl_chain)
                .map_err(|error| format!("failed to build authenticated TLS identity: {error}"))?;
        (rustls_cert_chain(&identity), rustls_private_key(&identity))
    };

    let mut config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| format!("failed to build TLS config: {}", e))?;

    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(Arc::new(config))
}

fn rustls_cert_chain(identity: &TlsIdentityDer) -> Vec<CertificateDer<'static>> {
    identity
        .certificate_chain_der()
        .into_iter()
        .map(CertificateDer::from)
        .collect()
}

fn rustls_private_key(identity: &TlsIdentityDer) -> PrivateKeyDer<'static> {
    PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
        identity.private_key_pkcs8_der().to_vec(),
    ))
}

impl RpcServerPortBuild {
    pub fn from_server_port(port: &ServerPortSetup) -> Result<Self, String> {
        let socket_addr = format!("{}:{}", port.ip, port.port)
            .parse::<SocketAddr>()
            .map_err(|_| format!("invalid socket address for [{}]", port.name))?;
        let is_secure =
            port.has_protocol("https") || port.has_protocol("wss") || port.has_protocol("wss2");
        let allow_http = port.has_protocol("http") || port.has_protocol("https");
        let allow_ws = port.has_protocol("ws")
            || port.has_protocol("ws2")
            || port.has_protocol("wss")
            || port.has_protocol("wss2");

        let tls_config = if is_secure {
            Some(build_tls_config(port)?)
        } else {
            None
        };

        let mut deferred_protocols = Vec::new();
        if port.allows_peer() {
            deferred_protocols.push(RpcServerPortDeferredProtocol {
                port_name: port.name.clone(),
                protocol: "peer".to_owned(),
                reason: format!(
                    "port [{}] peer handoff handled by overlay listener",
                    port.name
                ),
            });
        }
        let policy = if allow_http || allow_ws {
            Some(RpcServerPortPolicy {
                name: port.name.clone(),
                socket_addr,
                allow_http,
                allow_ws,
                auth: ServerAuthConfig {
                    user: (!port.user.is_empty()).then(|| port.user.clone()),
                    password: (!port.password.is_empty()).then(|| port.password.clone()),
                    admin_user: (!port.admin_user.is_empty()).then(|| port.admin_user.clone()),
                    admin_password: (!port.admin_password.is_empty())
                        .then(|| port.admin_password.clone()),
                    admin_nets_v4: port.admin_nets_v4.clone(),
                    admin_nets_v6: port.admin_nets_v6.clone(),
                    secure_gateway_nets_v4: port.secure_gateway_nets_v4.clone(),
                    secure_gateway_nets_v6: port.secure_gateway_nets_v6.clone(),
                    standalone_mode: port.standalone_mode,
                },
                tls_config,
            })
        } else {
            None
        };

        Ok(Self {
            policy,
            deferred_protocols,
        })
    }
}

#[derive(Clone)]
pub struct RpcServer<D> {
    dispatcher: Arc<D>,
    subscriptions: Arc<SubscriptionManager>,
    auth: ServerAuth,
    config: RpcServerConfig,
    state: Arc<RpcServerState>,
}

pub struct RpcServerState {
    pub p0_pool: tokio::sync::Semaphore,
    pub p1_pool: tokio::sync::Semaphore,
    pub p2_pool: tokio::sync::Semaphore,
}

impl Default for RpcServerState {
    fn default() -> Self {
        Self {
            p0_pool: tokio::sync::Semaphore::new(128),
            p1_pool: tokio::sync::Semaphore::new(handler_concurrency()),
            p2_pool: tokio::sync::Semaphore::new(handler_concurrency()),
        }
    }
}

impl<D> RpcServer<D>
where
    D: RpcDispatcher + 'static,
{
    fn api_version_from_params(params: &JsonValue) -> u32 {
        let JsonValue::Object(object) = params else {
            return 1;
        };
        match object.get("api_version") {
            Some(JsonValue::Unsigned(value)) => {
                u32::try_from(*value).ok().filter(|value| *value > 0)
            }
            Some(JsonValue::Signed(value)) if *value > 0 => u32::try_from(*value as u64).ok(),
            _ => None,
        }
        .unwrap_or(1)
    }

    pub fn new(dispatcher: D) -> Self {
        Self {
            dispatcher: Arc::new(dispatcher),
            subscriptions: Arc::new(SubscriptionManager::default()),
            auth: ServerAuth::default(),
            config: RpcServerConfig::default(),
            state: Arc::new(RpcServerState::default()),
        }
    }

    pub fn with_auth(dispatcher: D, auth: ServerAuth) -> Self {
        Self {
            dispatcher: Arc::new(dispatcher),
            subscriptions: Arc::new(SubscriptionManager::default()),
            auth,
            config: RpcServerConfig::default(),
            state: Arc::new(RpcServerState::default()),
        }
    }

    pub fn with_subscriptions(dispatcher: D, subscriptions: Arc<SubscriptionManager>) -> Self {
        Self {
            dispatcher: Arc::new(dispatcher),
            subscriptions,
            auth: ServerAuth::default(),
            config: RpcServerConfig::default(),
            state: Arc::new(RpcServerState::default()),
        }
    }

    pub fn with_auth_and_subscriptions(
        dispatcher: D,
        auth: ServerAuth,
        subscriptions: Arc<SubscriptionManager>,
    ) -> Self {
        Self {
            dispatcher: Arc::new(dispatcher),
            subscriptions,
            auth,
            config: RpcServerConfig::default(),
            state: Arc::new(RpcServerState::default()),
        }
    }

    pub fn with_port_policy(dispatcher: D, policy: RpcServerPortPolicy) -> Self {
        Self::with_port_policy_and_subscriptions(
            dispatcher,
            policy,
            Arc::new(SubscriptionManager::default()),
            None,
        )
    }

    /// Build a live listener with the exact per-port transport, authorization,
    /// and status policy parsed from `[server]`. ServerRuntime must use this
    /// constructor rather than the generic auth/subscription constructor: the
    /// latter intentionally has no port policy and therefore cannot enforce a
    /// HTTP-only or WebSocket-only configured listener.
    pub fn with_port_policy_and_subscriptions(
        dispatcher: D,
        policy: RpcServerPortPolicy,
        subscriptions: Arc<SubscriptionManager>,
        status_source: Option<Arc<dyn ServerStatusSource>>,
    ) -> Self {
        Self {
            dispatcher: Arc::new(dispatcher),
            subscriptions,
            auth: ServerAuth::new(policy.auth.clone()),
            config: RpcServerConfig {
                port_policy: Some(policy),
                status_source,
                ..RpcServerConfig::default()
            },
            state: Arc::new(RpcServerState::default()),
        }
    }

    pub fn with_port_policy_and_status_source(
        dispatcher: D,
        policy: RpcServerPortPolicy,
        status_source: Arc<dyn ServerStatusSource>,
    ) -> Self {
        Self::with_port_policy_and_subscriptions(
            dispatcher,
            policy,
            Arc::new(SubscriptionManager::default()),
            Some(status_source),
        )
    }

    pub fn with_server_port(dispatcher: D, port: &ServerPortSetup) -> Result<Self, String> {
        let policy = RpcServerPortPolicy::try_from(port)?;
        Ok(Self::with_port_policy(dispatcher, policy))
    }

    pub fn with_server_port_and_status_source(
        dispatcher: D,
        port: &ServerPortSetup,
        status_source: Arc<dyn ServerStatusSource>,
    ) -> Result<Self, String> {
        let policy = RpcServerPortPolicy::try_from(port)?;
        Ok(Self::with_port_policy_and_status_source(
            dispatcher,
            policy,
            status_source,
        ))
    }

    pub fn subscriptions(&self) -> Arc<SubscriptionManager> {
        self.subscriptions.clone()
    }

    async fn dispatch_async(
        &self,
        method: String,
        params: JsonValue,
        metadata: RequestMetadata,
    ) -> (RpcReply, JsonValue) {
        // Do not share replies between identical requests. Some RPCs are
        // intentionally non-deterministic (for example, parameterless
        // wallet_propose), and others mutate server state. rippled dispatches
        // each request independently, even when method and parameters match.
        // Request coalescing here replayed the first wallet response to every
        // concurrent caller and therefore produced duplicate account seeds.

        // rippled: ALL requests are rejected with tooBusy when the server is
        // overloaded AND the client is not unlimited (admin). This prevents
        // DDoS from starving consensus. rippled checks:
        // 1. consumer.disconnect() — per-IP budget exceeded → drop connection
        // 2. getFeeTrack().isLoadedLocal() — global server load too high
        // 3. isUnlimited(role) — admin/unlimited clients bypass
        //
        // We check: semaphore pool exhausted + server health failing.
        // Admin requests still go through (matching rippled's isUnlimited).
        if metadata.role != crate::RpcRole::Admin {
            let saturated = self.state.p1_pool.available_permits() == 0
                || self.state.p2_pool.available_permits() == 0;
            if saturated
                && let Some(status) = &self.config.status_source
                && status.server_okay().is_err()
            {
                let reply = RpcReply::error(
                    rpc::RpcErrorCode::TooBusy,
                    "Server is too busy. Try again later.",
                );
                return (reply, params);
            }
        }

        let permit = match method.as_str() {
            "submit" | "fee" => self.state.p0_pool.acquire().await.unwrap(),
            "ledger_data" => self.state.p2_pool.acquire().await.unwrap(),
            _ => self.state.p1_pool.acquire().await.unwrap(),
        };

        // Constant-time handlers with no ledger, NodeStore, or lock-heavy
        // work run inline: the blocking-pool hand-off (two thread wake-ups)
        // would otherwise dominate their latency.
        if INLINE_METHODS.contains(&method.as_str()) {
            let reply = self.dispatcher.dispatch(RpcRequest {
                method: &method,
                params: &params,
                metadata: &metadata,
                session: None,
            });
            drop(permit);
            return (reply, params);
        }

        let dispatcher = self.dispatcher.clone();
        let result = tokio::task::spawn_blocking(move || {
            let reply = dispatcher.dispatch(RpcRequest {
                method: &method,
                params: &params,
                metadata: &metadata,
                session: None,
            });
            (reply, params)
        })
        .await
        .expect("dispatcher::dispatch panicked");

        drop(permit);

        result
    }

    pub fn router(self) -> Router {
        let request_path = self.config.request_path.clone();
        let websocket_path = self.config.websocket_path.clone();
        Router::new()
            .route(&request_path, post(Self::handle_post))
            .route("/v2/batch", post(Self::handle_batch))
            .route(&websocket_path, get(Self::handle_get))
            .with_state(Arc::new(self))
    }

    pub async fn serve(self, listener: TcpListener) -> std::io::Result<()> {
        use axum::serve::ListenerExt;
        axum::serve(
            listener.tap_io(|stream| {
                let _ = stream.set_nodelay(true);
            }),
            self.router()
                .into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
    }

    async fn handle_post(
        State(server): State<Arc<Self>>,
        ConnectInfo(remote_addr): ConnectInfo<SocketAddr>,
        headers: HeaderMap,
        body: axum::body::Bytes,
    ) -> Response {
        tracing::debug!(target: "server", client_ip = %remote_addr.ip(), "HTTP POST request");
        // axum's Json extractor rejects with 415 if Content-Type is missing.
        // Parse the body manually to match reference behavior. The request is
        // parsed once, straight into the protocol representation (SIMD
        // parser, no intermediate serde_json tree). Object key order is not
        // observable downstream: both representations are sorted maps.
        let payload = match RawRequest::parse(&body) {
            Ok(v) => v,
            Err(_) => return StatusCode::BAD_REQUEST.into_response(),
        };
        if let Some(policy) = server.config.port_policy.as_ref()
            && !policy.allow_http
        {
            return StatusCode::FORBIDDEN.into_response();
        }
        if !authorized_http(&server.auth.config, &headers) {
            return StatusCode::FORBIDDEN.into_response();
        }

        // ETag derived from validated ledger hash — applied post-dispatch
        // only when the response is for the validated ledger.
        let validated_etag = server
            .config
            .status_source
            .as_ref()
            .and_then(|s| s.validated_ledger_hash())
            .map(|h| format!("\"{}\"", h));
        let mut etag_val = None;

        let mut request_metadata = RequestMetadata::from_headers(remote_addr, &headers);
        request_metadata.local_addr = server
            .config
            .port_policy
            .as_ref()
            .map(|policy| policy.socket_addr);
        request_metadata.forwarded_for = forwarded_for(&headers).unwrap_or_default();
        request_metadata.user = headers
            .get("x-user")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let mut rpc_request = match JsonRpcEnvelope::try_from(payload) {
            Ok(value) => value,
            Err(response) => return response.into_response(),
        };

        let params = normalize_rpc_params(
            rpc_request
                .params
                .take()
                .unwrap_or_else(|| JsonValue::Object(BTreeMap::new())),
        );
        let mut metadata = request_metadata;
        metadata.api_version = Self::api_version_from_params(&params);
        metadata.role = request_role(
            RpcRole::User,
            &server.auth,
            &metadata,
            &params,
            &metadata.user,
        );
        if !matches!(metadata.role, RpcRole::Identified | RpcRole::Proxy) {
            metadata.user.clear();
            metadata.forwarded_for.clear();
        }
        metadata.unlimited = matches!(metadata.role, RpcRole::Admin | RpcRole::Identified);

        // Apply ETag/304 only for requests targeting the validated ledger.
        // If ledger_index is absent or "validated", the response is cacheable.
        if let Some(ref etag) = validated_etag {
            let targets_validated = match &params {
                JsonValue::Array(arr) => arr
                    .first()
                    .and_then(|p| {
                        if let JsonValue::Object(o) = p {
                            o.get("ledger_index")
                        } else {
                            None
                        }
                    })
                    .is_none_or(|v| matches!(v, JsonValue::String(s) if s == "validated")),
                JsonValue::Object(o) => o
                    .get("ledger_index")
                    .is_none_or(|v| matches!(v, JsonValue::String(s) if s == "validated")),
                _ => true,
            };
            if targets_validated {
                etag_val = Some(etag.clone());
                if let Some(if_none_match) = headers.get(axum::http::header::IF_NONE_MATCH)
                    && if_none_match.as_bytes() == etag.as_bytes()
                {
                    return StatusCode::NOT_MODIFIED.into_response();
                }
            }
        }

        let (reply, params) = server
            .dispatch_async(rpc_request.method.clone(), params, metadata)
            .await;
        let body = match reply {
            RpcReply::PreRendered(bytes) => {
                let mut prefix = Vec::new();
                prefix.extend_from_slice(b"{");
                if let Some(ver) = rpc_request.jsonrpc.as_deref() {
                    prefix.extend_from_slice(b"\"jsonrpc\":\"");
                    prefix.extend_from_slice(ver.as_bytes());
                    prefix.extend_from_slice(b"\",");
                }
                if let Some(id_val) = rpc_request.id {
                    prefix.extend_from_slice(b"\"id\":");
                    prefix.extend_from_slice(&sonic_rs::to_vec(&id_val).unwrap_or_default());
                    prefix.extend_from_slice(b",");
                } else {
                    prefix.extend_from_slice(b"\"id\":null,");
                }
                prefix.extend_from_slice(b"\"result\":");

                let mut out = Vec::with_capacity(prefix.len() + bytes.len() + 1);
                out.extend_from_slice(&prefix);
                out.extend_from_slice(&bytes);
                out.extend_from_slice(b"}");
                out
            }
            RpcReply::Result(result) if !result_reports_error(&result) => {
                // Success fast path: serialize the protocol result in place
                // inside the envelope. Byte-identical to json_rpc_response +
                // sonic_rs::to_vec, without rebuilding a serde_json tree.
                let mut envelope = Envelope::with_capacity(3);
                if let Some(ver) = rpc_request.jsonrpc.as_deref() {
                    envelope.insert("jsonrpc", EnvelopeField::Str(ver));
                    envelope.insert(
                        "id",
                        rpc_request
                            .id
                            .as_ref()
                            .map_or(EnvelopeField::Null, EnvelopeField::Json),
                    );
                }
                envelope.insert("result", EnvelopeField::ProtoWithDefaultStatus(&result));
                match sonic_rs::to_vec(&envelope) {
                    Ok(b) => b,
                    Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
                }
            }
            _ => {
                let mut response =
                    json_rpc_response(rpc_request.id, rpc_request.jsonrpc.as_deref(), reply);
                // name included, matching the reference implementation behavior.
                if let Value::Object(resp) = &mut response
                    && let Some(Value::Object(result)) = resp.get_mut("result")
                    && (result.get("status") == Some(&Value::String("error".to_owned()))
                        || result.contains_key("error"))
                {
                    result.entry("request".to_owned()).or_insert_with(|| {
                        // The raw params is [{"key":"val"}] — unwrap the array.
                        let raw = from_protocol_json(&params);
                        let mut echo = match raw {
                            Value::Array(arr) if !arr.is_empty() => arr
                                .into_iter()
                                .next()
                                .unwrap_or(Value::Object(serde_json::Map::new())),
                            Value::Object(_) => raw,
                            _ => Value::Object(serde_json::Map::new()),
                        };
                        if let Value::Object(obj) = &mut echo {
                            obj.entry("command".to_owned())
                                .or_insert_with(|| Value::String(rpc_request.method.clone()));
                        }
                        echo
                    });
                }
                match sonic_rs::to_vec(&response) {
                    Ok(b) => b,
                    Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
                }
            }
        };
        let mut res = (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            body,
        )
            .into_response();

        if let Some(etag) = etag_val
            && let Ok(etag_header) = axum::http::HeaderValue::from_str(&etag)
        {
            res.headers_mut()
                .insert(axum::http::header::ETAG, etag_header);
        }

        res
    }

    /// POST /v2/batch — accepts a JSON array of RPC requests, resolves the
    /// target ledger once, and dispatches each request against the same snapshot.
    async fn handle_batch(
        State(server): State<Arc<Self>>,
        ConnectInfo(remote_addr): ConnectInfo<SocketAddr>,
        headers: HeaderMap,
        body: axum::body::Bytes,
    ) -> Response {
        if let Some(policy) = server.config.port_policy.as_ref()
            && !policy.allow_http
        {
            return StatusCode::FORBIDDEN.into_response();
        }
        if !authorized_http(&server.auth.config, &headers) {
            return StatusCode::FORBIDDEN.into_response();
        }

        // ETag derived from validated ledger hash — applied post-dispatch
        // only when the response is for the validated ledger.
        let validated_etag = server
            .config
            .status_source
            .as_ref()
            .and_then(|s| s.validated_ledger_hash())
            .map(|h| format!("\"{}\"", h));
        let etag_val = validated_etag.clone();
        if let Some(ref etag) = validated_etag
            && let Some(if_none_match) = headers.get(axum::http::header::IF_NONE_MATCH)
            && if_none_match.as_bytes() == etag.as_bytes()
        {
            return StatusCode::NOT_MODIFIED.into_response();
        }

        let requests: Vec<Value> = match sonic_rs::from_slice(&body) {
            Ok(v) => v,
            Err(_) => return StatusCode::BAD_REQUEST.into_response(),
        };

        if requests.is_empty() {
            return (
                StatusCode::OK,
                [(axum::http::header::CONTENT_TYPE, "application/json")],
                b"[]".to_vec(),
            )
                .into_response();
        }

        let mut metadata = RequestMetadata::from_headers(remote_addr, &headers);
        metadata.local_addr = server
            .config
            .port_policy
            .as_ref()
            .map(|policy| policy.socket_addr);
        metadata.forwarded_for = forwarded_for(&headers).unwrap_or_default();
        metadata.user = headers
            .get("x-user")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();

        let mut responses = Vec::with_capacity(requests.len());
        for payload in requests {
            let mut rpc_request = match JsonRpcEnvelope::try_from(RawRequest::from_value(payload)) {
                Ok(value) => value,
                Err(_) => {
                    responses.push(Value::Null);
                    continue;
                }
            };

            let params = normalize_rpc_params(
                rpc_request
                    .params
                    .take()
                    .unwrap_or_else(|| JsonValue::Object(BTreeMap::new())),
            );
            let mut req_metadata = metadata.clone();
            req_metadata.api_version = Self::api_version_from_params(&params);
            req_metadata.role = request_role(
                RpcRole::User,
                &server.auth,
                &req_metadata,
                &params,
                &req_metadata.user,
            );
            if !matches!(req_metadata.role, RpcRole::Identified | RpcRole::Proxy) {
                req_metadata.user.clear();
                req_metadata.forwarded_for.clear();
            }
            req_metadata.unlimited =
                matches!(req_metadata.role, RpcRole::Admin | RpcRole::Identified);
            let method_owned = rpc_request.method.clone();
            let (reply, _params) = server
                .dispatch_async(method_owned, params, req_metadata)
                .await;
            responses.push(json_rpc_response(
                rpc_request.id,
                rpc_request.jsonrpc.as_deref(),
                reply,
            ));
        }

        let body = sonic_rs::to_vec(&responses).unwrap_or_default();
        let mut res = (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            body,
        )
            .into_response();

        if let Some(etag) = etag_val
            && let Ok(etag_header) = axum::http::HeaderValue::from_str(&etag)
        {
            res.headers_mut()
                .insert(axum::http::header::ETAG, etag_header);
        }

        res
    }

    async fn handle_get(
        State(server): State<Arc<Self>>,
        ConnectInfo(remote_addr): ConnectInfo<SocketAddr>,
        headers: HeaderMap,
        ws: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
    ) -> Response {
        match ws {
            Ok(ws) => {
                if let Some(policy) = server.config.port_policy.as_ref()
                    && !policy.allow_ws
                {
                    return invalid_protocol_response(StatusCode::UNAUTHORIZED);
                }

                ws.on_upgrade(move |socket| async move {
                    server.handle_ws_socket(socket, remote_addr, headers).await;
                })
                .into_response()
            }
            Err(_) => {
                if let Some(policy) = server.config.port_policy.as_ref()
                    && policy.allow_ws
                    && let Some(status_source) = server.config.status_source.as_ref()
                {
                    return status_page_response(status_source.as_ref());
                }

                StatusCode::NOT_FOUND.into_response()
            }
        }
    }

    async fn handle_ws_socket(
        self: Arc<Self>,
        socket: WebSocket,
        remote_addr: SocketAddr,
        headers: HeaderMap,
    ) {
        tracing::debug!(target: "server", client_ip = %remote_addr.ip(), "New WebSocket connection");
        let (mut sink, mut stream) = socket.split();
        // Bounded per-connection egress queue (rippled send_queue_limit).
        let (sender, mut receiver) = mpsc::channel(WS_SEND_QUEUE_LIMIT);
        let mut metadata = RequestMetadata::from_headers(remote_addr, &headers);
        metadata.local_addr = self
            .config
            .port_policy
            .as_ref()
            .map(|policy| policy.socket_addr);
        metadata.forwarded_for = forwarded_for(&headers).unwrap_or_default();
        metadata.user = headers
            .get("x-user")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        metadata.is_websocket = true;
        let session = WSSession::new(1, metadata.clone(), sender, self.subscriptions.clone());
        let too_slow = session.too_slow_token();

        // Single writer per connection. Frames already queued are written
        // back-to-back and flushed once (tungstenite `send` = write + flush,
        // i.e. one syscall per frame), bounded per flush so a hot stream
        // cannot delay the flush indefinitely.
        let writer_too_slow = too_slow.clone();
        let writer = tokio::spawn(async move {
            loop {
                let first = tokio::select! {
                    biased;
                    _ = writer_too_slow.cancelled() => {
                        let _ = sink
                            .send(Message::Close(Some(axum::extract::ws::CloseFrame {
                                code: axum::extract::ws::close_code::POLICY,
                                reason: "Policy error: client is too slow.".into(),
                            })))
                            .await;
                        break;
                    }
                    message = receiver.recv() => match message {
                        Some(message) => message,
                        None => break,
                    },
                };
                let mut closing = matches!(first, Message::Close(_));
                if sink.feed(first).await.is_err() {
                    break;
                }
                let mut frames = 1;
                while !closing && frames < WS_MAX_FRAMES_PER_FLUSH {
                    let Ok(message) = receiver.try_recv() else {
                        break;
                    };
                    closing = matches!(message, Message::Close(_));
                    if sink.feed(message).await.is_err() {
                        return;
                    }
                    frames += 1;
                }
                if sink.flush().await.is_err() || closing {
                    break;
                }
            }
        });

        loop {
            let next = tokio::select! {
                biased;
                _ = too_slow.cancelled() => {
                    tracing::info!(target: "server", client_ip = %remote_addr.ip(), "WebSocket client too slow; disconnecting");
                    break;
                }
                next = stream.next() => next,
            };
            let Some(result) = next else {
                break;
            };
            let Ok(message) = result else {
                break;
            };

            match message {
                Message::Text(text) => {
                    // Single-pass SIMD parse straight into the protocol
                    // representation (see RawRequest).
                    let parsed = match RawRequest::parse(text.as_bytes()) {
                        Ok(v) => v,
                        Err(_) => {
                            let response = websocket_error_response(
                                None,
                                None,
                                None,
                                rpc::RpcErrorCode::BadSyntax,
                                rpc::RpcErrorCode::BadSyntax.message(),
                            );
                            let _ = session
                                .send_reply(sonic_rs::to_string(&response).unwrap_or_default())
                                .await;
                            continue;
                        }
                    };

                    let Ok(mut envelope) = JsonRpcEnvelope::try_from(parsed) else {
                        continue;
                    };

                    // The error path echoes the request as received. Only the
                    // array form differs from the normalized params, so only
                    // that form is retained separately.
                    let raw_params = envelope.params.take();
                    let had_params = raw_params.is_some();
                    let raw_array_params = match &raw_params {
                        Some(value @ JsonValue::Array(_)) => Some(value.clone()),
                        _ => None,
                    };
                    let params = normalize_rpc_params(
                        raw_params.unwrap_or_else(|| JsonValue::Object(BTreeMap::new())),
                    );
                    let mut metadata = metadata.clone();
                    metadata.api_version = Self::api_version_from_params(&params);
                    metadata.role = request_role(
                        RpcRole::User,
                        &self.auth,
                        &metadata,
                        &params,
                        &metadata.user,
                    );
                    if !matches!(metadata.role, RpcRole::Identified | RpcRole::Proxy) {
                        metadata.user.clear();
                        metadata.forwarded_for.clear();
                    }
                    metadata.unlimited =
                        matches!(metadata.role, RpcRole::Admin | RpcRole::Identified);
                    let api_version = metadata.api_version;
                    let is_subscription =
                        envelope.method == "subscribe" || envelope.method == "unsubscribe";

                    let (reply, params) = if is_subscription {
                        let reply = self.dispatcher.dispatch(RpcRequest {
                            method: &envelope.method,
                            params: &params,
                            metadata: &metadata,
                            session: Some(&session),
                        });
                        (reply, params)
                    } else {
                        // WSSession is not passed into spawn_blocking; subscription
                        // side-effects that need the session run synchronously in
                        // the branch above.
                        self.dispatch_async(envelope.method.clone(), params, metadata)
                            .await
                    };
                    let explicit_api_version = has_explicit_api_version(&params);
                    let reply_msg = match reply {
                        RpcReply::PreRendered(bytes) => {
                            let mut prefix = Vec::new();
                            prefix.extend_from_slice(b"{");
                            prefix.extend_from_slice(b"\"type\":\"response\",");
                            prefix.extend_from_slice(b"\"status\":\"success\",");
                            if let Some(ver) = envelope.jsonrpc.as_deref() {
                                prefix.extend_from_slice(b"\"jsonrpc\":\"");
                                prefix.extend_from_slice(ver.as_bytes());
                                prefix.extend_from_slice(b"\",");
                            }
                            if let Some(id_val) = &envelope.id {
                                prefix.extend_from_slice(b"\"id\":");
                                prefix.extend_from_slice(
                                    &sonic_rs::to_vec(id_val).unwrap_or_default(),
                                );
                                prefix.extend_from_slice(b",");
                            } else {
                                prefix.extend_from_slice(b"\"id\":null,");
                            }
                            if explicit_api_version {
                                prefix.extend_from_slice(b"\"api_version\":");
                                prefix.extend_from_slice(api_version.to_string().as_bytes());
                                prefix.extend_from_slice(b",");
                            }
                            prefix.extend_from_slice(b"\"result\":");

                            let mut out = Vec::with_capacity(prefix.len() + bytes.len() + 1);
                            out.extend_from_slice(&prefix);
                            out.extend_from_slice(&bytes);
                            out.extend_from_slice(b"}");
                            String::from_utf8(out).unwrap_or_default()
                        }
                        reply => {
                            let request_echo = if !had_params {
                                None
                            } else if let Some(array) = raw_array_params.as_ref() {
                                Some(array)
                            } else {
                                Some(&params)
                            };
                            websocket_response_text(
                                &envelope,
                                &params,
                                request_echo,
                                reply,
                                api_version,
                                explicit_api_version,
                            )
                        }
                    };
                    let _ = session.send_reply(reply_msg).await;
                }
                Message::Close(_) => break,
                Message::Ping(_) | Message::Pong(_) | Message::Binary(_) => {}
            }
        }

        session.complete();
        tracing::debug!(target: "server", client_ip = %remote_addr.ip(), "WebSocket disconnected");
        writer.abort();
    }
}

#[derive(Debug)]
struct JsonRpcEnvelope {
    id: Option<Value>,
    jsonrpc: Option<String>,
    method: String,
    params: Option<JsonValue>,
}

impl JsonRpcEnvelope {
    #[allow(clippy::result_large_err)]
    fn try_from(raw: RawRequest) -> Result<Self, Response> {
        let (id, jsonrpc, method, params) = match raw {
            RawRequest::Object {
                id,
                fields: mut map,
            } => {
                let jsonrpc = map
                    .get("jsonrpc")
                    .and_then(|v| v.as_str())
                    .map(String::from);

                if let Some(command) = map.get("command").and_then(|v| v.as_str()) {
                    let method = command.to_owned();
                    (id, jsonrpc, method, Some(JsonValue::Object(map)))
                } else if let Some(method_val) = map.get("method").and_then(|v| v.as_str()) {
                    let method = method_val.to_owned();
                    let params = map.remove("params");
                    (id, jsonrpc, method, params)
                } else {
                    return Err(json_rpc_error_response(
                        None,
                        jsonrpc.as_deref(),
                        rpc::RpcErrorCode::InvalidParams,
                        "Invalid JSON-RPC request.",
                    ));
                }
            }
            RawRequest::NotObject => {
                return Err(json_rpc_error_response(
                    None,
                    None,
                    rpc::RpcErrorCode::InvalidParams,
                    "Invalid JSON-RPC request.",
                ));
            }
        };

        Ok(Self {
            id,
            jsonrpc,
            method,
            params,
        })
    }
}

fn json_rpc_response(id: Option<Value>, jsonrpc: Option<&str>, reply: RpcReply) -> Value {
    let mut response = serde_json::Map::new();
    if let Some(ver) = jsonrpc {
        response.insert("jsonrpc".to_owned(), Value::String(ver.to_owned()));
        response.insert("id".to_owned(), id.unwrap_or(Value::Null));
    }
    match reply {
        RpcReply::Result(result) => {
            response.insert(
                "result".to_owned(),
                result_with_default_status(from_protocol_json(&result)),
            );
        }
        RpcReply::Error(error) => {
            let mut error_object = serde_json::Map::new();
            error_object.insert("code".to_owned(), Value::from(error.code));
            error_object.insert("token".to_owned(), Value::String(error.token));
            error_object.insert("message".to_owned(), Value::String(error.message));
            response.insert("error".to_owned(), Value::Object(error_object));
        }
        RpcReply::PreRendered(bytes) => {
            // Fallback for batch arrays or json_rpc_response usages where we MUST return a Value
            if let Ok(val) = serde_json::from_slice(&bytes) {
                response.insert("result".to_owned(), val);
            }
        }
    }
    Value::Object(response)
}

fn result_with_default_status(mut value: Value) -> Value {
    let Value::Object(object) = &mut value else {
        return value;
    };
    if object.contains_key("status") {
        return value;
    }

    let status = if object.contains_key("error") {
        "error"
    } else {
        "success"
    };
    object.insert("status".to_owned(), Value::String(status.to_owned()));
    value
}

fn json_rpc_error_response(
    id: Option<Value>,
    jsonrpc: Option<&str>,
    code: rpc::RpcErrorCode,
    message: impl Into<String>,
) -> Response {
    let reply = RpcReply::error(code, message);
    (StatusCode::OK, Json(json_rpc_response(id, jsonrpc, reply))).into_response()
}

fn sanitize_request_value(value: &Value) -> Value {
    let mut value = value.clone();
    let Value::Object(object) = &mut value else {
        return value;
    };

    for key in ["passphrase", "secret", "seed", "seed_hex"] {
        if object.contains_key(key) {
            object.insert(key.to_owned(), Value::String("<masked>".to_owned()));
        }
    }

    value
}

fn protocol_has_error(value: &JsonValue) -> bool {
    matches!(value, JsonValue::Object(object) if object.contains_key("error"))
}

fn websocket_error_response(
    id: Option<Value>,
    jsonrpc: Option<&str>,
    request: Option<&Value>,
    code: rpc::RpcErrorCode,
    message: impl Into<String>,
) -> Value {
    let mut response = serde_json::Map::new();
    response.insert("type".to_owned(), Value::String("response".to_owned()));
    response.insert("status".to_owned(), Value::String("error".to_owned()));
    if let Some(id) = id {
        response.insert("id".to_owned(), id);
    }
    if let Some(jsonrpc) = jsonrpc {
        response.insert("jsonrpc".to_owned(), Value::String(jsonrpc.to_owned()));
    }
    if let Some(request) = request {
        response.insert("request".to_owned(), sanitize_request_value(request));
    }

    let mut error = serde_json::Map::new();
    error.insert("error".to_owned(), Value::String(code.token().to_owned()));
    error.insert("error_code".to_owned(), Value::from(code.code()));
    error.insert("error_message".to_owned(), Value::String(message.into()));

    response.extend(error);
    Value::Object(response)
}

fn has_explicit_api_version(params: &JsonValue) -> bool {
    let JsonValue::Object(object) = params else {
        return false;
    };
    object.contains_key("api_version")
}

fn normalize_rpc_params(params: JsonValue) -> JsonValue {
    match params {
        JsonValue::Array(mut values) => values
            .drain(..)
            .next()
            .unwrap_or_else(|| JsonValue::Object(BTreeMap::new())),
        other => other,
    }
}

/// Serialize a WebSocket reply. Successful results take a direct path that
/// serializes the protocol tree in place (byte-identical to
/// `websocket_response` + `sonic_rs::to_string`); everything else falls back
/// to the `serde_json::Value` builder.
fn websocket_response_text(
    envelope: &JsonRpcEnvelope,
    params: &JsonValue,
    request_echo: Option<&JsonValue>,
    reply: RpcReply,
    api_version: u32,
    explicit_api_version: bool,
) -> String {
    if let RpcReply::Result(result) = &reply
        && !protocol_has_error(result)
    {
        let mut response = Envelope::with_capacity(6);
        response
            .insert("type", EnvelopeField::Str("response"))
            .insert("status", EnvelopeField::Str("success"))
            .insert("result", EnvelopeField::Proto(result));
        if let Some(id) = envelope.id.as_ref() {
            response.insert("id", EnvelopeField::Json(id));
        }
        if let Some(jsonrpc) = envelope.jsonrpc.as_deref() {
            response.insert("jsonrpc", EnvelopeField::Str(jsonrpc));
        }
        if explicit_api_version {
            response.insert("api_version", EnvelopeField::U32(api_version));
        }
        return sonic_rs::to_string(&response).unwrap_or_default();
    }
    let response = websocket_response(
        envelope,
        params,
        request_echo,
        reply,
        api_version,
        explicit_api_version,
    );
    sonic_rs::to_string(&response).unwrap_or_default()
}

fn websocket_response(
    envelope: &JsonRpcEnvelope,
    params: &JsonValue,
    request_echo: Option<&JsonValue>,
    reply: RpcReply,
    api_version: u32,
    explicit_api_version: bool,
) -> Value {
    match reply {
        RpcReply::Result(result) => {
            if protocol_has_error(&result) {
                let mut response = from_protocol_json(&result);
                let Value::Object(object) = &mut response else {
                    panic!("rpc error result must be an object");
                };
                object.insert("type".to_owned(), Value::String("response".to_owned()));
                object.insert("status".to_owned(), Value::String("error".to_owned()));
                if let Some(id) = envelope.id.clone() {
                    object.insert("id".to_owned(), id);
                }
                if let Some(jsonrpc) = envelope.jsonrpc.as_deref() {
                    object.insert("jsonrpc".to_owned(), Value::String(jsonrpc.to_owned()));
                }
                if explicit_api_version {
                    object.insert("api_version".to_owned(), Value::from(api_version));
                }

                let mut request = from_protocol_json(params);
                if let Value::Object(request) = &mut request {
                    request.remove("command");
                    request.remove("method");
                }
                object.insert("request".to_owned(), sanitize_request_value(&request));
                response
            } else {
                let mut response = serde_json::Map::new();
                response.insert("type".to_owned(), Value::String("response".to_owned()));
                response.insert("status".to_owned(), Value::String("success".to_owned()));
                response.insert("result".to_owned(), from_protocol_json(&result));
                if let Some(id) = envelope.id.clone() {
                    response.insert("id".to_owned(), id);
                }
                if let Some(jsonrpc) = envelope.jsonrpc.as_deref() {
                    response.insert("jsonrpc".to_owned(), Value::String(jsonrpc.to_owned()));
                }
                if explicit_api_version {
                    response.insert("api_version".to_owned(), Value::from(api_version));
                }
                Value::Object(response)
            }
        }
        RpcReply::Error(error) => websocket_error_response(
            envelope.id.clone(),
            envelope.jsonrpc.as_deref(),
            request_echo.map(from_protocol_json).as_ref(),
            rpc::RpcErrorCode::Internal,
            error.message,
        ),
        RpcReply::PreRendered(bytes) => match sonic_rs::from_slice::<Value>(&bytes) {
            Ok(mut v) => {
                if let Value::Object(obj) = &mut v {
                    obj.insert("type".to_owned(), Value::String("response".to_owned()));
                    obj.insert("status".to_owned(), Value::String("success".to_owned()));
                    if let Some(id) = envelope.id.clone() {
                        obj.insert("id".to_owned(), id);
                    }
                }
                v
            }
            Err(_) => Value::Null,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{
        JsonRpcEnvelope, RpcServer, RpcServerPortBuild, RpcServerPortPolicy,
        sanitize_request_value, websocket_response,
    };
    use crate::transport::{RpcDispatcher, RpcReply, RpcRequest};
    use app::ServerPortSetup;
    use axum::body::Body;
    use axum::http::Request;
    use protocol::JsonValue;
    use serde_json::{Value, json};
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct NoopDispatcher;

    impl RpcDispatcher for NoopDispatcher {
        fn dispatch(&self, _request: RpcRequest<'_>) -> RpcReply {
            RpcReply::result(JsonValue::Object(BTreeMap::new()))
        }
    }

    struct CountingDispatcher(AtomicU64);

    impl RpcDispatcher for CountingDispatcher {
        fn dispatch(&self, _request: RpcRequest<'_>) -> RpcReply {
            RpcReply::result(JsonValue::Unsigned(self.0.fetch_add(1, Ordering::SeqCst)))
        }
    }

    fn test_metadata() -> crate::session::RequestMetadata {
        crate::session::RequestMetadata::new(
            "127.0.0.1:50000".parse().unwrap(),
            &Request::new(Body::empty()),
        )
    }

    #[tokio::test]
    async fn identical_concurrent_requests_are_dispatched_independently() {
        let server = Arc::new(RpcServer::new(CountingDispatcher(AtomicU64::new(0))));
        let params = JsonValue::Object(BTreeMap::new());
        let mut tasks = Vec::new();

        for _ in 0..32 {
            let server = Arc::clone(&server);
            let params = params.clone();
            tasks.push(tokio::spawn(async move {
                server
                    .dispatch_async("wallet_propose".to_owned(), params, test_metadata())
                    .await
                    .0
            }));
        }

        let mut values = Vec::new();
        for task in tasks {
            match task.await.unwrap() {
                RpcReply::Result(JsonValue::Unsigned(value)) => values.push(value),
                reply => panic!("unexpected reply: {reply:?}"),
            }
        }
        values.sort_unstable();
        assert_eq!(values, (0..32).collect::<Vec<_>>());
    }

    #[test]
    fn server_port_policy_rejects_unsupported_transport_modes() {
        let secure_port = ServerPortSetup {
            name: "port_secure".to_owned(),
            ip: "127.0.0.1".to_owned(),
            port: 5006,
            limit: 0,
            protocols: vec!["https".to_owned()],
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
        };
        // https is now treated as http+TLS, so it should succeed
        assert!(RpcServerPortPolicy::try_from(&secure_port).is_ok());

        let peer_port = ServerPortSetup {
            name: "port_peer".to_owned(),
            ip: "127.0.0.1".to_owned(),
            port: 51235,
            limit: 0,
            protocols: vec!["peer".to_owned()],
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
        };
        assert!(RpcServerPortPolicy::try_from(&peer_port).is_err());
    }

    #[test]
    fn server_port_build_reports_deferred_modes_for_mixed_listener_ports() {
        let mixed_port = ServerPortSetup {
            name: "port_mixed".to_owned(),
            ip: "127.0.0.1".to_owned(),
            port: 5007,
            limit: 0,
            protocols: vec!["http".to_owned(), "peer".to_owned(), "https".to_owned()],
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
        };

        let build =
            RpcServerPortBuild::from_server_port(&mixed_port).expect("mixed port should classify");
        let policy = build.policy.expect("http listener should still be built");
        assert!(policy.allow_http);
        assert!(!policy.allow_ws);
        assert_eq!(build.deferred_protocols.len(), 1);
        assert!(
            build
                .deferred_protocols
                .iter()
                .any(|protocol| protocol.protocol == "peer")
        );
    }

    #[test]
    fn websocket_response_uses_serverhandler_success_shape() {
        let envelope = JsonRpcEnvelope {
            id: Some(Value::from(7)),
            jsonrpc: Some("2.0".to_owned()),
            method: "ping".to_owned(),
            params: Some(crate::json::to_protocol_json(json!({"api_version": 2}))),
        };
        let reply = RpcReply::result(JsonValue::Object(BTreeMap::from([(
            "ok".to_owned(),
            JsonValue::Bool(true),
        )])));

        let response = websocket_response(
            &envelope,
            &JsonValue::Object(BTreeMap::from([(
                "api_version".to_owned(),
                JsonValue::Unsigned(2),
            )])),
            envelope.params.as_ref(),
            reply,
            2,
            true,
        );

        assert_eq!(response["type"], "response");
        assert_eq!(response["status"], "success");
        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], 7);
        assert_eq!(response["api_version"], 2);
        assert_eq!(response["result"]["ok"], true);
    }

    #[test]
    fn websocket_success_fast_path_is_byte_identical_to_value_path() {
        let result = JsonValue::Object(BTreeMap::from([
            ("zz".to_owned(), JsonValue::Signed(-3)),
            ("aa".to_owned(), JsonValue::String("x\"y".to_owned())),
            (
                "nested".to_owned(),
                JsonValue::Array(vec![JsonValue::Null, JsonValue::Unsigned(9)]),
            ),
        ]));
        for (id, jsonrpc, explicit) in [
            (Some(json!(7)), Some("2.0"), true),
            (None, None, false),
            (Some(json!("a")), None, true),
            (Some(json!(1.5)), Some("2.0"), false),
        ] {
            let envelope = JsonRpcEnvelope {
                id,
                jsonrpc: jsonrpc.map(str::to_owned),
                method: "account_info".to_owned(),
                params: None,
            };
            let params = JsonValue::Object(BTreeMap::new());
            let expected = sonic_rs::to_string(&websocket_response(
                &envelope,
                &params,
                None,
                RpcReply::result(result.clone()),
                2,
                explicit,
            ))
            .unwrap();
            let actual = super::websocket_response_text(
                &envelope,
                &params,
                None,
                RpcReply::result(result.clone()),
                2,
                explicit,
            );
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn json_rpc_array_params_are_unwrapped_for_dispatch() {
        let params = super::normalize_rpc_params(JsonValue::Array(vec![JsonValue::Object(
            BTreeMap::from([
                ("api_version".to_owned(), JsonValue::Unsigned(2)),
                (
                    "transaction".to_owned(),
                    JsonValue::String("ABC".to_owned()),
                ),
            ]),
        )]));

        let JsonValue::Object(object) = &params else {
            panic!("array params should unwrap to first object");
        };

        assert_eq!(
            object.get("transaction"),
            Some(&JsonValue::String("ABC".to_owned()))
        );
        assert_eq!(
            RpcServer::<NoopDispatcher>::api_version_from_params(&params),
            2
        );
        assert!(super::has_explicit_api_version(&params));
    }

    #[test]
    fn websocket_response_masks_request_secrets_on_error() {
        let envelope = JsonRpcEnvelope {
            id: Some(Value::from(9)),
            jsonrpc: Some("2.0".to_owned()),
            method: "server_info".to_owned(),
            params: Some(crate::json::to_protocol_json(json!({
                "method": "server_state",
                "secret": "super-secret"
            }))),
        };
        let reply = RpcReply::result(JsonValue::Object(BTreeMap::from([
            (
                "error".to_owned(),
                JsonValue::String("unknownCmd".to_owned()),
            ),
            (
                "error_code".to_owned(),
                JsonValue::Signed(i64::from(rpc::RpcErrorCode::UnknownCommand.code())),
            ),
            (
                "error_message".to_owned(),
                JsonValue::String(rpc::RpcErrorCode::UnknownCommand.message().to_owned()),
            ),
        ])));

        let response = websocket_response(
            &envelope,
            &JsonValue::Object(BTreeMap::from([
                (
                    "command".to_owned(),
                    JsonValue::String("server_info".to_owned()),
                ),
                (
                    "method".to_owned(),
                    JsonValue::String("server_state".to_owned()),
                ),
                (
                    "secret".to_owned(),
                    JsonValue::String("super-secret".to_owned()),
                ),
            ])),
            envelope.params.as_ref(),
            reply,
            1,
            false,
        );

        assert_eq!(response["type"], "response");
        assert_eq!(response["status"], "error");
        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], 9);
        assert_eq!(response["error"], "unknownCmd");
        assert_eq!(response["request"]["secret"], "<masked>");
        assert!(response["request"].get("method").is_none());
        assert!(response["request"].get("command").is_none());
    }

    #[test]
    fn sanitize_request_value_masks_secret_variants() {
        let request = json!({
            "secret": "a",
            "seed": "b",
            "seed_hex": "c",
            "passphrase": "d"
        });

        let sanitized = sanitize_request_value(&request);
        assert_eq!(sanitized["secret"], "<masked>");
        assert_eq!(sanitized["seed"], "<masked>");
        assert_eq!(sanitized["seed_hex"], "<masked>");
        assert_eq!(sanitized["passphrase"], "<masked>");
    }
}
