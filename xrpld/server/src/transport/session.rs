use std::any::Any;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::ws::Message;
use http::{HeaderMap, Method, Request, Uri, Version};
use protocol::JsonValue;
use rpc::RpcRole;
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;

use crate::json::from_protocol_json;
use crate::subscriptions::{StreamKind, SubscriptionEvent, SubscriptionManager};

#[derive(Debug, Default)]
struct WsRpcState {
    api_version: u32,
    path_request_id: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct RequestMetadata {
    pub remote_addr: SocketAddr,
    pub local_addr: Option<SocketAddr>,
    pub headers: HeaderMap,
    pub method: Method,
    pub uri: Uri,
    pub version: Version,
    pub keep_alive: bool,
    pub role: RpcRole,
    pub user: String,
    pub forwarded_for: String,
    pub api_version: u32,
    pub unlimited: bool,
    pub is_websocket: bool,
}

impl RequestMetadata {
    pub fn new(remote_addr: SocketAddr, request: &Request<Body>) -> Self {
        let keep_alive = request
            .headers()
            .get(http::header::CONNECTION)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.eq_ignore_ascii_case("keep-alive"))
            || request.version() >= Version::HTTP_11;

        Self {
            remote_addr,
            local_addr: None,
            headers: request.headers().clone(),
            method: request.method().clone(),
            uri: request.uri().clone(),
            version: request.version(),
            keep_alive,
            role: RpcRole::Guest,
            user: String::new(),
            forwarded_for: String::new(),
            api_version: 1,
            unlimited: false,
            is_websocket: false,
        }
    }

    pub fn request_headers(&self) -> BTreeMap<String, String> {
        self.headers
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|text| (name.as_str().to_owned(), text.to_owned()))
            })
            .collect()
    }
}

pub struct Session {
    request: Request<Body>,
    metadata: RequestMetadata,
}

impl Session {
    pub fn new(request: Request<Body>, metadata: RequestMetadata) -> Self {
        Self { request, metadata }
    }

    pub fn request(&self) -> &Request<Body> {
        &self.request
    }

    pub fn metadata(&self) -> &RequestMetadata {
        &self.metadata
    }

    pub fn into_parts(self) -> (Request<Body>, RequestMetadata) {
        (self.request, self.metadata)
    }
}

pub struct WSSession {
    id: u64,
    metadata: RequestMetadata,
    sender: mpsc::UnboundedSender<Message>,
    subscriptions: Arc<SubscriptionManager>,
    tasks: Mutex<HashMap<StreamKind, Vec<JoinHandle<()>>>>,
    app_defined: Mutex<Option<Arc<dyn Any + Send + Sync>>>,
    rpc_state: Mutex<WsRpcState>,
}

impl WSSession {
    pub fn new(
        id: u64,
        metadata: RequestMetadata,
        sender: mpsc::UnboundedSender<Message>,
        subscriptions: Arc<SubscriptionManager>,
    ) -> Self {
        Self {
            id,
            metadata,
            sender,
            subscriptions,
            tasks: Mutex::new(HashMap::new()),
            app_defined: Mutex::new(None),
            rpc_state: Mutex::new(WsRpcState::default()),
        }
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn request(&self) -> &RequestMetadata {
        &self.metadata
    }

    pub fn remote_endpoint(&self) -> SocketAddr {
        self.metadata.remote_addr
    }

    pub fn send_text(
        &self,
        text: impl Into<String>,
    ) -> Result<(), mpsc::error::SendError<Message>> {
        self.sender.send(Message::Text(text.into().into()))
    }

    pub fn send_json(&self, value: &JsonValue) -> Result<(), mpsc::error::SendError<Message>> {
        self.sender
            .send(Message::Text(from_protocol_json(value).to_string().into()))
    }

    pub fn close(&self) -> Result<(), mpsc::error::SendError<Message>> {
        self.sender.send(Message::Close(None))
    }

    pub fn set_app_defined(&self, value: Arc<dyn Any + Send + Sync>) {
        *self.app_defined.lock().expect("app_defined mutex poisoned") = Some(value);
    }

    pub fn app_defined(&self) -> Option<Arc<dyn Any + Send + Sync>> {
        self.app_defined
            .lock()
            .expect("app_defined mutex poisoned")
            .clone()
    }

    pub fn subscribe_stream(&self, stream: StreamKind) {
        let mut rx = self.subscriptions.subscribe(stream);
        let sender = self.sender.clone();
        let handle = tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(SubscriptionEvent { payload, .. }) => {
                        let text = from_protocol_json(&payload).to_string();
                        if sender.send(Message::Text(text.into())).is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        self.tasks
            .lock()
            .expect("tasks mutex poisoned")
            .entry(stream)
            .or_default()
            .push(handle);
    }

    pub fn unsubscribe_stream(&self, stream: StreamKind) {
        if let Some(handles) = self
            .tasks
            .lock()
            .expect("tasks mutex poisoned")
            .remove(&stream)
        {
            for handle in handles {
                handle.abort();
            }
        }
    }

    pub fn complete(&self) {
        let mut tasks = self.tasks.lock().expect("tasks mutex poisoned");
        for handles in tasks.values_mut() {
            for handle in handles.drain(..) {
                handle.abort();
            }
        }
        tasks.clear();
        let _ = self.close();
    }

    pub fn api_version(&self) -> u32 {
        self.rpc_state
            .lock()
            .expect("rpc state mutex poisoned")
            .api_version
    }

    pub fn set_api_version(&self, api_version: u32) {
        self.rpc_state
            .lock()
            .expect("rpc state mutex poisoned")
            .api_version = api_version;
    }

    pub fn path_request_id(&self) -> Option<u64> {
        self.rpc_state
            .lock()
            .expect("rpc state mutex poisoned")
            .path_request_id
    }

    pub fn set_path_request_id(&self, path_request_id: Option<u64>) {
        self.rpc_state
            .lock()
            .expect("rpc state mutex poisoned")
            .path_request_id = path_request_id;
    }
}

impl rpc::PathFindSession for WSSession {
    fn session_id(&self) -> u64 {
        self.id()
    }

    fn api_version(&self) -> u32 {
        WSSession::api_version(self)
    }

    fn set_api_version(&self, api_version: u32) {
        WSSession::set_api_version(self, api_version);
    }

    fn current_path_request_id(&self) -> Option<u64> {
        WSSession::path_request_id(self)
    }

    fn set_current_path_request_id(&self, request_id: Option<u64>) {
        WSSession::set_path_request_id(self, request_id);
    }
}
