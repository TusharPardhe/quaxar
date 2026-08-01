//! Public overlay surface mirroring the current `Overlay` contract.

use std::collections::HashSet;
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use basics::base_uint::Uint256;
use protocol::{JsonValue, PublicKey};
use rustls::{ClientConfig, ServerConfig};

use crate::connect_attempt::{ConnectAttemptError, ConnectAttemptResult};
use crate::message::ProtocolMessage;
use crate::peer::{Peer, PeerId};

/// rippled `PeerFinder::Tuning` peer-budget defaults.
pub const PEERFINDER_DEFAULT_MAX_PEERS: usize = 21;
pub const PEERFINDER_MIN_OUTBOUND_PEERS: usize = 10;
pub const PEERFINDER_OUTBOUND_PERCENT: usize = 15;

/// Directional active-peer budgets derived with rippled's `Config::makeConfig`
/// rules. Fixed, reserved, and cluster peers are intentionally excluded by the
/// admission counter, not by these maxima.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerLimits {
    pub max_peers: usize,
    pub inbound_max: usize,
    pub outbound_max: usize,
}

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
    /// OpenSSL-based server acceptor for inbound TLS connections.
    /// Required for TLS Finished message extraction (Session-Signature derivation).
    pub server_ssl_acceptor: Option<Arc<openssl::ssl::SslAcceptor>>,
    pub public_ip: Option<IpAddr>,
    pub fixed_peer_ips: HashSet<IpAddr>,
    pub ip_limit: usize,
    /// Legacy `peers_max` budget. A zero value uses rippled's default of 21.
    pub peer_limit: usize,
    /// Explicit `peers_in_max`; it must be supplied together with
    /// `peer_limit_out` by configuration parsing.
    pub peer_limit_in: Option<usize>,
    /// Explicit `peers_out_max`; it must be supplied together with
    /// `peer_limit_in` by configuration parsing.
    pub peer_limit_out: Option<usize>,
    /// Mirrors `PeerFinder::Config::wantIncoming`. Runtime setup derives this
    /// from an enabled peer listener and peer privacy.
    pub want_incoming: bool,
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
            server_ssl_acceptor: None,
            public_ip: None,
            fixed_peer_ips: HashSet::new(),
            ip_limit: 0,
            peer_limit: 0,
            peer_limit_in: None,
            peer_limit_out: None,
            want_incoming: true,
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

impl Setup {
    /// Match rippled `PeerFinder::Config::makeConfig` exactly for peer maxima:
    /// legacy `peers_max` is raised to the minimum outbound count, receives a
    /// rounded 15% outbound target, and gives the remainder to inbound peers.
    /// Paired explicit limits bypass the legacy calculation; inbound capacity
    /// is disabled when the node does not accept inbound peers.
    pub fn peer_limits(&self) -> PeerLimits {
        if let (Some(inbound_max), Some(outbound_max)) = (self.peer_limit_in, self.peer_limit_out) {
            let inbound_max = self.want_incoming.then_some(inbound_max).unwrap_or(0);
            return PeerLimits {
                // rippled Config.cpp:112: config.maxPeers = 0 when explicit limits set
                max_peers: 0,
                inbound_max,
                outbound_max,
            };
        }

        let max_peers = if self.peer_limit == 0 {
            PEERFINDER_DEFAULT_MAX_PEERS
        } else {
            self.peer_limit.max(PEERFINDER_MIN_OUTBOUND_PEERS)
        };
        let outbound_max = if self.want_incoming {
            (((max_peers * PEERFINDER_OUTBOUND_PERCENT) + 50) / 100)
                .max(PEERFINDER_MIN_OUTBOUND_PEERS)
        } else {
            max_peers
        };
        PeerLimits {
            max_peers,
            inbound_max: max_peers.saturating_sub(outbound_max),
            outbound_max,
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
    /// Admit a proposal source into relay history, including duplicate
    /// source-slot accounting. Returns false for an existing suppression key.
    fn admit_proposal_source(&self, uid: Uint256, validator: PublicKey, peer_id: PeerId) -> bool;
    /// Admit a validation source into relay history, including duplicate
    /// source-slot accounting. Returns false for an existing suppression key.
    fn admit_validation_source(&self, uid: Uint256, validator: PublicKey, peer_id: PeerId) -> bool;
    /// Whether the peer has diverged far enough to reject untrusted consensus
    /// traffic before scheduling expensive verification work.
    fn peer_is_diverged(&self, peer_id: PeerId) -> bool;
    /// Add a local validation suppression entry without recording an ingress
    /// source or a relay timestamp, matching HashRouter::addSuppression.
    fn suppress_validation(&self, uid: Uint256);
    fn sweep_relay_history(&self, max_entries: u64);
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
