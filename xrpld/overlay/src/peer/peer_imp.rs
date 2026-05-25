//! First concrete peer owner aligned with the current `PeerImp` role.

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use basics::base_uint::Uint256;
use protocol::{JsonValue, PublicKey};
use resource::Charge;
use tokio::sync::{mpsc, watch};

use crate::message::{Message, ProtocolMessage, ProtocolPayload, TmHaveTransactions};
use crate::peer::{Peer, PeerId, ProtocolFeature};
use crate::protocol_version::ProtocolVersion;
use crate::slot::{MAX_TX_QUEUE_SIZE, SystemClock};
use crate::squelch::Squelch;
use crate::tuning::{CONVERGED_LEDGER_LIMIT, DIVERGED_LEDGER_LIMIT};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tracking {
    Unknown,
    Converged,
    Diverged,
}

impl Tracking {
    fn as_u8(self) -> u8 {
        match self {
            Self::Unknown => 0,
            Self::Converged => 1,
            Self::Diverged => 2,
        }
    }

    fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::Converged,
            2 => Self::Diverged,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug)]
pub struct PeerImp {
    id: PeerId,
    remote_address: SocketAddr,
    inbound: bool,
    fixed: AtomicBool,
    node_public: PublicKey,
    fingerprint: String,
    clustered: AtomicBool,
    reserved: AtomicBool,
    high_latency: AtomicBool,
    /// Exponential moving average of ping RTT in milliseconds.
    latency_ms: AtomicU32,
    /// Exponential moving average of useful bytes per request.
    /// Used for peer scoring: peers that return more useful data per
    /// request are preferred over peers that return mostly duplicates.
    useful_bytes_per_request: AtomicU32,
    /// Total requests sent to this peer (for averaging).
    total_requests: AtomicU32,
    compression_enabled: AtomicBool,
    tx_reduce_relay_enabled: AtomicBool,
    publisher_list_sequences: Mutex<HashMap<PublicKey, usize>>,
    outbound_state: Mutex<PeerOutboundState>,
    tx_queue: Mutex<HashSet<Uint256>>,
    known_ledgers: Mutex<HashSet<(Uint256, u32)>>,
    known_tx_sets: Mutex<HashSet<Uint256>>,
    features: RwLock<HashSet<ProtocolFeature>>,
    protocol_version: RwLock<ProtocolVersion>,
    last_status: Mutex<Option<i32>>,
    closed_ledger_hash: Mutex<Uint256>,
    previous_ledger_hash: Mutex<Uint256>,
    min_ledger: Mutex<u32>,
    max_ledger: Mutex<u32>,
    tracking: AtomicU8,
    endpoint_accept_after: Mutex<Option<Instant>>,
    recent_endpoints: Mutex<HashMap<SocketAddr, RecentEndpoint>>,
    listener_check: Mutex<ListenerCheckState>,
    charges: Mutex<Vec<(Charge, String)>>,
    squelch: Mutex<Squelch>,
}

#[derive(Debug, Clone, Copy)]
struct RecentEndpoint {
    hops: u32,
    last_seen: Instant,
}

#[derive(Debug, Clone, Copy)]
struct ListenerCheckState {
    checked: bool,
    can_accept: bool,
    in_progress: bool,
}

#[derive(Default)]
struct PeerOutboundState {
    queued_messages: Vec<Message>,
    session_tx: Option<mpsc::UnboundedSender<Message>>,
    session_stop: Option<watch::Sender<bool>>,
}

impl std::fmt::Debug for PeerOutboundState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PeerOutboundState")
            .field("queued_messages", &self.queued_messages.len())
            .field("session_attached", &self.session_tx.is_some())
            .finish()
    }
}

impl PeerImp {
    pub fn new(
        id: PeerId,
        remote_address: SocketAddr,
        node_public: PublicKey,
        fingerprint: impl Into<String>,
    ) -> Arc<Self> {
        Self::new_with_inbound(id, remote_address, false, node_public, fingerprint)
    }

    pub fn new_with_inbound(
        id: PeerId,
        remote_address: SocketAddr,
        inbound: bool,
        node_public: PublicKey,
        fingerprint: impl Into<String>,
    ) -> Arc<Self> {
        Arc::new(Self {
            id,
            remote_address,
            inbound,
            fixed: AtomicBool::new(false),
            node_public,
            fingerprint: fingerprint.into(),
            clustered: AtomicBool::new(false),
            reserved: AtomicBool::new(false),
            high_latency: AtomicBool::new(false),
            latency_ms: AtomicU32::new(0),
            useful_bytes_per_request: AtomicU32::new(0),
            total_requests: AtomicU32::new(0),
            compression_enabled: AtomicBool::new(false),
            tx_reduce_relay_enabled: AtomicBool::new(false),
            publisher_list_sequences: Mutex::new(HashMap::new()),
            outbound_state: Mutex::new(PeerOutboundState::default()),
            tx_queue: Mutex::new(HashSet::new()),
            known_ledgers: Mutex::new(HashSet::new()),
            known_tx_sets: Mutex::new(HashSet::new()),
            features: RwLock::new(HashSet::new()),
            protocol_version: RwLock::new(ProtocolVersion::new(2, 2)),
            last_status: Mutex::new(None),
            closed_ledger_hash: Mutex::new(Uint256::default()),
            previous_ledger_hash: Mutex::new(Uint256::default()),
            min_ledger: Mutex::new(0),
            max_ledger: Mutex::new(0),
            tracking: AtomicU8::new(Tracking::Unknown.as_u8()),
            endpoint_accept_after: Mutex::new(None),
            recent_endpoints: Mutex::new(HashMap::new()),
            listener_check: Mutex::new(ListenerCheckState {
                checked: true,
                can_accept: true,
                in_progress: false,
            }),
            charges: Mutex::new(Vec::new()),
            squelch: Mutex::new(Squelch::new(Arc::new(SystemClock))),
        })
    }

    pub fn inbound(&self) -> bool {
        self.inbound
    }

    pub fn set_fixed(&self, fixed: bool) {
        self.fixed.store(fixed, Ordering::Relaxed);
    }

    pub fn fixed(&self) -> bool {
        self.fixed.load(Ordering::Relaxed)
    }

    pub fn set_clustered(&self, clustered: bool) {
        self.clustered.store(clustered, Ordering::Relaxed);
    }

    pub fn set_reserved(&self, reserved: bool) {
        self.reserved.store(reserved, Ordering::Relaxed);
    }

    pub fn set_high_latency(&self, high_latency: bool) {
        self.high_latency.store(high_latency, Ordering::Relaxed);
    }

    /// Update latency from ping/pong RTT using reference exponential moving average.
    /// reference: latency_ = latency_ ? (*latency_ * 7 + rtt) / 8 : rtt
    /// Also updates high_latency flag (reference peerHighLatency = 300ms).
    pub fn update_latency(&self, rtt_ms: u32) {
        let current = self.latency_ms.load(Ordering::Relaxed);
        let new_latency = if current > 0 {
            (current * 7 + rtt_ms) / 8
        } else {
            rtt_ms
        };
        self.latency_ms.store(new_latency, Ordering::Relaxed);
        self.high_latency
            .store(new_latency >= 300, Ordering::Relaxed);
    }

    /// Get current latency in ms, 0 means unknown.
    pub fn latency_ms(&self) -> u32 {
        self.latency_ms.load(Ordering::Relaxed)
    }

    /// Record useful bytes received from a request to this peer.
    /// Uses EMA: useful_bytes = (useful_bytes * 7 + new_bytes) / 8
    pub fn record_useful_bytes(&self, bytes: u32) {
        self.total_requests.fetch_add(1, Ordering::Relaxed);
        let current = self.useful_bytes_per_request.load(Ordering::Relaxed);
        let new_avg = if current > 0 {
            (current * 7 + bytes) / 8
        } else {
            bytes
        };
        self.useful_bytes_per_request
            .store(new_avg, Ordering::Relaxed);
    }

    /// Get the useful-bytes-per-request score for this peer.
    pub fn useful_bytes_score(&self) -> u32 {
        self.useful_bytes_per_request.load(Ordering::Relaxed)
    }

    pub fn set_compression_enabled(&self, enabled: bool) {
        self.compression_enabled.store(enabled, Ordering::Relaxed);
    }

    pub fn set_tx_reduce_relay_enabled(&self, enabled: bool) {
        self.tx_reduce_relay_enabled
            .store(enabled, Ordering::Relaxed);
    }

    pub fn set_feature(&self, feature: ProtocolFeature, enabled: bool) {
        let mut features = self.features.write().expect("peer features lock");
        if enabled {
            features.insert(feature);
        } else {
            features.remove(&feature);
        }
    }

    pub fn set_protocol_version(&self, version: ProtocolVersion) {
        *self
            .protocol_version
            .write()
            .expect("peer protocol version lock") = version;
    }

    pub fn record_ledger(&self, hash: Uint256, sequence: u32) {
        self.known_ledgers
            .lock()
            .expect("peer known ledgers lock")
            .insert((hash, sequence));
        *self
            .closed_ledger_hash
            .lock()
            .expect("peer closed ledger lock") = hash;
        let mut min = self.min_ledger.lock().expect("peer min ledger lock");
        let mut max = self.max_ledger.lock().expect("peer max ledger lock");
        if *min == 0 || sequence < *min {
            *min = sequence;
        }
        if sequence > *max {
            *max = sequence;
        }
    }

    pub fn record_tx_set(&self, hash: Uint256) {
        self.known_tx_sets
            .lock()
            .expect("peer known tx sets lock")
            .insert(hash);
    }

    pub fn set_closed_ledger_hash(&self, hash: Uint256) {
        *self
            .closed_ledger_hash
            .lock()
            .expect("peer closed ledger lock") = hash;
    }

    pub fn clear_closed_ledger_hash(&self) {
        self.set_closed_ledger_hash(Uint256::zero());
    }

    pub fn set_previous_ledger_hash(&self, hash: Uint256) {
        *self
            .previous_ledger_hash
            .lock()
            .expect("peer previous ledger lock") = hash;
    }

    pub fn clear_previous_ledger_hash(&self) {
        self.set_previous_ledger_hash(Uint256::zero());
    }

    pub fn previous_ledger_hash(&self) -> Uint256 {
        *self
            .previous_ledger_hash
            .lock()
            .expect("peer previous ledger lock")
    }

    pub fn set_ledger_range(&self, min_sequence: u32, max_sequence: u32) {
        let (min_sequence, max_sequence) =
            if max_sequence < min_sequence || min_sequence == 0 || max_sequence == 0 {
                (0, 0)
            } else {
                (min_sequence, max_sequence)
            };
        *self.min_ledger.lock().expect("peer min ledger lock") = min_sequence;
        *self.max_ledger.lock().expect("peer max ledger lock") = max_sequence;
    }

    pub fn remember_status(&self, incoming_status: Option<i32>) -> Option<i32> {
        let mut last_status = self.last_status.lock().expect("peer last status lock");
        if let Some(status) = incoming_status {
            *last_status = Some(status);
            Some(status)
        } else {
            *last_status
        }
    }

    pub fn queued_messages(&self) -> Vec<Message> {
        self.outbound_state
            .lock()
            .expect("peer queued messages lock")
            .queued_messages
            .clone()
    }

    pub fn clear_queued_messages(&self) {
        self.outbound_state
            .lock()
            .expect("peer queued messages lock")
            .queued_messages
            .clear();
    }

    pub fn attach_session(
        &self,
        session_tx: mpsc::UnboundedSender<Message>,
        session_stop: watch::Sender<bool>,
    ) -> Vec<Message> {
        let mut outbound_state = self.outbound_state.lock().expect("peer outbound lock");
        outbound_state.session_tx = Some(session_tx);
        outbound_state.session_stop = Some(session_stop);
        std::mem::take(&mut outbound_state.queued_messages)
    }

    pub fn detach_session(&self) {
        let mut outbound_state = self.outbound_state.lock().expect("peer outbound lock");
        if let Some(session_stop) = outbound_state.session_stop.take() {
            let _ = session_stop.send(true);
        }
        outbound_state.session_tx = None;
    }

    pub fn has_dead_session_channel(&self) -> bool {
        self.outbound_state
            .lock()
            .expect("peer outbound lock")
            .session_tx
            .as_ref()
            .is_some_and(|sender| sender.is_closed())
    }

    pub fn clear_tx_queue(&self) {
        self.tx_queue.lock().expect("peer tx queue lock").clear();
    }

    pub fn apply_squelch(&self, validator: PublicKey, duration: Duration) -> bool {
        self.squelch
            .lock()
            .expect("peer squelch lock")
            .add_squelch(validator, duration)
    }

    pub fn remove_squelch(&self, validator: PublicKey) {
        self.squelch
            .lock()
            .expect("peer squelch lock")
            .remove_squelch(validator);
    }

    pub fn is_squelched(&self, validator: PublicKey) -> bool {
        self.squelch
            .lock()
            .expect("peer squelch lock")
            .is_squelched(validator)
    }

    pub fn build_tx_queue_message(&self) -> Option<Message> {
        let mut tx_queue = self.tx_queue.lock().expect("peer tx queue lock");
        if tx_queue.is_empty() {
            return None;
        }
        let hashes = tx_queue
            .iter()
            .map(|hash| hash.data().to_vec())
            .collect::<Vec<_>>();
        tx_queue.clear();
        Some(Message::new(
            ProtocolMessage::new(ProtocolPayload::HaveTransactions(TmHaveTransactions {
                hashes,
            })),
            None,
        ))
    }

    pub fn reserved(&self) -> bool {
        self.reserved.load(Ordering::Relaxed)
    }

    pub fn tracking(&self) -> Tracking {
        Tracking::from_u8(self.tracking.load(Ordering::Relaxed))
    }

    pub fn check_tracking(&self, validation_seq: u32) {
        let server_seq = *self.max_ledger.lock().expect("peer max ledger lock");
        if server_seq != 0 {
            self.check_tracking_pair(server_seq, validation_seq);
        }
    }

    pub fn check_tracking_pair(&self, seq1: u32, seq2: u32) {
        let diff = seq1.abs_diff(seq2) as usize;

        if diff < CONVERGED_LEDGER_LIMIT {
            self.tracking
                .store(Tracking::Converged.as_u8(), Ordering::Relaxed);
        }

        if diff > DIVERGED_LEDGER_LIMIT && self.tracking() != Tracking::Diverged {
            self.tracking
                .store(Tracking::Diverged.as_u8(), Ordering::Relaxed);
        }
    }

    pub fn begin_endpoint_accept_window(&self, now: Instant, interval: Duration) -> bool {
        let mut accept_after = self
            .endpoint_accept_after
            .lock()
            .expect("peer endpoint accept window lock");
        if accept_after.is_some_and(|deadline| deadline > now) {
            return false;
        }
        *accept_after = Some(now + interval);
        true
    }

    fn expire_recent_endpoints_locked(
        recent_endpoints: &mut HashMap<SocketAddr, RecentEndpoint>,
        now: Instant,
        ttl: Duration,
    ) {
        recent_endpoints
            .retain(|_, endpoint| now.saturating_duration_since(endpoint.last_seen) <= ttl);
    }

    pub fn set_listener_check_state(&self, checked: bool, can_accept: bool) {
        *self
            .listener_check
            .lock()
            .expect("peer listener check lock") = ListenerCheckState {
            checked,
            can_accept,
            in_progress: false,
        };
    }

    pub fn listener_checked(&self) -> bool {
        self.listener_check
            .lock()
            .expect("peer listener check lock")
            .checked
    }

    pub fn listener_can_accept(&self) -> bool {
        self.listener_check
            .lock()
            .expect("peer listener check lock")
            .can_accept
    }

    pub fn begin_listener_check(&self) -> bool {
        let mut listener_check = self
            .listener_check
            .lock()
            .expect("peer listener check lock");
        if listener_check.in_progress || listener_check.checked {
            return false;
        }
        listener_check.in_progress = true;
        true
    }

    pub fn finish_listener_check(&self, can_accept: bool) {
        let mut listener_check = self
            .listener_check
            .lock()
            .expect("peer listener check lock");
        listener_check.checked = true;
        listener_check.can_accept = can_accept;
        listener_check.in_progress = false;
    }
}

impl Peer for PeerImp {
    fn send(&self, message: Message) {
        let session_tx = {
            let mut outbound_state = self.outbound_state.lock().expect("peer outbound lock");
            if outbound_state.session_tx.is_none() {
                outbound_state.queued_messages.push(message.clone());
            }
            outbound_state.session_tx.clone()
        };
        if let Some(session_tx) = session_tx {
            let _ = session_tx.send(message);
        }
    }

    fn remote_address(&self) -> SocketAddr {
        self.remote_address
    }

    fn send_tx_queue(&self) {
        if let Some(message) = self.build_tx_queue_message() {
            self.send(message);
        }
    }

    fn add_tx_queue(&self, hash: Uint256) {
        if self.tx_queue.lock().expect("peer tx queue lock").len() >= MAX_TX_QUEUE_SIZE {
            self.send_tx_queue();
        }
        self.tx_queue
            .lock()
            .expect("peer tx queue lock")
            .insert(hash);
    }

    fn remove_tx_queue(&self, hash: Uint256) {
        self.tx_queue
            .lock()
            .expect("peer tx queue lock")
            .remove(&hash);
    }

    fn charge(&self, fee: Charge, context: String) {
        self.charges
            .lock()
            .expect("peer charges lock")
            .push((fee, context));
    }

    fn id(&self) -> PeerId {
        self.id
    }

    fn cluster(&self) -> bool {
        self.clustered.load(Ordering::Relaxed)
    }

    fn is_high_latency(&self) -> bool {
        self.high_latency.load(Ordering::Relaxed)
    }

    fn score(&self, have_item: bool) -> i32 {
        // Exact reference PeerImp::getScore constants
        const SP_RANDOM_MAX: i32 = 9_999;
        const SP_HAVE_ITEM: i32 = 10_000;
        const SP_LATENCY: i32 = 30; // per millisecond
        const SP_NO_LATENCY: i32 = 8_000;

        let mut score = basics::random::rand_int_to(SP_RANDOM_MAX as u32) as i32;
        if have_item {
            score += SP_HAVE_ITEM;
        }
        let latency = self.latency_ms.load(Ordering::Relaxed);
        if latency > 0 {
            score -= (latency as i32) * SP_LATENCY;
        } else {
            score -= SP_NO_LATENCY;
        }
        score
    }

    fn node_public(&self) -> PublicKey {
        self.node_public
    }

    fn json(&self) -> JsonValue {
        JsonValue::Object(std::collections::BTreeMap::from([
            ("id".to_owned(), JsonValue::Unsigned(self.id as u64)),
            (
                "public_key".to_owned(),
                JsonValue::String(self.node_public.to_node_public_base58()),
            ),
            (
                "address".to_owned(),
                JsonValue::String(self.remote_address.to_string()),
            ),
            (
                "latency".to_owned(),
                JsonValue::Unsigned(self.latency_ms.load(Ordering::Relaxed) as u64),
            ),
            (
                "version".to_owned(),
                JsonValue::String(format!(
                    "{}",
                    self.protocol_version
                        .read()
                        .expect("peer protocol version lock")
                )),
            ),
            ("inbound".to_owned(), JsonValue::Bool(self.inbound)),
        ]))
    }

    fn supports_feature(&self, feature: ProtocolFeature) -> bool {
        match feature {
            ProtocolFeature::ValidatorListPropagation => {
                *self
                    .protocol_version
                    .read()
                    .expect("peer protocol version lock")
                    >= ProtocolVersion::new(2, 1)
            }
            ProtocolFeature::ValidatorList2Propagation => {
                *self
                    .protocol_version
                    .read()
                    .expect("peer protocol version lock")
                    >= ProtocolVersion::new(2, 2)
            }
            ProtocolFeature::LedgerReplay => self
                .features
                .read()
                .expect("peer features lock")
                .contains(&feature),
        }
    }

    fn publisher_list_sequence(&self, publisher: PublicKey) -> Option<usize> {
        self.publisher_list_sequences
            .lock()
            .expect("peer publisher sequence lock")
            .get(&publisher)
            .copied()
    }

    fn set_publisher_list_sequence(&self, publisher: PublicKey, sequence: usize) {
        self.publisher_list_sequences
            .lock()
            .expect("peer publisher sequence lock")
            .insert(publisher, sequence);
    }

    fn fingerprint(&self) -> String {
        self.fingerprint.clone()
    }

    fn closed_ledger_hash(&self) -> Uint256 {
        *self
            .closed_ledger_hash
            .lock()
            .expect("peer closed ledger lock")
    }

    fn previous_ledger_hash(&self) -> Uint256 {
        *self
            .previous_ledger_hash
            .lock()
            .expect("peer previous ledger lock")
    }

    fn has_ledger(&self, hash: Uint256, sequence: u32) -> bool {
        if sequence != 0 {
            let (min, max) = self.ledger_range();
            if min != 0
                && sequence >= min
                && sequence <= max
                && self.tracking() == Tracking::Converged
            {
                return true;
            }
        }

        self.known_ledgers
            .lock()
            .expect("peer known ledgers lock")
            .iter()
            .any(|(known_hash, _)| *known_hash == hash)
    }

    fn ledger_range(&self) -> (u32, u32) {
        (
            *self.min_ledger.lock().expect("peer min ledger lock"),
            *self.max_ledger.lock().expect("peer max ledger lock"),
        )
    }

    fn has_tx_set(&self, hash: Uint256) -> bool {
        self.known_tx_sets
            .lock()
            .expect("peer known tx sets lock")
            .contains(&hash)
    }

    fn cycle_status(&self) {
        let closed = *self
            .closed_ledger_hash
            .lock()
            .expect("peer closed ledger lock");
        *self
            .previous_ledger_hash
            .lock()
            .expect("peer previous ledger lock") = closed;
        *self
            .closed_ledger_hash
            .lock()
            .expect("peer closed ledger lock") = Uint256::zero();
    }

    fn has_range(&self, min_sequence: u32, max_sequence: u32) -> bool {
        let (min, max) = self.ledger_range();
        min != 0
            && self.tracking() != Tracking::Diverged
            && min <= min_sequence
            && max >= max_sequence
    }

    fn compression_enabled(&self) -> bool {
        self.compression_enabled.load(Ordering::Relaxed)
    }

    fn tx_reduce_relay_enabled(&self) -> bool {
        self.tx_reduce_relay_enabled.load(Ordering::Relaxed)
    }

    fn features(&self) -> HashSet<ProtocolFeature> {
        self.features.read().expect("peer features lock").clone()
    }

    fn should_filter_recent_endpoint(
        &self,
        endpoint: SocketAddr,
        hops: u32,
        now: Instant,
        ttl: Duration,
    ) -> bool {
        let mut recent_endpoints = self
            .recent_endpoints
            .lock()
            .expect("peer recent endpoints lock");
        Self::expire_recent_endpoints_locked(&mut recent_endpoints, now, ttl);
        recent_endpoints
            .get(&endpoint)
            .is_some_and(|recent| recent.hops <= hops)
    }

    fn remember_recent_endpoint(
        &self,
        endpoint: SocketAddr,
        hops: u32,
        now: Instant,
        ttl: Duration,
    ) {
        let mut recent_endpoints = self
            .recent_endpoints
            .lock()
            .expect("peer recent endpoints lock");
        Self::expire_recent_endpoints_locked(&mut recent_endpoints, now, ttl);
        recent_endpoints
            .entry(endpoint)
            .and_modify(|recent| {
                if hops <= recent.hops {
                    recent.hops = hops;
                }
                recent.last_seen = now;
            })
            .or_insert(RecentEndpoint {
                hops,
                last_seen: now,
            });
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::time::{Duration, Instant};

    use basics::base_uint::Uint256;
    use protocol::{KeyType, SecretKey, derive_public_key};

    use super::{PeerImp, Tracking};
    use crate::peer::{Peer, ProtocolFeature};
    use crate::protocol_version::ProtocolVersion;

    #[test]
    fn tx_queue_builds_have_transactions_message_and_clears_queue() {
        let secret = SecretKey::from_bytes([3u8; 32]);
        let public = derive_public_key(KeyType::Secp256k1, &secret).expect("public key");
        let peer = PeerImp::new(
            7,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 51235),
            public,
            "peer-7",
        );

        peer.add_tx_queue(Uint256::from_u64(1));
        peer.add_tx_queue(Uint256::from_u64(2));
        let message = peer.build_tx_queue_message().expect("queue message");
        assert_eq!(
            message.protocol().message_type as i32,
            crate::ProtocolMessageType::MtHaveTransactions as i32
        );
        assert!(peer.build_tx_queue_message().is_none());
    }

    #[test]
    fn peer_tracks_runtime_flags_and_squelch_state() {
        let secret = SecretKey::from_bytes([8u8; 32]);
        let public = derive_public_key(KeyType::Secp256k1, &secret).expect("public key");
        let peer = PeerImp::new(
            9,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 51235),
            public,
            "peer-9",
        );

        assert!(!peer.cluster());
        peer.set_clustered(true);
        peer.set_reserved(true);
        peer.set_tx_reduce_relay_enabled(true);
        assert!(peer.cluster());
        assert!(peer.reserved());
        assert!(peer.tx_reduce_relay_enabled());

        assert!(peer.apply_squelch(public, Duration::from_secs(300)));
        assert!(peer.is_squelched(public));
        peer.remove_squelch(public);
        assert!(!peer.is_squelched(public));
    }

    #[test]
    fn peer_tracking_changes_has_range_when_divergence_changes() {
        let secret = SecretKey::from_bytes([9u8; 32]);
        let public = derive_public_key(KeyType::Secp256k1, &secret).expect("public key");
        let peer = PeerImp::new(
            11,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 51236),
            public,
            "peer-11",
        );

        let hash = Uint256::from_u64(55);
        peer.record_ledger(hash, 200);
        assert!(peer.has_range(200, 200));

        peer.check_tracking(400);
        assert_eq!(peer.tracking(), Tracking::Diverged);
        assert!(!peer.has_range(200, 200));

        peer.check_tracking(210);
        assert_eq!(peer.tracking(), Tracking::Converged);
        assert!(peer.has_range(200, 200));
    }

    #[test]
    fn has_ledger_recent_hash_and_converged_range_behavior() {
        let secret = SecretKey::from_bytes([11u8; 32]);
        let public = derive_public_key(KeyType::Secp256k1, &secret).expect("public key");
        let peer = PeerImp::new(
            13,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 51238),
            public,
            "peer-13",
        );

        let hash = Uint256::from_u64(77);
        peer.record_ledger(hash, 300);
        peer.check_tracking(300);

        assert!(peer.has_ledger(Uint256::zero(), 300));
        assert!(peer.has_ledger(hash, 0));

        peer.check_tracking(600);
        assert_eq!(peer.tracking(), Tracking::Diverged);
        assert!(!peer.has_ledger(Uint256::zero(), 300));
        assert!(peer.has_ledger(hash, 0));
    }

    #[test]
    fn peer_supports_validator_list_features_from_protocol_version() {
        let secret = SecretKey::from_bytes([10u8; 32]);
        let public = derive_public_key(KeyType::Secp256k1, &secret).expect("public key");
        let peer = PeerImp::new(
            12,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 51237),
            public,
            "peer-12",
        );

        peer.set_protocol_version(ProtocolVersion::new(2, 0));
        assert!(!peer.supports_feature(ProtocolFeature::ValidatorListPropagation));
        assert!(!peer.supports_feature(ProtocolFeature::ValidatorList2Propagation));

        peer.set_protocol_version(ProtocolVersion::new(2, 1));
        assert!(peer.supports_feature(ProtocolFeature::ValidatorListPropagation));
        assert!(!peer.supports_feature(ProtocolFeature::ValidatorList2Propagation));

        peer.set_protocol_version(ProtocolVersion::new(2, 2));
        assert!(peer.supports_feature(ProtocolFeature::ValidatorListPropagation));
        assert!(peer.supports_feature(ProtocolFeature::ValidatorList2Propagation));
    }

    #[test]
    fn remember_status_preserves_previous_status_for_event_only_messages() {
        let secret = SecretKey::from_bytes([12u8; 32]);
        let public = derive_public_key(KeyType::Secp256k1, &secret).expect("public key");
        let peer = PeerImp::new(
            14,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 51239),
            public,
            "peer-14",
        );

        assert_eq!(peer.remember_status(None), None);
        assert_eq!(peer.remember_status(Some(2)), Some(2));
        assert_eq!(peer.remember_status(None), Some(2));
        assert_eq!(peer.remember_status(Some(4)), Some(4));
        assert_eq!(peer.remember_status(None), Some(4));
    }

    #[test]
    fn recent_endpoint_filter_same_or_lower_hops_behavior() {
        let secret = SecretKey::from_bytes([13u8; 32]);
        let public = derive_public_key(KeyType::Secp256k1, &secret).expect("public key");
        let peer = PeerImp::new(
            15,
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 51240),
            public,
            "peer-15",
        );

        let endpoint = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), 51235);
        let now = Instant::now();
        peer.remember_recent_endpoint(endpoint, 2, now, Duration::from_secs(30));

        assert!(peer.should_filter_recent_endpoint(
            endpoint,
            2,
            now + Duration::from_secs(1),
            Duration::from_secs(30)
        ));
        assert!(peer.should_filter_recent_endpoint(
            endpoint,
            3,
            now + Duration::from_secs(1),
            Duration::from_secs(30)
        ));
        assert!(!peer.should_filter_recent_endpoint(
            endpoint,
            1,
            now + Duration::from_secs(1),
            Duration::from_secs(30)
        ));
        assert!(!peer.should_filter_recent_endpoint(
            endpoint,
            2,
            now + Duration::from_secs(31),
            Duration::from_secs(30)
        ));
    }
}
