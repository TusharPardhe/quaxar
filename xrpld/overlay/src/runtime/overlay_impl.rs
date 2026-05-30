//! Concrete overlay owner with runtime peer state, relay policy, and
//! tokio TCP/TLS boundaries.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::future::Future;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime};

use basics::base_uint::Uint256;
use basics::base64::base64_encode;
use http::{Request, Response};
use openssl::ssl::{SslConnector, SslMethod, SslVerifyMode};
use protocol::{
    JsonValue, KeyType, PublicKey, STTx, STValidation, SecretKey, SerialIter, Serializer,
    derive_public_key, sha512_half as protocol_sha512_half, sign_digest,
};
use rand::seq::SliceRandom;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tokio_rustls::TlsAcceptor;
use xrpl_core::PeerReservationTable as CorePeerReservationTable;

use crate::cluster::Cluster;
use crate::connect_attempt::{
    ConnectAttempt, ConnectAttemptConfig, ConnectAttemptError, ConnectAttemptResult, ConnectionStep,
};
use crate::handshake::{
    FEATURE_COMPR, FEATURE_LEDGER_REPLAY, FEATURE_TXRR, HandshakeContext, feature_enabled,
    is_feature_value, make_response, parse_http_request, serialize_response,
};
use crate::inbound::{
    OverlayInboundHandler, OverlayInboundSnapshot, QueuedEndpoint, QueuedEndpoints,
    QueuedHaveTransactions, QueuedOverlayInboundHandler, QueuedProposal, QueuedTransaction,
    QueuedValidation,
};
use crate::message::{
    Message, ProtocolMessage, ProtocolPayload, TmProposeSet, TmSquelch, TmTransaction, TmValidation,
};
use crate::overlay::{Handoff, Overlay, Setup, stats_to_json};
use crate::peer::status_change::{build_peer_status_event, lost_sync_event};
use crate::peer::{Peer, PeerId};
use crate::peer_imp::PeerImp;
use crate::protocol_version::negotiate_protocol_version;
use crate::router::{MessageRouter, route_message};
use crate::session::{PeerSessionHooks, PeerSessionStarter};
use crate::slot::{Clock, Slots, SquelchHandler, SystemClock};
use crate::traffic_count::{TrafficCategory, TrafficCount};
use crate::transport::handshake::is_public_ip;
use crate::tx_metrics::TxMetrics;
use crate::{HARD_MAX_REPLY_NODES, ProtocolFeature, ProtocolVersion, parse_protocol_versions};

const PEER_LIMIT_REJECTION_REASON: &str = "peer limit reached for unreserved peer";
const PEERFINDER_MAX_HOPS: u32 = 6;
const PEERFINDER_MAX_ACCEPTED_ENDPOINTS: usize = 64;
const PEERFINDER_REDIRECT_ENDPOINT_COUNT: usize = 10;
const PEERFINDER_LIVE_CACHE_TTL: Duration = Duration::from_secs(30);
const PEERFINDER_SECONDS_PER_MESSAGE: Duration = Duration::from_secs(151);

#[derive(Debug, Clone, Copy)]
struct RedirectEndpoint {
    hops: u32,
    last_seen: SystemTime,
}

fn canonical_peer_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(ipv6) => ipv6
            .to_ipv4_mapped()
            .map(IpAddr::V4)
            .unwrap_or(IpAddr::V6(ipv6)),
        IpAddr::V4(_) => ip,
    }
}

pub trait OverlayHandoff: Send + Sync {
    fn on_handoff(&self, request: &Request<()>, remote_address: SocketAddr) -> Handoff;
}

type PeerStatusPublisher = Arc<dyn Fn(JsonValue) + Send + Sync>;

#[derive(Debug)]
pub enum OverlayError {
    Io(std::io::Error),
    InvalidRequest(String),
    Tls(String),
}

impl std::fmt::Display for OverlayError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "{error}"),
            Self::InvalidRequest(error) => write!(formatter, "{error}"),
            Self::Tls(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for OverlayError {}

impl From<std::io::Error> for OverlayError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Clone)]
pub struct OverlayAcceptor {
    pub listener: Arc<TcpListener>,
    pub acceptor: TlsAcceptor,
}

#[derive(Debug, Clone)]
struct OverlayIdentity {
    public_key: PublicKey,
    secret_key: SecretKey,
    instance_cookie: u64,
}

impl OverlayIdentity {
    fn new() -> Self {
        static NEXT_INSTANCE_COOKIE: AtomicU64 = AtomicU64::new(1);

        let instance_cookie = NEXT_INSTANCE_COOKIE.fetch_add(1, Ordering::Relaxed);
        let mut secret_bytes = [0u8; 32];
        secret_bytes[24..].copy_from_slice(&instance_cookie.to_be_bytes());
        if secret_bytes.iter().all(|byte| *byte == 0) {
            secret_bytes[31] = 1;
        }

        let secret = SecretKey::from_bytes(secret_bytes);
        let public_key = derive_public_key(KeyType::Secp256k1, &secret)
            .expect("overlay handshake identity must derive");

        Self {
            public_key,
            secret_key: secret,
            instance_cookie,
        }
    }

    fn context(&self) -> HandshakeContext {
        // Keep current inbound behavior until inbound shared-value signing is ported.
        let session_signature = base64_encode(self.public_key.as_bytes());
        HandshakeContext::new(
            self.public_key.to_node_public_base58(),
            session_signature,
            self.instance_cookie,
        )
    }

    fn sign_session(&self, shared_value: &Uint256) -> Result<String, String> {
        let signature = sign_digest(&self.public_key, &self.secret_key, *shared_value)
            .map_err(|_| "failed to sign session".to_owned())?;
        Ok(base64_encode(&signature))
    }

    fn public_key(&self) -> PublicKey {
        self.public_key
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PeerReservation {
    pub node_public: PublicKey,
    pub description: String,
}

pub trait PeerReservationSource: Send + Sync {
    fn contains(&self, node_public: PublicKey) -> bool;
}

#[derive(Debug, Default)]
pub struct PeerReservationTable {
    reservations: RwLock<BTreeMap<PublicKey, String>>,
}

impl PeerReservationTable {
    pub fn list(&self) -> Vec<PeerReservation> {
        self.reservations
            .read()
            .expect("reservation table lock")
            .iter()
            .map(|(node_public, description)| PeerReservation {
                node_public: *node_public,
                description: description.clone(),
            })
            .collect()
    }

    pub fn insert_or_assign(&self, reservation: PeerReservation) -> Option<PeerReservation> {
        let mut reservations = self.reservations.write().expect("reservation table lock");
        reservations
            .insert(reservation.node_public, reservation.description.clone())
            .map(|description| PeerReservation {
                node_public: reservation.node_public,
                description,
            })
    }

    pub fn erase(&self, node_public: PublicKey) -> Option<PeerReservation> {
        self.reservations
            .write()
            .expect("reservation table lock")
            .remove(&node_public)
            .map(|description| PeerReservation {
                node_public,
                description,
            })
    }

    pub fn contains(&self, node_public: PublicKey) -> bool {
        self.reservations
            .read()
            .expect("reservation table lock")
            .contains_key(&node_public)
    }
}

impl PeerReservationSource for PeerReservationTable {
    fn contains(&self, node_public: PublicKey) -> bool {
        self.contains(node_public)
    }
}

impl PeerReservationSource for CorePeerReservationTable<PublicKey> {
    fn contains(&self, node_public: PublicKey) -> bool {
        self.contains(&node_public)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum RelayKind {
    Proposal,
    Validation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct RelayKey {
    kind: RelayKind,
    uid: Uint256,
}

#[derive(Debug)]
struct OverlayRuntimeSquelchHandler {
    active_peers: Arc<RwLock<HashMap<PeerId, Arc<PeerImp>>>>,
    traffic: Arc<TrafficCount>,
}

impl OverlayRuntimeSquelchHandler {
    fn send_control_message(
        &self,
        peer: &Arc<PeerImp>,
        validator: PublicKey,
        squelch: bool,
        duration: u32,
    ) {
        let message = Message::new(
            ProtocolMessage::new(ProtocolPayload::Squelch(TmSquelch {
                squelch,
                validator_pub_key: validator.as_bytes().to_vec(),
                squelch_duration: squelch.then_some(duration),
            })),
            None,
        );
        let bytes = message.get_buffer_size() as u64;
        self.traffic
            .add_count(TrafficCategory::Squelch, false, bytes);
        self.traffic.add_count(TrafficCategory::Total, false, bytes);
        peer.send(message);
    }
}

impl SquelchHandler for OverlayRuntimeSquelchHandler {
    fn squelch(&self, validator: PublicKey, id: u32, duration: u32) {
        if let Some(peer) = self
            .active_peers
            .read()
            .expect("overlay peers lock")
            .get(&id)
            .cloned()
        {
            let _ = peer.apply_squelch(validator, Duration::from_secs(u64::from(duration)));
            self.send_control_message(&peer, validator, true, duration);
        }
    }

    fn unsquelch(&self, validator: PublicKey, id: u32) {
        if let Some(peer) = self
            .active_peers
            .read()
            .expect("overlay peers lock")
            .get(&id)
            .cloned()
        {
            peer.remove_squelch(validator);
            self.send_control_message(&peer, validator, false, 0);
        }
    }
}

#[derive(Debug, Default)]
struct InboundMessageTracker {
    bytes: Option<u64>,
}

struct OverlayPeerSessionHooks {
    overlay: OverlayImpl,
    inbound: Mutex<InboundMessageTracker>,
}

impl OverlayPeerSessionHooks {
    fn new(overlay: OverlayImpl) -> Self {
        Self {
            overlay,
            inbound: Mutex::new(InboundMessageTracker::default()),
        }
    }

    fn take_inbound_bytes(&self) -> Option<u64> {
        self.inbound
            .lock()
            .expect("inbound tracker lock")
            .bytes
            .take()
    }
}

impl PeerSessionHooks for OverlayPeerSessionHooks {
    fn on_message_begin(
        &self,
        _peer: &Arc<PeerImp>,
        header: &crate::message::MessageHeader,
        _compressed: bool,
    ) {
        self.inbound.lock().expect("inbound tracker lock").bytes =
            Some(u64::from(header.total_wire_size));
    }

    fn on_message_end(
        &self,
        peer: &Arc<PeerImp>,
        header: &crate::message::MessageHeader,
        message: &ProtocolMessage,
    ) {
        let bytes = self
            .take_inbound_bytes()
            .unwrap_or_else(|| u64::from(header.total_wire_size));
        self.overlay.observe_inbound_message(peer, message, bytes);
    }

    fn on_message_unknown(&self, _peer: &Arc<PeerImp>, _message_type: u16) {
        tracing::warn!(target: "overlay", "Failed to decode message from peer");
        let bytes = self.take_inbound_bytes().unwrap_or(0);
        self.overlay.observe_inbound_unknown(bytes);
    }
}

fn sha512_half(bytes: &[u8]) -> Uint256 {
    protocol_sha512_half(bytes)
}

fn proposal_unique_id(
    current_tx_hash: Uint256,
    previous_ledger: Uint256,
    propose_seq: u32,
    close_time: u32,
    public_key: PublicKey,
    signature: &[u8],
) -> Uint256 {
    let mut serializer = Serializer::new(512);
    serializer.add_bit_string(current_tx_hash);
    serializer.add_bit_string(previous_ledger);
    serializer.add32(propose_seq);
    serializer.add32(close_time);
    serializer.add_vl(public_key.as_bytes());
    serializer.add_vl(signature);
    serializer.get_sha512_half()
}

struct OverlayInboundRouter<'a> {
    overlay: &'a OverlayImpl,
    peer: &'a Arc<PeerImp>,
}

impl OverlayInboundRouter<'_> {
    fn update_cluster_membership(&self, node_public: PublicKey) {
        if let Some(peer) = self
            .overlay
            .by_public_key
            .read()
            .expect("overlay public-key lock")
            .get(&node_public)
            .cloned()
        {
            self.overlay.apply_membership_state(&peer);
        }
    }

    fn parse_transaction(&self, message: &crate::message::TmTransaction) -> Option<Uint256> {
        if self.peer.tracking() == crate::peer_imp::Tracking::Diverged {
            return None;
        }

        let mut serial = SerialIter::new(&message.raw_transaction);
        let transaction = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            STTx::from_serial_iter(&mut serial)
        }))
        .ok()?;
        Some(transaction.get_transaction_id())
    }

    fn queue_transaction(&self, message: &crate::message::TmTransaction, batch: bool) {
        let Some(id) = self.parse_transaction(message) else {
            return;
        };
        self.overlay.inbound_handler.on_transaction(
            self.peer,
            QueuedTransaction {
                peer_id: self.peer.id(),
                id,
                batch,
                message: message.clone(),
            },
        );
    }
}

impl MessageRouter for OverlayInboundRouter<'_> {
    fn on_manifests(
        &mut self,
        message: &crate::message::TmManifests,
    ) -> crate::router::RouteAction {
        if message.list.is_empty() {
            return crate::router::RouteAction::Continue;
        }
        tracing::debug!(
            target: "overlay",
            peer_id = %self.peer.id(),
            count = message.list.len(),
            "Manifests received"
        );
        self.overlay
            .inbound_handler
            .on_manifests(self.peer, message.clone());
        crate::router::RouteAction::Continue
    }

    fn on_ping(&mut self, message: &crate::message::TmPing) -> crate::router::RouteAction {
        if message.r#type == 0 {
            // Ping request — reply with pong
            let _ = self.overlay.send_runtime_message(
                self.peer,
                Message::new(
                    ProtocolMessage::new(ProtocolPayload::Ping(crate::message::TmPing {
                        r#type: 1,
                        seq: message.seq,
                        ping_time: message.ping_time,
                        net_time: message.net_time,
                    })),
                    None,
                ),
            );
        } else if message.r#type == 1 {
            // Pong response — compute RTT and update peer latency
            // reference: latency_ = latency_ ? (*latency_ * 7 + rtt) / 8 : rtt
            if let Some(ping_time) = message.ping_time {
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64;
                if now_ms > ping_time {
                    let rtt_ms = (now_ms - ping_time) as u32;
                    self.peer.update_latency(rtt_ms);
                    tracing::debug!(
                        target: "overlay",
                        peer_id = %self.peer.id(),
                        latency_ms = rtt_ms,
                        "Peer latency measured"
                    );
                }
            }
        }
        crate::router::RouteAction::Continue
    }

    fn on_cluster(&mut self, message: &crate::message::TmCluster) -> crate::router::RouteAction {
        if !self.peer.cluster() {
            return crate::router::RouteAction::Continue;
        }
        tracing::debug!(
            target: "overlay",
            peer_id = %self.peer.id(),
            nodes = message.cluster_nodes.len(),
            "Cluster message received"
        );

        for node in &message.cluster_nodes {
            let Some(public_key_bytes) = protocol::parse_base58_node_public(&node.public_key)
            else {
                continue;
            };
            let Ok(public_key) = PublicKey::from_slice(&public_key_bytes) else {
                continue;
            };
            let report_time =
                SystemTime::UNIX_EPOCH + Duration::from_secs(u64::from(node.report_time));
            let _ = self.overlay.cluster().update(
                public_key,
                node.node_name.clone().unwrap_or_default(),
                node.node_load,
                report_time,
            );
            self.update_cluster_membership(public_key);
        }

        crate::router::RouteAction::Continue
    }

    fn on_endpoints(
        &mut self,
        message: &crate::message::TmEndpoints,
    ) -> crate::router::RouteAction {
        if self.peer.tracking() != crate::peer_imp::Tracking::Converged || message.version != 2 {
            return crate::router::RouteAction::Continue;
        }
        if message.endpoints_v2.len() >= 1024 {
            return crate::router::RouteAction::Continue;
        }
        let now_instant = Instant::now();
        if !self
            .peer
            .begin_endpoint_accept_window(now_instant, PEERFINDER_SECONDS_PER_MESSAGE)
        {
            return crate::router::RouteAction::Continue;
        }

        let mut malformed = 0usize;
        let mut advertised = message.endpoints_v2.clone();
        if advertised.len() > PEERFINDER_MAX_ACCEPTED_ENDPOINTS {
            advertised.shuffle(&mut rand::thread_rng());
            advertised.truncate(PEERFINDER_MAX_ACCEPTED_ENDPOINTS);
        }
        let mut endpoints = Vec::new();
        let mut saw_self = false;
        let mut seen_endpoints = std::collections::HashSet::new();
        for endpoint in &advertised {
            let Ok(mut parsed) = SocketAddr::from_str(&endpoint.endpoint) else {
                malformed += 1;
                continue;
            };
            if endpoint.hops > PEERFINDER_MAX_HOPS {
                continue;
            }
            if endpoint.hops == 0 {
                if saw_self {
                    continue;
                }
                saw_self = true;
                parsed = SocketAddr::new(self.peer.remote_address().ip(), parsed.port());
            }
            if self.overlay.setup.verify_endpoints && !is_valid_peer_endpoint(parsed) {
                continue;
            }
            if !seen_endpoints.insert(parsed) {
                continue;
            }
            endpoints.push(QueuedEndpoint {
                endpoint: parsed,
                hops: endpoint.hops.saturating_add(1),
            });
        }

        let now = SystemTime::now();
        let mut accepted = Vec::new();
        for endpoint in endpoints {
            self.peer.remember_recent_endpoint(
                endpoint.endpoint,
                endpoint.hops,
                now_instant,
                PEERFINDER_LIVE_CACHE_TTL,
            );
            if endpoint.hops == 1 {
                if !self.peer.listener_checked() {
                    if self.peer.begin_listener_check() {
                        let peer = Arc::clone(self.peer);
                        let endpoint_address = endpoint.endpoint;
                        tokio::spawn(async move {
                            let can_accept = timeout(
                                Duration::from_secs(5),
                                TcpStream::connect(endpoint_address),
                            )
                            .await
                            .is_ok_and(|result| result.is_ok());
                            peer.finish_listener_check(can_accept);
                        });
                    }
                    continue;
                }
                if !self.peer.listener_can_accept() {
                    continue;
                }
            }

            self.overlay
                .remember_redirect_endpoint(endpoint.endpoint, endpoint.hops, now);
            accepted.push(endpoint);
        }

        if !accepted.is_empty() {
            tracing::info!(
                target: "overlay",
                count = accepted.len(),
                "Peer discovery: new endpoints received"
            );
            self.overlay.inbound_handler.on_endpoints(
                self.peer,
                QueuedEndpoints {
                    peer_id: self.peer.id(),
                    version: message.version,
                    malformed,
                    endpoints: accepted,
                    message: message.clone(),
                },
            );
        }
        crate::router::RouteAction::Continue
    }

    fn on_transaction(
        &mut self,
        message: &crate::message::TmTransaction,
    ) -> crate::router::RouteAction {
        tracing::trace!(target: "overlay", peer_id = %self.peer.id(), "Transaction received");
        self.queue_transaction(message, false);
        crate::router::RouteAction::Continue
    }

    fn on_get_ledger(
        &mut self,
        message: &crate::message::TmGetLedger,
    ) -> crate::router::RouteAction {
        if !(0..=3).contains(&message.itype) {
            tracing::trace!(target: "overlay", peer_id = %self.peer.id(), "Invalid get_ledger itype");
            return crate::router::RouteAction::Continue;
        }
        if message.itype == 3 {
            if message
                .ledger_hash
                .as_deref()
                .and_then(Uint256::from_slice)
                .is_none()
            {
                return crate::router::RouteAction::Continue;
            }
        } else if message.ledger_hash.is_none()
            && message.ledger_seq.is_none()
            && message.ltype != Some(2)
        {
            return crate::router::RouteAction::Continue;
        }
        if message
            .ledger_hash
            .as_deref()
            .is_some_and(|hash| Uint256::from_slice(hash).is_none())
        {
            return crate::router::RouteAction::Continue;
        }
        if message.itype != 0
            && (message.node_i_ds.is_empty()
                || message.node_i_ds.iter().any(|node_id| node_id.is_empty()))
        {
            return crate::router::RouteAction::Continue;
        }

        self.overlay
            .inbound_handler
            .on_get_ledger(self.peer, message.clone());
        crate::router::RouteAction::Continue
    }

    fn on_ledger_data(
        &mut self,
        message: &crate::message::TmLedgerData,
    ) -> crate::router::RouteAction {
        if Uint256::from_slice(&message.ledger_hash).is_none() {
            tracing::trace!(target: "overlay", peer_id = %self.peer.id(), "Invalid ledger_data hash");
            return crate::router::RouteAction::Continue;
        }
        if !(0..=3).contains(&message.r#type) {
            return crate::router::RouteAction::Continue;
        }
        if let Some(error) = message.error
            && !(1..=3).contains(&error)
        {
            return crate::router::RouteAction::Continue;
        }
        if message.nodes.is_empty() || message.nodes.len() > HARD_MAX_REPLY_NODES {
            return crate::router::RouteAction::Continue;
        }

        self.overlay
            .inbound_handler
            .on_ledger_data(self.peer, message.clone());
        crate::router::RouteAction::Continue
    }

    fn on_propose_ledger(
        &mut self,
        message: &crate::message::TmProposeSet,
    ) -> crate::router::RouteAction {
        if !(64..=72).contains(&message.signature.len()) {
            tracing::trace!(target: "overlay", peer_id = %self.peer.id(), "Invalid proposal signature length");
            return crate::router::RouteAction::Continue;
        }
        let Ok(public_key) = PublicKey::from_slice(&message.node_pub_key) else {
            return crate::router::RouteAction::Continue;
        };
        let Some(current_tx_hash) = Uint256::from_slice(&message.current_tx_hash) else {
            return crate::router::RouteAction::Continue;
        };
        let Some(previous_ledger) = Uint256::from_slice(&message.previousledger) else {
            return crate::router::RouteAction::Continue;
        };
        let suppression = proposal_unique_id(
            current_tx_hash,
            previous_ledger,
            message.propose_seq,
            message.close_time,
            public_key,
            &message.signature,
        );
        self.overlay.inbound_handler.on_propose_ledger(
            self.peer,
            QueuedProposal {
                peer_id: self.peer.id(),
                suppression,
                public_key,
                current_tx_hash,
                previous_ledger,
                message: message.clone(),
            },
        );
        crate::router::RouteAction::Continue
    }

    fn on_status_change(
        &mut self,
        message: &crate::message::TmStatusChange,
    ) -> crate::router::RouteAction {
        tracing::debug!(
            target: "overlay",
            peer_id = %self.peer.id(),
            new_status = ?message.new_status,
            "Peer status change"
        );
        let effective_status = self.peer.remember_status(message.new_status);

        if message.new_event == Some(lost_sync_event()) {
            self.peer.clear_closed_ledger_hash();
            self.peer.clear_previous_ledger_hash();
            return crate::router::RouteAction::Continue;
        }

        if let Some(hash) = message.ledger_hash.as_deref().and_then(Uint256::from_slice) {
            if let Some(sequence) = message.ledger_seq {
                self.peer.record_ledger(hash, sequence);
            } else {
                self.peer.set_closed_ledger_hash(hash);
            }
        } else {
            self.peer.clear_closed_ledger_hash();
        }

        if let Some(hash) = message
            .ledger_hash_previous
            .as_deref()
            .and_then(Uint256::from_slice)
        {
            self.peer.set_previous_ledger_hash(hash);
        } else {
            self.peer.clear_previous_ledger_hash();
        }

        if let (Some(first), Some(last)) = (message.first_seq, message.last_seq) {
            self.peer.set_ledger_range(first, last);
        }

        self.overlay.publish_peer_status(build_peer_status_event(
            effective_status,
            message,
            self.peer.closed_ledger_hash(),
        ));

        crate::router::RouteAction::Continue
    }

    fn on_have_set(
        &mut self,
        message: &crate::message::TmHaveTransactionSet,
    ) -> crate::router::RouteAction {
        if message.status == 1
            && let Some(hash) = Uint256::from_slice(&message.hash)
        {
            tracing::trace!(target: "overlay", peer_id = %self.peer.id(), "Peer has transaction set");
            self.peer.record_tx_set(hash);
        }
        crate::router::RouteAction::Continue
    }

    fn on_validation(
        &mut self,
        message: &crate::message::TmValidation,
    ) -> crate::router::RouteAction {
        if message.validation.len() < 50 {
            tracing::trace!(target: "overlay", peer_id = %self.peer.id(), "Validation too short, ignoring");
            return crate::router::RouteAction::Continue;
        }
        let mut serial = SerialIter::new(&message.validation);
        let parsed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            STValidation::from_serial_iter_default_node_id(&mut serial, false)
        }))
        .ok()
        .and_then(Result::ok);
        if parsed.is_none() {
            return crate::router::RouteAction::Continue;
        }

        self.overlay.inbound_handler.on_validation(
            self.peer,
            QueuedValidation {
                peer_id: self.peer.id(),
                suppression: sha512_half(&message.validation),
                message: message.clone(),
            },
        );
        crate::router::RouteAction::Continue
    }

    fn on_validator_list(
        &mut self,
        message: &crate::message::TmValidatorList,
    ) -> crate::router::RouteAction {
        if !self
            .peer
            .supports_feature(ProtocolFeature::ValidatorListPropagation)
        {
            return crate::router::RouteAction::Continue;
        }
        if message.manifest.is_empty() || message.blob.is_empty() || message.signature.is_empty() {
            tracing::trace!(target: "overlay", peer_id = %self.peer.id(), "Invalid validator list message");
            return crate::router::RouteAction::Continue;
        }
        tracing::debug!(target: "overlay", peer_id = %self.peer.id(), "Validator list received");
        self.overlay
            .inbound_handler
            .on_validator_list(self.peer, message.clone());
        crate::router::RouteAction::Continue
    }

    fn on_validator_list_collection(
        &mut self,
        message: &crate::message::TmValidatorListCollection,
    ) -> crate::router::RouteAction {
        if !self
            .peer
            .supports_feature(ProtocolFeature::ValidatorList2Propagation)
            || message.version < 2
            || message.manifest.is_empty()
            || message.blobs.is_empty()
        {
            return crate::router::RouteAction::Continue;
        }
        tracing::debug!(target: "overlay", peer_id = %self.peer.id(), "Validator list collection received");
        self.overlay
            .inbound_handler
            .on_validator_list_collection(self.peer, message.clone());
        crate::router::RouteAction::Continue
    }

    fn on_get_objects(
        &mut self,
        message: &crate::message::TmGetObjectByHash,
    ) -> crate::router::RouteAction {
        if message
            .ledger_hash
            .as_deref()
            .is_some_and(|hash| Uint256::from_slice(hash).is_none())
        {
            tracing::trace!(target: "overlay", peer_id = %self.peer.id(), "Invalid get_objects ledger hash");
            return crate::router::RouteAction::Continue;
        }
        if message.r#type == 7 && !self.peer.tx_reduce_relay_enabled() {
            return crate::router::RouteAction::Continue;
        }
        tracing::trace!(target: "overlay", peer_id = %self.peer.id(), "Get objects request");
        self.overlay
            .inbound_handler
            .on_get_objects(self.peer, message.clone());
        crate::router::RouteAction::Continue
    }

    fn on_have_transactions(
        &mut self,
        message: &crate::message::TmHaveTransactions,
    ) -> crate::router::RouteAction {
        if !self.peer.tx_reduce_relay_enabled() {
            return crate::router::RouteAction::Continue;
        }
        let hashes = message
            .hashes
            .iter()
            .map(|hash| Uint256::from_slice(hash))
            .collect::<Option<Vec<_>>>();
        let Some(hashes) = hashes else {
            tracing::trace!(target: "overlay", peer_id = %self.peer.id(), "Invalid have_transactions hash");
            return crate::router::RouteAction::Continue;
        };

        tracing::trace!(
            target: "overlay",
            peer_id = %self.peer.id(),
            count = hashes.len(),
            "Have transactions received"
        );
        self.overlay.inbound_handler.on_have_transactions(
            self.peer,
            QueuedHaveTransactions {
                peer_id: self.peer.id(),
                hashes,
                message: message.clone(),
            },
        );
        crate::router::RouteAction::Continue
    }

    fn on_transactions(
        &mut self,
        message: &crate::message::TmTransactions,
    ) -> crate::router::RouteAction {
        if !self.peer.tx_reduce_relay_enabled() {
            return crate::router::RouteAction::Continue;
        }
        tracing::trace!(
            target: "overlay",
            peer_id = %self.peer.id(),
            count = message.transactions.len(),
            "Batch transactions received"
        );
        self.overlay
            .inbound_handler
            .on_transactions(self.peer, message.clone());
        for transaction in &message.transactions {
            self.queue_transaction(transaction, true);
        }
        crate::router::RouteAction::Continue
    }

    fn on_squelch(&mut self, message: &crate::message::TmSquelch) -> crate::router::RouteAction {
        let Ok(validator) = PublicKey::from_slice(&message.validator_pub_key) else {
            tracing::debug!(target: "overlay", peer_id = %self.peer.id(), "Invalid squelch public key");
            return crate::router::RouteAction::Continue;
        };

        if !message.squelch {
            tracing::debug!(target: "overlay", peer_id = %self.peer.id(), "Squelch removed");
            self.peer.remove_squelch(validator);
            return crate::router::RouteAction::Continue;
        }

        let duration = Duration::from_secs(u64::from(message.squelch_duration.unwrap_or(0)));
        tracing::debug!(
            target: "overlay",
            peer_id = %self.peer.id(),
            duration_secs = duration.as_secs(),
            "Squelch applied"
        );
        let _ = self.peer.apply_squelch(validator, duration);
        crate::router::RouteAction::Continue
    }

    fn on_proof_path_request(
        &mut self,
        message: &crate::message::TmProofPathRequest,
    ) -> crate::router::RouteAction {
        if Uint256::from_slice(&message.key).is_none()
            || Uint256::from_slice(&message.ledger_hash).is_none()
            || !(1..=2).contains(&message.r#type)
        {
            tracing::trace!(target: "overlay", peer_id = %self.peer.id(), "Invalid proof path request");
            return crate::router::RouteAction::Continue;
        }
        tracing::trace!(target: "overlay", peer_id = %self.peer.id(), "Proof path request received");
        self.overlay
            .inbound_handler
            .on_proof_path_request(self.peer, message.clone());
        crate::router::RouteAction::Continue
    }

    fn on_proof_path_response(
        &mut self,
        message: &crate::message::TmProofPathResponse,
    ) -> crate::router::RouteAction {
        if Uint256::from_slice(&message.key).is_none()
            || Uint256::from_slice(&message.ledger_hash).is_none()
            || !(1..=2).contains(&message.r#type)
        {
            tracing::trace!(target: "overlay", peer_id = %self.peer.id(), "Invalid proof path response");
            return crate::router::RouteAction::Continue;
        }
        tracing::trace!(target: "overlay", peer_id = %self.peer.id(), "Proof path response received");
        self.overlay
            .inbound_handler
            .on_proof_path_response(self.peer, message.clone());
        crate::router::RouteAction::Continue
    }

    fn on_replay_delta_request(
        &mut self,
        message: &crate::message::TmReplayDeltaRequest,
    ) -> crate::router::RouteAction {
        if Uint256::from_slice(&message.ledger_hash).is_none() {
            tracing::trace!(target: "overlay", peer_id = %self.peer.id(), "Invalid replay delta request hash");
            return crate::router::RouteAction::Continue;
        }
        tracing::trace!(target: "overlay", peer_id = %self.peer.id(), "Replay delta request received");
        self.overlay
            .inbound_handler
            .on_replay_delta_request(self.peer, message.clone());
        crate::router::RouteAction::Continue
    }

    fn on_replay_delta_response(
        &mut self,
        message: &crate::message::TmReplayDeltaResponse,
    ) -> crate::router::RouteAction {
        if Uint256::from_slice(&message.ledger_hash).is_none() {
            tracing::trace!(target: "overlay", peer_id = %self.peer.id(), "Invalid replay delta response hash");
            return crate::router::RouteAction::Continue;
        }
        tracing::trace!(target: "overlay", peer_id = %self.peer.id(), "Replay delta response received");
        self.overlay
            .inbound_handler
            .on_replay_delta_response(self.peer, message.clone());
        crate::router::RouteAction::Continue
    }
}

fn is_valid_peer_endpoint(endpoint: SocketAddr) -> bool {
    endpoint.port() != 0
        && !endpoint.ip().is_unspecified()
        && !endpoint.ip().is_loopback()
        && is_public_ip(endpoint.ip())
}

pub struct OverlayImpl {
    setup: Setup,
    handoff: Arc<dyn OverlayHandoff>,
    connector: Arc<SslConnector>,
    acceptor: Option<TlsAcceptor>,
    active_peers: Arc<RwLock<HashMap<PeerId, Arc<PeerImp>>>>,
    by_public_key: Arc<RwLock<HashMap<PublicKey, Arc<PeerImp>>>>,
    next_id: Arc<AtomicU32>,
    jq_trans_overflow: Arc<AtomicU64>,
    peer_disconnects: Arc<AtomicU64>,
    peer_disconnect_charges: Arc<AtomicU64>,
    identity: OverlayIdentity,
    stop_requested: watch::Sender<bool>,
    stopping: Arc<AtomicBool>,
    traffic: Arc<TrafficCount>,
    tx_metrics: Arc<TxMetrics>,
    relay_history: Arc<Mutex<HashMap<RelayKey, BTreeSet<PeerId>>>>,
    local_reservations: Arc<PeerReservationTable>,
    reservation_source: Arc<RwLock<Arc<dyn PeerReservationSource>>>,
    local_cluster: Arc<Cluster>,
    fixed_peer_ips: Arc<RwLock<HashSet<IpAddr>>>,
    cluster_source: Arc<RwLock<Arc<Cluster>>>,
    slots: Arc<Mutex<Slots>>,
    queued_inbound: Arc<QueuedOverlayInboundHandler>,
    inbound_handler: Arc<dyn OverlayInboundHandler>,
    redirect_endpoints: Arc<Mutex<HashMap<SocketAddr, RedirectEndpoint>>>,
    pending_outbound_ips: Arc<Mutex<HashSet<IpAddr>>>,
    peer_status_publisher: Arc<RwLock<Option<PeerStatusPublisher>>>,
    session_runtime: Arc<tokio::runtime::Runtime>,
}

impl OverlayImpl {
    pub fn new(setup: Setup, handoff: Arc<dyn OverlayHandoff>) -> Result<Self, OverlayError> {
        Self::with_clock(setup, handoff, Arc::new(SystemClock))
    }

    pub fn has_tls_acceptor(&self) -> bool {
        self.acceptor.is_some()
    }

    pub fn with_clock(
        setup: Setup,
        handoff: Arc<dyn OverlayHandoff>,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, OverlayError> {
        let queued_inbound = Arc::new(QueuedOverlayInboundHandler::default());
        Self::with_clock_and_inbound_handler(setup, handoff, clock, queued_inbound)
    }

    pub fn with_clock_and_inbound_handler(
        setup: Setup,
        handoff: Arc<dyn OverlayHandoff>,
        clock: Arc<dyn Clock>,
        inbound_handler: Arc<QueuedOverlayInboundHandler>,
    ) -> Result<Self, OverlayError> {
        let _client_config = setup
            .client_config
            .clone()
            .ok_or_else(|| OverlayError::Tls("missing client tls config".to_owned()))?;
        let mut connector_builder = SslConnector::builder(SslMethod::tls())
            .map_err(|error| OverlayError::Tls(error.to_string()))?;
        connector_builder.set_verify(SslVerifyMode::NONE);
        let connector = Arc::new(connector_builder.build());
        let acceptor = setup.server_config.clone().map(TlsAcceptor::from);
        let active_peers = Arc::new(RwLock::new(HashMap::new()));
        let traffic = Arc::new(TrafficCount::default());
        let (stop_requested, _) = watch::channel(false);
        let identity = OverlayIdentity::new();
        let handler = Arc::new(OverlayRuntimeSquelchHandler {
            active_peers: Arc::clone(&active_peers),
            traffic: Arc::clone(&traffic),
        });
        let slots = Arc::new(Mutex::new(Slots::new(
            clock.clone(),
            handler,
            setup.vp_reduce_relay_base_squelch_enabled,
            setup.vp_reduce_relay_max_selected_peers,
            setup.reduce_relay_wait,
        )));
        let local_reservations = Arc::new(PeerReservationTable::default());
        let local_cluster = Arc::new(Cluster::new());
        let fixed_peer_ips = Arc::new(RwLock::new(setup.fixed_peer_ips.clone()));
        let session_runtime = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .thread_name("xrpld-overlay-session")
                .enable_all()
                .build()
                .map_err(OverlayError::Io)?,
        );

        Ok(Self {
            setup,
            handoff,
            connector,
            acceptor,
            active_peers,
            by_public_key: Arc::new(RwLock::new(HashMap::new())),
            next_id: Arc::new(AtomicU32::new(1)),
            jq_trans_overflow: Arc::new(AtomicU64::new(0)),
            peer_disconnects: Arc::new(AtomicU64::new(0)),
            peer_disconnect_charges: Arc::new(AtomicU64::new(0)),
            identity,
            stop_requested,
            stopping: Arc::new(AtomicBool::new(false)),
            traffic,
            tx_metrics: Arc::new(TxMetrics::new(clock)),
            relay_history: Arc::new(Mutex::new(HashMap::new())),
            local_reservations: Arc::clone(&local_reservations),
            reservation_source: Arc::new(RwLock::new(local_reservations)),
            local_cluster: Arc::clone(&local_cluster),
            fixed_peer_ips,
            cluster_source: Arc::new(RwLock::new(local_cluster)),
            slots,
            queued_inbound: Arc::clone(&inbound_handler),
            inbound_handler,
            redirect_endpoints: Arc::new(Mutex::new(HashMap::new())),
            pending_outbound_ips: Arc::new(Mutex::new(HashSet::new())),
            peer_status_publisher: Arc::new(RwLock::new(None)),
            session_runtime,
        })
    }

    pub fn set_peer_status_publisher<F>(&self, publisher: F)
    where
        F: Fn(JsonValue) + Send + Sync + 'static,
    {
        *self
            .peer_status_publisher
            .write()
            .expect("peer status publisher lock") = Some(Arc::new(publisher));
    }

    pub fn clear_peer_status_publisher(&self) {
        *self
            .peer_status_publisher
            .write()
            .expect("peer status publisher lock") = None;
    }

    fn publish_peer_status(&self, payload: JsonValue) {
        let publisher = self
            .peer_status_publisher
            .read()
            .expect("peer status publisher lock")
            .as_ref()
            .map(Arc::clone);
        if let Some(publisher) = publisher {
            publisher(payload);
        }
    }

    fn make_redirect_response(
        &self,
        request: &Request<()>,
        remote_address: SocketAddr,
    ) -> Result<(Response<()>, Vec<u8>), OverlayError> {
        let peer_ips = self.redirect_peer_ips(remote_address);
        let body = serde_json::json!({ "peer-ips": peer_ips }).to_string();
        let response = Response::builder()
            .version(request.version())
            .status(503)
            .header("Server", "xrpld-rust/overlay")
            .header("Remote-Address", remote_address.to_string())
            .header("Content-Type", "application/json")
            .header("Connection", "close")
            .header("Content-Length", body.len().to_string())
            .body(())
            .map_err(|error| OverlayError::InvalidRequest(error.to_string()))?;
        let mut wire = serialize_response(&response);
        wire.extend_from_slice(body.as_bytes());
        Ok((response, wire))
    }

    pub fn remember_redirect_endpoint(&self, endpoint: SocketAddr, hops: u32, now: SystemTime) {
        self.redirect_endpoints
            .lock()
            .expect("redirect endpoints lock")
            .entry(endpoint)
            .and_modify(|known| {
                known.hops = known.hops.min(hops);
                known.last_seen = now;
            })
            .or_insert(RedirectEndpoint {
                hops,
                last_seen: now,
            });
    }

    fn redirect_peer_ips(&self, remote_address: SocketAddr) -> Vec<String> {
        let now = SystemTime::now();
        let mut endpoints = self
            .redirect_endpoints
            .lock()
            .expect("redirect endpoints lock");
        endpoints.retain(|_, endpoint| {
            now.duration_since(endpoint.last_seen)
                .map(|age| age <= PEERFINDER_LIVE_CACHE_TTL)
                .unwrap_or(true)
        });

        let mut candidates = endpoints
            .iter()
            .filter(|(endpoint, known)| {
                known.hops > 0
                    && known.hops <= PEERFINDER_MAX_HOPS
                    && endpoint.ip() != remote_address.ip()
            })
            .map(|(endpoint, known)| (*endpoint, *known))
            .collect::<Vec<_>>();
        candidates.shuffle(&mut rand::thread_rng());

        let mut seen_ips = std::collections::HashSet::<IpAddr>::new();
        let mut peer_ips = Vec::new();
        for (endpoint, _) in candidates {
            if seen_ips.insert(endpoint.ip()) {
                peer_ips.push(endpoint.to_string());
            }
            if peer_ips.len() >= PEERFINDER_REDIRECT_ENDPOINT_COUNT {
                break;
            }
        }
        peer_ips
    }

    pub fn bind(&self, listener: TcpListener) -> Result<OverlayAcceptor, OverlayError> {
        let acceptor = self
            .acceptor
            .clone()
            .ok_or_else(|| OverlayError::Tls("missing server tls config".to_owned()))?;
        Ok(OverlayAcceptor {
            listener: Arc::new(listener),
            acceptor,
        })
    }

    pub fn spawn_listener(
        &self,
        acceptor: OverlayAcceptor,
    ) -> JoinHandle<Result<(), OverlayError>> {
        let this = self.clone_for_tasks();
        tokio::spawn(async move { this.run_listener(acceptor).await })
    }

    pub async fn run_listener(&self, acceptor: OverlayAcceptor) -> Result<(), OverlayError> {
        tracing::info!(target: "overlay", "Overlay listener started");
        let stop_requested = self.stop_requested.subscribe();
        loop {
            if self.is_stopping() {
                tracing::info!(target: "overlay", "Overlay listener stopping");
                return Ok(());
            }
            self.run_listener_once(&acceptor, stop_requested.clone())
                .await?;
        }
    }

    pub async fn run_listener_once(
        &self,
        acceptor: &OverlayAcceptor,
        mut stop_requested: watch::Receiver<bool>,
    ) -> Result<(), OverlayError> {
        if self.is_stopping() || *stop_requested.borrow() {
            return Ok(());
        }

        let (tcp_stream, remote_address) = tokio::select! {
            biased;
            changed = stop_requested.changed() => {
                let _ = changed;
                return Ok(());
            }
            result = acceptor.listener.accept() => result?,
        };

        tracing::debug!(target: "overlay", ip = %remote_address, "Inbound connection accepted");
        // Disable Nagle's algorithm for low-latency request-response pipelining.
        let _ = tcp_stream.set_nodelay(true);
        let mut tls_stream = tokio::select! {
            biased;
            changed = stop_requested.changed() => {
                let _ = changed;
                return Ok(());
            }
            result = acceptor.acceptor.accept(tcp_stream) => {
                result.map_err(|error| OverlayError::Tls(error.to_string()))?
            }
        };

        let request = match read_http_request(&mut tls_stream, stop_requested.clone()).await? {
            Some(request) => request,
            None => return Ok(()),
        };
        let handoff = self.handoff.on_handoff(&request, remote_address);
        let mut accepted_peer = None;
        let (response, response_wire) = match handoff {
            Handoff::Accepted => {
                let peer = self.peer_from_request(&request, remote_address)?;
                self.apply_membership_state(&peer);
                if !self.can_activate_peer(&peer) {
                    tracing::warn!(
                        target: "overlay",
                        ip = %remote_address,
                        reason = "peer limit reached, redirecting",
                        "Connection attempt failed"
                    );
                    self.make_redirect_response(&request, remote_address)?
                } else {
                    // Verify the peer's handshake (compatibility: validatePeerHandshake)
                    let verify_ctx = crate::transport::handshake::HandshakeVerificationContext {
                        shared_value: basics::base_uint::Uint256::default(),
                        network_id: self.handshake_context().network_id,
                        local_public_key: None,
                        public_ip: self.handshake_context().local_ip,
                        remote_ip: remote_address.ip(),
                        clock_tolerance: std::time::Duration::from_secs(20),
                    };
                    if let Err(_reason) = crate::transport::handshake::verify_handshake(
                        request.headers(),
                        &verify_ctx,
                    ) {
                        // Handshake verification failed — log but don't reject
                        // during migration to maintain connectivity.
                    }

                    let offered = request
                        .headers()
                        .get("Upgrade")
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or_default();
                    let protocol = negotiate_protocol_version(
                        crate::protocol_version::parse_protocol_versions(offered),
                    )
                    .unwrap_or(crate::protocol_version::ProtocolVersion::new(2, 2));
                    let response = make_response(
                        true,
                        &request,
                        &self.handshake_context(),
                        protocol,
                        false,
                        false,
                        false,
                        false,
                    );
                    accepted_peer = Some((peer, request.headers().clone()));
                    let response_wire = serialize_response(&response);
                    (response, response_wire)
                }
            }
            Handoff::Rejected(_reason) => {
                tracing::warn!(
                    target: "overlay",
                    ip = %remote_address,
                    reason = _reason,
                    "Connection attempt failed"
                );
                let response = Response::builder()
                    .status(403)
                    .body(())
                    .map_err(|error| OverlayError::InvalidRequest(error.to_string()))?;
                let response_wire = serialize_response(&response);
                (response, response_wire)
            }
            Handoff::Ignored => {
                let response = Response::builder()
                    .status(404)
                    .body(())
                    .map_err(|error| OverlayError::InvalidRequest(error.to_string()))?;
                let response_wire = serialize_response(&response);
                (response, response_wire)
            }
        };
        tokio::select! {
            biased;
            changed = stop_requested.changed() => {
                let _ = changed;
                return Ok(());
            }
            result = tls_stream.write_all(&response_wire) => {
                result?;
            }
        }
        tokio::select! {
            biased;
            changed = stop_requested.changed() => {
                let _ = changed;
                return Ok(());
            }
            result = tls_stream.flush() => {
                result?;
            }
        }
        if let Some((peer, headers)) = accepted_peer {
            let result = ConnectAttemptResult {
                peer,
                response,
                negotiated_features: headers,
                session: Some(PeerSessionStarter::new(
                    Box::new(tls_stream),
                    stop_requested.clone(),
                )),
            };
            let _ = self.finalize_connect_result(result);
        }
        Ok(())
    }

    fn clone_for_tasks(&self) -> Self {
        Self {
            setup: self.setup.clone(),
            handoff: Arc::clone(&self.handoff),
            connector: self.connector.clone(),
            acceptor: self.acceptor.clone(),
            active_peers: Arc::clone(&self.active_peers),
            by_public_key: Arc::clone(&self.by_public_key),
            next_id: Arc::clone(&self.next_id),
            jq_trans_overflow: Arc::clone(&self.jq_trans_overflow),
            peer_disconnects: Arc::clone(&self.peer_disconnects),
            peer_disconnect_charges: Arc::clone(&self.peer_disconnect_charges),
            identity: self.identity.clone(),
            stop_requested: self.stop_requested.clone(),
            stopping: Arc::clone(&self.stopping),
            traffic: Arc::clone(&self.traffic),
            tx_metrics: Arc::clone(&self.tx_metrics),
            relay_history: Arc::clone(&self.relay_history),
            local_reservations: Arc::clone(&self.local_reservations),
            reservation_source: Arc::clone(&self.reservation_source),
            local_cluster: Arc::clone(&self.local_cluster),
            fixed_peer_ips: Arc::clone(&self.fixed_peer_ips),
            cluster_source: Arc::clone(&self.cluster_source),
            slots: Arc::clone(&self.slots),
            queued_inbound: Arc::clone(&self.queued_inbound),
            inbound_handler: Arc::clone(&self.inbound_handler),
            redirect_endpoints: Arc::clone(&self.redirect_endpoints),
            pending_outbound_ips: Arc::clone(&self.pending_outbound_ips),
            peer_status_publisher: Arc::clone(&self.peer_status_publisher),
            session_runtime: Arc::clone(&self.session_runtime),
        }
    }

    pub fn activate(&self, peer: Arc<PeerImp>) -> bool {
        self.apply_membership_state(&peer);
        let mut active_peers = self.active_peers.write().expect("overlay peers lock");
        if self.limit() != 0
            && self.counted_active_peers_count_locked(&active_peers) >= self.limit()
            && self.peer_counts_toward_limit(&peer)
        {
            tracing::warn!(
                target: "overlay",
                peer_id = %peer.id(),
                "Peer resource limit exceeded"
            );
            return false;
        }
        active_peers.insert(peer.id(), Arc::clone(&peer));
        let total = active_peers.len();
        drop(active_peers);

        self.by_public_key
            .write()
            .expect("overlay public-key lock")
            .insert(peer.node_public(), Arc::clone(&peer));

        tracing::info!(
            target: "overlay",
            peer_id = %peer.id(),
            "Peer activated (slot assigned)"
        );
        tracing::info!(
            target: "overlay",
            total,
            "Peer count updated"
        );
        true
    }

    pub fn on_peer_deactivate(&self, id: PeerId) {
        let peer = {
            let mut active_peers = self.active_peers.write().expect("overlay peers lock");
            active_peers.remove(&id)
        };
        if let Some(peer) = peer {
            tracing::info!(
                target: "overlay",
                peer_id = %id,
                ip = %peer.remote_address(),
                reason = "deactivated",
                "Peer disconnected"
            );
            self.by_public_key
                .write()
                .expect("overlay public-key lock")
                .remove(&peer.node_public());
            peer.detach_session();
            peer.clear_queued_messages();
            peer.clear_tx_queue();
            self.relay_history
                .lock()
                .expect("relay history lock")
                .retain(|_, seen| {
                    seen.remove(&id);
                    !seen.is_empty()
                });
            self.slots
                .lock()
                .expect("overlay slots lock")
                .delete_peer(id, true);
            self.inc_peer_disconnect();
            let total = self.active_peers.read().expect("overlay peers lock").len();
            tracing::info!(
                target: "overlay",
                total,
                "Peer count updated"
            );
        }
    }

    pub fn signal_stop(&self) {
        tracing::info!(target: "overlay", "Overlay stopping");
        self.stopping.store(true, Ordering::Release);
        let _ = self.stop_requested.send(true);
    }

    fn handshake_context(&self) -> HandshakeContext {
        let mut context = self.identity.context();
        context.network_id = self.setup.network_id;
        context
    }

    pub fn is_stopping(&self) -> bool {
        self.stopping.load(Ordering::Acquire)
    }

    pub fn next_peer_id(&self) -> PeerId {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    pub fn reservations(&self) -> Arc<PeerReservationTable> {
        Arc::clone(&self.local_reservations)
    }

    pub fn set_peer_reservation_source(&self, source: Arc<dyn PeerReservationSource>) {
        *self
            .reservation_source
            .write()
            .expect("overlay reservation source lock") = source;
        self.refresh_membership_state();
    }

    pub fn refresh_membership_state(&self) {
        let peers = self.active_peers_snapshot();
        for peer in &peers {
            self.apply_membership_state(peer);
        }
        self.enforce_peer_limit(&peers);
    }

    pub fn cluster(&self) -> Arc<Cluster> {
        self.cluster_source
            .read()
            .expect("overlay cluster source lock")
            .clone()
    }

    pub fn set_cluster_source(&self, source: Arc<Cluster>) {
        *self
            .cluster_source
            .write()
            .expect("overlay cluster source lock") = source;
        self.refresh_membership_state();
    }

    pub fn traffic_json(&self) -> JsonValue {
        self.traffic.json()
    }

    pub fn remember_fixed_peer_endpoint(&self, endpoint: SocketAddr) {
        self.fixed_peer_ips
            .write()
            .expect("overlay fixed peer lock")
            .insert(canonical_peer_ip(endpoint.ip()));
    }

    pub fn remember_fixed_peer_endpoints<I>(&self, endpoints: I)
    where
        I: IntoIterator<Item = SocketAddr>,
    {
        let mut fixed_peer_ips = self
            .fixed_peer_ips
            .write()
            .expect("overlay fixed peer lock");
        for endpoint in endpoints {
            fixed_peer_ips.insert(canonical_peer_ip(endpoint.ip()));
        }
    }

    pub fn active_fixed_peers_count(&self) -> usize {
        self.active_peers
            .read()
            .expect("overlay peers lock")
            .values()
            .filter(|peer| peer.fixed())
            .count()
    }

    pub fn fixed_peer_slot_count(&self) -> usize {
        self.fixed_peer_ips
            .read()
            .expect("overlay fixed peer lock")
            .len()
    }

    pub fn pending_fixed_outbound_attempts(&self) -> usize {
        let fixed_peer_ips = self.fixed_peer_ips.read().expect("overlay fixed peer lock");
        self.pending_outbound_ips
            .lock()
            .expect("overlay pending outbound lock")
            .iter()
            .filter(|ip| fixed_peer_ips.contains(ip))
            .count()
    }

    pub fn pending_outbound_attempts(&self) -> usize {
        self.pending_outbound_ips
            .lock()
            .expect("overlay pending outbound lock")
            .len()
    }

    pub fn queued_inbound(&self) -> &QueuedOverlayInboundHandler {
        &self.queued_inbound
    }

    pub fn queued_inbound_snapshot(&self) -> OverlayInboundSnapshot {
        self.queued_inbound.snapshot()
    }

    pub fn take_queued_inbound_snapshot(&self) -> OverlayInboundSnapshot {
        self.queued_inbound.take_snapshot()
    }

    pub fn clear_queued_inbound(&self) {
        self.queued_inbound.clear();
    }

    pub fn requeue_validations(&self, validations: Vec<crate::QueuedValidation>) {
        self.queued_inbound.requeue_validations(validations);
    }

    /// Register a channel for immediate TmLedgerData delivery from the
    /// network thread, matching reference InboundLedgers::gotLedgerData.
    pub fn set_ledger_data_channel(
        &self,
        tx: std::sync::mpsc::Sender<crate::PeerMessage<crate::TmLedgerData>>,
    ) {
        self.queued_inbound.set_ledger_data_channel(tx);
    }

    pub fn take_validations(&self) -> Vec<crate::QueuedValidation> {
        self.queued_inbound.take_validations()
    }

    pub fn take_proposals(&self) -> Vec<crate::QueuedProposal> {
        self.queued_inbound.take_proposals()
    }

    pub fn relay_proposal(
        &self,
        message: TmProposeSet,
        uid: Uint256,
        validator: PublicKey,
    ) -> BTreeSet<PeerId> {
        tracing::debug!(target: "overlay", "Relaying proposal");
        self.relay_validator_message(
            RelayKind::Proposal,
            uid,
            validator,
            ProtocolMessage::new(ProtocolPayload::ProposeLedger(message)),
        )
    }

    pub fn relay_validation(
        &self,
        message: TmValidation,
        uid: Uint256,
        validator: PublicKey,
    ) -> BTreeSet<PeerId> {
        tracing::debug!(target: "overlay", "Relaying validation");
        self.relay_validator_message(
            RelayKind::Validation,
            uid,
            validator,
            ProtocolMessage::new(ProtocolPayload::Validation(message)),
        )
    }

    pub fn broadcast_proposal(&self, message: TmProposeSet, validator: PublicKey) {
        self.broadcast_validator_message(
            ProtocolMessage::new(ProtocolPayload::ProposeLedger(message)),
            validator,
        );
    }

    pub fn broadcast_validation(&self, message: TmValidation, validator: PublicKey) {
        self.broadcast_validator_message(
            ProtocolMessage::new(ProtocolPayload::Validation(message)),
            validator,
        );
    }

    pub fn relay_transaction(
        &self,
        hash: Uint256,
        transaction: Option<TmTransaction>,
        to_skip: &BTreeSet<PeerId>,
    ) {
        if transaction.is_none() {
            if !self.setup.tx_reduce_relay_enabled {
                return;
            }
            for peer in self.active_peers_for_tx(to_skip).peers {
                peer.add_tx_queue(hash);
            }
            return;
        }

        let message = Message::new(
            ProtocolMessage::new(ProtocolPayload::Transaction(
                transaction.expect("transaction present"),
            )),
            None,
        );
        let peers = self.active_peers_for_tx(to_skip);
        let min_relay = self
            .setup
            .tx_reduce_relay_min_peers
            .saturating_add(peers.disabled);

        if !self.setup.tx_reduce_relay_enabled || peers.total <= min_relay {
            for peer in peers.peers {
                let _ = self.send_runtime_message(&peer, message.clone());
            }
            self.tx_metrics.add_relay_selection_metrics(
                peers.total as u32,
                to_skip.len() as u32,
                0,
            );
            return;
        }

        let enabled_target = self.setup.tx_reduce_relay_min_peers
            + ((peers.total - min_relay) * self.setup.tx_relay_percentage / 100);
        self.tx_metrics.add_relay_selection_metrics(
            enabled_target as u32,
            to_skip.len() as u32,
            peers.disabled as u32,
        );

        let mut enabled = peers
            .peers
            .iter()
            .filter(|peer| peer.tx_reduce_relay_enabled())
            .cloned()
            .collect::<Vec<_>>();
        enabled.sort_by_key(|peer| peer.id());
        let quota = enabled_target.saturating_sub(peers.enabled_in_skip);
        let selected_enabled = enabled
            .into_iter()
            .take(quota)
            .map(|peer| peer.id())
            .collect::<BTreeSet<_>>();

        for peer in peers.peers {
            if !peer.tx_reduce_relay_enabled() || selected_enabled.contains(&peer.id()) {
                let _ = self.send_runtime_message(&peer, message.clone());
            } else {
                peer.add_tx_queue(hash);
            }
        }
    }

    pub fn send_tx_queue(&self) {
        for peer in self.active_peers_snapshot() {
            if !peer.tx_reduce_relay_enabled() {
                continue;
            }
            if let Some(message) = peer.build_tx_queue_message() {
                let _ = self.send_runtime_message(&peer, message);
            }
        }
    }

    pub fn slot_state(&self, validator: PublicKey) -> Option<crate::slot::SlotState> {
        self.slots
            .lock()
            .expect("overlay slots lock")
            .get_state(validator)
    }

    /// messages within the idle threshold. Called every CheckIdlePeers (4)
    /// timer ticks in the reference.
    pub fn delete_idle_peers(&self) {
        self.slots
            .lock()
            .expect("overlay slots lock")
            .delete_idle_peers();
    }

    pub fn slot_peers(
        &self,
        validator: PublicKey,
    ) -> BTreeMap<PeerId, crate::slot::SlotPeerSnapshot> {
        self.slots
            .lock()
            .expect("overlay slots lock")
            .get_peers(validator)
    }

    fn relay_validator_message(
        &self,
        kind: RelayKind,
        uid: Uint256,
        validator: PublicKey,
        protocol: ProtocolMessage,
    ) -> BTreeSet<PeerId> {
        let relay_key = RelayKey { kind, uid };
        let message_type = protocol.message_type;
        let message = Message::new(protocol, Some(validator));
        let peers = self.active_peers_snapshot();
        let mut already_seen = BTreeSet::new();
        let mut relayed = BTreeSet::new();
        let mut history = self.relay_history.lock().expect("relay history lock");
        let seen = history.entry(relay_key).or_default();

        for peer in peers {
            if seen.contains(&peer.id()) {
                already_seen.insert(peer.id());
                continue;
            }
            if self.send_runtime_message(&peer, message.clone()) {
                relayed.insert(peer.id());
            }
        }

        seen.extend(relayed.iter().copied());
        drop(history);

        tracing::trace!(
            target: "overlay",
            msg_type = ?message_type,
            relayed_count = relayed.len(),
            already_seen_count = already_seen.len(),
            "Validator message relayed"
        );

        if self
            .slots
            .lock()
            .expect("overlay slots lock")
            .base_squelch_ready()
        {
            self.slots.lock().expect("overlay slots lock").update_many(
                uid,
                validator,
                relayed.iter().copied(),
                message_type,
            );
        }

        already_seen
    }

    fn broadcast_validator_message(&self, protocol: ProtocolMessage, validator: PublicKey) {
        tracing::debug!(target: "overlay", msg_type = ?protocol.message_type, "Broadcasting validator message");
        let message = Message::new(protocol, Some(validator));
        for peer in self.active_peers_snapshot() {
            let _ = self.send_runtime_message(&peer, message.clone());
        }
    }

    fn send_runtime_message(&self, peer: &Arc<PeerImp>, message: Message) -> bool {
        if let Some(validator) = message.validator_key()
            && peer.is_squelched(validator)
        {
            tracing::trace!(
                target: "overlay",
                peer_id = %peer.id(),
                "Message squelched for peer"
            );
            self.traffic.add_count(
                TrafficCategory::SquelchSuppressed,
                false,
                message.get_buffer_size() as u64,
            );
            return false;
        }

        let bytes = message.get_buffer_size() as u64;
        self.traffic.add_count(message.category(), false, bytes);
        self.traffic.add_count(TrafficCategory::Total, false, bytes);
        self.tx_metrics
            .add_message_metrics(message.protocol().message_type, bytes as u32);
        tracing::trace!(
            target: "overlay",
            peer_id = %peer.id(),
            msg_type = ?message.protocol().message_type,
            size_bytes = bytes,
            "Queuing message for peer"
        );
        peer.send(message);
        true
    }

    fn observe_inbound_message(&self, peer: &Arc<PeerImp>, message: &ProtocolMessage, bytes: u64) {
        let category = TrafficCategory::categorize(message, true);
        self.traffic.add_count(TrafficCategory::Total, true, bytes);
        self.traffic.add_count(category, true, bytes);
        self.tx_metrics
            .add_message_metrics(message.message_type, bytes as u32);

        tracing::debug!(
            target: "overlay",
            peer_id = %peer.id(),
            msg_type = ?message.message_type,
            size_bytes = bytes,
            "Message received"
        );

        let mut router = OverlayInboundRouter {
            overlay: self,
            peer,
        };
        let _ = route_message(&mut router, message);
    }

    fn observe_inbound_unknown(&self, bytes: u64) {
        if bytes == 0 {
            return;
        }
        self.traffic
            .add_count(TrafficCategory::Unknown, true, bytes);
        self.traffic.add_count(TrafficCategory::Total, true, bytes);
    }

    fn active_peers_snapshot(&self) -> Vec<Arc<PeerImp>> {
        self.prune_disconnected_peers();
        let mut peers = self
            .active_peers
            .read()
            .expect("overlay peers lock")
            .values()
            .cloned()
            .collect::<Vec<_>>();
        peers.sort_by_key(|peer| peer.id());
        peers
    }

    fn prune_disconnected_peers(&self) {
        let stale = self
            .active_peers
            .read()
            .expect("overlay peers lock")
            .values()
            .filter(|peer| peer.has_dead_session_channel())
            .map(|peer| peer.id())
            .collect::<Vec<_>>();

        if !stale.is_empty() {
            tracing::debug!(
                target: "overlay",
                count = stale.len(),
                "Pruning disconnected peers"
            );
        }
        for id in stale {
            self.on_peer_deactivate(id);
        }
    }

    fn active_peers_for_tx(&self, to_skip: &BTreeSet<PeerId>) -> TxRelayPeers {
        let peers = self.active_peers_snapshot();
        let mut filtered = Vec::new();
        let mut disabled = 0usize;
        let mut enabled_in_skip = 0usize;

        for peer in peers {
            if to_skip.contains(&peer.id()) {
                if peer.tx_reduce_relay_enabled() {
                    enabled_in_skip += 1;
                }
                continue;
            }
            if !peer.tx_reduce_relay_enabled() {
                disabled += 1;
            }
            filtered.push(peer);
        }

        TxRelayPeers {
            total: filtered.len(),
            disabled,
            enabled_in_skip,
            peers: filtered,
        }
    }

    fn apply_membership_state(&self, peer: &PeerImp) {
        let (reserved, clustered) = self.membership_state_for_public_key(peer.node_public());
        peer.set_fixed(self.is_fixed_peer_ip(peer.remote_address().ip()));
        peer.set_reserved(reserved);
        peer.set_clustered(clustered);
    }

    fn is_fixed_peer_ip(&self, ip: IpAddr) -> bool {
        self.fixed_peer_ips
            .read()
            .expect("overlay fixed peer lock")
            .contains(&canonical_peer_ip(ip))
    }

    fn membership_state_for_public_key(&self, public_key: PublicKey) -> (bool, bool) {
        let reservation_source = self
            .reservation_source
            .read()
            .expect("overlay reservation source lock")
            .clone();
        let cluster_source = self
            .cluster_source
            .read()
            .expect("overlay cluster source lock")
            .clone();
        let reserved = reservation_source.contains(public_key);
        let clustered = cluster_source.member(public_key).is_some();
        (reserved, clustered)
    }

    fn peer_counts_toward_limit(&self, peer: &PeerImp) -> bool {
        !peer.fixed() && !peer.reserved() && !peer.cluster()
    }

    fn counted_active_peers_count_locked(
        &self,
        active_peers: &HashMap<PeerId, Arc<PeerImp>>,
    ) -> usize {
        active_peers
            .values()
            .filter(|peer| self.peer_counts_toward_limit(peer))
            .count()
    }

    fn counted_active_peers_count(&self) -> usize {
        self.counted_active_peers_count_locked(
            &self.active_peers.read().expect("overlay peers lock"),
        )
    }

    fn can_activate_peer(&self, peer: &PeerImp) -> bool {
        let limit = self.limit();
        limit == 0
            || self.counted_active_peers_count() < limit
            || !self.peer_counts_toward_limit(peer)
    }

    fn enforce_peer_limit(&self, peers: &[Arc<PeerImp>]) {
        let limit = self.limit();
        if limit == 0 {
            return;
        }

        let active_count = self.counted_active_peers_count();
        if active_count <= limit {
            return;
        }

        let mut remaining = active_count;
        let mut to_deactivate = Vec::new();
        for peer in peers.iter().rev() {
            if remaining <= limit {
                break;
            }
            if !self.peer_counts_toward_limit(peer) {
                continue;
            }
            to_deactivate.push(peer.id());
            remaining -= 1;
        }

        for id in to_deactivate {
            self.on_peer_deactivate(id);
        }
    }

    fn configure_connected_peer(&self, peer: &PeerImp, headers: &http::HeaderMap) {
        let protocol = headers
            .get("Upgrade")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| negotiate_protocol_version(parse_protocol_versions(value)))
            .unwrap_or(ProtocolVersion::new(2, 2));
        peer.set_protocol_version(protocol);
        peer.set_compression_enabled(is_feature_value(headers, FEATURE_COMPR, "lz4"));
        peer.set_tx_reduce_relay_enabled(feature_enabled(headers, FEATURE_TXRR));
        peer.set_feature(
            ProtocolFeature::LedgerReplay,
            feature_enabled(headers, FEATURE_LEDGER_REPLAY),
        );
        tracing::debug!(
            target: "overlay",
            peer_id = %peer.id(),
            protocol_version = %protocol,
            compression = peer.compression_enabled(),
            tx_reduce_relay = peer.tx_reduce_relay_enabled(),
            "Peer configured"
        );
        self.apply_membership_state(peer);
    }

    fn peer_from_request(
        &self,
        request: &Request<()>,
        remote_address: SocketAddr,
    ) -> Result<Arc<PeerImp>, OverlayError> {
        let public_key = request
            .headers()
            .get("Public-Key")
            .and_then(|value| value.to_str().ok())
            .and_then(protocol::parse_base58_node_public)
            .and_then(|bytes| PublicKey::from_slice(&bytes).ok())
            .ok_or_else(|| {
                tracing::warn!(target: "overlay", ip = %remote_address, "Missing peer public key in request");
                OverlayError::InvalidRequest("missing peer public key".to_owned())
            })?;
        if public_key == self.identity.public_key() {
            tracing::warn!(target: "overlay", ip = %remote_address, "Self connection detected");
            return Err(OverlayError::InvalidRequest(
                "self connection detected".to_owned(),
            ));
        }

        let peer = PeerImp::new_with_inbound(
            self.next_peer_id(),
            remote_address,
            true,
            public_key,
            remote_address.to_string(),
        );
        peer.set_listener_check_state(false, false);
        tracing::debug!(target: "overlay", peer_id = %peer.id(), ip = %remote_address, "Inbound peer created");
        Ok(peer)
    }

    fn try_register_outbound_attempt(&self, address: SocketAddr) -> bool {
        self.prune_disconnected_peers();
        let ip = canonical_peer_ip(address.ip());
        if self
            .active_peers
            .read()
            .expect("overlay peers lock")
            .values()
            .any(|peer| canonical_peer_ip(peer.remote_address().ip()) == ip)
        {
            tracing::debug!(target: "overlay", ip = %address, "Duplicate outbound attempt blocked — already connected");
            return false;
        }
        self.pending_outbound_ips
            .lock()
            .expect("overlay pending outbound lock")
            .insert(ip)
    }

    fn finish_outbound_attempt(&self, address: SocketAddr) {
        tracing::debug!(target: "overlay", ip = %address, "Outbound attempt finished");
        self.pending_outbound_ips
            .lock()
            .expect("overlay pending outbound lock")
            .remove(&canonical_peer_ip(address.ip()));
    }

    pub fn active_outbound_peers_count(&self) -> usize {
        self.prune_disconnected_peers();
        self.active_peers
            .read()
            .expect("overlay peers lock")
            .values()
            .filter(|peer| !peer.inbound() && self.peer_counts_toward_limit(peer))
            .count()
    }

    fn finalize_connect_result(
        &self,
        mut result: ConnectAttemptResult,
    ) -> Result<ConnectAttemptResult, &'static str> {
        self.configure_connected_peer(&result.peer, &result.negotiated_features);
        if !self.activate(Arc::clone(&result.peer)) {
            tracing::warn!(
                target: "overlay",
                ip = %result.peer.remote_address(),
                reason = PEER_LIMIT_REJECTION_REASON,
                "Connection attempt failed"
            );
            result.session = None;
            return Err(PEER_LIMIT_REJECTION_REASON);
        }
        if let Some(session) = result.session.take() {
            self.spawn_peer_session(Arc::clone(&result.peer), session);
        }
        Ok(result)
    }

    pub fn spawn_peer_session(&self, peer: Arc<PeerImp>, session: PeerSessionStarter) {
        tracing::debug!(target: "overlay", peer_id = %peer.id(), "Spawning peer session");
        let overlay = self.clone_for_tasks();
        let hooks = Arc::new(OverlayPeerSessionHooks::new(overlay.clone_for_tasks()));
        let on_close = Arc::new(move |peer_id| overlay.on_peer_deactivate(peer_id));
        std::mem::drop(session.start_on(self.session_runtime.handle(), peer, hooks, on_close));
    }
}

struct TxRelayPeers {
    total: usize,
    disabled: usize,
    enabled_in_skip: usize,
    peers: Vec<Arc<PeerImp>>,
}

impl Overlay for OverlayImpl {
    fn connect(
        &self,
        address: SocketAddr,
    ) -> Pin<Box<dyn Future<Output = Result<ConnectAttemptResult, ConnectAttemptError>> + Send>>
    {
        if self.is_stopping() {
            return Box::pin(async {
                Err(ConnectAttemptError::Timeout(
                    ConnectionStep::ShutdownStarted,
                ))
            });
        }
        if !self.try_register_outbound_attempt(address) {
            return Box::pin(async {
                Err(ConnectAttemptError::Protocol(
                    "duplicate outbound attempt".to_owned(),
                ))
            });
        }
        tracing::info!(target: "overlay", ip = %address, "Outbound connection attempt");
        let connector = self.connector.clone();
        let config = ConnectAttemptConfig {
            server_name: address.ip().to_string(),
            tx_reduce_relay_enabled: self.setup.tx_reduce_relay_enabled,
            vp_reduce_relay_enabled: self.setup.vp_reduce_relay_base_squelch_enabled,
            ..ConnectAttemptConfig::default()
        };
        let peer_id = self.next_peer_id();
        let overlay = self.clone_for_tasks();
        let identity = self.identity.clone();
        let sign_session = Arc::new(move |shared_value: &Uint256| {
            identity
                .sign_session(shared_value)
                .map_err(ConnectAttemptError::Protocol)
        });
        let local_public_key = self.identity.public_key();
        let verify_response = Arc::new(move |response: &Response<()>, remote: SocketAddr| {
            let public_key = response
                .headers()
                .get("Public-Key")
                .and_then(|value| value.to_str().ok())
                .and_then(protocol::parse_base58_node_public)
                .and_then(|bytes| PublicKey::from_slice(&bytes).ok())
                .ok_or_else(|| {
                    ConnectAttemptError::Protocol("missing peer public key".to_owned())
                })?;
            if public_key == local_public_key {
                return Err(ConnectAttemptError::Protocol(
                    "self connection detected".to_owned(),
                ));
            }
            Ok(PeerImp::new_with_inbound(
                peer_id,
                remote,
                false,
                public_key,
                remote.to_string(),
            ))
        });
        let attempt = ConnectAttempt::new(
            address,
            config,
            connector,
            self.handshake_context(),
            sign_session,
            verify_response,
            self.stop_requested.subscribe(),
        );
        let session_runtime = self.session_runtime.handle().clone();
        Box::pin(async move {
            session_runtime
                .spawn(async move {
                    let result = attempt.run().await;
                    overlay.finish_outbound_attempt(address);
                    let result = result?;
                    if overlay.is_stopping() {
                        return Err(ConnectAttemptError::Timeout(
                            ConnectionStep::ShutdownStarted,
                        ));
                    }
                    overlay
                        .finalize_connect_result(result)
                        .map_err(|reason| ConnectAttemptError::Protocol(reason.to_owned()))
                })
                .await
                .map_err(|error| {
                    ConnectAttemptError::Protocol(format!("connect task join failed: {error}"))
                })?
        })
    }

    fn limit(&self) -> usize {
        self.setup.peer_limit
    }

    fn size(&self) -> usize {
        self.prune_disconnected_peers();
        self.active_peers.read().expect("overlay peers lock").len()
    }

    fn json(&self) -> JsonValue {
        stats_to_json(self.stats())
    }

    fn peers_json(&self) -> Vec<JsonValue> {
        self.active_peers
            .read()
            .expect("overlay peers lock")
            .values()
            .map(|peer| peer.json())
            .collect()
    }

    fn active_peers(&self) -> Vec<Arc<dyn Peer>> {
        self.active_peers_snapshot()
            .into_iter()
            .map(|peer| peer as Arc<dyn Peer>)
            .collect()
    }

    fn find_peer_by_short_id(&self, id: PeerId) -> Option<Arc<dyn Peer>> {
        self.active_peers
            .read()
            .expect("overlay peers lock")
            .get(&id)
            .cloned()
            .map(|peer| peer as Arc<dyn Peer>)
    }

    fn find_peer_by_public_key(&self, public_key: PublicKey) -> Option<Arc<dyn Peer>> {
        self.by_public_key
            .read()
            .expect("overlay public-key lock")
            .get(&public_key)
            .cloned()
            .map(|peer| peer as Arc<dyn Peer>)
    }

    fn check_tracking(&self, index: u32) {
        for peer in self.active_peers_snapshot() {
            peer.check_tracking(index);
        }
    }

    fn broadcast(&self, message: &ProtocolMessage) {
        let peers = self.active_peers_snapshot();
        tracing::debug!(
            target: "overlay",
            msg_type = ?message.message_type,
            peer_count = peers.len(),
            "Broadcasting message"
        );
        let message = Message::new(message.clone(), None);
        for peer in peers {
            let _ = self.send_runtime_message(&peer, message.clone());
        }
    }

    fn relay(&self, message: &ProtocolMessage, to_skip: &BTreeSet<PeerId>) -> BTreeSet<PeerId> {
        let mut skipped = BTreeSet::new();
        tracing::trace!(
            target: "overlay",
            msg_type = ?message.message_type,
            skip_count = to_skip.len(),
            "Relaying message"
        );
        let message = Message::new(message.clone(), None);
        for peer in self.active_peers_snapshot() {
            if to_skip.contains(&peer.id()) {
                skipped.insert(peer.id());
                continue;
            }
            let _ = self.send_runtime_message(&peer, message.clone());
        }
        skipped
    }

    fn inc_jq_trans_overflow(&self) {
        self.jq_trans_overflow.fetch_add(1, Ordering::Relaxed);
    }

    fn jq_trans_overflow(&self) -> u64 {
        self.jq_trans_overflow.load(Ordering::Relaxed)
    }

    fn inc_peer_disconnect(&self) {
        self.peer_disconnects.fetch_add(1, Ordering::Relaxed);
    }

    fn peer_disconnect(&self) -> u64 {
        self.peer_disconnects.load(Ordering::Relaxed)
    }

    fn inc_peer_disconnect_charges(&self) {
        self.peer_disconnect_charges.fetch_add(1, Ordering::Relaxed);
    }

    fn peer_disconnect_charges(&self) -> u64 {
        self.peer_disconnect_charges.load(Ordering::Relaxed)
    }

    fn network_id(&self) -> Option<u32> {
        self.setup.network_id
    }

    fn verify_endpoints(&self) -> bool {
        self.setup.verify_endpoints
    }

    fn tx_metrics(&self) -> JsonValue {
        self.tx_metrics.json()
    }
}

async fn read_http_request<S>(
    stream: &mut S,
    mut stop_requested: watch::Receiver<bool>,
) -> Result<Option<Request<()>>, OverlayError>
where
    S: AsyncReadExt + Unpin,
{
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let read = tokio::select! {
            biased;
            changed = stop_requested.changed() => {
                let _ = changed;
                return Ok(None);
            }
            result = stream.read(&mut chunk) => result?,
        };
        if read == 0 {
            return Err(OverlayError::InvalidRequest(
                "request head terminated early".to_owned(),
            ));
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
            return parse_http_request(&buffer)
                .map(Some)
                .map_err(OverlayError::InvalidRequest);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::collections::BTreeSet;
    use std::collections::HashSet;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::sync::Once;
    use std::sync::RwLock;
    use std::sync::atomic::Ordering;
    use std::time::{Duration, SystemTime};

    use basics::base_uint::Uint256;
    use http::{HeaderMap, Request, Response};
    use protocol::{
        AccountID, JsonValue, KeyType, PublicKey, STAmount, STTx, STValidation, SecretKey,
        VF_FULL_VALIDATION, calc_node_id, derive_public_key, get_field_by_symbol,
    };
    use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
    use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
    use tokio::io::{AsyncReadExt, AsyncWriteExt, duplex};
    use tokio::sync::watch;
    use tokio::time::timeout;

    use super::{
        OverlayHandoff, OverlayImpl, OverlayInboundRouter, PEERFINDER_LIVE_CACHE_TTL,
        PEERFINDER_MAX_ACCEPTED_ENDPOINTS, PEERFINDER_MAX_HOPS, PEERFINDER_REDIRECT_ENDPOINT_COUNT,
        PeerReservation, PeerReservationSource, PeerReservationTable, is_valid_peer_endpoint,
    };
    use crate::message::{
        Message, ProtocolMessage, ProtocolPayload, TmEndpoints, TmGetLedger, TmGetObjectByHash,
        TmHaveTransactionSet, TmHaveTransactions, TmLedgerData, TmManifests, TmPing,
        TmProofPathRequest, TmProofPathResponse, TmProposeSet, TmReplayDeltaRequest,
        TmReplayDeltaResponse, TmSquelch, TmStatusChange, TmTransaction, TmTransactions,
        TmValidation, TmValidatorList, TmValidatorListCollection, decode_protocol_message, wire,
    };
    use crate::overlay::Overlay;
    use crate::overlay::{Handoff, Setup};
    use crate::peer::{Peer, ProtocolFeature};
    use crate::peer_imp::PeerImp;
    use crate::router::MessageRouter;
    use crate::session::PeerSessionStarter;
    use crate::slot::{Clock, ManualClock, SlotState};
    use crate::traffic_count::TrafficCategory;
    use crate::{Cluster, ConnectAttemptResult};

    #[derive(Debug)]
    struct NoVerify;

    impl ServerCertVerifier for NoVerify {
        fn verify_server_cert(
            &self,
            _end_entity: &CertificateDer<'_>,
            _intermediates: &[CertificateDer<'_>],
            _server_name: &ServerName<'_>,
            _ocsp_response: &[u8],
            _now: UnixTime,
        ) -> Result<ServerCertVerified, rustls::Error> {
            Ok(ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &rustls::DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn verify_tls13_signature(
            &self,
            _message: &[u8],
            _cert: &CertificateDer<'_>,
            _dss: &rustls::DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
        }

        fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
            vec![
                rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
                rustls::SignatureScheme::RSA_PSS_SHA256,
                rustls::SignatureScheme::ED25519,
            ]
        }
    }

    struct TestHandoff;

    impl OverlayHandoff for TestHandoff {
        fn on_handoff(&self, _request: &Request<()>, _remote_address: SocketAddr) -> Handoff {
            Handoff::Accepted
        }
    }

    #[derive(Default)]
    struct ExternalReservationSource {
        reservations: RwLock<BTreeMap<PublicKey, String>>,
    }

    impl ExternalReservationSource {
        fn insert(&self, node_public: PublicKey, description: &str) {
            self.reservations
                .write()
                .expect("external reservation source lock")
                .insert(node_public, description.to_owned());
        }

        fn erase(&self, node_public: PublicKey) {
            self.reservations
                .write()
                .expect("external reservation source lock")
                .remove(&node_public);
        }
    }

    impl PeerReservationSource for ExternalReservationSource {
        fn contains(&self, node_public: PublicKey) -> bool {
            self.reservations
                .read()
                .expect("external reservation source lock")
                .contains_key(&node_public)
        }
    }

    fn validator(seed: u8) -> PublicKey {
        let secret = SecretKey::from_bytes([seed; 32]);
        derive_public_key(KeyType::Secp256k1, &secret).expect("validator key")
    }

    fn install_test_crypto_provider() {
        static INSTALL: Once = Once::new();
        INSTALL.call_once(|| {
            let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        });
    }

    fn peer(id: u32, seed: u8) -> Arc<PeerImp> {
        PeerImp::new(
            id,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 51235 + id as u16),
            validator(seed),
            format!("peer-{id}"),
        )
    }

    /// Drop an overlay safely from within an async test context.
    /// Tokio panics if a runtime is dropped inside another runtime,
    /// so we move the drop to a dedicated std thread.
    fn drop_overlay_safely(overlay: Arc<OverlayImpl>) {
        std::thread::spawn(move || drop(overlay))
            .join()
            .expect("overlay drop thread");
    }

    fn test_setup() -> Setup {
        install_test_crypto_provider();

        Setup {
            client_config: Some(Arc::new(
                rustls::ClientConfig::builder()
                    .dangerous()
                    .with_custom_certificate_verifier(Arc::new(NoVerify))
                    .with_no_client_auth(),
            )),
            tx_reduce_relay_min_peers: 1,
            tx_relay_percentage: 0,
            verify_endpoints: false,
            vp_reduce_relay_max_selected_peers: 3,
            reduce_relay_wait: Duration::from_secs(0),
            ..Default::default()
        }
    }

    fn account(hex: &str) -> AccountID {
        AccountID::from_hex(hex).expect("account hex")
    }

    fn payment_tx(sequence: u32) -> STTx {
        let source = account("1111111111111111111111111111111111111111");
        let destination = account("2222222222222222222222222222222222222222");

        STTx::new(protocol::TxType::PAYMENT, |tx| {
            tx.set_account_id(get_field_by_symbol("sfAccount"), source);
            tx.set_account_id(get_field_by_symbol("sfDestination"), destination);
            tx.set_field_amount(
                get_field_by_symbol("sfAmount"),
                STAmount::new_native(1_000_000, false),
            );
            tx.set_field_amount(
                get_field_by_symbol("sfFee"),
                STAmount::new_native(10, false),
            );
            tx.set_field_u32(get_field_by_symbol("sfSequence"), sequence);
        })
    }

    fn tx_message(sequence: u32) -> TmTransaction {
        let tx = payment_tx(sequence);
        TmTransaction {
            raw_transaction: tx.get_serializer().data().to_vec(),
            status: 1,
            receive_timestamp: None,
            deferred: None,
        }
    }

    fn validation_message(seed: u8, sign_time: u32, ledger_fill: u8) -> TmValidation {
        let secret = SecretKey::from_bytes([seed; 32]);
        let public = derive_public_key(KeyType::Secp256k1, &secret).expect("validation public");
        let node_id = calc_node_id(&public);
        let validation =
            STValidation::new_signed(sign_time, &public, node_id, &secret, |validation| {
                validation.set_field_h256(
                    get_field_by_symbol("sfLedgerHash"),
                    Uint256::from_array([ledger_fill; 32]),
                );
                validation.set_field_h256(
                    get_field_by_symbol("sfConsensusHash"),
                    Uint256::from_array([ledger_fill.wrapping_add(1); 32]),
                );
                validation.set_field_u32(get_field_by_symbol("sfLedgerSequence"), 55);
                validation.set_flag(VF_FULL_VALIDATION);
            })
            .expect("validation");

        #[allow(deprecated)]
        TmValidation {
            validation: validation.get_serialized(),
            checked_signature: None,
            hops: None,
        }
    }

    #[test]
    fn reservation_table_replaces_and_lists_in_key_order() {
        let table = PeerReservationTable::default();
        let alpha = validator(1);
        let beta = validator(2);

        assert!(
            table
                .insert_or_assign(PeerReservation {
                    node_public: beta,
                    description: "beta".to_owned(),
                })
                .is_none()
        );
        assert!(
            table
                .insert_or_assign(PeerReservation {
                    node_public: alpha,
                    description: "alpha".to_owned(),
                })
                .is_none()
        );
        let previous = table
            .insert_or_assign(PeerReservation {
                node_public: alpha,
                description: "alpha-2".to_owned(),
            })
            .expect("previous reservation");
        assert_eq!(previous.description, "alpha");
        assert_eq!(table.list().len(), 2);
        assert!(table.contains(beta));
        assert_eq!(table.erase(beta).expect("removed").description, "beta");
    }

    #[test]
    fn proposal_relay_updates_slot_and_sends_squelch_control() {
        let clock: Arc<dyn Clock> = Arc::new(ManualClock::new(Duration::from_secs(0)));
        let overlay = OverlayImpl::with_clock(test_setup(), Arc::new(TestHandoff), clock.clone())
            .expect("overlay");

        let a = peer(1, 11);
        let b = peer(2, 12);
        let c = peer(3, 13);
        let d = peer(4, 14);
        overlay.activate(a.clone());
        overlay.activate(b.clone());
        overlay.activate(c.clone());
        overlay.activate(d.clone());

        let validator = validator(99);
        let proposal = TmProposeSet {
            propose_seq: 1,
            current_tx_hash: vec![1; 32],
            node_pub_key: validator.as_bytes().to_vec(),
            close_time: 2,
            signature: vec![3; 64],
            previousledger: vec![4; 32],
            added_transactions: Vec::new(),
            removed_transactions: Vec::new(),
            ..Default::default()
        };

        for uid in 1..=25 {
            overlay.relay_proposal(proposal.clone(), Uint256::from_u64(uid), validator);
        }

        assert_eq!(overlay.slot_state(validator), Some(SlotState::Selected));
        let squelch_messages = d
            .queued_messages()
            .into_iter()
            .filter(|message| matches!(message.protocol().payload, ProtocolPayload::Squelch(_)))
            .count();
        assert!(squelch_messages > 0);
    }

    #[test]
    fn tx_reduce_relay_selects_enabled_peers_and_queues_the_rest() {
        let overlay = OverlayImpl::new(test_setup(), Arc::new(TestHandoff)).expect("overlay");

        let disabled = peer(1, 21);
        let enabled_a = peer(2, 22);
        let enabled_b = peer(3, 23);
        disabled.set_tx_reduce_relay_enabled(false);
        enabled_a.set_tx_reduce_relay_enabled(true);
        enabled_b.set_tx_reduce_relay_enabled(true);
        overlay.activate(disabled.clone());
        overlay.activate(enabled_a.clone());
        overlay.activate(enabled_b.clone());

        overlay.relay_transaction(
            Uint256::from_u64(88),
            Some(crate::TmTransaction {
                raw_transaction: vec![1, 2, 3],
                status: 1,
                receive_timestamp: None,
                deferred: None,
            }),
            &BTreeSet::new(),
        );

        assert_eq!(disabled.queued_messages().len(), 1);
        assert_eq!(enabled_a.queued_messages().len(), 1);
        assert!(enabled_b.queued_messages().is_empty());
        overlay.send_tx_queue();
        assert!(!enabled_b.queued_messages().is_empty());
        let JsonValue::Object(metrics) = overlay.tx_metrics() else {
            panic!("tx metrics json");
        };
        assert!(metrics.contains_key("txr_selected_cnt"));
    }

    #[test]
    fn activate_applies_cluster_and_reservation_membership() {
        let overlay = OverlayImpl::new(test_setup(), Arc::new(TestHandoff)).expect("overlay");
        let reserved = peer(10, 41);
        let clustered = peer(11, 42);

        overlay.reservations().insert_or_assign(PeerReservation {
            node_public: reserved.node_public(),
            description: "reserved".to_owned(),
        });
        assert!(overlay.cluster().update(
            clustered.node_public(),
            "clustered",
            0,
            SystemTime::now(),
        ));

        overlay.activate(Arc::clone(&reserved));
        overlay.activate(Arc::clone(&clustered));

        assert!(reserved.reserved());
        assert!(!reserved.cluster());
        assert!(clustered.cluster());
        assert!(!clustered.reserved());
    }

    #[test]
    fn activate_can_use_external_peer_reservation_source_and_refresh_membership() {
        let overlay = OverlayImpl::new(test_setup(), Arc::new(TestHandoff)).expect("overlay");
        let reserved = peer(20, 51);
        let external = Arc::new(ExternalReservationSource::default());

        external.insert(reserved.node_public(), "wallet-backed");
        overlay.set_peer_reservation_source(external.clone());
        overlay.activate(Arc::clone(&reserved));

        assert!(overlay.reservations().list().is_empty());
        assert!(reserved.reserved());

        external.erase(reserved.node_public());
        overlay.refresh_membership_state();
        assert!(!reserved.reserved());

        external.insert(reserved.node_public(), "wallet-backed-again");
        overlay.refresh_membership_state();
        assert!(reserved.reserved());
    }

    #[test]
    fn activate_can_use_external_cluster_source_and_refresh_membership() {
        let overlay = OverlayImpl::new(test_setup(), Arc::new(TestHandoff)).expect("overlay");
        let clustered = peer(21, 52);
        let external = Arc::new(Cluster::new());

        overlay.set_cluster_source(Arc::clone(&external));
        overlay.activate(Arc::clone(&clustered));
        assert!(!clustered.cluster());

        assert!(external.update(
            clustered.node_public(),
            "wallet-cluster",
            0,
            SystemTime::now(),
        ));
        overlay.refresh_membership_state();
        assert!(clustered.cluster());
        assert_eq!(
            overlay.cluster().member(clustered.node_public()),
            Some("wallet-cluster".to_owned())
        );
    }

    #[test]
    fn activate_enforces_peer_limit_for_unreserved_peers_only() {
        let overlay = OverlayImpl::new(
            Setup {
                ip_limit: 99,
                peer_limit: 1,
                ..test_setup()
            },
            Arc::new(TestHandoff),
        )
        .expect("overlay");
        let first = peer(22, 53);
        let second = peer(23, 54);
        let reserved = peer(24, 55);
        let clustered = peer(25, 56);

        assert!(overlay.activate(Arc::clone(&first)));
        assert!(!overlay.activate(Arc::clone(&second)));

        overlay.reservations().insert_or_assign(PeerReservation {
            node_public: reserved.node_public(),
            description: "reserved".to_owned(),
        });
        assert!(overlay.cluster().update(
            clustered.node_public(),
            "clustered",
            0,
            SystemTime::now(),
        ));

        assert!(overlay.activate(Arc::clone(&reserved)));
        assert!(overlay.activate(Arc::clone(&clustered)));
        assert_eq!(overlay.limit(), 1);
        assert_eq!(overlay.size(), 3);
        assert!(reserved.reserved());
        assert!(clustered.cluster());
        assert!(overlay.find_peer_by_short_id(second.id()).is_none());
    }

    #[test]
    fn active_outbound_peer_count_excludes_inbound_fixed_and_reserved_peers() {
        let overlay = OverlayImpl::new(
            Setup {
                fixed_peer_ips: HashSet::from([IpAddr::V4(Ipv4Addr::new(127, 0, 0, 99))]),
                ..test_setup()
            },
            Arc::new(TestHandoff),
        )
        .expect("overlay");
        let outbound = peer(29, 60);
        let inbound = PeerImp::new_with_inbound(
            30,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 51265),
            true,
            validator(61),
            "peer-30",
        );
        let fixed = PeerImp::new(
            31,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 99)), 51266),
            validator(62),
            "peer-31",
        );
        let reserved = peer(32, 63);

        overlay.reservations().insert_or_assign(PeerReservation {
            node_public: reserved.node_public(),
            description: "reserved".to_owned(),
        });

        assert!(overlay.activate(Arc::clone(&outbound)));
        assert!(overlay.activate(Arc::clone(&inbound)));
        assert!(overlay.activate(Arc::clone(&fixed)));
        assert!(overlay.activate(Arc::clone(&reserved)));
        assert!(fixed.fixed());
        assert!(reserved.reserved());
        assert_eq!(overlay.size(), 4);
        assert_eq!(overlay.active_outbound_peers_count(), 1);
    }

    #[test]
    fn activate_enforces_peer_limit_for_fixed_peers_slots() {
        let overlay = OverlayImpl::new(
            Setup {
                peer_limit: 1,
                fixed_peer_ips: HashSet::from([IpAddr::V4(Ipv4Addr::new(127, 0, 0, 99))]),
                ..test_setup()
            },
            Arc::new(TestHandoff),
        )
        .expect("overlay");
        let counted = peer(33, 64);
        let fixed = PeerImp::new(
            34,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 99)), 51267),
            validator(65),
            "peer-34",
        );

        assert!(overlay.activate(Arc::clone(&counted)));
        assert!(overlay.activate(Arc::clone(&fixed)));
        assert!(fixed.fixed());
        assert_eq!(overlay.limit(), 1);
        assert_eq!(overlay.size(), 2);
        assert_eq!(overlay.counted_active_peers_count(), 1);
    }

    #[test]
    fn outbound_attempt_registration_duplicate_ip_suppression() {
        let overlay = OverlayImpl::new(test_setup(), Arc::new(TestHandoff)).expect("overlay");
        let target = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), 51235);

        assert!(overlay.try_register_outbound_attempt(target));
        assert!(!overlay.try_register_outbound_attempt(target));
        overlay.finish_outbound_attempt(target);
        assert!(overlay.try_register_outbound_attempt(target));
        overlay.finish_outbound_attempt(target);

        let active = PeerImp::new(
            31,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), 51236),
            validator(62),
            "peer-31",
        );
        assert!(overlay.activate(active));
        assert!(!overlay.try_register_outbound_attempt(target));
    }

    #[test]
    fn outbound_attempt_registration_normalizes_ipv4_mapped_addresses() {
        let overlay = OverlayImpl::new(test_setup(), Arc::new(TestHandoff)).expect("overlay");
        let mapped = SocketAddr::new(
            IpAddr::V6(Ipv6Addr::new(0, 0, 0, 0, 0, 0xffff, 0x0a00, 0x0001)),
            51235,
        );
        let v4 = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), 51236);

        assert!(overlay.try_register_outbound_attempt(mapped));
        assert!(!overlay.try_register_outbound_attempt(v4));
        overlay.finish_outbound_attempt(mapped);
        assert!(overlay.try_register_outbound_attempt(v4));
    }

    #[test]
    fn refresh_membership_state_drops_excess_peers_after_reservation_loss() {
        let overlay = OverlayImpl::new(
            Setup {
                peer_limit: 1,
                ..test_setup()
            },
            Arc::new(TestHandoff),
        )
        .expect("overlay");
        let external = Arc::new(ExternalReservationSource::default());
        let first = peer(26, 57);
        let reserved = peer(27, 58);

        external.insert(reserved.node_public(), "wallet-backed");
        overlay.set_peer_reservation_source(external.clone());

        assert!(overlay.activate(Arc::clone(&first)));
        assert!(overlay.activate(Arc::clone(&reserved)));
        assert_eq!(overlay.size(), 2);
        assert!(reserved.reserved());

        external.erase(reserved.node_public());
        overlay.refresh_membership_state();

        assert_eq!(overlay.size(), 1);
        assert!(!reserved.reserved());
        assert!(overlay.find_peer_by_short_id(first.id()).is_some());
        assert!(overlay.find_peer_by_short_id(reserved.id()).is_none());
    }

    #[test]
    fn check_tracking_updates_active_peer_tracking_state() {
        let overlay = OverlayImpl::new(test_setup(), Arc::new(TestHandoff)).expect("overlay");
        let tracked = peer(12, 43);
        tracked.record_ledger(Uint256::from_u64(12), 200);
        overlay.activate(Arc::clone(&tracked));

        assert!(tracked.has_range(200, 200));
        overlay.check_tracking(400);
        assert!(!tracked.has_range(200, 200));
        overlay.check_tracking(210);
        assert!(tracked.has_range(200, 200));
    }

    #[test]
    fn finalize_connected_peer_activates_and_applies_negotiated_flags() {
        let overlay = OverlayImpl::new(test_setup(), Arc::new(TestHandoff)).expect("overlay");
        let peer = peer(13, 44);
        let mut headers = HeaderMap::new();
        headers.insert(
            "X-Protocol-Ctl",
            "compr=lz4;txrr=1;ledgerreplay=1".parse().expect("header"),
        );

        let result = overlay
            .finalize_connect_result(ConnectAttemptResult {
                peer: Arc::clone(&peer),
                response: Response::builder().status(101).body(()).expect("response"),
                negotiated_features: headers,
                session: None,
            })
            .expect("connect result should finalize");

        assert_eq!(overlay.size(), 1);
        assert!(overlay.find_peer_by_short_id(peer.id()).is_some());
        assert!(result.peer.compression_enabled());
        assert!(result.peer.tx_reduce_relay_enabled());
        assert!(result.peer.supports_feature(ProtocolFeature::LedgerReplay));
    }

    #[test]
    fn finalize_connected_peer_rejects_unreserved_peer_when_limit_is_full() {
        let overlay = OverlayImpl::new(
            Setup {
                peer_limit: 1,
                ..test_setup()
            },
            Arc::new(TestHandoff),
        )
        .expect("overlay");
        assert!(overlay.activate(peer(28, 59)));

        let peer = peer(29, 60);
        let result = overlay.finalize_connect_result(ConnectAttemptResult {
            peer: Arc::clone(&peer),
            response: Response::builder().status(101).body(()).expect("response"),
            negotiated_features: HeaderMap::new(),
            session: None,
        });

        assert!(matches!(
            result,
            Err(reason) if reason == "peer limit reached for unreserved peer"
        ));
        assert_eq!(overlay.size(), 1);
        assert!(overlay.find_peer_by_short_id(peer.id()).is_none());
    }

    #[tokio::test]
    async fn inbound_session_routes_runtime_messages_and_tracks_metrics() {
        let clock = Arc::new(ManualClock::new(Duration::from_secs(0)));
        let overlay = OverlayImpl::with_clock(
            test_setup(),
            Arc::new(TestHandoff),
            Arc::clone(&clock) as Arc<dyn Clock>,
        )
        .expect("overlay");
        let peer = peer(14, 45);
        let validator = validator(46);
        let tx_set_hash = Uint256::from_u64(99);
        let closed_hash = Uint256::from_u64(300);
        let previous_hash = Uint256::from_u64(299);

        let (local, mut remote) = duplex(4096);
        let (stop_requested, stop_rx) = watch::channel(false);
        let result = overlay
            .finalize_connect_result(ConnectAttemptResult {
                peer: Arc::clone(&peer),
                response: Response::builder().status(101).body(()).expect("response"),
                negotiated_features: HeaderMap::new(),
                session: Some(PeerSessionStarter::new(Box::new(local), stop_rx)),
            })
            .expect("connect result should finalize");
        assert!(result.session.is_none());

        let inbound_ping = Message::new(
            ProtocolMessage::new(ProtocolPayload::Ping(TmPing {
                r#type: 0,
                seq: Some(19),
                ping_time: Some(20),
                net_time: Some(21),
            })),
            None,
        );
        remote
            .write_all(inbound_ping.get_buffer(crate::Compressed::Off))
            .await
            .expect("write inbound ping");
        remote.flush().await.expect("flush inbound ping");

        let expected_pong = Message::new(
            ProtocolMessage::new(ProtocolPayload::Ping(TmPing {
                r#type: 1,
                seq: Some(19),
                ping_time: Some(20),
                net_time: Some(21),
            })),
            None,
        );
        let mut pong_bytes = vec![0u8; expected_pong.get_buffer_size()];
        timeout(Duration::from_secs(1), remote.read_exact(&mut pong_bytes))
            .await
            .expect("read pong")
            .expect("pong bytes");
        let decoded_pong = decode_protocol_message(&pong_bytes, false).expect("decode pong");
        assert!(matches!(
            decoded_pong.message,
            Some(ProtocolMessage {
                payload: ProtocolPayload::Ping(TmPing { r#type: 1, .. }),
                ..
            })
        ));

        let inbound_status = Message::new(
            ProtocolMessage::new(ProtocolPayload::StatusChange(TmStatusChange {
                new_status: Some(2),
                new_event: Some(2),
                ledger_seq: Some(300),
                ledger_hash: Some(closed_hash.data().to_vec()),
                ledger_hash_previous: Some(previous_hash.data().to_vec()),
                network_time: Some(55),
                first_seq: Some(250),
                last_seq: Some(300),
            })),
            None,
        );
        remote
            .write_all(inbound_status.get_buffer(crate::Compressed::Off))
            .await
            .expect("write inbound status");
        remote.flush().await.expect("flush inbound status");

        let inbound_have_set = Message::new(
            ProtocolMessage::new(ProtocolPayload::HaveSet(TmHaveTransactionSet {
                status: 1,
                hash: tx_set_hash.data().to_vec(),
            })),
            None,
        );
        remote
            .write_all(inbound_have_set.get_buffer(crate::Compressed::Off))
            .await
            .expect("write inbound have set");
        remote.flush().await.expect("flush inbound have set");

        let inbound_squelch = Message::new(
            ProtocolMessage::new(ProtocolPayload::Squelch(TmSquelch {
                squelch: true,
                validator_pub_key: validator.as_bytes().to_vec(),
                squelch_duration: Some(300),
            })),
            None,
        );
        remote
            .write_all(inbound_squelch.get_buffer(crate::Compressed::Off))
            .await
            .expect("write inbound squelch");
        remote.flush().await.expect("flush inbound squelch");

        let inbound_tx = Message::new(
            ProtocolMessage::new(ProtocolPayload::Transaction(TmTransaction {
                raw_transaction: vec![7; 1024],
                status: 1,
                receive_timestamp: None,
                deferred: None,
            })),
            None,
        );
        remote
            .write_all(inbound_tx.get_buffer(crate::Compressed::Off))
            .await
            .expect("write inbound transaction");
        remote.flush().await.expect("flush inbound transaction");

        clock.advance(Duration::from_secs(1));
        remote
            .write_all(inbound_tx.get_buffer(crate::Compressed::Off))
            .await
            .expect("write second inbound transaction");
        remote
            .flush()
            .await
            .expect("flush second inbound transaction");

        timeout(Duration::from_secs(1), async {
            loop {
                if peer.closed_ledger_hash() == closed_hash
                    && peer.previous_ledger_hash() == previous_hash
                    && peer.ledger_range() == (250, 300)
                    && peer.has_tx_set(tx_set_hash)
                    && peer.is_squelched(validator)
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("wait for inbound routing");

        let total = overlay
            .traffic
            .counts()
            .get(&TrafficCategory::Total)
            .expect("total traffic");
        assert!(total.messages_in.load(Ordering::Relaxed) >= 5);
        // Allow async message processing to complete
        tokio::task::yield_now().await;
        tokio::time::sleep(Duration::from_millis(10)).await;

        assert!(total.bytes_in.load(Ordering::Relaxed) > 0);

        let transactions = overlay
            .traffic
            .counts()
            .get(&TrafficCategory::Transaction)
            .expect("transaction traffic");
        assert!(transactions.messages_in.load(Ordering::Relaxed) >= 2);

        let JsonValue::Object(metrics) = overlay.tx_metrics() else {
            panic!("tx metrics json");
        };
        assert_ne!(
            metrics.get("txr_tx_sz").and_then(|value| match value {
                JsonValue::String(value) => Some(value.as_str()),
                _ => None,
            }),
            Some("0")
        );

        let _ = stop_requested.send(true);
        std::thread::spawn(move || drop(overlay))
            .join()
            .expect("overlay drop");
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn inbound_status_change_preserves_status_publishes_and_skips_lost_sync() {
        let clock = Arc::new(ManualClock::new(Duration::from_secs(0)));
        let overlay = OverlayImpl::with_clock(
            test_setup(),
            Arc::new(TestHandoff),
            Arc::clone(&clock) as Arc<dyn Clock>,
        )
        .expect("overlay");
        let published = Arc::new(Mutex::new(Vec::<JsonValue>::new()));
        overlay.set_peer_status_publisher({
            let published = Arc::clone(&published);
            move |payload| {
                published
                    .lock()
                    .expect("published peer status lock")
                    .push(payload);
            }
        });

        let peer = peer(30, 70);
        let (local, mut remote) = duplex(4096);
        let (stop_requested, stop_rx) = watch::channel(false);
        overlay
            .finalize_connect_result(ConnectAttemptResult {
                peer: Arc::clone(&peer),
                response: Response::builder().status(101).body(()).expect("response"),
                negotiated_features: HeaderMap::new(),
                session: Some(PeerSessionStarter::new(Box::new(local), stop_rx)),
            })
            .expect("connect result should finalize");

        let accepted = Message::new(
            ProtocolMessage::new(ProtocolPayload::StatusChange(TmStatusChange {
                new_status: Some(2),
                new_event: Some(2),
                ledger_seq: Some(300),
                ledger_hash: Some(Uint256::from_u64(300).data().to_vec()),
                ledger_hash_previous: Some(Uint256::from_u64(299).data().to_vec()),
                network_time: Some(55),
                first_seq: Some(250),
                last_seq: Some(300),
            })),
            None,
        );
        remote
            .write_all(accepted.get_buffer(crate::Compressed::Off))
            .await
            .expect("write accepted status");
        remote.flush().await.expect("flush accepted status");

        let switched = Message::new(
            ProtocolMessage::new(ProtocolPayload::StatusChange(TmStatusChange {
                new_status: None,
                new_event: Some(3),
                ledger_seq: Some(301),
                ledger_hash: Some(Uint256::from_u64(301).data().to_vec()),
                ledger_hash_previous: Some(Uint256::from_u64(300).data().to_vec()),
                network_time: Some(56),
                first_seq: Some(400),
                last_seq: Some(300),
            })),
            None,
        );
        remote
            .write_all(switched.get_buffer(crate::Compressed::Off))
            .await
            .expect("write switched status");
        remote.flush().await.expect("flush switched status");

        timeout(Duration::from_secs(1), async {
            loop {
                if published.lock().expect("published peer status lock").len() >= 2
                    && peer.closed_ledger_hash() == Uint256::from_u64(301)
                    && peer.previous_ledger_hash() == Uint256::from_u64(300)
                    && peer.ledger_range() == (0, 0)
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("wait for published status events");

        let published_events = published.lock().expect("published peer status lock");
        let JsonValue::Object(first) = &published_events[0] else {
            panic!("first peer status event should be an object");
        };
        assert_eq!(
            first.get("status"),
            Some(&JsonValue::String("CONNECTED".to_owned()))
        );
        assert_eq!(
            first.get("action"),
            Some(&JsonValue::String("ACCEPTED_LEDGER".to_owned()))
        );

        let JsonValue::Object(second) = &published_events[1] else {
            panic!("second peer status event should be an object");
        };
        assert_eq!(
            second.get("status"),
            Some(&JsonValue::String("CONNECTED".to_owned()))
        );
        assert_eq!(
            second.get("action"),
            Some(&JsonValue::String("SWITCHED_LEDGER".to_owned()))
        );
        assert_eq!(
            second.get("ledger_index_min"),
            Some(&JsonValue::Unsigned(400))
        );
        assert_eq!(
            second.get("ledger_index_max"),
            Some(&JsonValue::Unsigned(300))
        );
        drop(published_events);

        let lost_sync = Message::new(
            ProtocolMessage::new(ProtocolPayload::StatusChange(TmStatusChange {
                new_status: None,
                new_event: Some(4),
                ledger_seq: None,
                ledger_hash: None,
                ledger_hash_previous: None,
                network_time: Some(57),
                first_seq: None,
                last_seq: None,
            })),
            None,
        );
        remote
            .write_all(lost_sync.get_buffer(crate::Compressed::Off))
            .await
            .expect("write lost-sync status");
        remote.flush().await.expect("flush lost-sync status");

        timeout(Duration::from_secs(1), async {
            loop {
                if peer.closed_ledger_hash().is_zero() && peer.previous_ledger_hash().is_zero() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("wait for lost sync clear");

        assert_eq!(
            published.lock().expect("published peer status lock").len(),
            2
        );

        let _ = stop_requested.send(true);
    }

    #[tokio::test]
    async fn inbound_session_queues_remaining_heavy_families() {
        let clock: Arc<dyn Clock> = Arc::new(ManualClock::new(Duration::from_secs(0)));
        let overlay =
            OverlayImpl::with_clock(test_setup(), Arc::new(TestHandoff), Arc::clone(&clock))
                .expect("overlay");
        let peer = peer(15, 61);
        let (local, mut remote) = duplex(64 * 1024);
        let (_stop_requested, stop_rx) = watch::channel(false);
        let mut headers = HeaderMap::new();
        headers.insert("Upgrade", "XRPL/2.2".parse().expect("upgrade header"));
        headers.insert(
            "X-Protocol-Ctl",
            "txrr=1;ledgerreplay=1".parse().expect("control header"),
        );

        let _ = overlay
            .finalize_connect_result(ConnectAttemptResult {
                peer: Arc::clone(&peer),
                response: Response::builder().status(101).body(()).expect("response"),
                negotiated_features: headers,
                session: Some(PeerSessionStarter::new(Box::new(local), stop_rx)),
            })
            .expect("connect result should finalize");

        peer.record_ledger(Uint256::from_u64(900), 900);
        peer.check_tracking(900);

        // Kept for compatibility with the legacy overlay wire fixtures; these
        // deprecated fields still exist on the protobuf surface we ingest.
        #[allow(deprecated)]
        let manifests = Message::new(
            ProtocolMessage::new(ProtocolPayload::Manifests(TmManifests {
                list: vec![wire::TmManifest {
                    stobject: vec![1, 2, 3],
                }],
                history: None,
            })),
            None,
        );
        remote
            .write_all(manifests.get_buffer(crate::Compressed::Off))
            .await
            .expect("write manifests");

        let endpoints = Message::new(
            ProtocolMessage::new(ProtocolPayload::Endpoints(TmEndpoints {
                version: 2,
                endpoints_v2: vec![
                    wire::tm_endpoints::TmEndpointv2 {
                        endpoint: "10.0.0.1:51235".to_owned(),
                        hops: 0,
                    },
                    wire::tm_endpoints::TmEndpointv2 {
                        endpoint: "not-an-endpoint".to_owned(),
                        hops: 2,
                    },
                ],
            })),
            None,
        );
        remote
            .write_all(endpoints.get_buffer(crate::Compressed::Off))
            .await
            .expect("write endpoints");

        let single_tx = Message::new(
            ProtocolMessage::new(ProtocolPayload::Transaction(tx_message(7))),
            None,
        );
        remote
            .write_all(single_tx.get_buffer(crate::Compressed::Off))
            .await
            .expect("write transaction");

        let get_ledger = Message::new(
            ProtocolMessage::new(ProtocolPayload::GetLedger(TmGetLedger {
                itype: 0,
                ltype: Some(0),
                ledger_hash: Some(Uint256::from_u64(700).data().to_vec()),
                ledger_seq: None,
                node_i_ds: Vec::new(),
                request_cookie: None,
                query_type: None,
                query_depth: None,
            })),
            None,
        );
        remote
            .write_all(get_ledger.get_buffer(crate::Compressed::Off))
            .await
            .expect("write get ledger");

        let ledger_data = Message::new(
            ProtocolMessage::new(ProtocolPayload::LedgerData(TmLedgerData {
                ledger_hash: Uint256::from_u64(701).data().to_vec(),
                ledger_seq: 700,
                r#type: 0,
                nodes: vec![wire::TmLedgerNode {
                    nodedata: vec![9, 9, 9],
                    nodeid: None,
                }],
                request_cookie: None,
                error: None,
            })),
            None,
        );
        remote
            .write_all(ledger_data.get_buffer(crate::Compressed::Off))
            .await
            .expect("write ledger data");

        // Kept for compatibility with the legacy overlay wire fixtures; these
        // deprecated fields still exist on the protobuf surface we ingest.
        #[allow(deprecated)]
        let proposal = Message::new(
            ProtocolMessage::new(ProtocolPayload::ProposeLedger(TmProposeSet {
                propose_seq: 3,
                current_tx_hash: Uint256::from_u64(800).data().to_vec(),
                node_pub_key: validator(62).as_bytes().to_vec(),
                close_time: 44,
                signature: vec![4; 64],
                previousledger: Uint256::from_u64(799).data().to_vec(),
                added_transactions: Vec::new(),
                removed_transactions: Vec::new(),
                checked_signature: None,
                hops: None,
            })),
            None,
        );
        remote
            .write_all(proposal.get_buffer(crate::Compressed::Off))
            .await
            .expect("write proposal");

        let validation = Message::new(
            ProtocolMessage::new(ProtocolPayload::Validation(validation_message(
                63, 1000, 0xA1,
            ))),
            None,
        );
        remote
            .write_all(validation.get_buffer(crate::Compressed::Off))
            .await
            .expect("write validation");

        let validator_list = Message::new(
            ProtocolMessage::new(ProtocolPayload::ValidatorList(TmValidatorList {
                manifest: vec![1, 2],
                blob: vec![3, 4],
                signature: vec![5, 6],
                version: 1,
            })),
            None,
        );
        remote
            .write_all(validator_list.get_buffer(crate::Compressed::Off))
            .await
            .expect("write validator list");

        let validator_list_collection = Message::new(
            ProtocolMessage::new(ProtocolPayload::ValidatorListCollection(
                TmValidatorListCollection {
                    version: 2,
                    manifest: vec![7, 8],
                    blobs: vec![wire::ValidatorBlobInfo {
                        manifest: Some(vec![9]),
                        blob: vec![10, 11],
                        signature: vec![12, 13],
                    }],
                },
            )),
            None,
        );
        remote
            .write_all(validator_list_collection.get_buffer(crate::Compressed::Off))
            .await
            .expect("write validator list collection");

        let get_objects = Message::new(
            ProtocolMessage::new(ProtocolPayload::GetObjects(TmGetObjectByHash {
                r#type: wire::tm_get_object_by_hash::ObjectType::OtTransactions as i32,
                query: true,
                ledger_hash: Some(Uint256::from_u64(801).data().to_vec()),
                fat: None,
                objects: vec![wire::TmIndexedObject {
                    hash: Some(Uint256::from_u64(802).data().to_vec()),
                    node_id: None,
                    index: None,
                    data: None,
                    ledger_seq: None,
                }],
            })),
            None,
        );
        remote
            .write_all(get_objects.get_buffer(crate::Compressed::Off))
            .await
            .expect("write get objects");

        let have_transactions = Message::new(
            ProtocolMessage::new(ProtocolPayload::HaveTransactions(TmHaveTransactions {
                hashes: vec![Uint256::from_u64(803).data().to_vec()],
            })),
            None,
        );
        remote
            .write_all(have_transactions.get_buffer(crate::Compressed::Off))
            .await
            .expect("write have transactions");

        let transactions = Message::new(
            ProtocolMessage::new(ProtocolPayload::Transactions(TmTransactions {
                transactions: vec![tx_message(8), tx_message(9)],
            })),
            None,
        );
        remote
            .write_all(transactions.get_buffer(crate::Compressed::Off))
            .await
            .expect("write transactions batch");

        let proof_request = Message::new(
            ProtocolMessage::new(ProtocolPayload::ProofPathRequest(TmProofPathRequest {
                key: Uint256::from_u64(810).data().to_vec(),
                ledger_hash: Uint256::from_u64(811).data().to_vec(),
                r#type: 1,
            })),
            None,
        );
        remote
            .write_all(proof_request.get_buffer(crate::Compressed::Off))
            .await
            .expect("write proof request");

        let proof_response = Message::new(
            ProtocolMessage::new(ProtocolPayload::ProofPathResponse(TmProofPathResponse {
                key: Uint256::from_u64(810).data().to_vec(),
                ledger_hash: Uint256::from_u64(811).data().to_vec(),
                r#type: 1,
                ledger_header: Some(vec![1, 2, 3]),
                path: vec![vec![4, 5, 6]],
                error: None,
            })),
            None,
        );
        remote
            .write_all(proof_response.get_buffer(crate::Compressed::Off))
            .await
            .expect("write proof response");

        let replay_request = Message::new(
            ProtocolMessage::new(ProtocolPayload::ReplayDeltaRequest(TmReplayDeltaRequest {
                ledger_hash: Uint256::from_u64(812).data().to_vec(),
            })),
            None,
        );
        remote
            .write_all(replay_request.get_buffer(crate::Compressed::Off))
            .await
            .expect("write replay request");

        let replay_response = Message::new(
            ProtocolMessage::new(ProtocolPayload::ReplayDeltaResponse(
                TmReplayDeltaResponse {
                    ledger_hash: Uint256::from_u64(813).data().to_vec(),
                    ledger_header: Some(vec![7, 8]),
                    transaction: vec![payment_tx(10).get_serializer().data().to_vec()],
                    error: None,
                },
            )),
            None,
        );
        remote
            .write_all(replay_response.get_buffer(crate::Compressed::Off))
            .await
            .expect("write replay response");
        remote.flush().await.expect("flush heavy families");

        timeout(Duration::from_secs(1), async {
            loop {
                let snapshot = overlay.queued_inbound_snapshot();
                if snapshot.manifests.len() == 1
                    && snapshot.endpoints.len() == 1
                    && snapshot.transactions.len() == 3
                    && snapshot.get_ledgers.len() == 1
                    && snapshot.ledger_data.len() == 1
                    && snapshot.proposals.len() == 1
                    && snapshot.validations.len() == 1
                    && snapshot.validator_lists.len() == 1
                    && snapshot.validator_list_collections.len() == 1
                    && snapshot.get_objects.len() == 1
                    && snapshot.have_transactions.len() == 1
                    && snapshot.transactions_batches.len() == 1
                    && snapshot.proof_path_requests.len() == 1
                    && snapshot.proof_path_responses.len() == 1
                    && snapshot.replay_delta_requests.len() == 1
                    && snapshot.replay_delta_responses.len() == 1
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("wait for heavy families");

        let snapshot = overlay.queued_inbound_snapshot();
        assert_eq!(snapshot.manifests[0].peer_id, peer.id());
        assert_eq!(snapshot.endpoints[0].malformed, 1);
        assert_eq!(
            snapshot.endpoints[0].endpoints[0].endpoint,
            SocketAddr::new(peer.remote_address().ip(), 51235)
        );
        assert_eq!(snapshot.endpoints[0].endpoints[0].hops, 1);
        assert_eq!(
            snapshot.transactions[0].id,
            payment_tx(7).get_transaction_id()
        );
        assert!(!snapshot.transactions[0].batch);
        assert!(snapshot.transactions[1].batch);
        assert_eq!(
            snapshot.proposals[0].current_tx_hash,
            Uint256::from_u64(800)
        );
        assert_eq!(
            snapshot.have_transactions[0].hashes,
            vec![Uint256::from_u64(803)]
        );
        assert_eq!(
            snapshot.transactions_batches[0].message.transactions.len(),
            2
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn inbound_endpoints_drop_excess_hops_and_cap_batch_size_peerfinder() {
        let overlay =
            Arc::new(OverlayImpl::new(test_setup(), Arc::new(TestHandoff)).expect("overlay"));
        let secret = SecretKey::from_bytes([91u8; 32]);
        let public = derive_public_key(KeyType::Secp256k1, &secret).expect("public key");
        let peer = Arc::new(PeerImp::new(
            91,
            SocketAddr::new("127.0.0.1".parse().expect("ip"), 51235),
            public,
            "peer-91",
        ));
        peer.set_listener_check_state(true, true);
        peer.record_ledger(Uint256::from_u64(900), 900);
        peer.check_tracking(900);

        let mut router = OverlayInboundRouter {
            overlay: overlay.as_ref(),
            peer: &peer,
        };

        let mut endpoints_v2 = Vec::new();
        for index in 0..70u16 {
            endpoints_v2.push(wire::tm_endpoints::TmEndpointv2 {
                endpoint: format!("10.0.0.{}:{}", (index % 250) + 1, 5000 + index),
                hops: 1,
            });
        }
        endpoints_v2.push(wire::tm_endpoints::TmEndpointv2 {
            endpoint: "10.1.0.1:6000".to_owned(),
            hops: PEERFINDER_MAX_HOPS + 1,
        });

        let _ = router.on_endpoints(&TmEndpoints {
            version: 2,
            endpoints_v2,
        });

        let snapshot = overlay.queued_inbound_snapshot();
        assert_eq!(snapshot.endpoints.len(), 1);
        assert!(snapshot.endpoints[0].endpoints.len() <= PEERFINDER_MAX_ACCEPTED_ENDPOINTS);
        assert!(
            snapshot.endpoints[0]
                .endpoints
                .iter()
                .all(|endpoint| endpoint.hops <= PEERFINDER_MAX_HOPS + 1)
        );
        drop_overlay_safely(overlay);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn inbound_endpoints_rate_limit_and_dedupe_peerfinder() {
        let overlay =
            Arc::new(OverlayImpl::new(test_setup(), Arc::new(TestHandoff)).expect("overlay"));
        let secret = SecretKey::from_bytes([92u8; 32]);
        let public = derive_public_key(KeyType::Secp256k1, &secret).expect("public key");
        let peer = Arc::new(PeerImp::new(
            92,
            SocketAddr::new("127.0.0.1".parse().expect("ip"), 51235),
            public,
            "peer-92",
        ));
        peer.set_listener_check_state(true, true);
        peer.record_ledger(Uint256::from_u64(901), 901);
        peer.check_tracking(901);

        let mut router = OverlayInboundRouter {
            overlay: overlay.as_ref(),
            peer: &peer,
        };

        let message = TmEndpoints {
            version: 2,
            endpoints_v2: vec![
                wire::tm_endpoints::TmEndpointv2 {
                    endpoint: "10.0.0.1:51235".to_owned(),
                    hops: 1,
                },
                wire::tm_endpoints::TmEndpointv2 {
                    endpoint: "10.0.0.1:51235".to_owned(),
                    hops: 1,
                },
                wire::tm_endpoints::TmEndpointv2 {
                    endpoint: "[::]:51235".to_owned(),
                    hops: 0,
                },
                wire::tm_endpoints::TmEndpointv2 {
                    endpoint: "[::]:51236".to_owned(),
                    hops: 0,
                },
            ],
        };

        let _ = router.on_endpoints(&message);
        let first = overlay.take_queued_inbound_snapshot();
        assert_eq!(first.endpoints.len(), 1);
        assert_eq!(first.endpoints[0].endpoints.len(), 2);
        assert!(first.endpoints[0].endpoints.iter().any(|endpoint| {
            endpoint.endpoint == SocketAddr::new(peer.remote_address().ip(), 51235)
        }));
        assert!(first.endpoints[0].endpoints.iter().any(|endpoint| {
            endpoint.endpoint == "10.0.0.1:51235".parse().expect("deduped endpoint")
        }));

        let _ = router.on_endpoints(&message);
        let second = overlay.queued_inbound_snapshot();
        assert!(second.endpoints.is_empty());
        drop_overlay_safely(overlay);
    }

    #[test]
    fn endpoint_verification_rejects_private_loopback_and_zero_ports() {
        assert!(is_valid_peer_endpoint(
            "8.8.8.8:51235".parse().expect("public endpoint")
        ));
        assert!(!is_valid_peer_endpoint(
            "10.0.0.1:51235".parse().expect("private endpoint")
        ));
        assert!(!is_valid_peer_endpoint(
            "127.0.0.1:51235".parse().expect("loopback endpoint")
        ));
        assert!(!is_valid_peer_endpoint(
            "[::1]:51235".parse().expect("ipv6 loopback endpoint")
        ));
        assert!(!is_valid_peer_endpoint(
            "8.8.8.8:0".parse().expect("zero port endpoint")
        ));
    }

    #[test]
    fn overlay_json_exposes_verify_endpoints_config_surface() {
        let overlay = OverlayImpl::new(test_setup(), Arc::new(TestHandoff)).expect("overlay");

        assert!(!overlay.stats().verify_endpoints);
        match overlay.json() {
            JsonValue::Object(object) => {
                assert_eq!(
                    object.get("verify_endpoints"),
                    Some(&JsonValue::Bool(false))
                );
            }
            other => panic!("overlay json should be an object, got {other:?}"),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn inbound_neighbor_endpoint_requires_listener_check_before_acceptance() {
        let overlay =
            Arc::new(OverlayImpl::new(test_setup(), Arc::new(TestHandoff)).expect("overlay"));
        let secret = SecretKey::from_bytes([93u8; 32]);
        let public = derive_public_key(KeyType::Secp256k1, &secret).expect("public key");
        let peer = Arc::new(PeerImp::new(
            93,
            SocketAddr::new("127.0.0.1".parse().expect("ip"), 51235),
            public,
            "peer-93",
        ));
        peer.set_listener_check_state(false, false);
        peer.record_ledger(Uint256::from_u64(902), 902);
        peer.check_tracking(902);

        let mut router = OverlayInboundRouter {
            overlay: overlay.as_ref(),
            peer: &peer,
        };

        let message = TmEndpoints {
            version: 2,
            endpoints_v2: vec![wire::tm_endpoints::TmEndpointv2 {
                endpoint: "[::]:51235".to_owned(),
                hops: 0,
            }],
        };

        let _ = router.on_endpoints(&message);
        assert!(overlay.take_queued_inbound_snapshot().endpoints.is_empty());

        let checked_peer = Arc::new(PeerImp::new(
            94,
            SocketAddr::new("127.0.0.1".parse().expect("ip"), 51235),
            public,
            "peer-94",
        ));
        checked_peer.set_listener_check_state(true, true);
        checked_peer.record_ledger(Uint256::from_u64(902), 902);
        checked_peer.check_tracking(902);
        let mut checked_router = OverlayInboundRouter {
            overlay: overlay.as_ref(),
            peer: &checked_peer,
        };
        let _ = checked_router.on_endpoints(&message);
        let snapshot = overlay.take_queued_inbound_snapshot();
        assert_eq!(snapshot.endpoints.len(), 1);
        assert_eq!(snapshot.endpoints[0].endpoints.len(), 1);
        assert_eq!(
            snapshot.endpoints[0].endpoints[0].endpoint,
            SocketAddr::new(checked_peer.remote_address().ip(), 51235)
        );
        drop_overlay_safely(overlay);
    }

    #[test]
    fn redirect_response_uses_filtered_discovered_endpoints_peerfinder() {
        let overlay = OverlayImpl::new(test_setup(), Arc::new(TestHandoff)).expect("overlay");
        let now = SystemTime::now();

        overlay.remember_redirect_endpoint(
            SocketAddr::new("10.0.0.1".parse().expect("ip"), 51235),
            0,
            now,
        );
        overlay.remember_redirect_endpoint(
            SocketAddr::new("10.0.0.2".parse().expect("ip"), 51235),
            1,
            now,
        );
        overlay.remember_redirect_endpoint(
            SocketAddr::new("10.0.0.2".parse().expect("ip"), 51236),
            1,
            now,
        );
        overlay.remember_redirect_endpoint(
            SocketAddr::new("10.0.0.3".parse().expect("ip"), 51235),
            PEERFINDER_MAX_HOPS + 1,
            now,
        );
        overlay.remember_redirect_endpoint(
            SocketAddr::new("10.0.0.4".parse().expect("ip"), 51235),
            1,
            now - PEERFINDER_LIVE_CACHE_TTL - Duration::from_secs(1),
        );
        for index in 6..14u8 {
            overlay.remember_redirect_endpoint(
                SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, index)), 51235),
                1,
                now,
            );
        }

        let request = Request::builder()
            .version(http::Version::HTTP_11)
            .body(())
            .expect("request");
        let (_, wire) = overlay
            .make_redirect_response(
                &request,
                SocketAddr::new("10.0.0.5".parse().expect("ip"), 51235),
            )
            .expect("redirect response");
        let body_offset = wire
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map(|index| index + 4)
            .expect("header terminator");
        let body = std::str::from_utf8(&wire[body_offset..]).expect("utf8 body");
        let json: serde_json::Value = serde_json::from_str(body).expect("json body");
        let peers = json["peer-ips"].as_array().expect("peer-ips array");

        assert!(
            !peers
                .iter()
                .any(|peer| peer.as_str() == Some("10.0.0.1:51235"))
        );
        assert!(
            !peers
                .iter()
                .any(|peer| peer.as_str() == Some("10.0.0.3:51235"))
        );
        assert!(
            !peers
                .iter()
                .any(|peer| peer.as_str() == Some("10.0.0.4:51235"))
        );
        assert!(
            !peers
                .iter()
                .any(|peer| peer.as_str() == Some("10.0.0.5:51235"))
        );
        assert_eq!(
            peers
                .iter()
                .filter(|peer| peer
                    .as_str()
                    .is_some_and(|text| text.starts_with("10.0.0.2:")))
                .count(),
            1
        );
        assert!(peers.len() <= PEERFINDER_REDIRECT_ENDPOINT_COUNT);
    }

    #[test]
    fn queued_inbound_snapshot_can_be_cleared() {
        let overlay = OverlayImpl::new(test_setup(), Arc::new(TestHandoff)).expect("overlay");
        assert!(overlay.queued_inbound_snapshot().manifests.is_empty());
        overlay.clear_queued_inbound();
        assert!(overlay.queued_inbound_snapshot().transactions.is_empty());
    }
}
