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
    JsonValue, KeyType, PublicKey, STTx, SecretKey, SerialIter, Serializer, derive_public_key,
    sha512_half as protocol_sha512_half, sign_digest,
};
use rand::seq::SliceRandom;
use resource::{Consumer, Disposition, NullCollector, NullJournal, ResourceManager, make_manager};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::timeout;
use xrpl_core::PeerReservationTable as CorePeerReservationTable;

use crate::cluster::Cluster;
use crate::connect_attempt::{
    ConnectAttempt, ConnectAttemptConfig, ConnectAttemptError, ConnectAttemptResult, ConnectionStep,
};
use crate::handshake::{
    FEATURE_COMPR, FEATURE_LEDGER_REPLAY, FEATURE_TXRR, HandshakeContext,
    HandshakeVerificationContext, feature_enabled, is_feature_value, make_response,
    negotiate_inbound_peer_upgrade, parse_http_request, serialize_response, verify_handshake,
};
use crate::inbound::{
    OverlayInboundHandler, OverlayInboundSnapshot, QueuedEndpoint, QueuedEndpoints,
    QueuedHaveTransactions, QueuedOverlayInboundHandler, QueuedProposal, QueuedTransaction,
    QueuedValidation,
};
use crate::message::{
    Message, ProtocolMessage, ProtocolMessageType, ProtocolPayload, TmProposeSet, TmSquelch,
    TmTransaction, TmValidation,
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
/// rippled `HashRouter::Setup` defaults. Entries expire after the hold window,
/// while a message may be relayed again after the shorter relay window.
const RELAY_HISTORY_HOLD_TIME: Duration = Duration::from_secs(300);
const RELAY_HISTORY_RELAY_TIME: Duration = Duration::from_secs(30);
const PEERFINDER_MAX_HOPS: u32 = 6;
const PEERFINDER_MAX_ACCEPTED_ENDPOINTS: usize = 64;
const PEERFINDER_REDIRECT_ENDPOINT_COUNT: usize = 10;
const PEERFINDER_LIVE_CACHE_TTL: Duration = Duration::from_secs(30);
const PEERFINDER_SECONDS_PER_MESSAGE: Duration = Duration::from_secs(151);
/// rippled `SSLHTTPPeer::doHandshake` arms the BaseHTTPPeer deadline before
/// accepting TLS. Keep an equivalent bound around both inbound transport
/// phases so one silent socket cannot retain an admission slot indefinitely.
const INBOUND_TLS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);
const INBOUND_HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const INBOUND_TLS_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(15);

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

/// An accepted TCP connection owns this reservation before TLS and HTTP work.
/// It holds the inbound Resource consumer and an IP admission count until the
/// connection is rejected or the consumer is transferred to its active peer.
#[derive(Default)]
struct InboundReservations {
    by_ip: HashMap<IpAddr, usize>,
}

struct InboundReservation {
    reservations: Arc<Mutex<InboundReservations>>,
    /// PeerFinder applies `ipLimit` only to public remote addresses. Private
    /// remotes still retain their Resource consumer, but carry no IP slot.
    remote_ip: Option<IpAddr>,
    consumer: Option<Consumer>,
}

impl InboundReservation {
    fn into_consumer(mut self) -> Consumer {
        let consumer = self
            .consumer
            .take()
            .expect("inbound reservation must own a resource consumer");
        self.release_ip();
        consumer
    }

    fn release_ip(&mut self) {
        let Some(remote_ip) = self.remote_ip.take() else {
            return;
        };
        let mut reservations = self.reservations.lock().expect("inbound reservation lock");
        let count = reservations
            .by_ip
            .get_mut(&remote_ip)
            .expect("inbound reservation IP must exist");
        *count -= 1;
        if *count == 0 {
            reservations.by_ip.remove(&remote_ip);
        }
    }
}

impl Drop for InboundReservation {
    fn drop(&mut self) {
        if self.consumer.is_some() {
            self.release_ip();
        }
    }
}

/// An outbound attempt occupies its endpoint reservation until the attempt
/// either fails or has atomically established an active peer. Drop covers
/// cancellation and every asynchronous error path.
struct OutboundAttemptReservation {
    attempts: Arc<Mutex<HashSet<IpAddr>>>,
    remote_ip: IpAddr,
}

#[derive(Default)]
struct SessionTaskTracker {
    active: Mutex<usize>,
    drained: std::sync::Condvar,
}

impl SessionTaskTracker {
    fn begin(&self) {
        *self.active.lock().expect("overlay session task lock") += 1;
    }

    fn complete(&self) {
        let mut active = self.active.lock().expect("overlay session task lock");
        *active = active.saturating_sub(1);
        if *active == 0 {
            self.drained.notify_all();
        }
    }

    fn wait_for_drain(&self) {
        let mut active = self.active.lock().expect("overlay session task lock");
        while *active != 0 {
            active = self
                .drained
                .wait(active)
                .expect("overlay session task condvar");
        }
    }
}

impl Drop for OutboundAttemptReservation {
    fn drop(&mut self) {
        self.attempts
            .lock()
            .expect("overlay pending outbound lock")
            .remove(&self.remote_ip);
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
    pub acceptor: Arc<openssl::ssl::SslAcceptor>,
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
        // Use random bytes for the identity to ensure uniqueness across
        // containers/processes that start from the same binary image.
        use basics::random::rand_int_full;
        for chunk in secret_bytes.chunks_mut(8) {
            let r: u64 = rand_int_full();
            let len = chunk.len().min(8);
            chunk[..len].copy_from_slice(&r.to_be_bytes()[..len]);
        }
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

/// The behavioral subset of rippled's `HashRouter::Entry` needed for
/// validator proposal and validation relay. The peer set is reset whenever
/// the relay interval permits a new relay, and entries are retained only for
/// the reference hold interval.
#[derive(Debug, Default)]
struct RelayHistoryEntry {
    peers: BTreeSet<PeerId>,
    relayed_at: Option<Instant>,
    last_touched: Option<Instant>,
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

    fn reject_malformed(&self, context: &'static str) -> crate::router::RouteAction {
        self.peer.charge(
            (*resource::FEE_MALFORMED_REQUEST).clone(),
            context.to_owned(),
        );
        crate::router::RouteAction::Continue
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
        // rippled 3.2.1 (OverlayImpl.cpp:667-757): process all trusted manifests,
        // cap untrusted processing at kMaxManifestsPerMessage=200, charge only
        // excess untrusted. Drop empty messages with useless-data charge.
        // Oversized messages (> 200 entries) are still processed up to the cap
        // rather than dropped entirely, matching rippled's trust-first behavior.
        const MAX_MANIFESTS_PER_MESSAGE: usize = 200;

        if message.list.is_empty() {
            self.peer.charge(
                (*resource::FEE_USELESS_DATA).clone(),
                "empty manifests".to_owned(),
            );
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
            // TMPing PING costs moderate peer work before replying, exactly as
            // PeerImp::onMessage(TMPing) does.
            self.peer.charge(
                (*resource::FEE_MODERATE_BURDEN_PEER).clone(),
                "ping request".to_owned(),
            );
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
                false,
            );
        } else if message.r#type == 1 {
            // A PONG is valid only for the exact outstanding local cookie.
            // Never derive RTT from the peer-provided timestamp.
            if let Some(rtt_ms) = self.peer.acknowledge_ping(message.seq) {
                tracing::debug!(
                    target: "overlay",
                    peer_id = %self.peer.id(),
                    latency_ms = rtt_ms,
                    "Peer latency measured"
                );
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
            self.peer.charge(
                (*resource::FEE_MODERATE_BURDEN_PEER).clone(),
                "oversized endpoints".to_owned(),
            );
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
            return self.reject_malformed("invalid get_ledger type");
        }
        // Verify ledger type (ltACCEPTED through ltCLOSED).
        if message.ltype.is_some_and(|ltype| !(0..=2).contains(&ltype)) {
            return self.reject_malformed("invalid get_ledger ledger type");
        }
        if message.itype == 3 {
            if message
                .ledger_hash
                .as_deref()
                .and_then(Uint256::from_slice)
                .is_none()
            {
                return self.reject_malformed("get_ledger candidate without hash");
            }
        } else if message.ledger_hash.is_none()
            && message.ledger_seq.is_none()
            && message.ltype != Some(2)
        {
            return self.reject_malformed("get_ledger without ledger selector");
        }
        if message
            .ledger_hash
            .as_deref()
            .is_some_and(|hash| Uint256::from_slice(hash).is_none())
        {
            return self.reject_malformed("get_ledger malformed ledger hash");
        }
        if message.itype != 0
            && (message.node_i_ds.is_empty()
                || message
                    .node_i_ds
                    .iter()
                    .any(|node_id| !is_valid_shamap_node_id_wire(node_id)))
        {
            return self.reject_malformed("get_ledger invalid node ids");
        }
        // rippled accepts only qtINDIRECT when querytype is present.
        if message.query_type.is_some_and(|query_type| query_type != 0) {
            return self.reject_malformed("get_ledger invalid query type");
        }
        if message.query_depth.is_some_and(|depth| {
            depth > crate::tuning::MAX_QUERY_DEPTH as u32 || message.itype == 0
        }) {
            return self.reject_malformed("get_ledger invalid query depth");
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
            return self.reject_malformed("ledger_data malformed ledger hash");
        }
        if !(0..=3).contains(&message.r#type) {
            return self.reject_malformed("ledger_data invalid type");
        }
        if let Some(error) = message.error
            && !(1..=3).contains(&error)
        {
            return self.reject_malformed("ledger_data invalid error code");
        }
        if message.nodes.is_empty() || message.nodes.len() > HARD_MAX_REPLY_NODES {
            return self.reject_malformed("ledger_data invalid node count");
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
        tracing::trace!(target: "overlay",
            sig_len = message.signature.len(),
            key_len = message.node_pub_key.len(),
            tx_hash_len = message.current_tx_hash.len(),
            prev_ledger_len = message.previousledger.len(),
        );
        if !(64..=72).contains(&message.signature.len()) {
            tracing::trace!(target: "overlay", peer_id = %self.peer.id(), "Invalid proposal signature length");
            return self.reject_malformed("proposal invalid signature length");
        }
        let Ok(public_key) = PublicKey::from_slice(&message.node_pub_key) else {
            return self.reject_malformed("proposal invalid public key");
        };
        if public_key.key_type() != Some(KeyType::Secp256k1) {
            return self.reject_malformed("proposal unsupported public key");
        }
        let Some(current_tx_hash) = Uint256::from_slice(&message.current_tx_hash) else {
            return self.reject_malformed("proposal malformed transaction hash");
        };
        let Some(previous_ledger) = Uint256::from_slice(&message.previousledger) else {
            return self.reject_malformed("proposal malformed previous ledger");
        };
        let suppression = proposal_unique_id(
            current_tx_hash,
            previous_ledger,
            message.propose_seq,
            message.close_time,
            public_key,
            &message.signature,
        );
        let peer_pos = QueuedProposal {
            peer_id: self.peer.id(),
            suppression,
            public_key,
            current_tx_hash,
            previous_ledger,
            message: message.clone(),
        };
        self.overlay
            .inbound_handler
            .on_propose_ledger(self.peer, peer_pos);
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
        tracing::trace!(target: "overlay", peer_id = %self.peer.id(), len = message.validation.len(), "on_validation: received TMValidation from peer");
        if message.validation.len() < 50 {
            tracing::trace!(target: "overlay", peer_id = %self.peer.id(), "Validation too short, ignoring");
            return self.reject_malformed("validation too short");
        }
        let suppression = sha512_half(&message.validation);
        self.overlay.inbound_handler.on_validation(
            self.peer,
            QueuedValidation {
                peer_id: self.peer.id(),
                suppression,
                message: message.clone(),
                validation: None,
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
            return self.reject_malformed("validator list missing payload");
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
        {
            return crate::router::RouteAction::Continue;
        }
        if message.version < 2 || message.manifest.is_empty() || message.blobs.is_empty() {
            return self.reject_malformed("validator list collection malformed");
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
        // Match PeerImp::onMessage(TMGetObjectByHash): the transaction-query
        // branch is selected before the generic ledger-hash validation. A
        // transaction-set query does not use that ledger selector, so rejecting
        // it here would incorrectly prevent the requested-tx job from running.
        if message.r#type == 7 {
            if !self.peer.tx_reduce_relay_enabled() {
                return self.reject_malformed("tx reduce-relay disabled");
            }
            tracing::trace!(
                target: "overlay",
                peer_id = %self.peer.id(),
                "Transaction get-objects query"
            );
            self.overlay
                .inbound_handler
                .on_get_objects(self.peer, message.clone());
            return crate::router::RouteAction::Continue;
        }
        if message
            .ledger_hash
            .as_deref()
            .is_some_and(|hash| Uint256::from_slice(hash).is_none())
        {
            tracing::trace!(target: "overlay", peer_id = %self.peer.id(), "Invalid get_objects ledger hash");
            return self.reject_malformed("get_objects malformed ledger hash");
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
            return self.reject_malformed("have_transactions malformed hash");
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
        // rippled expands every TMTransactions member through
        // handleTransaction(..., eraseTxQueue = false, batch = true), so each
        // member enters the same direct JtTransaction path as a normal relay.
        for transaction in &message.transactions {
            self.queue_transaction(transaction, true);
        }
        crate::router::RouteAction::Continue
    }

    fn on_squelch(&mut self, message: &crate::message::TmSquelch) -> crate::router::RouteAction {
        let Ok(validator) = PublicKey::from_slice(&message.validator_pub_key) else {
            tracing::debug!(target: "overlay", peer_id = %self.peer.id(), "Invalid squelch public key");
            return self.reject_malformed("squelch malformed public key");
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
            return self.reject_malformed("proof path request malformed");
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
            return self.reject_malformed("proof path response malformed");
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
            return self.reject_malformed("replay delta request malformed hash");
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
            return self.reject_malformed("replay delta response malformed hash");
        }
        tracing::trace!(target: "overlay", peer_id = %self.peer.id(), "Replay delta response received");
        self.overlay
            .inbound_handler
            .on_replay_delta_response(self.peer, message.clone());
        crate::router::RouteAction::Continue
    }
}

fn request_connects_as_peer(request: &Request<()>) -> bool {
    request
        .headers()
        .get("Connect-As")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|values| {
            values
                .split(',')
                .any(|value| value.trim().eq_ignore_ascii_case("peer"))
        })
}

fn is_valid_peer_endpoint(endpoint: SocketAddr) -> bool {
    endpoint.port() != 0
        && !endpoint.ip().is_unspecified()
        && !endpoint.ip().is_loopback()
        && is_public_ip(endpoint.ip())
}

/// Structural equivalent of SHAMapNodeId::deserialize_shamap_node_id without
/// adding an overlay-to-SHAMap crate dependency. The wire form is 32 masked
/// key bytes followed by depth; bits below the selected depth must be zero.
fn is_valid_shamap_node_id_wire(data: &[u8]) -> bool {
    if data.len() != 33 {
        return false;
    }
    let depth = data[32] as usize;
    if depth > 64 {
        return false;
    }
    let full_bytes = depth / 2;
    if depth % 2 == 0 {
        data[full_bytes..32].iter().all(|byte| *byte == 0)
    } else {
        data[full_bytes] & 0x0f == 0 && data[full_bytes + 1..32].iter().all(|byte| *byte == 0)
    }
}

type ManifestsMessageProvider = Arc<dyn Fn() -> Option<ProtocolMessage> + Send + Sync>;
type OutboundPeerFailureHandler = Arc<dyn Fn(SocketAddr, bool) + Send + Sync>;
type OutboundPeerCloseHandler = Arc<dyn Fn(SocketAddr, bool) + Send + Sync>;

pub struct OverlayImpl {
    setup: Setup,
    handoff: Arc<dyn OverlayHandoff>,
    connector: Arc<SslConnector>,
    active_peers: Arc<RwLock<HashMap<PeerId, Arc<PeerImp>>>>,
    by_public_key: Arc<RwLock<HashMap<PublicKey, Arc<PeerImp>>>>,
    next_id: Arc<AtomicU32>,
    jq_trans_overflow: Arc<AtomicU64>,
    peer_disconnects: Arc<AtomicU64>,
    peer_disconnect_charges: Arc<AtomicU64>,
    resource_manager: Arc<ResourceManager>,
    identity: OverlayIdentity,
    stop_requested: watch::Sender<bool>,
    stopping: Arc<AtomicBool>,
    traffic: Arc<TrafficCount>,
    tx_metrics: Arc<TxMetrics>,
    relay_history: Arc<Mutex<HashMap<RelayKey, RelayHistoryEntry>>>,
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
    inbound_reservations: Arc<Mutex<InboundReservations>>,
    peer_status_publisher: Arc<RwLock<Option<PeerStatusPublisher>>>,
    /// Hashes from the current local closed ledger and its parent, included in
    /// every outbound HTTP upgrade request like rippled's makeHandshake.
    handshake_ledgers: Arc<RwLock<Option<(Uint256, Uint256)>>>,
    /// Cached manifest message provider installed by the app state owner.
    manifests_message_provider: Arc<RwLock<Option<ManifestsMessageProvider>>>,
    /// App-owned PeerFinder failure sink for outbound peers that remain Not
    /// Useful. Overlay retains only this callback, never PeerFinder state.
    outbound_peer_failure_handler: Arc<RwLock<Option<OutboundPeerFailureHandler>>>,
    /// App-owned PeerFinder close sink. Normal closure releases endpoint
    /// attempt suppression without lowering bootcache valence.
    outbound_peer_close_handler: Arc<RwLock<Option<OutboundPeerCloseHandler>>>,
    /// Counts accepted inbound transports and active peer sessions through
    /// their final close. AppOverlayRuntime waits for this owner after
    /// broadcasting stop.
    session_tasks: Arc<SessionTaskTracker>,
    session_runtime: Arc<tokio::runtime::Runtime>,
}

impl OverlayImpl {
    pub fn new(setup: Setup, handoff: Arc<dyn OverlayHandoff>) -> Result<Self, OverlayError> {
        Self::with_clock(setup, handoff, Arc::new(SystemClock))
    }

    pub fn has_tls_acceptor(&self) -> bool {
        self.setup.server_ssl_acceptor.is_some()
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
        let resource_manager =
            Arc::new(make_manager(Arc::new(NullCollector), Arc::new(NullJournal)));

        Ok(Self {
            setup,
            handoff,
            connector,
            active_peers,
            by_public_key: Arc::new(RwLock::new(HashMap::new())),
            next_id: Arc::new(AtomicU32::new(1)),
            jq_trans_overflow: Arc::new(AtomicU64::new(0)),
            peer_disconnects: Arc::new(AtomicU64::new(0)),
            peer_disconnect_charges: Arc::new(AtomicU64::new(0)),
            resource_manager,
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
            inbound_reservations: Arc::new(Mutex::new(InboundReservations::default())),
            peer_status_publisher: Arc::new(RwLock::new(None)),
            handshake_ledgers: Arc::new(RwLock::new(None)),
            manifests_message_provider: Arc::new(RwLock::new(None)),
            outbound_peer_failure_handler: Arc::new(RwLock::new(None)),
            outbound_peer_close_handler: Arc::new(RwLock::new(None)),
            session_tasks: Arc::new(SessionTaskTracker::default()),
            session_runtime,
        })
    }

    pub fn set_manifests_message_provider<F>(&self, provider: F)
    where
        F: Fn() -> Option<ProtocolMessage> + Send + Sync + 'static,
    {
        *self
            .manifests_message_provider
            .write()
            .expect("manifest message provider lock") = Some(Arc::new(provider));
    }

    pub fn set_outbound_peer_failure_handler(
        &self,
        handler: impl Fn(SocketAddr, bool) + Send + Sync + 'static,
    ) {
        *self
            .outbound_peer_failure_handler
            .write()
            .expect("outbound peer failure handler lock") = Some(Arc::new(handler));
    }

    pub fn set_outbound_peer_close_handler(
        &self,
        handler: impl Fn(SocketAddr, bool) + Send + Sync + 'static,
    ) {
        *self
            .outbound_peer_close_handler
            .write()
            .expect("outbound peer close handler lock") = Some(Arc::new(handler));
    }

    fn send_cached_manifests(&self, peer: &Arc<PeerImp>) {
        let provider = self
            .manifests_message_provider
            .read()
            .expect("manifest message provider lock")
            .as_ref()
            .map(Arc::clone);
        if let Some(message) = provider.and_then(|provider| provider()) {
            peer.send(Message::new(message, None));
        }
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
            .setup
            .server_ssl_acceptor
            .clone()
            .ok_or_else(|| OverlayError::Tls("missing server openssl acceptor".to_owned()))?;
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
        if self.is_stopping() || *stop_requested.borrow() {
            return Ok(());
        }

        // Door::doAccept creates and starts an independent SSLHTTPPeer for
        // every accepted socket before accepting again. Do the equivalent
        // here: TLS/HTTP handshaking must never serially block the listener.
        std::mem::drop(self.spawn_tracked_inbound_transport(
            tcp_stream,
            remote_address,
            stop_requested,
            Arc::clone(&acceptor.acceptor),
        ));
        Ok(())
    }

    /// Continue an accepted TLS peer connection from a shared server listener.
    /// The server owns the socket only until it identifies a peer TLS ClientHello;
    /// this method then follows the same OverlayImpl::onHandoff path as a
    /// dedicated peer listener.
    pub fn spawn_handoff(
        &self,
        tcp_stream: TcpStream,
        remote_address: SocketAddr,
    ) -> JoinHandle<Result<(), OverlayError>> {
        let stop_requested = self.stop_requested.subscribe();
        let acceptor = match self.setup.server_ssl_acceptor.clone() {
            Some(acceptor) => acceptor,
            None => {
                return tokio::spawn(async {
                    Err(OverlayError::Tls("missing server TLS acceptor".to_owned()))
                });
            }
        };
        self.spawn_tracked_inbound_transport(tcp_stream, remote_address, stop_requested, acceptor)
    }

    /// Own every accepted transport until it either exits before activation or
    /// transfers the stream to a tracked PeerSession. This preserves shutdown
    /// drain semantics while allowing the accept loop to continue immediately.
    fn spawn_tracked_inbound_transport(
        &self,
        tcp_stream: TcpStream,
        remote_address: SocketAddr,
        stop_requested: watch::Receiver<bool>,
        acceptor: Arc<openssl::ssl::SslAcceptor>,
    ) -> JoinHandle<Result<(), OverlayError>> {
        let this = self.clone_for_tasks();
        let tracker = Arc::clone(&self.session_tasks);
        tracker.begin();
        tokio::spawn(async move {
            let result = this
                .handle_inbound_stream(tcp_stream, remote_address, stop_requested, acceptor)
                .await;
            tracker.complete();
            result
        })
    }

    async fn handle_inbound_stream(
        &self,
        tcp_stream: TcpStream,
        remote_address: SocketAddr,
        mut stop_requested: watch::Receiver<bool>,
        acceptor: Arc<openssl::ssl::SslAcceptor>,
    ) -> Result<(), OverlayError> {
        tracing::debug!(target: "overlay", ip = %remote_address, "Inbound connection accepted");
        // The transport remains independently bounded by the TLS and HTTP
        // deadlines below. Peer resource/slot admission is intentionally
        // deferred until the request-processing callback accepts this as a
        // peer handoff, matching rippled OverlayImpl::onHandoff.
        // Disable Nagle's algorithm for low-latency request-response pipelining.
        let _ = tcp_stream.set_nodelay(true);
        let ssl = openssl::ssl::Ssl::new(acceptor.context())
            .map_err(|error| OverlayError::Tls(error.to_string()))?;
        let mut tls_stream = tokio_openssl::SslStream::new(ssl, tcp_stream)
            .map_err(|error| OverlayError::Tls(error.to_string()))?;
        let accept_result = tokio::select! {
            biased;
            changed = stop_requested.changed() => {
                let _ = changed;
                return Ok(());
            }
            result = timeout(
                INBOUND_TLS_HANDSHAKE_TIMEOUT,
                std::pin::Pin::new(&mut tls_stream).accept(),
            ) => match result {
                Ok(result) => result,
                Err(_) => {
                    tracing::debug!(
                        target: "overlay",
                        ip = %remote_address,
                        timeout_secs = INBOUND_TLS_HANDSHAKE_TIMEOUT.as_secs(),
                        "Inbound TLS handshake timed out"
                    );
                    return Ok(());
                }
            },
        };
        if let Err(error) = accept_result {
            tracing::debug!(target: "overlay", ip = %remote_address, %error, "TLS accept failed");
            return Ok(());
        }

        let request_stop_requested = stop_requested.clone();
        let request_result = tokio::select! {
            biased;
            changed = stop_requested.changed() => {
                let _ = changed;
                return Ok(());
            }
            result = timeout(
                INBOUND_HTTP_REQUEST_TIMEOUT,
                read_http_request(&mut tls_stream, request_stop_requested),
            ) => match result {
                Ok(result) => result?,
                Err(_) => {
                    tracing::debug!(
                        target: "overlay",
                        ip = %remote_address,
                        timeout_secs = INBOUND_HTTP_REQUEST_TIMEOUT.as_secs(),
                        "Inbound HTTP upgrade request timed out"
                    );
                    return Ok(());
                }
            },
        };
        let (request, read_ahead) = match request_result {
            Some(request) => request,
            None => return Ok(()),
        };

        // Derive TLS shared value from Finished messages (rippled: makeSharedValue).
        // This is used for Session-Signature signing and peer verification.
        let inbound_shared_value = {
            let ssl = tls_stream.ssl();
            let mut local_finished = [0u8; 64];
            let mut peer_finished = [0u8; 64];
            let local_len = ssl.finished(&mut local_finished);
            let peer_len = ssl.peer_finished(&mut peer_finished);
            crate::transport::handshake::make_shared_value_from_finished_messages(
                &local_finished[..local_len],
                &peer_finished[..peer_len],
            )
        };

        // HTTP API interception
        let path = request.uri().path();
        if path == "/health" || path == "/crawl" || path.starts_with("/vl/") {
            tracing::info!(target: "overlay", ip = %remote_address, path = %path, "HTTP API request received");
            let body = format!("{{\"status\": \"ok\", \"path\": \"{}\"}}", path);
            let response_str = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            tls_stream
                .write_all(response_str.as_bytes())
                .await
                .map_err(|e| OverlayError::Io(e))?;
            shutdown_inbound_tls(&mut tls_stream, stop_requested.clone()).await;
            return Ok(());
        }

        let handoff = self.handoff.on_handoff(&request, remote_address);
        let mut accepted_peer = None;
        let (response, response_wire) = match handoff {
            Handoff::Accepted => {
                // Rippled's onHandoff sequence is processRequest → Resource
                // consumer → inbound PeerFinder slot → Connect-As. The
                // transport has already completed its independently timed
                // asynchronous TLS/HTTP work; reserve both peer-admission
                // resources here before validating Connect-As.
                let inbound_reservation = match self.reserve_inbound(remote_address) {
                    Some(reservation) => reservation,
                    None => {
                        tracing::warn!(
                            target: "overlay",
                            ip = %remote_address,
                            "Inbound peer resource or IP slot limit reached"
                        );
                        return Ok(());
                    }
                };
                if !request_connects_as_peer(&request) {
                    tracing::warn!(
                        target: "overlay",
                        ip = %remote_address,
                        "Inbound peer redirected: missing Connect-As peer token"
                    );
                    let (_, wire) = self.make_redirect_response(&request, remote_address)?;
                    tls_stream.write_all(&wire).await?;
                    tls_stream.flush().await?;
                    shutdown_inbound_tls(&mut tls_stream, stop_requested.clone()).await;
                    return Ok(());
                }
                let Some(protocol) = negotiate_inbound_peer_upgrade(&request) else {
                    tracing::warn!(
                        target: "overlay",
                        ip = %remote_address,
                        "Inbound peer rejected: unsupported Upgrade"
                    );
                    let response = Response::builder()
                        .status(http::StatusCode::BAD_REQUEST)
                        .header("Connection", "close")
                        .body(())
                        .map_err(|error| OverlayError::InvalidRequest(error.to_string()))?;
                    let wire = serialize_response(&response);
                    tls_stream.write_all(&wire).await?;
                    tls_stream.flush().await?;
                    shutdown_inbound_tls(&mut tls_stream, stop_requested.clone()).await;
                    return Ok(());
                };
                let peer = match self.peer_from_request(&request, remote_address) {
                    Ok(peer) => peer,
                    Err(error) => {
                        tracing::warn!(target: "overlay", ip = %remote_address, %error, "Inbound peer rejected");
                        let response = Response::builder()
                            .status(http::StatusCode::FORBIDDEN)
                            .header("Connection", "close")
                            .body(())
                            .map_err(|error| OverlayError::InvalidRequest(error.to_string()))?;
                        let wire = serialize_response(&response);
                        tls_stream.write_all(&wire).await?;
                        tls_stream.flush().await?;
                        shutdown_inbound_tls(&mut tls_stream, stop_requested.clone()).await;
                        return Ok(());
                    }
                };
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
                    // Derive shared value must succeed — rippled rejects at
                    // OverlayImpl.cpp:282-292 when makeSharedValue fails.
                    let shared_value = match inbound_shared_value {
                        Some(sv) => sv,
                        None => {
                            tracing::warn!(
                                target: "overlay",
                                ip = %remote_address,
                                "Inbound shared value derivation failed — disconnecting"
                            );
                            return Ok(());
                        }
                    };
                    // Verify the peer's handshake (compatibility: validatePeerHandshake)
                    let verify_ctx = crate::transport::handshake::HandshakeVerificationContext {
                        shared_value,
                        network_id: self.handshake_context().network_id,
                        local_public_key: Some(self.identity.public_key()),
                        public_ip: self.setup.public_ip,
                        remote_ip: remote_address.ip(),
                        clock_tolerance: std::time::Duration::from_secs(20),
                    };
                    if let Err(reason) = crate::transport::handshake::verify_handshake(
                        request.headers(),
                        &verify_ctx,
                    ) {
                        tracing::warn!(
                            target: "overlay",
                            ip = %remote_address,
                            %reason,
                            "Inbound handshake verification failed — disconnecting"
                        );
                        return Ok(());
                    }

                    // Build handshake context with real TLS Session-Signature.
                    let mut handshake_ctx = self.handshake_context();
                    handshake_ctx.remote_ip = Some(remote_address.ip());
                    // Rippled advertises configured public_ip as Local-IP;
                    // a wildcard/local listener is intentionally not claimed.
                    handshake_ctx.local_ip = self.setup.public_ip;
                    if let Some(sv) = inbound_shared_value {
                        match self.identity.sign_session(&sv) {
                            Ok(sig) => handshake_ctx.session_signature = sig,
                            Err(err) => {
                                tracing::warn!(target: "overlay", %err, "Failed to sign inbound session");
                                return Ok(());
                            }
                        }
                    }
                    let (compr_enabled, ledger_replay_enabled, txrr_enabled, vprr_enabled) =
                        self.inbound_handshake_features();
                    let response = make_response(
                        true,
                        &request,
                        &handshake_ctx,
                        protocol,
                        compr_enabled,
                        ledger_replay_enabled,
                        txrr_enabled,
                        vprr_enabled,
                    );
                    accepted_peer = Some((peer, request.headers().clone(), inbound_reservation));
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
        if accepted_peer.is_none() {
            // A redirect or rejected handoff has no PeerSession writer. Send
            // close_notify here before releasing the TLS stream.
            shutdown_inbound_tls(&mut tls_stream, stop_requested.clone()).await;
            return Ok(());
        }
        if let Some((peer, headers, inbound_reservation)) = accepted_peer {
            let result = ConnectAttemptResult {
                peer,
                response,
                negotiated_features: headers,
                session: Some(
                    PeerSessionStarter::new(Box::new(tls_stream), stop_requested.clone())
                        .with_initial_buffer(read_ahead),
                ),
            };
            let _ = self.finalize_inbound_connect_result(result, inbound_reservation);
        }
        Ok(())
    }

    fn clone_for_tasks(&self) -> Self {
        Self {
            setup: self.setup.clone(),
            handoff: Arc::clone(&self.handoff),
            connector: self.connector.clone(),
            active_peers: Arc::clone(&self.active_peers),
            by_public_key: Arc::clone(&self.by_public_key),
            next_id: Arc::clone(&self.next_id),
            jq_trans_overflow: Arc::clone(&self.jq_trans_overflow),
            peer_disconnects: Arc::clone(&self.peer_disconnects),
            peer_disconnect_charges: Arc::clone(&self.peer_disconnect_charges),
            resource_manager: Arc::clone(&self.resource_manager),
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
            inbound_reservations: Arc::clone(&self.inbound_reservations),
            peer_status_publisher: Arc::clone(&self.peer_status_publisher),
            handshake_ledgers: Arc::clone(&self.handshake_ledgers),
            manifests_message_provider: Arc::clone(&self.manifests_message_provider),
            outbound_peer_failure_handler: Arc::clone(&self.outbound_peer_failure_handler),
            outbound_peer_close_handler: Arc::clone(&self.outbound_peer_close_handler),
            session_tasks: Arc::clone(&self.session_tasks),
            session_runtime: Arc::clone(&self.session_runtime),
        }
    }

    pub fn activate(&self, peer: Arc<PeerImp>) -> bool {
        self.activate_with_consumer(peer, None)
    }

    fn activate_with_inbound_reservation(
        &self,
        peer: Arc<PeerImp>,
        reservation: InboundReservation,
    ) -> bool {
        self.activate_with_consumer(peer, Some(reservation))
    }

    fn activate_with_consumer(
        &self,
        peer: Arc<PeerImp>,
        inbound_reservation: Option<InboundReservation>,
    ) -> bool {
        self.apply_membership_state(&peer);
        let limits = self.setup.peer_limits();
        // The public-key map is the activation gate. Holding it across the
        // active-peer insertion makes duplicate node identity rejection
        // atomic, rather than allowing two independent maps to race.
        let mut by_public_key = self.by_public_key.write().expect("overlay public-key lock");
        if by_public_key.contains_key(&peer.node_public()) {
            tracing::warn!(target: "overlay", peer_id = %peer.id(), "Duplicate node public key rejected");
            return false;
        }
        let mut active_peers = self.active_peers.write().expect("overlay peers lock");
        let active_in_direction =
            self.directional_active_peers_count_locked(&active_peers, peer.inbound());
        let direction_limit = if peer.inbound() {
            limits.inbound_max
        } else {
            limits.outbound_max
        };
        if self.peer_counts_toward_limit(&peer) && active_in_direction >= direction_limit {
            tracing::warn!(
                target: "overlay",
                peer_id = %peer.id(),
                inbound = peer.inbound(),
                active_in_direction,
                direction_limit,
                "Peer directional resource limit exceeded"
            );
            return false;
        }
        active_peers.insert(peer.id(), Arc::clone(&peer));
        by_public_key.insert(peer.node_public(), Arc::clone(&peer));
        let total = active_peers.len();
        drop(active_peers);
        drop(by_public_key);

        let consumer = inbound_reservation.map_or_else(
            || {
                if peer.inbound() {
                    self.resource_manager
                        .new_inbound_endpoint(peer.remote_address())
                } else {
                    self.resource_manager
                        .new_outbound_endpoint(peer.remote_address())
                }
            },
            InboundReservation::into_consumer,
        );
        consumer.set_public_key(*peer.node_public().as_bytes());
        if !peer.inbound() {
            let failure_handler = Arc::clone(&self.outbound_peer_failure_handler);
            peer.set_outbound_failure_notifier(Arc::new(move |address, fixed| {
                if let Some(handler) = failure_handler
                    .read()
                    .expect("outbound peer failure handler lock")
                    .as_ref()
                    .map(Arc::clone)
                {
                    handler(address, fixed);
                }
            }));
        }
        peer.install_resource_consumer(consumer, Arc::clone(&self.peer_disconnect_charges));
        peer.start_lifecycle_timer(self.session_runtime.handle());

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
            // Keep the public-key gate before the peer map here as in
            // activation. Remove an identity entry only when it still names
            // this exact peer, so an old close cannot erase a newer mapping.
            let mut by_public_key = self.by_public_key.write().expect("overlay public-key lock");
            let mut active_peers = self.active_peers.write().expect("overlay peers lock");
            let peer = active_peers.remove(&id);
            if let Some(peer) = &peer
                && by_public_key
                    .get(&peer.node_public())
                    .is_some_and(|mapped| mapped.id() == id)
            {
                by_public_key.remove(&peer.node_public());
            }
            peer
        };
        if let Some(peer) = peer {
            tracing::info!(
                target: "overlay",
                peer_id = %id,
                ip = %peer.remote_address(),
                reason = "deactivated",
                "Peer disconnected"
            );
            peer.stop_lifecycle_timer();
            peer.detach_session();
            peer.clear_queued_messages();
            peer.clear_tx_queue();
            self.relay_history
                .lock()
                .expect("relay history lock")
                .values_mut()
                .for_each(|entry| {
                    entry.peers.remove(&id);
                });
            self.slots
                .lock()
                .expect("overlay slots lock")
                .delete_peer(id, true);
            self.inc_peer_disconnect();
            if !peer.inbound() {
                if let Some(handler) = self
                    .outbound_peer_close_handler
                    .read()
                    .expect("outbound peer close handler lock")
                    .as_ref()
                    .map(Arc::clone)
                {
                    handler(peer.remote_address(), peer.fixed());
                }
            }
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

    pub fn stop_receiver(&self) -> watch::Receiver<bool> {
        self.stop_requested.subscribe()
    }

    /// Wait for accepted inbound transports and peer sessions to finish their
    /// stop-aware handshakes, drain queues, and emit TLS close_notify after
    /// `signal_stop` has been broadcast.
    pub fn wait_for_session_shutdown(&self) {
        self.session_tasks.wait_for_drain();
    }

    /// Update the ledger hashes advertised on future outbound handshakes.
    pub fn set_handshake_ledgers(&self, closed_ledger: Uint256, previous_ledger: Uint256) {
        *self
            .handshake_ledgers
            .write()
            .expect("overlay handshake ledger lock") = Some((closed_ledger, previous_ledger));
    }

    fn handshake_context(&self) -> HandshakeContext {
        let mut context = self.identity.context();
        context.network_id = self.setup.network_id;
        if let Some((closed_ledger, previous_ledger)) = *self
            .handshake_ledgers
            .read()
            .expect("overlay handshake ledger lock")
        {
            context.closed_ledger = Some(closed_ledger.to_string());
            context.previous_ledger = Some(previous_ledger.to_string());
        }
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

    pub fn requeue_transactions(&self, transactions: Vec<crate::QueuedTransaction>) {
        self.queued_inbound.requeue_transactions(transactions);
    }

    /// Register a channel for immediate TmLedgerData delivery from the
    /// network thread, matching reference InboundLedgers::gotLedgerData.
    pub fn set_ledger_data_channel(
        &self,
        tx: std::sync::mpsc::SyncSender<crate::PeerMessage<crate::TmLedgerData>>,
    ) {
        self.queued_inbound.set_ledger_data_channel(tx);
    }

    /// Drain accepted endpoint advertisements for the live PeerFinder cache.
    pub fn take_endpoints(&self) -> Vec<crate::QueuedEndpoints> {
        self.queued_inbound.take_endpoints()
    }

    pub fn take_validations(&self) -> Vec<crate::QueuedValidation> {
        self.queued_inbound.take_validations()
    }

    pub fn take_manifests(&self) -> Vec<crate::PeerMessage<crate::TmManifests>> {
        self.queued_inbound.take_manifests()
    }

    pub fn take_proposals(&self) -> Vec<crate::QueuedProposal> {
        self.queued_inbound.take_proposals()
    }

    pub fn take_validator_lists(&self) -> Vec<crate::PeerMessage<crate::TmValidatorList>> {
        self.queued_inbound.take_validator_lists()
    }

    pub fn take_ledger_data(&self) -> Vec<crate::PeerMessage<crate::TmLedgerData>> {
        self.queued_inbound.take_ledger_data()
    }

    pub fn take_get_ledgers(&self) -> Vec<crate::PeerMessage<crate::TmGetLedger>> {
        self.queued_inbound.take_get_ledgers()
    }

    pub fn take_transactions(&self) -> Vec<crate::QueuedTransaction> {
        self.queued_inbound.take_transactions()
    }

    pub fn take_get_objects(&self) -> Vec<crate::PeerMessage<crate::TmGetObjectByHash>> {
        self.queued_inbound.take_get_objects()
    }

    pub fn admit_proposal_source(
        &self,
        uid: Uint256,
        validator: PublicKey,
        peer_id: PeerId,
    ) -> bool {
        let added = self.register_relay_source(RelayKind::Proposal, uid, peer_id);
        if !added {
            self.update_slot_for_recent_duplicate(
                RelayKind::Proposal,
                uid,
                validator,
                peer_id,
                ProtocolMessageType::MtProposeLedger,
            );
        }
        added
    }

    pub fn admit_validation_source(
        &self,
        uid: Uint256,
        validator: PublicKey,
        peer_id: PeerId,
    ) -> bool {
        let added = self.register_relay_source(RelayKind::Validation, uid, peer_id);
        if !added {
            self.update_slot_for_recent_duplicate(
                RelayKind::Validation,
                uid,
                validator,
                peer_id,
                ProtocolMessageType::MtValidation,
            );
        }
        added
    }

    pub fn peer_is_diverged(&self, peer_id: PeerId) -> bool {
        self.active_peers
            .read()
            .expect("overlay peers lock")
            .get(&peer_id)
            .is_some_and(|peer| peer.tracking() == crate::peer_imp::Tracking::Diverged)
    }

    pub fn suppress_validation(&self, uid: Uint256) {
        let now = Instant::now();
        let mut history = self.relay_history.lock().expect("relay history lock");
        history.retain(|_, entry| {
            entry
                .last_touched
                .is_some_and(|touched| now.duration_since(touched) < RELAY_HISTORY_HOLD_TIME)
        });
        history
            .entry(RelayKey {
                kind: RelayKind::Validation,
                uid,
            })
            .or_default()
            .last_touched = Some(now);
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

        tracing::debug!(target: "overlay", %hash, "relay_transaction: sending tx to peers");
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
                let _ = self.send_runtime_message(&peer, message.clone(), false);
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
                let _ = self.send_runtime_message(&peer, message.clone(), false);
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
                let _ = self.send_runtime_message(&peer, message, false);
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

    fn register_relay_source(&self, kind: RelayKind, uid: Uint256, peer_id: PeerId) -> bool {
        let now = Instant::now();
        let mut history = self.relay_history.lock().expect("relay history lock");
        history.retain(|_, entry| {
            entry
                .last_touched
                .is_some_and(|touched| now.duration_since(touched) < RELAY_HISTORY_HOLD_TIME)
        });
        match history.entry(RelayKey { kind, uid }) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                let mut state = RelayHistoryEntry {
                    last_touched: Some(now),
                    ..RelayHistoryEntry::default()
                };
                state.peers.insert(peer_id);
                entry.insert(state);
                true
            }
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                let state = entry.get_mut();
                state.last_touched = Some(now);
                state.peers.insert(peer_id);
                false
            }
        }
    }

    fn update_slot_for_recent_duplicate(
        &self,
        kind: RelayKind,
        uid: Uint256,
        validator: PublicKey,
        peer_id: PeerId,
        message_type: ProtocolMessageType,
    ) {
        let now = Instant::now();
        let recently_relayed = self
            .relay_history
            .lock()
            .expect("relay history lock")
            .get(&RelayKey { kind, uid })
            .and_then(|entry| entry.relayed_at)
            .is_some_and(|relayed_at| now.duration_since(relayed_at) < crate::slot::IDLED);
        if !recently_relayed {
            return;
        }

        let mut slots = self.slots.lock().expect("overlay slots lock");
        if slots.base_squelch_ready() {
            slots.update_slot_and_squelch(uid, validator, peer_id, message_type, || {});
        }
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
        let mut relayed = BTreeSet::new();
        let now = Instant::now();
        let mut history = self.relay_history.lock().expect("relay history lock");
        // `HashRouter::emplace` expires aged entries before admitting a new
        // key. This is time-based retention, not arbitrary eviction.
        history.retain(|_, entry| {
            entry
                .last_touched
                .is_some_and(|touched| now.duration_since(touched) < RELAY_HISTORY_HOLD_TIME)
        });
        let entry = history.entry(relay_key).or_default();
        entry.last_touched = Some(now);
        let relay_due = entry
            .relayed_at
            .is_none_or(|relayed_at| now.duration_since(relayed_at) >= RELAY_HISTORY_RELAY_TIME);
        if !relay_due {
            return entry.peers.clone();
        }
        // `HashRouter::shouldRelay` admits the new relay and
        // `releasePeerSet` returns (and clears) every ingress source so we do
        // not relay a proposal or validation back to its sender.
        entry.relayed_at = Some(now);
        let already_seen = std::mem::take(&mut entry.peers);

        for peer in peers {
            if already_seen.contains(&peer.id()) {
                continue;
            }
            if self.send_runtime_message(&peer, message.clone(), false) {
                relayed.insert(peer.id());
            }
        }

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
                already_seen.iter().copied(),
                message_type,
            );
        }

        already_seen
    }

    fn broadcast_validator_message(&self, protocol: ProtocolMessage, validator: PublicKey) {
        tracing::debug!(target: "overlay", msg_type = ?protocol.message_type, "Broadcasting validator message");
        let message = Message::new(protocol, Some(validator));
        for peer in self.active_peers_snapshot() {
            let _ = self.send_runtime_message(&peer, message.clone(), false);
        }
    }

    fn send_runtime_message(&self, peer: &Arc<PeerImp>, message: Message, force: bool) -> bool {
        if !force {
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
            .filter(|peer| peer.disconnect_requested() || peer.has_dead_session_channel())
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

    /// Mirrors rippled `Counts::adjust(Active)`: fixed, reserved, and cluster
    /// peers remain active but do not consume a public directional slot.
    fn peer_counts_toward_limit(&self, peer: &PeerImp) -> bool {
        !peer.fixed() && !peer.reserved() && !peer.cluster()
    }

    fn directional_active_peers_count_locked(
        &self,
        active_peers: &HashMap<PeerId, Arc<PeerImp>>,
        inbound: bool,
    ) -> usize {
        active_peers
            .values()
            .filter(|peer| peer.inbound() == inbound && self.peer_counts_toward_limit(peer))
            .count()
    }

    pub fn active_inbound_peers_count(&self) -> usize {
        self.prune_disconnected_peers();
        self.directional_active_peers_count_locked(
            &self.active_peers.read().expect("overlay peers lock"),
            true,
        )
    }

    fn can_activate_peer(&self, peer: &PeerImp) -> bool {
        if !self.peer_counts_toward_limit(peer) {
            return true;
        }
        let limits = self.setup.peer_limits();
        let active_peers = self.active_peers.read().expect("overlay peers lock");
        let (active_in_direction, direction_limit) = if peer.inbound() {
            (
                self.directional_active_peers_count_locked(&active_peers, true),
                limits.inbound_max,
            )
        } else {
            (
                self.directional_active_peers_count_locked(&active_peers, false),
                limits.outbound_max,
            )
        };
        active_in_direction < direction_limit
    }

    fn enforce_peer_limit(&self, peers: &[Arc<PeerImp>]) {
        let limits = self.setup.peer_limits();
        let mut remaining_inbound = self.active_inbound_peers_count();
        let mut remaining_outbound = self.active_outbound_peers_count();
        let mut to_deactivate = Vec::new();

        for peer in peers.iter().rev() {
            if !self.peer_counts_toward_limit(peer) {
                continue;
            }
            let (remaining, limit) = if peer.inbound() {
                (&mut remaining_inbound, limits.inbound_max)
            } else {
                (&mut remaining_outbound, limits.outbound_max)
            };
            if *remaining > limit {
                to_deactivate.push(peer.id());
                *remaining -= 1;
            }
        }

        for id in to_deactivate {
            self.on_peer_deactivate(id);
        }
    }

    /// Mirrors `makeResponse`: only advertise optional features enabled by
    /// this node's configuration. Compression and ledger replay have no
    /// independent Setup switch in the current Rust runtime, so they remain
    /// enabled whenever the transport supports them.
    fn inbound_handshake_features(&self) -> (bool, bool, bool, bool) {
        (
            true,
            true,
            self.setup.tx_reduce_relay_enabled,
            self.setup.vp_reduce_relay_base_squelch_enabled,
        )
    }

    fn configure_connected_peer(&self, peer: &PeerImp, headers: &http::HeaderMap) {
        let protocol = headers
            .get("Upgrade")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| negotiate_protocol_version(parse_protocol_versions(value)))
            .unwrap_or(ProtocolVersion::new(2, 2));
        peer.set_protocol_version(protocol);
        peer.set_compression_enabled(is_feature_value(headers, FEATURE_COMPR, "lz4"));
        peer.set_tx_reduce_relay_enabled(
            self.setup.tx_reduce_relay_enabled && feature_enabled(headers, FEATURE_TXRR),
        );
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

    fn reserve_inbound(&self, remote_address: SocketAddr) -> Option<InboundReservation> {
        // This is the source-ordered peer-admission portion of rippled
        // OverlayImpl::onHandoff: after request processing identifies a peer,
        // acquire the Resource consumer before the inbound PeerFinder slot.
        // TLS and HTTP timeouts above bound pre-handoff transports.
        let consumer = self.resource_manager.new_inbound_endpoint(remote_address);
        if consumer.disposition() == Disposition::Drop {
            let _ = consumer.disconnect_with_manager_journal();
            tracing::warn!(target: "overlay", ip = %remote_address, "Inbound resource consumer rejected peer handoff");
            return None;
        }

        let remote_ip = canonical_peer_ip(remote_address.ip());
        // PeerFinder::Logic::newInboundSlot limits only public remote
        // addresses. Private/RFC-1918 peers receive an inbound slot but do not
        // consume `ipLimit`, which is designed to prevent public-IP fanout.
        let reserved_ip_slot = is_public_ip(remote_ip).then_some(remote_ip);
        if let Some(remote_ip) = reserved_ip_slot {
            let ip_limit = if self.setup.ip_limit == 0 {
                2
            } else {
                self.setup.ip_limit
            };
            let mut reservations = self
                .inbound_reservations
                .lock()
                .expect("inbound reservation lock");
            let active_from_ip = self
                .active_peers
                .read()
                .expect("overlay peers lock")
                .values()
                .filter(|peer| canonical_peer_ip(peer.remote_address().ip()) == remote_ip)
                .count();
            let pending_from_ip = reservations
                .by_ip
                .get(&remote_ip)
                .copied()
                .unwrap_or_default();
            if active_from_ip.saturating_add(pending_from_ip) >= ip_limit {
                return None;
            }
            *reservations.by_ip.entry(remote_ip).or_default() += 1;
        }
        Some(InboundReservation {
            reservations: Arc::clone(&self.inbound_reservations),
            remote_ip: reserved_ip_slot,
            consumer: Some(consumer),
        })
    }

    pub fn outbound_endpoint_is_active_or_pending(&self, address: SocketAddr) -> bool {
        self.prune_disconnected_peers();
        let ip = canonical_peer_ip(address.ip());
        self.active_peers
            .read()
            .expect("overlay peers lock")
            .values()
            .any(|peer| canonical_peer_ip(peer.remote_address().ip()) == ip)
            || self
                .pending_outbound_ips
                .lock()
                .expect("overlay pending outbound lock")
                .contains(&ip)
    }

    fn reserve_outbound_attempt(&self, address: SocketAddr) -> Option<OutboundAttemptReservation> {
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
            return None;
        }
        let inserted = self
            .pending_outbound_ips
            .lock()
            .expect("overlay pending outbound lock")
            .insert(ip);
        inserted.then_some(OutboundAttemptReservation {
            attempts: Arc::clone(&self.pending_outbound_ips),
            remote_ip: ip,
        })
    }

    pub fn active_outbound_peers_count(&self) -> usize {
        self.prune_disconnected_peers();
        self.directional_active_peers_count_locked(
            &self.active_peers.read().expect("overlay peers lock"),
            false,
        )
    }

    pub fn peer_limits(&self) -> crate::overlay::PeerLimits {
        self.setup.peer_limits()
    }

    fn finalize_connect_result(
        &self,
        result: ConnectAttemptResult,
    ) -> Result<ConnectAttemptResult, &'static str> {
        self.finalize_connect_result_with_reservation(result, None)
    }

    fn finalize_inbound_connect_result(
        &self,
        result: ConnectAttemptResult,
        reservation: InboundReservation,
    ) -> Result<ConnectAttemptResult, &'static str> {
        self.finalize_connect_result_with_reservation(result, Some(reservation))
    }

    fn finalize_connect_result_with_reservation(
        &self,
        mut result: ConnectAttemptResult,
        reservation: Option<InboundReservation>,
    ) -> Result<ConnectAttemptResult, &'static str> {
        self.configure_connected_peer(&result.peer, &result.negotiated_features);
        let activated = match reservation {
            Some(reservation) => {
                self.activate_with_inbound_reservation(Arc::clone(&result.peer), reservation)
            }
            None => self.activate(Arc::clone(&result.peer)),
        };
        if !activated {
            tracing::warn!(
                target: "overlay",
                ip = %result.peer.remote_address(),
                reason = PEER_LIMIT_REJECTION_REASON,
                "Connection attempt failed"
            );
            result.session = None;
            return Err(PEER_LIMIT_REJECTION_REASON);
        }
        self.send_cached_manifests(&result.peer);
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
        self.session_tasks.begin();
        let tracker = Arc::clone(&self.session_tasks);
        let session_task = session.start_on(self.session_runtime.handle(), peer, hooks, on_close);
        std::mem::drop(self.session_runtime.handle().spawn(async move {
            let _ = session_task.await;
            tracker.complete();
        }));
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
        let outbound_attempt = match self.reserve_outbound_attempt(address) {
            Some(reservation) => reservation,
            None => {
                return Box::pin(async { Err(ConnectAttemptError::DuplicateOutboundAttempt) });
            }
        };
        tracing::info!(target: "overlay", ip = %address, "Outbound connection attempt");
        let connector = self.connector.clone();
        let config = ConnectAttemptConfig {
            server_name: address.ip().to_string(),
            compr_enabled: true,
            ledger_replay_enabled: true,
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
        let network_id = self.setup.network_id;
        let public_ip = self.setup.public_ip;
        let verify_response = Arc::new(
            move |response: &Response<()>, remote: SocketAddr, shared_value: &Uint256| {
                let handshake_peer = verify_handshake(
                    response.headers(),
                    &HandshakeVerificationContext {
                        shared_value: *shared_value,
                        network_id,
                        local_public_key: Some(local_public_key),
                        public_ip,
                        remote_ip: remote.ip(),
                        clock_tolerance: Duration::from_secs(20),
                    },
                )
                .map_err(ConnectAttemptError::Protocol)?;
                Ok(PeerImp::new_with_inbound(
                    peer_id,
                    remote,
                    false,
                    handshake_peer.public_key,
                    remote.to_string(),
                ))
            },
        );
        let mut handshake_context = self.handshake_context();
        handshake_context.remote_ip = Some(address.ip());
        handshake_context.local_ip = self.setup.public_ip;
        let attempt = ConnectAttempt::new(
            address,
            config,
            connector,
            handshake_context,
            sign_session,
            verify_response,
            self.stop_requested.subscribe(),
        );
        let session_runtime = self.session_runtime.handle().clone();
        Box::pin(async move {
            session_runtime
                .spawn(async move {
                    // Keep this reservation alive while finalization acquires
                    // the active-peer/public-key gate. Dropping it any sooner
                    // reopens the fixed-endpoint duplicate race.
                    let _outbound_attempt = outbound_attempt;
                    let result = attempt.run().await?;
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
        self.setup.peer_limits().max_peers
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
            let _ = self.send_runtime_message(&peer, message.clone(), true);
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
            let _ = self.send_runtime_message(&peer, message.clone(), false);
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

    fn admit_proposal_source(&self, uid: Uint256, validator: PublicKey, peer_id: PeerId) -> bool {
        OverlayImpl::admit_proposal_source(self, uid, validator, peer_id)
    }

    fn admit_validation_source(&self, uid: Uint256, validator: PublicKey, peer_id: PeerId) -> bool {
        OverlayImpl::admit_validation_source(self, uid, validator, peer_id)
    }

    fn peer_is_diverged(&self, peer_id: PeerId) -> bool {
        OverlayImpl::peer_is_diverged(self, peer_id)
    }

    fn suppress_validation(&self, uid: Uint256) {
        OverlayImpl::suppress_validation(self, uid)
    }

    fn sweep_relay_history(&self, _max_entries: u64) {
        // Match HashRouter's aged-container expiry. Its setup has no fixed
        // entry-count admission cap: entries live for holdTime (300 seconds)
        // after their last use, and can relay again after relayTime (30
        // seconds). The argument is retained for the public trait surface.
        let now = Instant::now();
        let mut history = self.relay_history.lock().expect("relay history lock");
        let before = history.len();
        history.retain(|_, entry| {
            entry
                .last_touched
                .is_some_and(|touched| now.duration_since(touched) < RELAY_HISTORY_HOLD_TIME)
        });
        let after = history.len();
        if before != after {
            tracing::debug!(
                target: "overlay",
                before, after,
                freed = before.saturating_sub(after),
                "relay_history sweep (HashRouter hold-time expiry)"
            );
        }
    }
}

async fn shutdown_inbound_tls<S>(stream: &mut S, mut stop_requested: watch::Receiver<bool>)
where
    S: tokio::io::AsyncWrite + Unpin,
{
    if *stop_requested.borrow() {
        return;
    }
    tokio::select! {
        biased;
        changed = stop_requested.changed() => {
            let _ = changed;
        }
        _ = timeout(INBOUND_TLS_SHUTDOWN_TIMEOUT, stream.shutdown()) => {}
    }
}

async fn read_http_request<S>(
    stream: &mut S,
    mut stop_requested: watch::Receiver<bool>,
) -> Result<Option<(Request<()>, Vec<u8>)>, OverlayError>
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
        if let Some(header_end) = buffer
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map(|index| index + 4)
        {
            let read_ahead = buffer.split_off(header_end);
            let request = parse_http_request(&buffer).map_err(OverlayError::InvalidRequest)?;
            return Ok(Some((request, read_ahead)));
        }
    }
}

#[cfg(test)]
mod tests;
