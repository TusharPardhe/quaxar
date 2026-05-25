//! Outbound RPC subscription/session helper aligned with `RPCSub`.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use app::paths::PathFindSession;
use protocol::JsonValue;

static NEXT_RPC_SUB_ID: AtomicU64 = AtomicU64::new(0);

fn next_id() -> u64 {
    NEXT_RPC_SUB_ID.fetch_add(1, Ordering::Relaxed) + 1
}

#[derive(Debug, Clone, PartialEq)]
pub struct RpcSubEvent {
    pub seq: u64,
    pub payload: JsonValue,
    pub broadcast: bool,
}

#[derive(Debug, Default)]
struct RpcSubState {
    api_version: u32,
    username: String,
    password: String,
    path_request_id: Option<u64>,
    next_seq: u64,
    queue: VecDeque<RpcSubEvent>,
}

#[derive(Debug)]
pub struct RpcSub {
    id: u64,
    url: String,
    state: Mutex<RpcSubState>,
}

impl RpcSub {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            id: next_id(),
            url: url.into(),
            state: Mutex::new(RpcSubState {
                next_seq: 1,
                ..RpcSubState::default()
            }),
        }
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn set_username(&self, username: impl Into<String>) {
        self.state.lock().expect("rpc sub mutex poisoned").username = username.into();
    }

    pub fn set_password(&self, password: impl Into<String>) {
        self.state.lock().expect("rpc sub mutex poisoned").password = password.into();
    }

    pub fn username(&self) -> String {
        self.state
            .lock()
            .expect("rpc sub mutex poisoned")
            .username
            .clone()
    }

    pub fn password(&self) -> String {
        self.state
            .lock()
            .expect("rpc sub mutex poisoned")
            .password
            .clone()
    }

    pub fn send(&self, payload: JsonValue, broadcast: bool) {
        let mut state = self.state.lock().expect("rpc sub mutex poisoned");
        let seq = state.next_seq;
        state.next_seq += 1;
        state.queue.push_back(RpcSubEvent {
            seq,
            payload,
            broadcast,
        });
    }

    pub fn drain(&self) -> Vec<RpcSubEvent> {
        self.state
            .lock()
            .expect("rpc sub mutex poisoned")
            .queue
            .drain(..)
            .collect()
    }
}

impl PathFindSession for RpcSub {
    fn session_id(&self) -> u64 {
        self.id
    }

    fn api_version(&self) -> u32 {
        self.state
            .lock()
            .expect("rpc sub mutex poisoned")
            .api_version
    }

    fn set_api_version(&self, api_version: u32) {
        self.state
            .lock()
            .expect("rpc sub mutex poisoned")
            .api_version = api_version;
    }

    fn current_path_request_id(&self) -> Option<u64> {
        self.state
            .lock()
            .expect("rpc sub mutex poisoned")
            .path_request_id
    }

    fn set_current_path_request_id(&self, request_id: Option<u64>) {
        self.state
            .lock()
            .expect("rpc sub mutex poisoned")
            .path_request_id = request_id;
    }
}
