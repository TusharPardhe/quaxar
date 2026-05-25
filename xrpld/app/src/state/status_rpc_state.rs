//! Narrow app-owned state for later status-style RPC surfaces.
//!
//! This keeps a small set of explicit runtime status values together under one
//! owner:
//! - the current ledger index that some status RPCs report even without TxQ
//!   detail,
//! - and an optional TxQ RPC report snapshot when a caller has produced one.
//! - optional peer-count and network-id values when the shell has them,
//! - and an optional last-close snapshot using typed data instead of raw JSON.
//! - and optional config/runtime/status fields that higher shells can set
//!   explicitly without pretending the broader live owner graph is already
//!   ported.
//!
//! The boundary stays explicit on purpose. This owner does not derive one value
//! from the other and does not try to mirror broader ledger-master state.

use std::sync::Mutex;
use std::time::Duration;
use tx::QueueTxQRpcReport;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusRpcLastClose {
    pub proposers: u32,
    pub converge_time: Duration,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StatusRpcGitInfo {
    pub hash: Option<String>,
    pub branch: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StatusRpcSnapshot {
    pub current_ledger_index: Option<u32>,
    pub queue_report: Option<QueueTxQRpcReport>,
    pub peer_count: Option<u32>,
    pub network_id: Option<u32>,
    pub last_close: Option<StatusRpcLastClose>,
    pub hostid: Option<String>,
    pub server_domain: Option<String>,
    pub node_size: Option<String>,
    pub io_latency_ms: Option<u64>,
    pub complete_ledgers: Option<String>,
    pub fetch_pack: Option<u32>,
    pub git_info: Option<StatusRpcGitInfo>,
}

#[derive(Default)]
pub struct StatusRpcState {
    state: Mutex<StatusRpcSnapshot>,
}

impl std::fmt::Debug for StatusRpcState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let snapshot = self.snapshot();
        f.debug_struct("StatusRpcState")
            .field("current_ledger_index", &snapshot.current_ledger_index)
            .field("has_queue_report", &snapshot.queue_report.is_some())
            .field("peer_count", &snapshot.peer_count)
            .field("network_id", &snapshot.network_id)
            .field("has_last_close", &snapshot.last_close.is_some())
            .field("hostid", &snapshot.hostid)
            .field("server_domain", &snapshot.server_domain)
            .field("node_size", &snapshot.node_size)
            .field("io_latency_ms", &snapshot.io_latency_ms)
            .field("complete_ledgers", &snapshot.complete_ledgers)
            .field("fetch_pack", &snapshot.fetch_pack)
            .field("has_git_info", &snapshot.git_info.is_some())
            .finish()
    }
}

impl StatusRpcState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn snapshot(&self) -> StatusRpcSnapshot {
        self.state
            .lock()
            .expect("status rpc state mutex must not be poisoned")
            .clone()
    }

    pub fn current_ledger_index(&self) -> Option<u32> {
        self.state
            .lock()
            .expect("status rpc state mutex must not be poisoned")
            .current_ledger_index
    }

    pub fn set_current_ledger_index(&self, current_ledger_index: Option<u32>) -> Option<u32> {
        let mut state = self
            .state
            .lock()
            .expect("status rpc state mutex must not be poisoned");
        let previous = state.current_ledger_index;
        state.current_ledger_index = current_ledger_index;
        previous
    }

    pub fn queue_report(&self) -> Option<QueueTxQRpcReport> {
        self.state
            .lock()
            .expect("status rpc state mutex must not be poisoned")
            .queue_report
            .clone()
    }

    pub fn set_queue_report(
        &self,
        queue_report: Option<QueueTxQRpcReport>,
    ) -> Option<QueueTxQRpcReport> {
        let mut state = self
            .state
            .lock()
            .expect("status rpc state mutex must not be poisoned");
        let previous = state.queue_report.clone();
        state.queue_report = queue_report;
        previous
    }

    pub fn peer_count(&self) -> Option<u32> {
        self.state
            .lock()
            .expect("status rpc state mutex must not be poisoned")
            .peer_count
    }

    pub fn set_peer_count(&self, peer_count: Option<u32>) -> Option<u32> {
        let mut state = self
            .state
            .lock()
            .expect("status rpc state mutex must not be poisoned");
        let previous = state.peer_count;
        state.peer_count = peer_count;
        previous
    }

    pub fn network_id(&self) -> Option<u32> {
        self.state
            .lock()
            .expect("status rpc state mutex must not be poisoned")
            .network_id
    }

    pub fn set_network_id(&self, network_id: Option<u32>) -> Option<u32> {
        let mut state = self
            .state
            .lock()
            .expect("status rpc state mutex must not be poisoned");
        let previous = state.network_id;
        state.network_id = network_id;
        previous
    }

    pub fn last_close(&self) -> Option<StatusRpcLastClose> {
        self.state
            .lock()
            .expect("status rpc state mutex must not be poisoned")
            .last_close
            .clone()
    }

    pub fn set_last_close(
        &self,
        last_close: Option<StatusRpcLastClose>,
    ) -> Option<StatusRpcLastClose> {
        let mut state = self
            .state
            .lock()
            .expect("status rpc state mutex must not be poisoned");
        let previous = state.last_close.clone();
        state.last_close = last_close;
        previous
    }

    pub fn hostid(&self) -> Option<String> {
        self.state
            .lock()
            .expect("status rpc state mutex must not be poisoned")
            .hostid
            .clone()
    }

    pub fn set_hostid(&self, hostid: Option<String>) -> Option<String> {
        let mut state = self
            .state
            .lock()
            .expect("status rpc state mutex must not be poisoned");
        let previous = state.hostid.clone();
        state.hostid = hostid;
        previous
    }

    pub fn server_domain(&self) -> Option<String> {
        self.state
            .lock()
            .expect("status rpc state mutex must not be poisoned")
            .server_domain
            .clone()
    }

    pub fn set_server_domain(&self, server_domain: Option<String>) -> Option<String> {
        let mut state = self
            .state
            .lock()
            .expect("status rpc state mutex must not be poisoned");
        let previous = state.server_domain.clone();
        state.server_domain = server_domain;
        previous
    }

    pub fn node_size(&self) -> Option<String> {
        self.state
            .lock()
            .expect("status rpc state mutex must not be poisoned")
            .node_size
            .clone()
    }

    pub fn set_node_size(&self, node_size: Option<String>) -> Option<String> {
        let mut state = self
            .state
            .lock()
            .expect("status rpc state mutex must not be poisoned");
        let previous = state.node_size.clone();
        state.node_size = node_size;
        previous
    }

    pub fn io_latency_ms(&self) -> Option<u64> {
        self.state
            .lock()
            .expect("status rpc state mutex must not be poisoned")
            .io_latency_ms
    }

    pub fn set_io_latency_ms(&self, io_latency_ms: Option<u64>) -> Option<u64> {
        let mut state = self
            .state
            .lock()
            .expect("status rpc state mutex must not be poisoned");
        let previous = state.io_latency_ms;
        state.io_latency_ms = io_latency_ms;
        previous
    }

    pub fn complete_ledgers(&self) -> Option<String> {
        self.state
            .lock()
            .expect("status rpc state mutex must not be poisoned")
            .complete_ledgers
            .clone()
    }

    pub fn set_complete_ledgers(&self, complete_ledgers: Option<String>) -> Option<String> {
        let mut state = self
            .state
            .lock()
            .expect("status rpc state mutex must not be poisoned");
        let previous = state.complete_ledgers.clone();
        state.complete_ledgers = complete_ledgers;
        previous
    }

    pub fn fetch_pack(&self) -> Option<u32> {
        self.state
            .lock()
            .expect("status rpc state mutex must not be poisoned")
            .fetch_pack
    }

    pub fn set_fetch_pack(&self, fetch_pack: Option<u32>) -> Option<u32> {
        let mut state = self
            .state
            .lock()
            .expect("status rpc state mutex must not be poisoned");
        let previous = state.fetch_pack;
        state.fetch_pack = fetch_pack;
        previous
    }

    pub fn git_info(&self) -> Option<StatusRpcGitInfo> {
        self.state
            .lock()
            .expect("status rpc state mutex must not be poisoned")
            .git_info
            .clone()
    }

    pub fn set_git_info(&self, git_info: Option<StatusRpcGitInfo>) -> Option<StatusRpcGitInfo> {
        let mut state = self
            .state
            .lock()
            .expect("status rpc state mutex must not be poisoned");
        let previous = state.git_info.clone();
        state.git_info = git_info;
        previous
    }
}
