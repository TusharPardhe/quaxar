//! Public overlay surface mirroring the current `Overlay` contract.

use std::collections::HashSet;
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use protocol::{JsonValue, PublicKey};
use rustls::{ClientConfig, ServerConfig};

use crate::connect_attempt::{ConnectAttemptError, ConnectAttemptResult};
use crate::message::ProtocolMessage;
use crate::peer::{Peer, PeerId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Promote {
    Automatic,
    Never,
    Always,
}

#[derive(Clone)]
pub struct Setup {
    pub client_config: Option<Arc<ClientConfig>>,
    pub server_config: Option<Arc<ServerConfig>>,
    pub public_ip: Option<IpAddr>,
    pub fixed_peer_ips: HashSet<IpAddr>,
    pub ip_limit: usize,
    pub peer_limit: usize,
    pub verify_endpoints: bool,
    pub crawl_options: u32,
    pub network_id: Option<u32>,
    pub vl_enabled: bool,
    pub tx_reduce_relay_enabled: bool,
    pub tx_reduce_relay_min_peers: usize,
    pub tx_relay_percentage: usize,
    pub vp_reduce_relay_base_squelch_enabled: bool,
    pub vp_reduce_relay_max_selected_peers: u16,
    pub reduce_relay_wait: Duration,
}

impl Default for Setup {
    fn default() -> Self {
        Self {
            client_config: None,
            server_config: None,
            public_ip: None,
            fixed_peer_ips: HashSet::new(),
            ip_limit: 0,
            peer_limit: 0,
            verify_endpoints: true,
            crawl_options: 0,
            network_id: None,
            vl_enabled: true,
            tx_reduce_relay_enabled: true,
            tx_reduce_relay_min_peers: 2,
            tx_relay_percentage: 25,
            vp_reduce_relay_base_squelch_enabled: true,
            vp_reduce_relay_max_selected_peers: crate::slot::MAX_SELECTED_PEERS,
            reduce_relay_wait: crate::slot::WAIT_ON_BOOTUP,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Handoff {
    Accepted,
    Rejected(String),
    Ignored,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OverlayStats {
    pub active_peers: usize,
    pub limit: usize,
    pub verify_endpoints: bool,
    pub jq_trans_overflow: u64,
    pub peer_disconnects: u64,
    pub peer_disconnect_charges: u64,
}

pub trait Overlay: Send + Sync {
    fn connect(
        &self,
        address: SocketAddr,
    ) -> Pin<Box<dyn Future<Output = Result<ConnectAttemptResult, ConnectAttemptError>> + Send>>;
    fn limit(&self) -> usize;
    fn size(&self) -> usize;
    fn json(&self) -> JsonValue;
    fn active_peers(&self) -> Vec<Arc<dyn Peer>>;
    fn peers_json(&self) -> Vec<JsonValue>;
    fn find_peer_by_short_id(&self, id: PeerId) -> Option<Arc<dyn Peer>>;
    fn find_peer_by_public_key(&self, public_key: PublicKey) -> Option<Arc<dyn Peer>>;
    fn check_tracking(&self, index: u32);
    fn broadcast(&self, message: &ProtocolMessage);
    fn relay(&self, message: &ProtocolMessage, to_skip: &BTreeSet<PeerId>) -> BTreeSet<PeerId>;
    fn inc_jq_trans_overflow(&self);
    fn jq_trans_overflow(&self) -> u64;
    fn inc_peer_disconnect(&self);
    fn peer_disconnect(&self) -> u64;
    fn inc_peer_disconnect_charges(&self);
    fn peer_disconnect_charges(&self) -> u64;
    fn network_id(&self) -> Option<u32>;
    fn verify_endpoints(&self) -> bool;
    fn tx_metrics(&self) -> JsonValue;
    fn stats(&self) -> OverlayStats {
        OverlayStats {
            active_peers: self.size(),
            limit: self.limit(),
            verify_endpoints: self.verify_endpoints(),
            jq_trans_overflow: self.jq_trans_overflow(),
            peer_disconnects: self.peer_disconnect(),
            peer_disconnect_charges: self.peer_disconnect_charges(),
        }
    }
}

pub fn peers_to_json(peers: &[Arc<dyn Peer>]) -> JsonValue {
    JsonValue::Array(peers.iter().map(|peer| peer.json()).collect())
}

pub fn stats_to_json(stats: OverlayStats) -> JsonValue {
    JsonValue::Object(BTreeMap::from([
        (
            "active".to_owned(),
            JsonValue::Unsigned(stats.active_peers as u64),
        ),
        ("limit".to_owned(), JsonValue::Unsigned(stats.limit as u64)),
        (
            "verify_endpoints".to_owned(),
            JsonValue::Bool(stats.verify_endpoints),
        ),
        (
            "jq_trans_overflow".to_owned(),
            JsonValue::Unsigned(stats.jq_trans_overflow),
        ),
        (
            "peer_disconnects".to_owned(),
            JsonValue::Unsigned(stats.peer_disconnects),
        ),
        (
            "peer_disconnect_charges".to_owned(),
            JsonValue::Unsigned(stats.peer_disconnect_charges),
        ),
    ]))
}
