use app::ServerPortSetup;
use basics::make_ssl_context::{
    TlsIdentityDer, anonymous_tls_identity_der, authenticated_tls_identity_der,
};
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Json, State};
use axum::http::{HeaderMap, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use futures::{SinkExt, StreamExt};
use protocol::JsonValue;
use rustls::ServerConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use serde::Deserialize;
use serde_json::Value;
// sonic-rs provides SIMD-accelerated JSON serialization for outbound
// RPC responses (3-10x faster than serde_json on large payloads).
use tokio::net::TcpListener;
use tokio::sync::mpsc;

use crate::auth::{ServerAuth, ServerAuthConfig, authorized_http, forwarded_for, request_role};
use crate::json::{from_protocol_json, to_protocol_json};
use crate::session::{RequestMetadata, Session, WSSession};
use crate::status::{ServerStatusSource, invalid_protocol_response, status_page_response};
use crate::subscriptions::SubscriptionManager;
use crate::transport::{RpcDispatcher, RpcReply, RpcRequest};
use rpc::RpcRole;

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

fn build_tls_config(port: &ServerPortSetup) -> Result<Arc<ServerConfig>, String> {
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
    pub in_flight: dashmap::DashMap<String, tokio::sync::watch::Receiver<Option<RpcReply>>>,
    pub p0_pool: tokio::sync::Semaphore,
    pub p1_pool: tokio::sync::Semaphore,
    pub p2_pool: tokio::sync::Semaphore,
}

impl Default for RpcServerState {
    fn default() -> Self {
        Self {
            in_flight: dashmap::DashMap::new(),
            p0_pool: tokio::sync::Semaphore::new(128),
            p1_pool: tokio::sync::Semaphore::new(64),
            p2_pool: tokio::sync::Semaphore::new(16),
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
        Self {
            dispatcher: Arc::new(dispatcher),
            subscriptions: Arc::new(SubscriptionManager::default()),
            auth: ServerAuth::new(policy.auth.clone()),
            config: RpcServerConfig {
                port_policy: Some(policy),
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
        let mut server = Self::with_port_policy(dispatcher, policy);
        server.config.status_source = Some(status_source);
        server
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
    ) -> RpcReply {
        let hash_key = format!(
            "{}:{}",
            method,
            sonic_rs::to_string(&params).unwrap_or_default()
        );

        // Atomically check-or-insert to prevent the race where two threads
        // both see an empty slot and both start computing.
        use dashmap::mapref::entry::Entry;
        let rx_opt = match self.state.in_flight.entry(hash_key.clone()) {
            Entry::Occupied(e) => Some(e.get().clone()),
            Entry::Vacant(_) => None,
        };

        if let Some(mut rx) = rx_opt
            && rx.changed().await.is_ok()
            && let Some(reply) = rx.borrow().clone()
        {
            return reply;
        }

        let (tx, rx) = tokio::sync::watch::channel(None);
        self.state.in_flight.insert(hash_key.clone(), rx);

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
                let _ = tx.send(Some(reply.clone()));
                self.state.in_flight.remove(&hash_key);
                return reply;
            }
        }

        let permit = match method.as_str() {
            "submit" | "fee" => self.state.p0_pool.acquire().await.unwrap(),
            "ledger_data" => self.state.p2_pool.acquire().await.unwrap(),
            _ => self.state.p1_pool.acquire().await.unwrap(),
        };

        let dispatcher = self.dispatcher.clone();
        let method_owned = method.clone();
        let reply = tokio::task::spawn_blocking(move || {
            dispatcher.dispatch(RpcRequest {
                method: &method_owned,
                params: &params,
                metadata: &metadata,
                session: None,
            })
        })
        .await
        .expect("dispatcher::dispatch panicked");

        drop(permit);

        let _ = tx.send(Some(reply.clone()));
        self.state.in_flight.remove(&hash_key);

        reply
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
        axum::serve(
            listener,
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
        // Parse the body manually to match reference behavior.
        // Use serde_json for inbound parsing (preserves key ordering, small
        // payloads). sonic-rs is used for outbound serialization where the
        // large response payloads benefit from SIMD acceleration.
        let payload: Value = match serde_json::from_slice(&body) {
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

        let mut request = Request::new(Body::from(Vec::<u8>::new()));
        *request.headers_mut() = headers.clone();
        let mut metadata = RequestMetadata::new(remote_addr, &request);
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
        let request = Session::new(request, metadata);
        let rpc_request = match JsonRpcEnvelope::try_from(payload) {
            Ok(value) => value,
            Err(response) => return response.into_response(),
        };

        let params = normalize_rpc_params(to_protocol_json(
            rpc_request
                .params
                .unwrap_or_else(|| Value::Object(serde_json::Map::new())),
        ));
        let mut metadata = request.metadata().clone();
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

        let method_owned = rpc_request.method.clone();
        let params_owned = params.clone();
        let metadata_owned = metadata.clone();
        let reply = server
            .dispatch_async(method_owned, params_owned, metadata_owned)
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
                // Use sonic-rs (SIMD-accelerated) for output serialization instead of
                // axum's default serde_json path for a 3-10x throughput improvement.
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

        let mut base_request = Request::new(Body::from(Vec::<u8>::new()));
        *base_request.headers_mut() = headers.clone();
        let mut metadata = RequestMetadata::new(remote_addr, &base_request);
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
            let rpc_request = match JsonRpcEnvelope::try_from(payload) {
                Ok(value) => value,
                Err(_) => {
                    responses.push(Value::Null);
                    continue;
                }
            };

            let params = normalize_rpc_params(to_protocol_json(
                rpc_request
                    .params
                    .unwrap_or_else(|| Value::Object(serde_json::Map::new())),
            ));
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
            let params_owned = params.clone();
            let req_metadata_owned = req_metadata.clone();
            let reply = server
                .dispatch_async(method_owned, params_owned, req_metadata_owned)
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
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let mut request = Request::new(Body::from(Vec::<u8>::new()));
        *request.headers_mut() = headers.clone();
        let mut metadata = RequestMetadata::new(remote_addr, &request);
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

        let writer = tokio::spawn(async move {
            while let Some(message) = receiver.recv().await {
                if sink.send(message).await.is_err() {
                    break;
                }
            }
        });

        while let Some(result) = stream.next().await {
            let Ok(message) = result else {
                break;
            };

            match message {
                Message::Text(text) => {
                    // Parse text frames with serde_json for inbound (preserves key
                    // ordering). sonic-rs used only for outbound serialization.
                    let parsed: Value = match serde_json::from_str(&text) {
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
                                .send_text(sonic_rs::to_string(&response).unwrap_or_default());
                            continue;
                        }
                    };

                    let Ok(envelope) = JsonRpcEnvelope::try_from(parsed) else {
                        continue;
                    };

                    let request_params = envelope
                        .params
                        .clone()
                        .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
                    let params = normalize_rpc_params(to_protocol_json(request_params));
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
                    let dispatcher = self.dispatcher.clone();
                    let method_owned = envelope.method.clone();
                    let params_owned = params.clone();
                    let metadata_owned = metadata.clone();
                    let is_subscription =
                        method_owned == "subscribe" || method_owned == "unsubscribe";

                    let reply = if is_subscription {
                        dispatcher.dispatch(RpcRequest {
                            method: &method_owned,
                            params: &params_owned,
                            metadata: &metadata_owned,
                            session: Some(&session),
                        })
                    } else {
                        // Note: WSSession is not passed into spawn_blocking because it
                        // holds an unbounded sender (not blocking) and its subscription
                        // callbacks are async. Subscription side-effects that need the
                        // session are handled by the dispatcher synchronously before
                        // returning; passing session: None here is intentional for the
                        // blocking offload — the session ref is used post-dispatch.
                        self.dispatch_async(method_owned, params_owned, metadata_owned)
                            .await
                    };
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
                            if has_explicit_api_version(&params) {
                                prefix.extend_from_slice(b"\"api_version\":");
                                prefix
                                    .extend_from_slice(metadata.api_version.to_string().as_bytes());
                                prefix.extend_from_slice(b",");
                            }
                            prefix.extend_from_slice(b"\"result\":");

                            let mut out = Vec::with_capacity(prefix.len() + bytes.len() + 1);
                            out.extend_from_slice(&prefix);
                            out.extend_from_slice(&bytes);
                            out.extend_from_slice(b"}");
                            String::from_utf8(out).unwrap_or_default()
                        }
                        _ => {
                            let response = websocket_response(
                                &envelope,
                                &params,
                                reply,
                                metadata.api_version,
                                has_explicit_api_version(&params),
                            );
                            sonic_rs::to_string(&response).unwrap_or_default()
                        }
                    };
                    let _ = session.send_text(reply_msg);
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

#[derive(Debug, Deserialize)]
struct JsonRpcEnvelope {
    #[serde(default)]
    id: Option<Value>,
    #[serde(default)]
    jsonrpc: Option<String>,
    method: String,
    #[serde(default)]
    params: Option<Value>,
}

impl JsonRpcEnvelope {
    #[allow(clippy::result_large_err)]
    fn try_from(value: Value) -> Result<Self, Response> {
        let (id, jsonrpc, method, params) = match value {
            Value::Object(mut map) => {
                let id = map.get("id").cloned();
                let jsonrpc = map
                    .get("jsonrpc")
                    .and_then(|v| v.as_str())
                    .map(String::from);

                if let Some(command) = map.get("command").and_then(|v| v.as_str()) {
                    let method = command.to_owned();
                    (id, jsonrpc, method, Some(Value::Object(map)))
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
            _ => {
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

fn websocket_response(
    envelope: &JsonRpcEnvelope,
    params: &JsonValue,
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
            envelope.params.as_ref(),
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
    use std::sync::Once;
    fn install_crypto() {
        static INSTALL: Once = Once::new();
        INSTALL.call_once(|| {
            let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        });
    }
    use super::{
        JsonRpcEnvelope, RpcServer, RpcServerPortBuild, RpcServerPortPolicy,
        sanitize_request_value, websocket_response,
    };
    use crate::transport::{RpcDispatcher, RpcReply, RpcRequest};
    use app::ServerPortSetup;
    use protocol::JsonValue;
    use serde_json::{Value, json};
    use std::collections::BTreeMap;

    struct NoopDispatcher;

    impl RpcDispatcher for NoopDispatcher {
        fn dispatch(&self, _request: RpcRequest<'_>) -> RpcReply {
            RpcReply::result(JsonValue::Object(BTreeMap::new()))
        }
    }

    #[test]
    fn server_port_policy_rejects_unsupported_transport_modes() {
        install_crypto();
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
        install_crypto();
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
            params: Some(json!({"api_version": 2})),
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
            params: Some(json!({
                "method": "server_state",
                "secret": "super-secret"
            })),
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
