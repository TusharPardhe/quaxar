use std::any::Any;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::ws::{Message, Utf8Bytes};
use http::{HeaderMap, Method, Request, Uri, Version};
use protocol::{JsonValue, MPTID};
use rpc::RpcRole;
use tokio::sync::mpsc::error::{SendError, TrySendError};
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

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
        Self::from_parts(
            remote_addr,
            request.headers().clone(),
            request.method().clone(),
            request.uri().clone(),
            request.version(),
        )
    }

    /// Metadata for a request whose body has already been consumed by the
    /// router. Equivalent to `new` over a default (`GET / HTTP/1.1`) request
    /// carrying `headers`, without building and cloning a throwaway request.
    pub fn from_headers(remote_addr: SocketAddr, headers: &HeaderMap) -> Self {
        Self::from_parts(
            remote_addr,
            headers.clone(),
            Method::GET,
            Uri::from_static("/"),
            Version::HTTP_11,
        )
    }

    fn from_parts(
        remote_addr: SocketAddr,
        headers: HeaderMap,
        method: Method,
        uri: Uri,
        version: Version,
    ) -> Self {
        let keep_alive = headers
            .get(http::header::CONNECTION)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.eq_ignore_ascii_case("keep-alive"))
            || version >= Version::HTTP_11;

        Self {
            remote_addr,
            local_addr: None,
            headers,
            method,
            uri,
            version,
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
    sender: mpsc::Sender<Message>,
    /// Cancelled when the bounded egress queue overflows or a subscription
    /// falls behind; the connection is then closed (rippled closes slow
    /// clients rather than silently dropping messages).
    too_slow: CancellationToken,
    subscriptions: Arc<SubscriptionManager>,
    tasks: Mutex<HashMap<StreamKind, Vec<JoinHandle<()>>>>,
    /// rippled `InfoSub::mptSubscriptions_`, shared with the delivery task.
    mpt_subscriptions: Arc<Mutex<HashSet<MPTID>>>,
    mpt_task: Mutex<Option<JoinHandle<()>>>,
    app_defined: Mutex<Option<Arc<dyn Any + Send + Sync>>>,
    rpc_state: Mutex<WsRpcState>,
}

/// Enqueue without waiting; a full queue marks the client as too slow.
fn enqueue(
    sender: &mpsc::Sender<Message>,
    too_slow: &CancellationToken,
    message: Message,
) -> Result<(), SendError<Message>> {
    match sender.try_send(message) {
        Ok(()) => Ok(()),
        Err(TrySendError::Full(message)) => {
            too_slow.cancel();
            Err(SendError(message))
        }
        Err(TrySendError::Closed(message)) => Err(SendError(message)),
    }
}

impl WSSession {
    pub fn new(
        id: u64,
        metadata: RequestMetadata,
        sender: mpsc::Sender<Message>,
        subscriptions: Arc<SubscriptionManager>,
    ) -> Self {
        Self {
            id,
            metadata,
            sender,
            too_slow: CancellationToken::new(),
            subscriptions,
            tasks: Mutex::new(HashMap::new()),
            mpt_subscriptions: Arc::new(Mutex::new(HashSet::new())),
            mpt_task: Mutex::new(None),
            app_defined: Mutex::new(None),
            rpc_state: Mutex::new(WsRpcState::default()),
        }
    }

    /// Token cancelled when this client can no longer keep up.
    pub fn too_slow_token(&self) -> CancellationToken {
        self.too_slow.clone()
    }

    pub fn is_too_slow(&self) -> bool {
        self.too_slow.is_cancelled()
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

    pub fn send_text(&self, text: impl Into<String>) -> Result<(), SendError<Message>> {
        enqueue(
            &self.sender,
            &self.too_slow,
            Message::Text(text.into().into()),
        )
    }

    /// Enqueue an RPC reply, waiting for egress capacity (backpressure on
    /// this connection's request loop) instead of disconnecting when the
    /// queue is momentarily full of subscription events.
    pub async fn send_reply(&self, text: String) -> Result<(), SendError<Message>> {
        let message = Message::Text(text.into());
        tokio::select! {
            biased;
            _ = self.too_slow.cancelled() => Err(SendError(Message::Close(None))),
            sent = self.sender.send(message) => sent,
        }
    }

    pub fn send_json(&self, value: &JsonValue) -> Result<(), SendError<Message>> {
        // Serialize the protocol tree directly (sorted keys, same bytes as
        // converting to serde_json::Value first).
        let text = sonic_rs::to_string(value).unwrap_or_default();
        enqueue(&self.sender, &self.too_slow, Message::Text(text.into()))
    }

    pub fn close(&self) -> Result<(), SendError<Message>> {
        enqueue(&self.sender, &self.too_slow, Message::Close(None))
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
        let too_slow = self.too_slow.clone();
        let handle = tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(SubscriptionEvent { payload, .. }) => {
                        let Ok(text) = Utf8Bytes::try_from(payload) else {
                            continue;
                        };
                        // Wait for egress capacity: a ledger close publishes
                        // every transaction back-to-back, far faster than a
                        // healthy client drains, so the shared broadcast ring
                        // absorbs the burst. A client that falls a whole ring
                        // behind is disconnected below (Lagged).
                        tokio::select! {
                            biased;
                            _ = too_slow.cancelled() => break,
                            sent = sender.send(Message::Text(text)) => {
                                if sent.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                    // Falling behind the publisher would leave a silent gap in
                    // the stream; treat it like an egress overflow instead.
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        too_slow.cancel();
                        break;
                    }
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

    /// rippled `InfoSub::totalSubscriptionCount`. Quaxar tracks no account,
    /// real-time account or account-history sets yet, so only MPT issuances
    /// contribute.
    pub fn total_subscription_count(&self) -> usize {
        self.mpt_subscriptions
            .lock()
            .expect("mpt subscriptions mutex poisoned")
            .len()
    }

    /// rippled `InfoSub::tryReserveMPTSubscriptions` followed by
    /// `NetworkOPs::subMPT`: under one lock, charge only net-new issuances
    /// against `cap` and insert all of them, or record nothing.
    pub fn try_subscribe_mpts(&self, mpt_ids: &HashSet<MPTID>, cap: usize) -> bool {
        {
            let mut held = self
                .mpt_subscriptions
                .lock()
                .expect("mpt subscriptions mutex poisoned");
            let fresh = mpt_ids.iter().filter(|id| !held.contains(*id)).count();
            if crate::subscriptions::exceeds_subscription_cap(held.len(), fresh, cap) {
                return false;
            }
            held.extend(mpt_ids.iter().copied());
        }
        self.ensure_mpt_delivery_task();
        true
    }

    /// rippled `NetworkOPs::unsubMPT`.
    pub fn unsubscribe_mpts(&self, mpt_ids: &HashSet<MPTID>) {
        let now_empty = {
            let mut held = self
                .mpt_subscriptions
                .lock()
                .expect("mpt subscriptions mutex poisoned");
            for id in mpt_ids {
                held.remove(id);
            }
            held.is_empty()
        };
        if now_empty
            && let Some(handle) = self
                .mpt_task
                .lock()
                .expect("mpt task mutex poisoned")
                .take()
        {
            handle.abort();
        }
    }

    fn ensure_mpt_delivery_task(&self) {
        let mut task = self.mpt_task.lock().expect("mpt task mutex poisoned");
        if task.as_ref().is_some_and(|handle| !handle.is_finished()) {
            return;
        }
        let mut rx = self.subscriptions.subscribe_mpt_transactions();
        let sender = self.sender.clone();
        let too_slow = self.too_slow.clone();
        let held = Arc::clone(&self.mpt_subscriptions);
        *task = Some(tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        // One delivery per connection even when several of its
                        // issuances are affected (rippled collects listeners
                        // into a HashSet before sending).
                        let wanted = {
                            let held = held.lock().expect("mpt subscriptions mutex poisoned");
                            event.mpt_ids.iter().any(|id| held.contains(id))
                        };
                        if !wanted {
                            continue;
                        }
                        let Ok(text) = Utf8Bytes::try_from(event.payload) else {
                            continue;
                        };
                        // Same egress policy as subscribe_stream.
                        tokio::select! {
                            biased;
                            _ = too_slow.cancelled() => break,
                            sent = sender.send(Message::Text(text)) => {
                                if sent.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        too_slow.cancel();
                        break;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }));
    }

    pub fn complete(&self) {
        if let Some(handle) = self
            .mpt_task
            .lock()
            .expect("mpt task mutex poisoned")
            .take()
        {
            handle.abort();
        }
        self.mpt_subscriptions
            .lock()
            .expect("mpt subscriptions mutex poisoned")
            .clear();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn session(
        capacity: usize,
        subscriptions: Arc<SubscriptionManager>,
    ) -> (WSSession, mpsc::Receiver<Message>) {
        let (sender, receiver) = mpsc::channel(capacity);
        let metadata =
            RequestMetadata::from_headers("127.0.0.1:1".parse().unwrap(), &HeaderMap::new());
        (WSSession::new(1, metadata, sender, subscriptions), receiver)
    }

    #[test]
    fn full_egress_queue_marks_client_too_slow_instead_of_growing() {
        let (session, _receiver) = session(2, Arc::new(SubscriptionManager::default()));
        assert!(session.send_text("a").is_ok());
        assert!(session.send_text("b").is_ok());
        assert!(!session.is_too_slow());
        assert!(session.send_text("c").is_err());
        assert!(session.is_too_slow());
    }

    #[test]
    fn from_headers_matches_default_request_metadata() {
        let mut headers = HeaderMap::new();
        headers.insert("x-user", "alice".parse().unwrap());
        let mut request = Request::new(Body::empty());
        *request.headers_mut() = headers.clone();
        let addr: SocketAddr = "10.0.0.1:5".parse().unwrap();
        let a = RequestMetadata::new(addr, &request);
        let b = RequestMetadata::from_headers(addr, &headers);
        assert_eq!(a.headers, b.headers);
        assert_eq!(a.method, b.method);
        assert_eq!(a.uri, b.uri);
        assert_eq!(a.version, b.version);
        assert_eq!(a.keep_alive, b.keep_alive);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn publish_burst_larger_than_egress_queue_is_delivered_in_order() {
        let subscriptions = Arc::new(SubscriptionManager::default());
        let (session, mut receiver) = session(8, subscriptions.clone());
        session.subscribe_stream(StreamKind::Transactions);
        tokio::task::yield_now().await;
        // A ledger-close-sized burst, far beyond the 8-frame egress queue.
        for i in 0..500_u64 {
            subscriptions.publish_json(StreamKind::Transactions, JsonValue::Unsigned(i));
        }
        for expected in 0..500_u64 {
            let message = tokio::time::timeout(std::time::Duration::from_secs(5), receiver.recv())
                .await
                .expect("burst must drain")
                .expect("channel open");
            let Message::Text(text) = message else {
                panic!("text frame expected");
            };
            assert_eq!(text.as_str(), expected.to_string());
        }
        assert!(!session.is_too_slow());
    }

    #[tokio::test]
    async fn lagging_subscriber_is_disconnected_not_silently_skipped() {
        let subscriptions = Arc::new(SubscriptionManager::new(4));
        // Egress queue large enough that only the broadcast ring can lag.
        let (session, mut receiver) = session(1024, subscriptions.clone());
        session.subscribe_stream(StreamKind::Ledger);
        tokio::task::yield_now().await;
        // Publish faster than the forwarding task is scheduled.
        for i in 0..64 {
            subscriptions.publish_json(StreamKind::Ledger, JsonValue::Unsigned(i));
        }
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            session.too_slow_token().cancelled(),
        )
        .await
        .expect("lagging subscriber must be flagged too slow");
        // Whatever was delivered is a gap-free prefix.
        let mut expected = 0;
        while let Ok(Message::Text(text)) = receiver.try_recv() {
            assert_eq!(text.as_str(), expected.to_string());
            expected += 1;
        }
    }
}
